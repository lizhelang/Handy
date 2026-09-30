use super::{
    native::{Configuration, NativePeer, NativePlan},
    protocol::{Binding, MAX_FRAME},
    session::{self, NoTrace},
    transport::{self, Channel},
    GuardedWriterLease, GuardianError, GuardianStatus, GuardianTransactionAuthority,
};
use crate::{
    native_code::{CodeRole, NativeCodeVerifier, VerifiedCodeEvidence},
    native_quiescence, MaintenanceMarker,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CStr, CString},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt, net::UnixStream},
    },
    path::Path,
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};
const MODE: &str = "--inputia-internal-suspension-guardian";
const SOCKET_FD: RawFd = 198;
const LOCK_FD: RawFd = 199;
const MAX_HOLD_MS: u64 = 30_000;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bootstrap {
    binding: Binding,
    configuration: Configuration,
    marker: MaintenanceMarker,
}

pub(super) fn begin(
    authority: GuardianTransactionAuthority,
    updater: VerifiedCodeEvidence,
    roles: &[VerifiedCodeEvidence],
) -> Result<GuardedWriterLease, GuardianError> {
    if updater.expectation().role != CodeRole::Updater
        || updater.expectation().subject != authority.subject
    {
        return Err(GuardianError::InvalidAuthority);
    }
    let expected: Vec<_> = roles
        .iter()
        .map(|role| role.expectation().clone())
        .collect();
    let request = native_quiescence::validate_request(
        &authority.subject,
        &authority.marker,
        &expected,
        "suspend",
    )?;
    native_quiescence::actual_marker(&authority.subject, &authority.marker)?;
    verify_lock(&authority.lock)?;
    for role in roles.iter().chain(std::iter::once(&updater)) {
        NativeCodeVerifier
            .verify(role.expectation(), role.tree())
            .map_err(|_| GuardianError::InvalidAuthority)?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let state = Arc::new(Mutex::new(GuardianStatus::Starting));
    let cancelled = Arc::new(AtomicBool::new(false));
    let (send, receive) = std::sync::mpsc::sync_channel(8);
    let configuration = Configuration {
        writers: request,
        updater: updater.expectation().clone(),
    };
    let binding = Binding {
        lease_id: id.clone(),
        subject: authority.subject,
        epoch: authority.marker.epoch.clone(),
        deadline_mono_ms: session::monotonic_ms().saturating_add(MAX_HOLD_MS),
    };
    let thread_state = state.clone();
    let thread_cancelled = cancelled.clone();
    std::thread::Builder::new()
        .name("inputia-writer-guardian".into())
        .spawn(move || {
            let result = (|| -> Result<(), GuardianError> {
                let effects = NativePlan::prepare(&configuration)?;
                let (mut parent, child) = UnixStream::pair()?;
                let pid = spawn(
                    &effects.executable,
                    child.as_raw_fd(),
                    authority.lock.as_raw_fd(),
                    &binding.lease_id,
                )?;
                drop(child);
                let peer = NativePeer::open(pid, &configuration.updater)?;
                write_bootstrap(
                    &mut parent,
                    &Bootstrap {
                        binding: binding.clone(),
                        configuration,
                        marker: authority.marker.clone(),
                    },
                )?;
                let channel = Channel::new(parent, binding.clone())?;
                let subject = binding.subject.clone();
                session::parent_loop(
                    channel,
                    binding,
                    effects,
                    peer,
                    session::ParentControl {
                        state: thread_state.clone(),
                        cancelled: thread_cancelled.clone(),
                        requests: receive,
                        marker_valid: Box::new(|| {
                            super::native::marker_valid(&subject, &authority.marker)
                        }),
                    },
                    NoTrace,
                )?;
                // 仅wait自己的child；它已封STOP/恢复或被精确确认死亡，不向PID发终止信号。
                let mut status = 0;
                unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                Ok(())
            })();
            if let Err(error) = result {
                session::set_status(&thread_state, GuardianStatus::RecoveryRequired);
                eprintln!("Inputia guardian requires recovery: {error}");
            }
            drop(authority.lock); // close引用，不LOCK_UN；guardian仍持同一描述符直到真实终态。
        })?;
    Ok(GuardedWriterLease {
        id,
        state,
        cancelled,
        requests: send,
    })
}
fn verify_lock(lock: &File) -> Result<(), GuardianError> {
    let user = inputia_settings::maintenance::current_user_context()
        .map_err(|_| GuardianError::InvalidAuthority)?;
    let path = user
        .home
        .join("Library/Application Support/Inputia/Updater/update.lock");
    let (parent, leaf) = crate::filesystem::parent(&path, user.uid, false)
        .map_err(|_| GuardianError::InvalidAuthority)?;
    let actual = crate::filesystem::open_child(&parent, &leaf, false)
        .map_err(|_| GuardianError::InvalidAuthority)?;
    let held = lock.metadata()?;
    let current = actual.metadata()?;
    if !held.is_file()
        || held.uid() != user.uid
        || held.mode() & 0o7777 != 0o600
        || held.nlink() != 1
        || held.dev() != current.dev()
        || held.ino() != current.ino()
    {
        return Err(GuardianError::InvalidAuthority);
    }
    // 同一open-file-description重申锁不会解锁；另open得到的FD会被原事务锁拒绝。
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(GuardianError::InvalidAuthority);
    }
    Ok(())
}
pub(super) fn entry() -> Result<Option<i32>, GuardianError> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|arg| arg != MODE) {
        return Ok(None);
    }
    if args.len() != 3 {
        return Err(GuardianError::InvalidAuthority);
    }
    let nonce = args[2].to_str().ok_or(GuardianError::InvalidAuthority)?;
    if !inputia_settings::installation::valid_uuid(nonce) {
        return Err(GuardianError::InvalidAuthority);
    }
    // 保留FD只由固定posix_spawn file actions移交；随后exec不能继续继承。
    transport::set_cloexec(SOCKET_FD)?;
    transport::set_cloexec(LOCK_FD)?;
    let mut socket = unsafe { UnixStream::from_raw_fd(SOCKET_FD) };
    let lock = unsafe { File::from_raw_fd(LOCK_FD) };
    verify_lock(&lock)?;
    let bootstrap = read_bootstrap(&mut socket)?;
    if bootstrap.binding.lease_id != nonce
        || bootstrap.binding.subject != bootstrap.configuration.updater.subject
        || bootstrap.binding.subject != bootstrap.configuration.writers.subject
        || bootstrap.binding.epoch != bootstrap.marker.epoch
        || bootstrap.binding.deadline_mono_ms <= session::monotonic_ms()
        || bootstrap.binding.deadline_mono_ms > session::monotonic_ms().saturating_add(MAX_HOLD_MS)
    {
        return Err(GuardianError::InvalidAuthority);
    }
    native_quiescence::actual_marker(&bootstrap.binding.subject, &bootstrap.marker)?;
    let peer = NativePeer::open(unsafe { libc::getppid() }, &bootstrap.configuration.updater)?;
    let effects = NativePlan::prepare(&bootstrap.configuration)?;
    let channel = Channel::new(socket, bootstrap.binding.clone())?;
    session::guardian_loop(
        channel,
        bootstrap.binding.clone(),
        effects,
        peer,
        || super::native::marker_valid(&bootstrap.binding.subject, &bootstrap.marker),
        NoTrace,
    )?;
    drop(lock);
    Ok(Some(0))
}
fn write_bootstrap(socket: &mut UnixStream, bootstrap: &Bootstrap) -> Result<(), GuardianError> {
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    no_sigpipe(socket.as_raw_fd())?;
    let bytes = serde_json::to_vec(bootstrap).map_err(|_| GuardianError::InvalidAuthority)?;
    if bytes.len() > MAX_FRAME {
        return Err(GuardianError::InvalidAuthority);
    }
    socket.write_all(&(bytes.len() as u32).to_be_bytes())?;
    socket.write_all(&bytes)?;
    socket.set_write_timeout(None)?;
    Ok(())
}
fn read_bootstrap(socket: &mut UnixStream) -> Result<Bootstrap, GuardianError> {
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    no_sigpipe(socket.as_raw_fd())?;
    let mut prefix = [0; 4];
    socket.read_exact(&mut prefix)?;
    let count = u32::from_be_bytes(prefix) as usize;
    if count == 0 || count > MAX_FRAME {
        return Err(GuardianError::InvalidAuthority);
    }
    let mut bytes = vec![0; count];
    socket.read_exact(&mut bytes)?;
    socket.set_read_timeout(None)?;
    serde_json::from_slice(&bytes).map_err(|_| GuardianError::InvalidAuthority)
}
fn no_sigpipe(fd: RawFd) -> std::io::Result<()> {
    let yes: libc::c_int = 1;
    if unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&yes as *const libc::c_int).cast(),
            std::mem::size_of_val(&yes) as _,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
