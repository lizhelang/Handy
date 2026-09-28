//! Passive, fail-closed permission lifecycle. Native checks and destruction never run on the UI thread.
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

struct Lifecycle {
    epoch: AtomicU64,
    gate: AtomicBool,
    ready_epoch: AtomicU64,
    authorized_epoch: AtomicU64,
    authorized: AtomicBool,
    busy: AtomicBool,
    worker_id: AtomicU64,
    worker_transition: Mutex<()>,
    admission_transition: Mutex<()>,
    close_serial: AtomicU64,
    invalidated: AtomicBool,
    auto_initialize: AtomicBool,
    lease_until: AtomicU64,
    lease_recovery_pending: AtomicBool,
    maintenance: AtomicBool,
    state: Mutex<State>,
}
#[derive(Clone)]
struct State {
    accessibility: &'static str,
    input_monitoring: &'static str,
    enigo: &'static str,
    shortcuts: &'static str,
    targets: &'static str,
    error: Option<String>,
}
impl Lifecycle {
    fn new() -> Self {
        // Unit tests exercise coordinator/input logic without TCC; keep them open.
        // Production macOS still starts fail-closed until a verified probe/recovery.
        let open = cfg!(test) || !cfg!(target_os = "macos");
        Self {
            epoch: AtomicU64::new(1),
            gate: AtomicBool::new(open),
            ready_epoch: AtomicU64::new(if open { 1 } else { 0 }),
            authorized_epoch: AtomicU64::new(if open { 1 } else { 0 }),
            authorized: AtomicBool::new(open),
            busy: AtomicBool::new(false),
            worker_id: AtomicU64::new(0),
            worker_transition: Mutex::new(()),
            admission_transition: Mutex::new(()),
            close_serial: AtomicU64::new(0),
            invalidated: AtomicBool::new(!open),
            auto_initialize: AtomicBool::new(true),
            lease_until: AtomicU64::new(u64::MAX),
            lease_recovery_pending: AtomicBool::new(false),
            maintenance: AtomicBool::new(false),
            state: Mutex::new(State {
                accessibility: "unknown",
                input_monitoring: "unknown",
                enigo: "stopped",
                shortcuts: "stopped",
                targets: "stopped",
                error: None,
            }),
        }
    }
    // 仅短状态发布临界区使用；不得在此锁内进行 native 检查或退休。
    fn invalidate_locked(&self) {
        self.close_serial.fetch_add(1, Ordering::SeqCst);
        self.auto_initialize.store(false, Ordering::SeqCst);
        let new_boundary = !self.invalidated.swap(true, Ordering::SeqCst);
        self.gate.store(false, Ordering::SeqCst);
        self.ready_epoch.store(0, Ordering::SeqCst);
        self.authorized_epoch.store(0, Ordering::SeqCst);
        self.authorized.store(false, Ordering::SeqCst);
        if new_boundary {
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn close(&self, reason: &str) {
        let _transition = self.admission_transition.lock().unwrap();
        self.invalidate_locked();
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        if matches!(reason, "maintenance" | "maintenance_marker") {
            self.maintenance.store(true, Ordering::SeqCst);
        }
        log::warn!(
            "input_permission_closed reason={reason} epoch={}",
            self.epoch.load(Ordering::SeqCst)
        );
        if let Ok(mut state) = self.state.try_lock() {
            state.error = Some(reason.into());
        }
    }
    fn fail_shortcuts(&self, worker_epoch: u64, reason: &str) -> bool {
        let _transition = self.admission_transition.lock().unwrap();
        if self.epoch.load(Ordering::SeqCst) != worker_epoch {
            log::debug!("input_permission_stale_shortcut_fault ignored_epoch={worker_epoch}");
            return false;
        }
        self.invalidate_locked();
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        let mut state = self.state.lock().unwrap();
        state.shortcuts = "restart_required";
        state.error = Some(reason.into());
        log::warn!(
            "input_permission_closed reason={reason} worker_epoch={worker_epoch} epoch={}",
            self.epoch.load(Ordering::SeqCst)
        );
        true
    }
    fn expire_lease(&self, now: u64) {
        let _transition = self.admission_transition.lock().unwrap();
        let deadline = self.lease_until.load(Ordering::SeqCst);
        if now <= deadline || !self.authorized.load(Ordering::SeqCst) {
            return;
        }
        self.lease_until.store(0, Ordering::SeqCst);
        self.invalidate_locked();
        self.lease_recovery_pending.store(true, Ordering::SeqCst);
        if self.lease_recovery_pending.load(Ordering::SeqCst) {
            if let Ok(mut state) = self.state.try_lock() {
                state.error = Some("permission_lease_expired".into());
            }
            log::warn!(
                "input_permission_closed reason=permission_lease_expired epoch={}",
                self.epoch.load(Ordering::SeqCst)
            );
        }
    }
    fn lease_recovery_allowed(&self) -> bool {
        self.lease_recovery_pending.load(Ordering::SeqCst)
            && !self.maintenance.load(Ordering::SeqCst)
    }
    fn initialization_current(&self, epoch: u64, serial: u64, explicit: bool) -> bool {
        self.epoch.load(Ordering::SeqCst) == epoch
            && self.close_serial.load(Ordering::SeqCst) == serial
            && !self.maintenance.load(Ordering::SeqCst)
            && (explicit || self.lease_recovery_allowed())
    }
    fn begin_initialization(&self, epoch: u64, serial: u64, explicit: bool) -> Result<(), String> {
        let _transition = self.admission_transition.lock().unwrap();
        if !self.initialization_current(epoch, serial, explicit) {
            return Err("stale_permission_initialization".into());
        }
        self.invalidated.store(false, Ordering::SeqCst);
        self.authorized_epoch.store(epoch, Ordering::SeqCst);
        self.authorized.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn finish_initialization(&self, epoch: u64, serial: u64, explicit: bool) -> Result<(), String> {
        let _transition = self.admission_transition.lock().unwrap();
        if !self.initialization_current(epoch, serial, explicit)
            || !self.authorized.load(Ordering::SeqCst)
        {
            return Err("stale_permission_initialization".into());
        }
        let mut st = self.state.lock().unwrap();
        st.enigo = "ready";
        st.shortcuts = "ready";
        st.targets = "ready";
        st.error = None;
        self.ready_epoch.store(epoch, Ordering::SeqCst);
        self.gate.store(true, Ordering::SeqCst);
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn check(&self, epoch: u64) -> Result<(), String> {
        if self.gate.load(Ordering::SeqCst)
            && self.ready_epoch.load(Ordering::SeqCst) == epoch
            && self.epoch.load(Ordering::SeqCst) == epoch
        {
            Ok(())
        } else {
            Err("input_permission_epoch_invalid".into())
        }
    }
}
fn lifecycle() -> &'static Lifecycle {
    static VALUE: OnceLock<Lifecycle> = OnceLock::new();
    VALUE.get_or_init(Lifecycle::new)
}
fn monotonic_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
fn check_lease() {
    if !cfg!(target_os = "macos") {
        return;
    }
    lifecycle().expire_lease(monotonic_ms());
}
pub fn callback_allowed() -> bool {
    check_lease();
    let s = lifecycle();
    s.authorized.load(Ordering::SeqCst)
        && s.authorized_epoch.load(Ordering::SeqCst) == s.epoch.load(Ordering::SeqCst)
}
pub fn capture_epoch() -> Result<u64, String> {
    let epoch = REQUEST_EPOCH
        .with(|slot| slot.get())
        .unwrap_or_else(|| lifecycle().epoch.load(Ordering::SeqCst));
    check_epoch(epoch)?;
    Ok(epoch)
}
thread_local! { static REQUEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) }; }
/// Keeps a request tied to its admission epoch across clipboard work and sleeps.
pub struct RequestScope(Option<u64>);
impl RequestScope {
    pub fn enter(epoch: u64) -> Result<Self, String> {
        check_epoch(epoch)?;
        Ok(Self(REQUEST_EPOCH.with(|slot| slot.replace(Some(epoch)))))
    }
}
impl Drop for RequestScope {
    fn drop(&mut self) {
        REQUEST_EPOCH.with(|slot| slot.set(self.0));
    }
}
pub fn check_epoch(epoch: u64) -> Result<(), String> {
    check_lease();
    lifecycle().check(epoch)
}
/// A remote IME may cache this passive proof only within the existing native lease.
pub fn proof_valid_for_ms(maximum: u64) -> u64 {
    if capture_epoch().is_err() {
        return 0;
    }
    lifecycle()
        .lease_until
        .load(Ordering::SeqCst)
        .saturating_sub(monotonic_ms())
        .min(maximum)
}
pub fn initializing_epoch() -> Result<u64, String> {
    let s = lifecycle();
    let epoch = s.epoch.load(Ordering::SeqCst);
    if callback_allowed() && !s.maintenance.load(Ordering::SeqCst) {
        Ok(epoch)
    } else {
        Err("input_permission_not_verified".into())
    }
}
pub fn close_gate(reason: &str) {
    lifecycle().close(reason);
}
pub fn mark_shortcuts_ready(epoch: u64) {
    if lifecycle().epoch.load(Ordering::SeqCst) == epoch {
        lifecycle().state.lock().unwrap().shortcuts = "ready";
    }
}
pub fn mark_shortcuts_failed(epoch: u64, error: &str) {
    lifecycle().fail_shortcuts(epoch, error);
}
fn root(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    if let Some(profile) = crate::candidate_profile::current() {
        return profile
            .handy_root
            .parent()
            .map(|p| p.to_path_buf())
            .ok_or("profile_root_missing".into());
    }
    app.path()
        .home_dir()
        .map(|p| p.join("Library/Application Support/Inputia"))
        .map_err(|e| e.to_string())
}
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn read_json(path: &std::path::Path) -> std::io::Result<Value> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "macos")]
        options.custom_flags(0x100); // Darwin O_NOFOLLOW
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20000); // Linux O_NOFOLLOW
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("permission_record_not_regular"));
    }
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes)?;
    if bytes.len() > 16_384 {
        return Err(std::io::Error::other("permission_record_too_large"));
    }
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}
fn write_json(path: &std::path::Path, value: &Value) -> std::io::Result<()> {
    use std::io::Write;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::SeqCst)
    ));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(value.to_string().as_bytes())?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
