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
    invalidated: AtomicBool,
    auto_initialize: AtomicBool,
    lease_until: AtomicU64,
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
            invalidated: AtomicBool::new(!open),
            auto_initialize: AtomicBool::new(true),
            lease_until: AtomicU64::new(u64::MAX),
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
    fn close_atomic(&self) {
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
        self.close_atomic();
        if let Ok(mut state) = self.state.try_lock() {
            state.error = Some(reason.into());
        }
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
    let s = lifecycle();
    let deadline = s.lease_until.load(Ordering::SeqCst);
    if s.authorized.load(Ordering::SeqCst)
        && monotonic_ms() > deadline
        && s.lease_until
            .compare_exchange(deadline, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        s.close_atomic();
    }
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
    if lifecycle().epoch.load(Ordering::SeqCst) == epoch {
        let mut s = lifecycle().state.lock().unwrap();
        s.shortcuts = "restart_required";
        s.error = Some(error.into());
    }
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
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            if s.maintenance.load(Ordering::SeqCst) || marker_active(&app) {
                s.maintenance.store(true, Ordering::SeqCst);
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
            if !recover {
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
            s.invalidated.store(false, Ordering::SeqCst);
            if s.epoch.load(Ordering::SeqCst) != epoch || s.maintenance.load(Ordering::SeqCst) {
                return Err("stale_permission_initialization".into());
            }
            s.authorized_epoch.store(epoch, Ordering::SeqCst);
            s.authorized.store(true, Ordering::SeqCst);
            {
                let mut st = s.state.lock().unwrap();
                st.enigo = "initializing";
                st.shortcuts = "initializing";
            }
            crate::input::initialize_enigo(&app, epoch)?;
            crate::shortcut::init_shortcuts(&app)?;
            crate::shortcut::health_ready(&app, epoch)?;
            let mut st = s.state.lock().unwrap();
            if s.epoch.load(Ordering::SeqCst) != epoch || !s.authorized.load(Ordering::SeqCst) {
                return Err("stale_permission_initialization".into());
            }
            st.enigo = "ready";
            st.shortcuts = "ready";
            st.targets = "ready";
            st.error = None;
            s.ready_epoch.store(epoch, Ordering::SeqCst);
            s.gate.store(true, Ordering::SeqCst);
            if s.epoch.load(Ordering::SeqCst) != epoch || s.maintenance.load(Ordering::SeqCst) {
                s.gate.store(false, Ordering::SeqCst);
                return Err("stale_permission_initialization".into());
            }
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
    lifecycle().maintenance.store(true, Ordering::SeqCst);
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
        if active && !lifecycle().maintenance.swap(true, Ordering::SeqCst) {
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