fn spawn(executable: &Path, socket: RawFd, lock: RawFd, nonce: &str) -> Result<i32, GuardianError> {
    let executable = CString::new(executable.as_os_str().as_bytes())
        .map_err(|_| GuardianError::InvalidAuthority)?;
    let mode = CString::new(MODE).map_err(|_| GuardianError::InvalidAuthority)?;
    let nonce = CString::new(nonce).map_err(|_| GuardianError::InvalidAuthority)?;
    spawn_raw(
        &executable,
        socket,
        lock,
        &[executable.clone(), mode, nonce],
        &[],
    )
}

#[cfg(test)]
pub(super) fn spawn_fixture(
    executable: &Path,
    socket: RawFd,
    lock: RawFd,
    config: &str,
) -> Result<i32, GuardianError> {
    let executable = CString::new(executable.as_os_str().as_bytes())
        .map_err(|_| GuardianError::InvalidAuthority)?;
    let args = [
        executable.clone(),
        CString::new("--exact").unwrap(),
        CString::new("guardian::tests::fixture_entry").unwrap(),
        CString::new("--nocapture").unwrap(),
    ];
    let env = [
        CString::new(format!("INPUTIA_GUARDIAN_TEST_CONFIG={config}")).unwrap(),
        CString::new("INPUTIA_GUARDIAN_TEST_ROLE=guardian").unwrap(),
    ];
    spawn_raw(&executable, socket, lock, &args, &env)
}