fn marker(app: &AppHandle) -> Result<Value, std::io::Error> {
    read_json(
        &root(app)
            .map_err(std::io::Error::other)?
            .join("permission-maintenance.json"),
    )
}
fn marker_active(app: &AppHandle) -> bool {
    match marker(app) {
        Ok(value) => value.get("active").and_then(Value::as_bool).unwrap_or(true),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}
fn maintenance_ack_matches(ime: &Value, marker: Option<&Value>) -> bool {
    let expected = marker.and_then(|m| m.get("epoch")).and_then(Value::as_str);
    expected.is_some_and(|epoch| {
        !epoch.is_empty() && ime.get("marker_epoch").and_then(Value::as_str) == Some(epoch)
    })
}
pub fn snapshot(app: &AppHandle) -> Value {
    let s = lifecycle();
    let state = s.state.lock().unwrap().clone();
    let maintenance = s.maintenance.load(Ordering::SeqCst);
    let mut maintenance_state = if !maintenance {
        "inactive"
    } else if state.enigo == "restart_required"
        || state.shortcuts == "restart_required"
        || state.targets == "restart_required"
    {
        "restart_required"
    } else if state.enigo == "stopped"
        && state.shortcuts == "stopped"
        && state.targets == "stopped"
        && !s.busy.load(Ordering::SeqCst)
    {
        "active"
    } else {
        "retiring"
    };
    let mut ime = root(app)
        .ok()
        .and_then(|p| read_json(&p.join("permission-health-ime.json")).ok())
        .unwrap_or(json!({"component":"ime","state":"unknown","stale":true}));
    if ime
        .get("updated_at_ms")
        .and_then(Value::as_u64)
        .map(|t| t as u128 > now_ms() || now_ms().saturating_sub(t as u128) > 5000)
        .unwrap_or(true)
    {
        ime["state"] = json!("unknown");
        ime["stale"] = json!(true);
    }
    #[cfg(target_os = "macos")]
    {
        let valid_pid = ime
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .is_some_and(|pid| {
                let mut buf = [0u8; 4096];
                let len = unsafe {
                    {
                        unsafe extern "C" { fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, size: u32) -> i32; }
                        proc_pidpath(pid, buf.as_mut_ptr() as *mut _, buf.len() as u32)
                    }
                };
                if len <= 0 {
                    return false;
                }
                let path = String::from_utf8_lossy(&buf[..len as usize]);
                let Ok(home) = app.path().home_dir() else { return false; };
                let names = if crate::candidate_profile::current().is_some() {
                    vec![home.join("Library/Input Methods/InputiaUnifiedCandidate.app/Contents/MacOS/InputiaInputMethod")]
                } else {
                    vec![home.join("Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod"),
                        std::path::PathBuf::from("/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod")]
                };
                names.iter().any(|expected| expected.as_os_str() == std::ffi::OsStr::new(path.trim_end_matches('\0')))
            });
        if !valid_pid {
            ime["state"] = json!("unknown");
            ime["stale"] = json!(true);
        }
    }
    if maintenance_state == "active"
        && (!maintenance_ack_matches(&ime, marker(app).ok().as_ref())
            || ime.get("stale").and_then(Value::as_bool).unwrap_or(false)
            || !matches!(
                ime.get("state").and_then(Value::as_str),
                Some("maintenance" | "suspended")
            ))
    {
        maintenance_state = "retiring";
    }
    json!({"epoch":s.epoch.load(Ordering::SeqCst),"gate_open":s.gate.load(Ordering::SeqCst),"permission":{"accessibility":state.accessibility,"input_monitoring":state.input_monitoring},"health":{"enigo":state.enigo,"shortcuts":state.shortcuts,"targets":state.targets},"probe_in_flight":s.busy.load(Ordering::SeqCst),"maintenance":maintenance,"maintenance_state":maintenance_state,"last_error":state.error,"components":{"background":{"component":"background","bundle_id":app.config().identifier,"state":if s.gate.load(Ordering::SeqCst) {"ready"} else {"paused"}},"ime":ime}})
}
#[cfg(target_os = "macos")]
fn probe() -> (bool, bool) {
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightListenEventAccess() -> bool;
    }
    // Both APIs are passive; no prompt-capable API is used here.
    unsafe { (AXIsProcessTrusted() != 0, CGPreflightListenEventAccess()) }
}
#[cfg(not(target_os = "macos"))]
fn probe() -> (bool, bool) {
    (true, true)
}
fn retire(app: &AppHandle) -> Result<(), String> {
    {
        let mut s = lifecycle().state.lock().unwrap();
        s.enigo = "retiring";
        s.shortcuts = "retiring";
        s.targets = "retiring";
    }
    let keys = crate::shortcut::retire_shortcuts(app);
    let input = crate::input::retire_enigo(app);
    #[cfg(target_os = "macos")]
    let targets = crate::ime_target_broker::invalidate_all(app);
    #[cfg(not(target_os = "macos"))]
    let targets: Result<(), String> = Ok(());
    let mut s = lifecycle().state.lock().unwrap();
    s.shortcuts = if keys.is_ok() {
        "stopped"
    } else {
        "restart_required"
    };
    s.enigo = if input.is_ok() {
        "stopped"
    } else {
        "restart_required"
    };
    s.targets = retirement_health(&targets);
    keys.and(input).and(targets)
}
fn retirement_health(result: &Result<(), String>) -> &'static str {
    if result.is_ok() {
        "stopped"
    } else {
        "restart_required"
    }
}
/// Exactly one native lifecycle worker can exist. A timeout closes admission but
/// does not pretend to cancel that worker or free its slot.
fn launch(app: &AppHandle, recover: bool) {
    let s = lifecycle();
    let transition = s.worker_transition.lock().unwrap();
    if s.busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let worker_id = s.worker_id.fetch_add(1, Ordering::SeqCst) + 1;
    drop(transition);
    let app = app.clone();
    let epoch = s.epoch.load(Ordering::SeqCst);
    let close_serial = s.close_serial.load(Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            if s.maintenance.load(Ordering::SeqCst) || marker_active(&app) {
                close_gate("maintenance_marker");
                return retire(&app);
            }
            let (ax, input) = probe();
            if s.epoch.load(Ordering::SeqCst) != epoch {
                return Err("stale_permission_probe".into());
            }
            {
                let mut st = s.state.lock().unwrap();
                st.accessibility = if ax { "granted" } else { "denied" };
                st.input_monitoring = if input { "granted" } else { "denied" };
            }
            if !ax {
                close_gate("permission_required");
                retire(&app)?;
                return Err("permission_required".into());
            }
            s.lease_until.store(monotonic_ms() + 3500, Ordering::SeqCst);
            if s.gate.load(Ordering::SeqCst) {
                crate::shortcut::health_ready(&app, epoch)?;
                return s.check(epoch);
            }
            if !recover && !s.lease_recovery_allowed() {
                // A revoked generation must release its native targets even if AX has since returned.
                return retire(&app);
            }
            retire(&app)?;
            if s.epoch.load(Ordering::SeqCst) != epoch
                || s.maintenance.load(Ordering::SeqCst)
                || marker_active(&app)
            {
                return Err("stale_permission_initialization".into());
            }
            if !recover && !s.lease_recovery_allowed() {
                return Err("lease_recovery_superseded".into());
            }
            if !recover {
                // 租约恢复在旧资源完整退休后重新被动检查，避免沿用退休前的授权。
                let (ax, input) = probe();
                {
                    let mut st = s.state.lock().unwrap();
                    st.accessibility = if ax { "granted" } else { "denied" };
                    st.input_monitoring = if input { "granted" } else { "denied" };
                }
                if !ax {
                    return Err("permission_required".into());
                }
                s.lease_until.store(monotonic_ms() + 3500, Ordering::SeqCst);
            }
            s.begin_initialization(epoch, close_serial, recover)?;
            {
                let mut st = s.state.lock().unwrap();
                st.enigo = "initializing";
                st.shortcuts = "initializing";
            }
            crate::input::initialize_enigo(&app, epoch)?;
            crate::shortcut::init_shortcuts(&app)?;
            crate::shortcut::health_ready(&app, epoch)?;
            if marker_active(&app) {
                return Err("maintenance_marker".into());
            }
            s.finish_initialization(epoch, close_serial, recover)?;
            log::info!(
                "input_permission_ready epoch={epoch} recovery={}",
                if recover {
                    "explicit_or_startup"
                } else {
                    "lease"
                }
            );
            app.manage(crate::commands::ShortcutsInitialized);
            Ok(())
        })();
        if let Err(error) = result {
            close_gate(&error);
            let _ = retire(&app);
        }
        {
            let _transition = s.worker_transition.lock().unwrap();
            let _ = tx.send(());
            s.busy.store(false, Ordering::SeqCst);
        }
        publish_health(&app);
    });
    std::thread::spawn(move || {
        if rx.recv_timeout(Duration::from_millis(1500)).is_ok() {
            return;
        }
        let _transition = s.worker_transition.lock().unwrap();
        if s.busy.load(Ordering::SeqCst) && s.worker_id.load(Ordering::SeqCst) == worker_id {
            close_gate("permission_worker_timeout_restart_required");
            let mut st = s.state.lock().unwrap();
            st.accessibility = "unknown";
            st.input_monitoring = "unknown";
            st.enigo = "restart_required";
            st.shortcuts = "restart_required";
            st.targets = "restart_required";
        }
    });
}
pub fn request_initialize(app: &AppHandle) {
    if lifecycle().auto_initialize.load(Ordering::SeqCst) {
        launch(app, true);
    }
}
pub fn request_recheck(app: &AppHandle) {
    launch(app, false);
}
fn publish_health(app: &AppHandle) {
    let s = lifecycle();
    let st = s.state.lock().unwrap();
    let state = if s.gate.load(Ordering::SeqCst) {
        "ready"
    } else if st.enigo == "restart_required"
        || st.shortcuts == "restart_required"
        || st.targets == "restart_required"
    {
        "restart_required"
    } else if s.maintenance.load(Ordering::SeqCst)
        && st.enigo == "stopped"
        && st.shortcuts == "stopped"
        && st.targets == "stopped"
        && !s.busy.load(Ordering::SeqCst)
    {
        "maintenance"
    } else {
        "retiring"
    };
    let value = json!({"schema_version":1,"component":"background","pid":std::process::id(),"updated_at_ms":now_ms() as u64,"state":state,"maintenance_state":if state=="maintenance" {"active"} else {state},"permission_epoch":s.epoch.load(Ordering::SeqCst)});
    drop(st);
    if let Ok(root) = root(app) {
        let mut value = value;
        value["marker_epoch"] = marker(app)
            .ok()
            .and_then(|v| v.get("epoch").cloned())
            .unwrap_or(Value::Null);
        value["maintenance_marker_epoch"] = value["marker_epoch"].clone();
        if let Err(error) = write_json(&root.join("permission-health-background.json"), &value) {
            log::debug!("permission health write failed: {error}");
        }
    }
}

