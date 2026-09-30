use super::{
    protocol::{Binding, Entry, InitialState, Ledger, Resolution},
    recovery::{Effects, ExecutionState, Executor, Peer, PeerState},
    session::{self, Point, Trace},
    transport::{self, Channel, ProcessWatch},
    GuardianError, GuardianStatus,
};
use crate::{native_quiescence::WriterProcessIdentity, Subject};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::OpenOptionsExt, net::UnixStream},
    },
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[repr(C)]
#[derive(Clone, Copy)]
struct AuditToken {
    words: [u32; 8],
}
unsafe extern "C" {
    static mach_task_self_: u32;
    fn task_name_for_pid(task: u32, pid: i32, port: *mut u32) -> i32;
    fn mach_port_deallocate(task: u32, port: u32) -> i32;
}
fn bsd(pid: i32) -> std::io::Result<libc::proc_bsdinfo> {
    let mut result = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let count = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            result.as_mut_ptr().cast(),
            std::mem::size_of::<libc::proc_bsdinfo>() as _,
        )
    };
    if count != std::mem::size_of::<libc::proc_bsdinfo>() as i32 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { result.assume_init() })
}
#[derive(Clone, Copy)]
struct Kernel {
    pid: i32,
    seconds: u64,
    micros: u64,
    audit: AuditToken,
}
impl Kernel {
    fn capture(pid: i32) -> std::io::Result<Self> {
        let before = bsd(pid)?;
        let mut port = 0;
        let task = unsafe { mach_task_self_ };
        if unsafe { task_name_for_pid(task, pid, &mut port) } != 0 {
            return Err(std::io::Error::other("fixture audit unavailable"));
        }
        let mut audit = AuditToken { words: [0; 8] };
        let mut count = 8;
        let result =
            unsafe { libc::task_info(port, 15, audit.words.as_mut_ptr().cast(), &mut count) };
        unsafe { mach_port_deallocate(task, port) };
        if result != 0 || count != 8 || audit.words[5] != pid as u32 || audit.words[7] == 0 {
            return Err(std::io::Error::other("fixture audit identity"));
        }
        let after = bsd(pid)?;
        if before.pbi_start_tvsec != after.pbi_start_tvsec
            || before.pbi_start_tvusec != after.pbi_start_tvusec
        {
            return Err(std::io::Error::other("fixture instance changed"));
        }
        Ok(Self {
            pid,
            seconds: before.pbi_start_tvsec,
            micros: before.pbi_start_tvusec,
            audit,
        })
    }
    fn state(self) -> Result<ExecutionState, GuardianError> {
        let current = match bsd(self.pid) {
            Ok(v) => v,
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {
                return Ok(ExecutionState::OriginalExited)
            }
            Err(e) => return Err(e.into()),
        };
        if current.pbi_status == 5
            || current.pbi_start_tvsec != self.seconds
            || current.pbi_start_tvusec != self.micros
        {
            return Ok(ExecutionState::OriginalExited);
        }
        let now = Self::capture(self.pid)?;
        if now.audit.words[7] != self.audit.words[7] {
            return Ok(ExecutionState::OriginalExited);
        }
        Ok(if current.pbi_status == 4 {
            ExecutionState::Stopped
        } else {
            ExecutionState::Running
        })
    }
    fn signal(self, signal: i32) -> Result<(), GuardianError> {
        let current = Self::capture(self.pid)?;
        if current.seconds != self.seconds
            || current.micros != self.micros
            || current.audit.words != self.audit.words
        {
            return Err(GuardianError::InvalidAuthority);
        }
        let pointer =
            unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
        if pointer.is_null() {
            return Err(GuardianError::NativeUnavailable);
        }
        let effect: unsafe extern "C" fn(*mut AuditToken, i32) -> i32 =
            unsafe { std::mem::transmute(pointer) };
        let mut audit = self.audit;
        if unsafe { effect(&mut audit, signal) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn wait(self, expected: ExecutionState) -> Result<(), GuardianError> {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if self.state()? == expected {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(GuardianError::RecoveryPending)
    }
}
struct FixtureEffects {
    kernels: Vec<Kernel>,
    entries: Vec<Entry>,
    closed: bool,
    stops: usize,
    cancel_after_authorize: Option<Arc<AtomicBool>>,
}
impl FixtureEffects {
    fn from_pids(pids: &[i32]) -> Result<Self, GuardianError> {
        let mut kernels: Vec<_> = pids
            .iter()
            .map(|p| Kernel::capture(*p))
            .collect::<Result<_, _>>()?;
        kernels.sort_by_key(|k| k.pid);
        let entries = kernels
            .iter()
            .enumerate()
            .map(|(index, kernel)| {
                Ok(Entry {
                    index: index as u32,
                    initial_state: if kernel.state()? == ExecutionState::Stopped {
                        InitialState::ObservedStopped
                    } else {
                        InitialState::Running
                    },
                    identity: WriterProcessIdentity {
                        pid: kernel.pid,
                        uid: unsafe { libc::geteuid() },
                        start_seconds: kernel.seconds,
                        start_microseconds: kernel.micros,
                        pid_version: kernel.audit.words[7],
                        role: "control".into(),
                        bundle_id: "com.inputia.fixture".into(),
                        release_id: "inputia-old".into(),
                        executable_path: "/fixture/Inputia.app/Contents/MacOS/Inputia".into(),
                        cdhash: "b".repeat(40),
                    },
                })
            })
            .collect::<Result<_, GuardianError>>()?;
        Ok(Self {
            kernels,
            entries,
            closed: false,
            stops: 0,
            cancel_after_authorize: None,
        })
    }
}
impl Effects for FixtureEffects {
    fn entries(&self) -> &[Entry] {
        &self.entries
    }
    fn stop(
        &mut self,
        index: u32,
        authorize: &mut dyn FnMut() -> bool,
    ) -> Result<(), GuardianError> {
        if self.closed || !authorize() {
            return Err(GuardianError::Cancelled);
        }
        if let Some(cancel) = &self.cancel_after_authorize {
            cancel.store(true, Ordering::Release);
        }
        self.kernels[index as usize].signal(libc::SIGSTOP)?;
        self.stops += 1;
        self.kernels[index as usize].wait(ExecutionState::Stopped)
    }
    fn close_stop(&mut self) -> Result<(), GuardianError> {
        self.closed = true;
        Ok(())
    }
    fn resume(&mut self, index: u32) -> Result<Resolution, GuardianError> {
        if !self.closed
            || self.entries[index as usize].initial_state == InitialState::ObservedStopped
        {
            return Err(GuardianError::InvalidAuthority);
        }
        let kernel = self.kernels[index as usize];
        match kernel.state()? {
            ExecutionState::OriginalExited => Ok(Resolution::OriginalExited),
            ExecutionState::Running => Ok(Resolution::Running),
            ExecutionState::Stopped => {
                kernel.signal(libc::SIGCONT)?;
                kernel.wait(ExecutionState::Running)?;
                Ok(Resolution::Resumed)
            }
        }
    }
    fn state(&self, index: u32) -> Result<ExecutionState, GuardianError> {
        self.kernels[index as usize].state()
    }
    fn assert_holding(&self) -> Result<(), GuardianError> {
        if self.closed
            || self
                .kernels
                .iter()
                .any(|k| !matches!(k.state(), Ok(ExecutionState::Stopped)))
        {
            return Err(GuardianError::RecoveryPending);
        }
        Ok(())
    }
}
struct FixturePeer {
    kernel: Kernel,
    watch: ProcessWatch,
}
impl FixturePeer {
    fn new(pid: i32) -> Result<Self, GuardianError> {
        let kernel = Kernel::capture(pid)?;
        let watch = ProcessWatch::new(pid)?;
        if kernel.state()? == ExecutionState::OriginalExited {
            return Err(GuardianError::InvalidAuthority);
        }
        Ok(Self { kernel, watch })
    }
}
impl Peer for FixturePeer {
    fn state(&self) -> Result<PeerState, GuardianError> {
        let _ = self.watch.events()?;
        Ok(if self.kernel.state()? == ExecutionState::OriginalExited {
            PeerState::Exited
        } else {
            PeerState::Alive
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Fixture {
    directory: std::path::PathBuf,
    targets: Vec<i32>,
    binding: Binding,
    crash_role: String,
    crash_point: String,
    abort: bool,
    #[serde(default)]
    hold_recovery: bool,
}
struct FaultTrace {
    fixture: Fixture,
    role: &'static str,
    cancel: Arc<AtomicBool>,
}
impl Trace for FaultTrace {
    fn at(&mut self, point: Point) {
        if self.role == "guardian" && point == Point::BeforeCont && self.fixture.hold_recovery {
            std::fs::write(self.fixture.directory.join("recovery_waiting"), "armed").unwrap();
            let deadline = Instant::now() + Duration::from_secs(6);
            while !self.fixture.directory.join("allow_recovery").exists()
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if self.role == self.fixture.crash_role && format!("{point:?}") == self.fixture.crash_point
        {
            std::fs::write(
                self.fixture.directory.join("injected"),
                format!("{}:{point:?}", self.role),
            )
            .unwrap();
            if self.fixture.abort {
                std::process::abort();
            }
            unsafe { libc::raise(libc::SIGKILL) };
        }
        if self.role == "updater" && point == Point::Holding {
            self.cancel.store(true, Ordering::Release);
        }
    }
}
fn binding() -> Binding {
    Binding {
        lease_id: uuid::Uuid::new_v4().to_string(),
        epoch: uuid::Uuid::new_v4().to_string(),
        deadline_mono_ms: session::monotonic_ms() + 20_000,
        subject: Subject {
            transaction_id: uuid::Uuid::new_v4().to_string(),
            installation_id: uuid::Uuid::new_v4().to_string(),
            new_release_id: "inputia-new".into(),
            plan_sha256: "a".repeat(64),
        },
    }
}
struct Target {
    child: Child,
    kernel: Kernel,
}
impl Target {
    fn new(stopped: bool) -> Self {
        let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let kernel = Kernel::capture(child.id() as i32).unwrap();
        if stopped {
            kernel.signal(libc::SIGSTOP).unwrap();
            kernel.wait(ExecutionState::Stopped).unwrap();
        }
        Self { child, kernel }
    }
}
impl Drop for Target {
    fn drop(&mut self) {
        let _ = self.kernel.signal(libc::SIGCONT);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn fixture_entry() {
    let Ok(path) = std::env::var("INPUTIA_GUARDIAN_TEST_CONFIG") else {
        return;
    };
    // 仅此cfg(test)入口读取夹具PID；生产入口没有该环境变量/任意PID工厂。
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let role = std::env::var("INPUTIA_GUARDIAN_TEST_ROLE").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    if role == "guardian" {
        transport::set_cloexec(198).unwrap();
        transport::set_cloexec(199).unwrap();
        let socket = unsafe { UnixStream::from_raw_fd(198) };
        let lock = unsafe { File::from_raw_fd(199) };
        let peer = FixturePeer::new(unsafe { libc::getppid() }).unwrap();
        let effects = FixtureEffects::from_pids(&fixture.targets).unwrap();
        let trace = FaultTrace {
            fixture: fixture.clone(),
            role: "guardian",
            cancel,
        };
        session::guardian_loop(
            Channel::new(socket, fixture.binding.clone()).unwrap(),
            fixture.binding.clone(),
            effects,
            peer,
            || true,
            trace,
        )
        .unwrap();
        drop(lock);
        std::fs::write(fixture.directory.join("guardian_done"), "resumed").unwrap();
    } else {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(fixture.directory.join("update.lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        let effects = FixtureEffects::from_pids(&fixture.targets).unwrap();
        let (parent, child) = UnixStream::pair().unwrap();
        let pid = super::runtime::spawn_fixture(
            &std::env::current_exe().unwrap(),
            child.as_raw_fd(),
            lock.as_raw_fd(),
            &path,
        )
        .unwrap();
        drop(child);
        let peer = FixturePeer::new(pid).unwrap();
        let (_, requests) = std::sync::mpsc::sync_channel(8);
        let state = Arc::new(Mutex::new(GuardianStatus::Starting));
        let trace = FaultTrace {
            fixture: fixture.clone(),
            role: "updater",
            cancel: cancel.clone(),
        };
        session::parent_loop(
            Channel::new(parent, fixture.binding.clone()).unwrap(),
            fixture.binding.clone(),
            effects,
            peer,
            session::ParentControl {
                state: state.clone(),
                cancelled: cancel,
                requests,
                marker_valid: Box::new(|| true),
            },
            trace,
        )
        .unwrap();
        assert_eq!(*state.lock().unwrap(), GuardianStatus::Resumed);
        std::fs::write(fixture.directory.join("updater_done"), "resumed").unwrap();
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        drop(lock);
    }
}
#[test]
fn real_single_process_crash_windows_preserve_original_stops() {
    let cases = [
        ("updater", "Ready", false),
        ("updater", "ArmSaved", false),
        ("updater", "ArmAck", true),
        ("guardian", "Ready", false),
        ("guardian", "ArmSent", false),
        ("guardian", "ArmAck", false),
        ("guardian", "BeforeStop", false),
        ("guardian", "AfterStop", true),
        ("guardian", "Holding", false),
        ("guardian", "BeforeCont", false),
        ("guardian", "AfterCont", false),
        ("guardian", "BeforeTerminal", false),
        ("guardian", "Terminal", false),
        ("updater", "Disarmed", false),
    ];
    for (role, point, abort) in cases {
        let directory = tempfile::tempdir().unwrap();
        let running = Target::new(false);
        let stopped = Target::new(true);
        let fixture = Fixture {
            directory: directory.path().into(),
            targets: vec![running.child.id() as i32, stopped.child.id() as i32],
            binding: binding(),
            crash_role: role.into(),
            crash_point: point.into(),
            abort,
            hold_recovery: false,
        };
        let config = directory.path().join("fixture.json");
        std::fs::write(&config, serde_json::to_vec(&fixture).unwrap()).unwrap();
        let mut updater = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "guardian::tests::fixture_entry", "--nocapture"])
            .env("INPUTIA_GUARDIAN_TEST_CONFIG", &config)
            .env("INPUTIA_GUARDIAN_TEST_ROLE", "updater")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        let survivor = directory.path().join(if role == "updater" {
            "guardian_done"
        } else {
            "updater_done"
        });
        while !survivor.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let passed = survivor.exists() && directory.path().join("injected").exists();
        let _ = updater.kill();
        let _ = updater.wait();
        assert!(passed, "{role}:{point}");
        assert_eq!(
            running.kernel.state().unwrap(),
            ExecutionState::Running,
            "{role}:{point}"
        );
        assert_eq!(
            stopped.kernel.state().unwrap(),
            ExecutionState::Stopped,
            "{role}:{point}"
        );
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.path().join("update.lock"))
            .unwrap();
        let lock_deadline = Instant::now() + Duration::from_secs(1);
        while unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
            && Instant::now() < lock_deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "terminal releases last flock ref"
        );
    }
}
#[test]
fn cancellation_after_authorize_cannot_land_a_stop_after_recovery() {
    let target = Target::new(false);
    let b = binding();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut effects = FixtureEffects::from_pids(&[target.child.id() as i32]).unwrap();
    effects.cancel_after_authorize = Some(cancel.clone());
    let entry = effects.entries()[0].clone();
    let mut ledger = Ledger::new(b.clone(), vec![entry.clone()]).unwrap();
    ledger.arm(0, &entry.digest(&b).unwrap()).unwrap();
    let mut executor = Executor { ledger, effects };
    assert!(executor.stop(0, &cancel, &mut || true).is_err());
    executor.recover_once().unwrap();
    assert_eq!(target.kernel.state().unwrap(), ExecutionState::Running);
    assert_eq!(executor.effects.stops, 1);
    assert!(executor
        .stop(0, &AtomicBool::new(false), &mut || true)
        .is_err());
    assert_eq!(executor.effects.stops, 1);
}
#[test]
fn live_owner_timeout_is_not_takeover_and_pid_version_change_is_rejected() {
    let target = Target::new(false);
    let peer = Target::new(false);
    let b = binding();
    let effects = FixtureEffects::from_pids(&[target.child.id() as i32]).unwrap();
    let entry = effects.entries()[0].clone();
    let mut ledger = Ledger::new(b.clone(), vec![entry.clone()]).unwrap();
    ledger.arm(0, &entry.digest(&b).unwrap()).unwrap();
    let mut executor = Executor { ledger, effects };
    executor
        .stop(0, &AtomicBool::new(false), &mut || true)
        .unwrap();
    assert!(matches!(
        executor.take_over(&FixturePeer::new(peer.child.id() as i32).unwrap()),
        Err(GuardianError::PeerStillAlive)
    ));
    assert_eq!(target.kernel.state().unwrap(), ExecutionState::Stopped);
    let mut wrong = target.kernel;
    wrong.audit.words[7] += 1;
    assert!(wrong.signal(libc::SIGCONT).is_err());
    assert_eq!(target.kernel.state().unwrap(), ExecutionState::Stopped);
    executor.recover_once().unwrap();
}

#[test]
fn inherited_transaction_lock_remains_until_surviving_guardian_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let target = Target::new(false);
    let fixture = Fixture {
        directory: directory.path().into(),
        targets: vec![target.child.id() as i32],
        binding: binding(),
        crash_role: "updater".into(),
        crash_point: "Holding".into(),
        abort: false,
        hold_recovery: true,
    };
    let config = directory.path().join("fixture.json");
    std::fs::write(&config, serde_json::to_vec(&fixture).unwrap()).unwrap();
    let mut updater = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "guardian::tests::fixture_entry", "--nocapture"])
        .env("INPUTIA_GUARDIAN_TEST_CONFIG", &config)
        .env("INPUTIA_GUARDIAN_TEST_ROLE", "updater")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.path().join("recovery_waiting").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(directory.path().join("recovery_waiting").exists());
    assert!(!updater.wait().unwrap().success());
    assert_eq!(target.kernel.state().unwrap(), ExecutionState::Stopped);
    let contender = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("update.lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        -1,
        "guardian keeps same OFD locked after updater death"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EWOULDBLOCK)
    );
    std::fs::write(directory.path().join("allow_recovery"), "continue").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.path().join("guardian_done").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(directory.path().join("guardian_done").exists());
    assert_eq!(target.kernel.state().unwrap(), ExecutionState::Running);
    assert_eq!(
        unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}

struct ControlledPeer(Arc<AtomicBool>);
impl Peer for ControlledPeer {
    fn state(&self) -> Result<PeerState, GuardianError> {
        Ok(if self.0.load(Ordering::Acquire) {
            PeerState::Alive
        } else {
            PeerState::Exited
        })
    }
}
struct SlowRead {
    marker: Arc<AtomicBool>,
    revoke: bool,
}
impl Effects for SlowRead {
    fn entries(&self) -> &[Entry] {
        &[]
    }
    fn stop(&mut self, _: u32, _: &mut dyn FnMut() -> bool) -> Result<(), GuardianError> {
        unreachable!()
    }
    fn close_stop(&mut self) -> Result<(), GuardianError> {
        Ok(())
    }
    fn resume(&mut self, _: u32) -> Result<Resolution, GuardianError> {
        unreachable!()
    }
    fn state(&self, _: u32) -> Result<ExecutionState, GuardianError> {
        unreachable!()
    }
    fn assert_holding(&self) -> Result<(), GuardianError> {
        // 确定地让真实调用窗口跨过deadline/授权撤销，模拟阻塞的验签/内核扫描。
        std::thread::sleep(Duration::from_millis(80));
        if self.revoke {
            self.marker.store(false, Ordering::Release);
        }
        Ok(())
    }
}
struct CountHolding(Arc<std::sync::atomic::AtomicUsize>);
impl Trace for CountHolding {
    fn at(&mut self, point: Point) {
        if point == Point::Holding {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}
#[test]
fn guardian_rechecks_deadline_and_marker_after_blocking_native_assert() {
    use super::protocol::Message;
    for revoke in [false, true] {
        let mut binding = binding();
        if !revoke {
            binding.deadline_mono_ms = session::monotonic_ms() + 30;
        }
        let marker = Arc::new(AtomicBool::new(true));
        let thread_marker = marker.clone();
        let (parent, child) = UnixStream::pair().unwrap();
        let child_binding = binding.clone();
        let child = std::thread::spawn(move || {
            session::guardian_loop(
                Channel::new(child, child_binding.clone()).unwrap(),
                child_binding,
                SlowRead {
                    marker: thread_marker.clone(),
                    revoke,
                },
                ControlledPeer(Arc::new(AtomicBool::new(true))),
                || thread_marker.load(Ordering::Acquire),
                session::NoTrace,
            )
            .unwrap();
        });
        let mut parent = Channel::new(parent, binding).unwrap();
        let mut holding = false;
        loop {
            if let Some((message, _)) = parent.receive(Duration::from_secs(1)).unwrap() {
                match message {
                    Message::Holding {} => holding = true,
                    Message::Resumed {} => {
                        parent.send(Message::DisarmAck {}).unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        }
        child.join().unwrap();
        assert!(
            !holding,
            "no expired/revoked success may escape native scan"
        );
    }
}
#[test]
fn parent_rechecks_deadline_and_marker_after_blocking_native_assert() {
    use super::protocol::Message;
    for revoke in [false, true] {
        let mut binding = binding();
        if !revoke {
            binding.deadline_mono_ms = session::monotonic_ms() + 30;
        }
        let marker = Arc::new(AtomicBool::new(true));
        let alive = Arc::new(AtomicBool::new(true));
        let server_alive = alive.clone();
        let (parent, child) = UnixStream::pair().unwrap();
        let child_binding = binding.clone();
        let child = std::thread::spawn(move || {
            let mut child = Channel::new(child, child_binding).unwrap();
            child.send(Message::Ready { entries: vec![] }).unwrap();
            child.send(Message::Holding {}).unwrap();
            std::thread::sleep(Duration::from_millis(160));
            server_alive.store(false, Ordering::Release);
        });
        let (_, requests) = std::sync::mpsc::sync_channel(8);
        let successes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        session::parent_loop(
            Channel::new(parent, binding.clone()).unwrap(),
            binding,
            SlowRead {
                marker: marker.clone(),
                revoke,
            },
            ControlledPeer(alive),
            session::ParentControl {
                state: Arc::new(Mutex::new(GuardianStatus::Starting)),
                cancelled: Arc::new(AtomicBool::new(false)),
                requests,
                marker_valid: Box::new(|| marker.load(Ordering::Acquire)),
            },
            CountHolding(successes.clone()),
        )
        .unwrap();
        child.join().unwrap();
        assert_eq!(
            successes.load(Ordering::SeqCst),
            0,
            "parent must not report invalid Holding"
        );
    }
}