fn spawn_raw(
    executable: &CStr,
    socket: RawFd,
    lock: RawFd,
    args: &[CString],
    environment: &[CString],
) -> Result<i32, GuardianError> {
    let mut argv: Vec<_> = args
        .iter()
        .map(|v| v.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    let mut env: Vec<_> = environment
        .iter()
        .map(|v| v.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    // 防止原FD恰为198/199时dup2动作彼此覆盖。仅本轮临时副本，仍指向同一OFD。
    let inherited_socket = duplicate_above_reserved(socket)?;
    let inherited_lock = duplicate_above_reserved(lock)?;
    let mut actions = std::mem::MaybeUninit::<libc::posix_spawn_file_actions_t>::uninit();
    let mut attributes = std::mem::MaybeUninit::<libc::posix_spawnattr_t>::uninit();
    let init = unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) };
    if init != 0 {
        return Err(std::io::Error::from_raw_os_error(init).into());
    }
    let mut actions = unsafe { actions.assume_init() };
    let result = (|| -> Result<i32, GuardianError> {
        let init = unsafe { libc::posix_spawnattr_init(attributes.as_mut_ptr()) };
        if init != 0 {
            return Err(std::io::Error::from_raw_os_error(init).into());
        }
        let mut attributes = unsafe { attributes.assume_init() };
        let run = (|| -> Result<i32, GuardianError> {
            for (from, to) in [
                (inherited_socket.as_raw_fd(), SOCKET_FD),
                (inherited_lock.as_raw_fd(), LOCK_FD),
            ] {
                let code =
                    unsafe { libc::posix_spawn_file_actions_adddup2(&mut actions, from, to) };
                if code != 0 {
                    return Err(std::io::Error::from_raw_os_error(code).into());
                }
            }
            let code = unsafe {
                libc::posix_spawnattr_setflags(
                    &mut attributes,
                    libc::POSIX_SPAWN_CLOEXEC_DEFAULT as _,
                )
            };
            if code != 0 {
                return Err(std::io::Error::from_raw_os_error(code).into());
            }
            let mut pid = 0;
            let code = unsafe {
                libc::posix_spawn(
                    &mut pid,
                    executable.as_ptr(),
                    &actions,
                    &attributes,
                    argv.as_mut_ptr(),
                    env.as_mut_ptr(),
                )
            };
            if code != 0 {
                return Err(std::io::Error::from_raw_os_error(code).into());
            }
            Ok(pid)
        })();
        unsafe { libc::posix_spawnattr_destroy(&mut attributes) };
        run
    })();
    unsafe { libc::posix_spawn_file_actions_destroy(&mut actions) };
    result
}

fn duplicate_above_reserved(fd: RawFd) -> std::io::Result<OwnedFd> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, LOCK_FD + 1) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}