pub fn resume(app: &AppHandle) -> Result<(), String> {
    if lifecycle().busy.load(Ordering::SeqCst) {
        return Err("permission_worker_busy".into());
    }
    let root = root(app)?;
    let path = root.join("permission-maintenance.json");
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    lifecycle().maintenance.store(false, Ordering::SeqCst);
    lifecycle().auto_initialize.store(true, Ordering::SeqCst);
    launch(app, true);
    Ok(())
}
pub fn prepare_maintenance(app: &AppHandle) -> Result<(), String> {
    close_gate("maintenance");
    let root = root(app)?;
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    static MARKER_SERIAL: AtomicU64 = AtomicU64::new(0);
    let marker = json!({"schema_version":1,"active":true,"epoch":format!("{}-{}-{}",now_ms(),std::process::id(),MARKER_SERIAL.fetch_add(1, Ordering::SeqCst))});
    write_json(&root.join("permission-maintenance.json"), &marker).map_err(|e| e.to_string())?;
    launch(app, false);
    Ok(())
}
pub fn start_monitor(app: &AppHandle) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let watchdog_app = app.clone();
    std::thread::spawn(move || {
        let mut observed_epoch = lifecycle().epoch.load(Ordering::SeqCst);
        loop {
            check_lease();
            let epoch = lifecycle().epoch.load(Ordering::SeqCst);
            if epoch != observed_epoch {
                observed_epoch = epoch;
                if let Some(coordinator) =
                    watchdog_app.try_state::<crate::TranscriptionCoordinator>()
                {
                    coordinator.request_cancel();
                }
                launch(&watchdog_app, false);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    let app = app.clone();
    std::thread::spawn(move || loop {
        let active = marker_active(&app);
        if active && !lifecycle().maintenance.load(Ordering::SeqCst) {
            close_gate("maintenance_marker");
        }
        launch(&app, false);
        publish_health(&app);
        std::thread::sleep(Duration::from_secs(2));
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ready_lifecycle() -> Lifecycle {
        let s = Lifecycle::new();
        s.invalidated.store(false, Ordering::SeqCst);
        s.lease_until.store(100, Ordering::SeqCst);
        s
    }
    #[test]
    fn retired_shortcut_worker_cannot_cancel_lease_recovery_or_close_new_generation() {
        let s = ready_lifecycle();
        let worker_epoch = s.epoch.load(Ordering::SeqCst);
        s.expire_lease(101);
        let recovery_epoch = s.epoch.load(Ordering::SeqCst);
        let serial = s.close_serial.load(Ordering::SeqCst);
        assert!(!s.fail_shortcuts(worker_epoch, "old_listener_exited"));
        assert!(s.lease_recovery_allowed());
        assert_eq!(s.close_serial.load(Ordering::SeqCst), serial);
        s.begin_initialization(recovery_epoch, serial, false)
            .unwrap();
        s.finish_initialization(recovery_epoch, serial, false)
            .unwrap();
        assert!(!s.fail_shortcuts(worker_epoch, "old_listener_exited"));
        assert!(s.check(recovery_epoch).is_ok());
        assert_eq!(s.state.lock().unwrap().shortcuts, "ready");
    }
    #[test]
    fn current_shortcut_worker_fault_closes_admission_and_requires_manual_recovery() {
        let s = ready_lifecycle();
        assert!(s.fail_shortcuts(1, "current_listener_exited"));
        assert!(s.check(1).is_err());
        assert!(!s.authorized.load(Ordering::SeqCst));
        assert!(!s.lease_recovery_allowed());
        assert_eq!(s.state.lock().unwrap().shortcuts, "restart_required");
        assert_eq!(
            s.state.lock().unwrap().error.as_deref(),
            Some("current_listener_exited")
        );
    }
    #[test]
    fn concurrent_close_invalidates_recovery_token_without_needing_new_epoch() {
        let s = ready_lifecycle();
        s.expire_lease(101);
        let epoch = s.epoch.load(Ordering::SeqCst);
        let serial = s.close_serial.load(Ordering::SeqCst);
        assert!(s.lease_recovery_allowed());
        s.close("native_listener_failed");
        assert_eq!(s.epoch.load(Ordering::SeqCst), epoch);
        assert!(s.begin_initialization(epoch, serial, false).is_err());
        assert!(!s.authorized.load(Ordering::SeqCst));
    }
    #[test]
    fn close_racing_final_recovery_publication_always_leaves_gate_closed() {
        for _ in 0..32 {
            let s = std::sync::Arc::new(ready_lifecycle());
            s.expire_lease(101);
            let epoch = s.epoch.load(Ordering::SeqCst);
            let serial = s.close_serial.load(Ordering::SeqCst);
            s.begin_initialization(epoch, serial, false).unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let closing = s.clone();
            let close_barrier = barrier.clone();
            let thread = std::thread::spawn(move || {
                close_barrier.wait();
                closing.close("maintenance");
            });
            barrier.wait();
            let _ = s.finish_initialization(epoch, serial, false);
            thread.join().unwrap();
            assert!(!s.gate.load(Ordering::SeqCst));
            assert!(!s.authorized.load(Ordering::SeqCst));
            assert!(!s.lease_recovery_allowed());
            assert!(s.maintenance.load(Ordering::SeqCst));
        }
    }
    #[test]
    fn lease_recovery_publishes_only_current_verified_generation() {
        let s = ready_lifecycle();
        s.expire_lease(101);
        let epoch = s.epoch.load(Ordering::SeqCst);
        let serial = s.close_serial.load(Ordering::SeqCst);
        s.begin_initialization(epoch, serial, false).unwrap();
        assert!(s.check(epoch).is_err());
        s.finish_initialization(epoch, serial, false).unwrap();
        assert!(s.check(1).is_err());
        assert!(s.check(epoch).is_ok());
        assert!(!s.lease_recovery_allowed());
    }
    #[test]
    fn expired_lease_invalidates_old_epoch_and_only_requests_verified_recovery() {
        let s = ready_lifecycle();
        s.expire_lease(100);
        assert!(s.check(1).is_ok());
        s.expire_lease(101);
        assert_eq!(s.epoch.load(Ordering::SeqCst), 2);
        assert!(s.check(1).is_err());
        assert!(s.check(2).is_err());
        assert!(!s.authorized.load(Ordering::SeqCst));
        assert!(s.lease_recovery_allowed());
        assert_eq!(
            s.state.lock().unwrap().error.as_deref(),
            Some("permission_lease_expired")
        );
        s.expire_lease(102);
        assert_eq!(s.epoch.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn revocation_native_timeout_and_explicit_maintenance_cancel_lease_recovery() {
        for reason in [
            "permission_required",
            "permission_worker_timeout_restart_required",
            "maintenance",
            "native_listener_failed",
        ] {
            let s = ready_lifecycle();
            s.expire_lease(101);
            assert!(s.lease_recovery_allowed());
            s.close(reason);
            assert!(!s.lease_recovery_allowed());
            s.expire_lease(102);
            assert!(!s.lease_recovery_allowed());
            assert!(s.check(1).is_err());
        }
    }
    #[test]
    fn maintenance_blocks_lease_recovery_even_before_marker_worker_runs() {
        let s = ready_lifecycle();
        s.expire_lease(101);
        s.maintenance.store(true, Ordering::SeqCst);
        assert!(!s.lease_recovery_allowed());
        assert!(s.check(2).is_err());
    }
    #[test]
    fn missing_target_retirement_receipt_cannot_acknowledge_maintenance() {
        assert_eq!(
            retirement_health(&Err("target_retirement_timeout_restart_required".into())),
            "restart_required"
        );
        assert_eq!(retirement_health(&Ok(())), "stopped");
    }
    #[test]
    fn revocation_rejects_old_epoch_even_after_recovery() {
        let s = Lifecycle::new();
        s.invalidated.store(false, Ordering::SeqCst);
        s.ready_epoch.store(1, Ordering::SeqCst);
        s.gate.store(true, Ordering::SeqCst);
        let epoch = s.epoch.load(Ordering::SeqCst);
        assert!(s.check(epoch).is_ok());
        s.close("mock_revoked");
        assert!(s.check(epoch).is_err());
        s.ready_epoch
            .store(s.epoch.load(Ordering::SeqCst), Ordering::SeqCst);
        s.gate.store(true, Ordering::SeqCst);
        assert!(s.check(epoch).is_err());
        assert!(s.check(s.epoch.load(Ordering::SeqCst)).is_ok());
    }
    #[test]
    fn timeout_keeps_native_slot_occupied() {
        let s = Lifecycle::new();
        assert!(s
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok());
        s.close("mock_timeout");
        assert!(s
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err());
        assert!(!s.authorized.load(Ordering::SeqCst));
    }
}
