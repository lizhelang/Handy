//! Passive, fail-closed permission lifecycle. Native checks and destruction never run on the UI thread.
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const PROBE_INTERVAL_MS: u64 = 2_000;
const PUBLIC_LEASE_MS: u64 = 3_500;
const INITIALIZATION_LEASE_MS: u64 = 8_000;
const RECOVERY_DELAYS_MS: [u64; 5] = [1_000, 2_000, 4_000, 8_000, 8_000];

/// 只对已知的临时原生故障安排重建；未知错误、配置错误和权限拒绝保持关闭。
fn recovery_reason(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "permission_lease_expired" => "permission_lease_expired",
        "initialization_lease_expired" => "initialization_lease_expired",
        "快捷键原生监听器意外退出" | "快捷键原生监听器不可用" | "快捷键监听器不可用"
        | "快捷键监听器不可用或已过期" => "shortcut_listener_unavailable",
        "快捷键监听器启动超时或已退出" => "shortcut_startup_unconfirmed",
        "Failed to create HotkeyManager: Failed to create event tap: Event tap thread terminated unexpectedly"
        | "Failed to create HotkeyManager: Failed to create event tap: Failed to create run loop source"
        | "Failed to create HotkeyManager: Failed to create event tap: Failed to get current run loop" => "shortcut_native_initialization_failed",
        "快捷键生命周期忙碌" | "快捷键健康检查忙碌" | "快捷键初始化仍在进行"
        | "快捷键线程状态忙碌" | "快捷键录制线程忙碌" => "shortcut_worker_busy",
        "快捷键健康检查通道掉线" | "快捷键注册通道掉线" | "快捷键注销通道掉线" => {
            "shortcut_channel_unavailable"
        }
        "快捷键注册回执超时或通道掉线，结果未知" | "快捷键注销回执超时或通道掉线，结果未知"
        | "快捷键操作回执未知" => "shortcut_receipt_unconfirmed",
        "Carbon 注册或注销回执未知" => "carbon_receipt_unconfirmed",
        "fallback 主线程回执超时，清理结果未知" | "fallback 主线程回执丢失，清理结果未知" => {
            "fallback_retirement_unconfirmed"
        }
        "原生键盘线程仍在退出" | "旧原生键盘线程尚未退出"
        | "旧快捷键监听器尚未退出，不能创建第二套" | "旧录制监听器尚未退出"
        | "旧快捷键录制线程尚未退出"
        | "旧 Carbon shadow 清理未确认，禁止注册新的 primary" => "shortcut_retirement_pending",
        "input_device_busy" | "input_device_busy_restart_required" => "input_device_busy",
        "Failed to initialize Enigo: no connection could be established: (failed creating event source)" => {
            "input_event_source_unavailable"
        }
        "target_retirement_pending" | "target_retirement_unavailable"
        | "target_retirement_timeout_restart_required" => "target_retirement_unconfirmed",
        _ => return None,
    })
}

fn health_reason(reason: Option<&str>) -> Option<&'static str> {
    reason.map(|reason| {
        recovery_reason(reason).unwrap_or(match reason {
            "permission_required" => "permission_required",
            "maintenance" | "maintenance_marker" => "maintenance",
            "shortcut_settings_retirement_unconfirmed" => "settings_retirement_unconfirmed",
            "recovery_exhausted" => "recovery_exhausted",
            "stale_permission_probe"
            | "stale_permission_initialization"
            | "lease_recovery_superseded"
            | "stale_input_initialization" => "superseded",
            _ => "unclassified_error",
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkerTicket {
    id: u64,
    epoch: u64,
    close_serial: u64,
}

thread_local! {
    static WORKER_TICKET: std::cell::Cell<Option<WorkerTicket>> = const { std::cell::Cell::new(None) };
}

struct WorkerScope(Option<WorkerTicket>);
impl WorkerScope {
    fn enter(ticket: WorkerTicket) -> Self {
        Self(WORKER_TICKET.with(|current| current.replace(Some(ticket))))
    }
}
impl Drop for WorkerScope {
    fn drop(&mut self) {
        WORKER_TICKET.with(|current| current.set(self.0));
    }
}

struct Lifecycle {
    epoch: AtomicU64,
    gate: AtomicBool,
    ready_epoch: AtomicU64,
    authorized_epoch: AtomicU64,
    authorized: AtomicBool,
    busy: AtomicBool,
    worker_id: AtomicU64,
    worker_epoch: AtomicU64,
    worker_serial: AtomicU64,
    worker_started: AtomicU64,
    worker_phase_started: AtomicU64,
    worker_phase: Mutex<&'static str>,
    worker_slow: AtomicBool,
    worker_hard_timeout: AtomicBool,
    next_probe_after: AtomicU64,
    worker_transition: Mutex<()>,
    admission_transition: Mutex<()>,
    close_serial: AtomicU64,
    invalidated: AtomicBool,
    auto_initialize: AtomicBool,
    lease_until: AtomicU64,
    initialization_until: AtomicU64,
    lease_recovery_pending: AtomicBool,
    explicit_recovery_pending: AtomicBool,
    recovery_attempts: AtomicU64,
    recovery_after: AtomicU64,
    retired_epoch: AtomicU64,
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
            worker_epoch: AtomicU64::new(0),
            worker_serial: AtomicU64::new(0),
            worker_started: AtomicU64::new(0),
            worker_phase_started: AtomicU64::new(0),
            worker_phase: Mutex::new("idle"),
            worker_slow: AtomicBool::new(false),
            worker_hard_timeout: AtomicBool::new(false),
            next_probe_after: AtomicU64::new(0),
            worker_transition: Mutex::new(()),
            admission_transition: Mutex::new(()),
            close_serial: AtomicU64::new(0),
            invalidated: AtomicBool::new(!open),
            auto_initialize: AtomicBool::new(true),
            lease_until: AtomicU64::new(u64::MAX),
            initialization_until: AtomicU64::new(0),
            lease_recovery_pending: AtomicBool::new(false),
            explicit_recovery_pending: AtomicBool::new(false),
            recovery_attempts: AtomicU64::new(0),
            recovery_after: AtomicU64::new(0),
            retired_epoch: AtomicU64::new(0),
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
        pause_native_blocking();
        self.close_serial.fetch_add(1, Ordering::SeqCst);
        self.auto_initialize.store(false, Ordering::SeqCst);
        let new_boundary = !self.invalidated.swap(true, Ordering::SeqCst);
        self.gate.store(false, Ordering::SeqCst);
        self.ready_epoch.store(0, Ordering::SeqCst);
        self.authorized_epoch.store(0, Ordering::SeqCst);
        self.authorized.store(false, Ordering::SeqCst);
        self.initialization_until.store(0, Ordering::SeqCst);
        if new_boundary {
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn worker_current(&self, ticket: WorkerTicket) -> bool {
        self.busy.load(Ordering::SeqCst)
            && self.worker_id.load(Ordering::SeqCst) == ticket.id
            && self.epoch.load(Ordering::SeqCst) == ticket.epoch
            && self.close_serial.load(Ordering::SeqCst) == ticket.close_serial
    }
    fn worker_may_retire_after_failure(&self, ticket: WorkerTicket) -> bool {
        self.busy.load(Ordering::SeqCst)
            && self.worker_id.load(Ordering::SeqCst) == ticket.id
            && !self.gate.load(Ordering::SeqCst)
    }
    fn schedule_recovery_locked(&self, now: u64) -> bool {
        if self.maintenance.load(Ordering::SeqCst) {
            self.lease_recovery_pending.store(false, Ordering::SeqCst);
            return false;
        }
        let attempt = self.recovery_attempts.load(Ordering::SeqCst) as usize;
        let Some(delay) = RECOVERY_DELAYS_MS.get(attempt) else {
            self.lease_recovery_pending.store(false, Ordering::SeqCst);
            return false;
        };
        self.recovery_attempts
            .store((attempt + 1) as u64, Ordering::SeqCst);
        self.recovery_after
            .store(now.saturating_add(*delay), Ordering::SeqCst);
        self.lease_recovery_pending.store(true, Ordering::SeqCst);
        true
    }
    /// 仅当前票据能发布失败；旧检查不得覆盖后来的撤销或租约恢复。
    fn fail_worker(&self, ticket: WorkerTicket, reason: &str, now: u64) -> bool {
        let _transition = self.admission_transition.lock().unwrap();
        if !self.worker_current(ticket) {
            return false;
        }
        self.invalidate_locked();
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        let recoverable = recovery_reason(reason).is_some();
        let scheduled = recoverable && self.schedule_recovery_locked(now);
        self.state.lock().unwrap().error = Some(
            if recoverable && !scheduled {
                "recovery_exhausted"
            } else {
                reason
            }
            .into(),
        );
        log::warn!(
            "input_permission_worker_failed worker={} epoch={} reason={} recovery={}",
            ticket.id,
            ticket.epoch,
            health_reason(Some(reason)).unwrap_or("unclassified_error"),
            scheduled
        );
        true
    }
    fn record_slow_worker(&self, ticket: WorkerTicket) -> bool {
        if self.busy.load(Ordering::SeqCst) && self.worker_id.load(Ordering::SeqCst) == ticket.id {
            self.worker_slow.store(true, Ordering::SeqCst);
            return true;
        }
        false
    }
    fn recovery_due(&self, now: u64) -> bool {
        self.lease_recovery_allowed() && now >= self.recovery_after.load(Ordering::SeqCst)
    }
    fn worker_blocked(&self, now: u64) -> bool {
        self.busy.load(Ordering::SeqCst)
            && (self.worker_hard_timeout.load(Ordering::SeqCst)
                || now.saturating_sub(self.worker_phase_started.load(Ordering::SeqCst))
                    >= INITIALIZATION_LEASE_MS)
    }
    fn public_recovery_pending(&self, now: u64) -> bool {
        !self.maintenance.load(Ordering::SeqCst)
            && (self.lease_recovery_allowed()
                || self.explicit_recovery_pending.load(Ordering::SeqCst))
            && !self.worker_blocked(now)
    }
    fn resource_health(&self, health: &'static str, now: u64) -> &'static str {
        if self.worker_blocked(now) {
            "restart_required"
        } else {
            health
        }
    }
    fn public_error_reason(&self, state: &State, now: u64) -> Option<&'static str> {
        if self.worker_blocked(now) {
            Some("native_worker_still_running")
        } else {
            health_reason(state.error.as_deref())
        }
    }
    fn probe_due(&self, now: u64) -> bool {
        if !self.gate.load(Ordering::SeqCst)
            && !self.lease_recovery_allowed()
            && !self.explicit_recovery_pending.load(Ordering::SeqCst)
            && self.retired_epoch.load(Ordering::SeqCst) == self.epoch.load(Ordering::SeqCst)
        {
            return false;
        }
        !self.busy.load(Ordering::SeqCst)
            && (self.explicit_recovery_pending.load(Ordering::SeqCst)
                || (now >= self.next_probe_after.load(Ordering::SeqCst)
                    && (!self.lease_recovery_allowed() || self.recovery_due(now))))
    }
    fn record_probe(&self, ticket: WorkerTicket, ax: bool, input: bool) -> bool {
        let _transition = self.admission_transition.lock().unwrap();
        if !self.worker_current(ticket) {
            return false;
        }
        let mut state = self.state.lock().unwrap();
        state.accessibility = if ax { "granted" } else { "denied" };
        state.input_monitoring = if input { "granted" } else { "denied" };
        true
    }
    fn refresh_ready(&self, ticket: WorkerTicket, observed: u64, now: u64) -> Result<(), String> {
        let _transition = self.admission_transition.lock().unwrap();
        if !self.worker_current(ticket)
            || !self.gate.load(Ordering::SeqCst)
            || now >= observed.saturating_add(PUBLIC_LEASE_MS)
        {
            return Err("stale_permission_probe".into());
        }
        self.lease_until
            .store(observed.saturating_add(PUBLIC_LEASE_MS), Ordering::SeqCst);
        Ok(())
    }
    fn close(&self, reason: &str) {
        let _transition = self.admission_transition.lock().unwrap();
        self.invalidate_locked();
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        self.explicit_recovery_pending
            .store(false, Ordering::SeqCst);
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
        self.fail_shortcuts_for_owner(worker_epoch, reason, None)
    }
    fn fail_shortcuts_for_owner(
        &self,
        worker_epoch: u64,
        reason: &str,
        owner: Option<&AtomicBool>,
    ) -> bool {
        let _transition = self.admission_transition.lock().unwrap();
        if owner.is_some_and(|owner| !owner.swap(false, Ordering::AcqRel)) {
            return false;
        }
        if self.epoch.load(Ordering::SeqCst) != worker_epoch {
            log::debug!("input_permission_stale_shortcut_fault ignored_epoch={worker_epoch}");
            return false;
        }
        self.invalidate_locked();
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        let recoverable = recovery_reason(reason).is_some();
        let scheduled = recoverable && self.schedule_recovery_locked(monotonic_ms());
        let mut state = self.state.lock().unwrap();
        state.shortcuts = "restart_required";
        state.error = Some(
            if recoverable && !scheduled {
                "recovery_exhausted"
            } else {
                reason
            }
            .into(),
        );
        log::warn!(
            "input_permission_closed reason={reason} worker_epoch={worker_epoch} epoch={}",
            self.epoch.load(Ordering::SeqCst)
        );
        true
    }
    fn retire_shortcut_owner(&self, epoch: u64, owner: &AtomicBool, pause: impl FnOnce()) -> bool {
        let _transition = self.admission_transition.lock().unwrap();
        if !owner.swap(false, Ordering::AcqRel) || self.epoch.load(Ordering::SeqCst) != epoch {
            return false;
        }
        pause();
        true
    }
    fn expire_lease(&self, now: u64) {
        let _transition = self.admission_transition.lock().unwrap();
        let ready = self.gate.load(Ordering::SeqCst);
        let deadline = if ready {
            self.lease_until.load(Ordering::SeqCst)
        } else {
            self.initialization_until.load(Ordering::SeqCst)
        };
        if now <= deadline || !self.authorized.load(Ordering::SeqCst) {
            return;
        }
        if !ready && self.busy.load(Ordering::SeqCst) {
            self.worker_hard_timeout.store(true, Ordering::SeqCst);
        }
        self.lease_until.store(0, Ordering::SeqCst);
        self.invalidate_locked();
        let reason = if ready {
            "permission_lease_expired"
        } else {
            "initialization_lease_expired"
        };
        let scheduled = self.schedule_recovery_locked(now);
        if !scheduled {
            self.state.lock().unwrap().error = Some("recovery_exhausted".into());
        }
        if self.lease_recovery_pending.load(Ordering::SeqCst) {
            if let Ok(mut state) = self.state.try_lock() {
                state.error = Some(reason.into());
            }
            log::warn!(
                "input_permission_closed reason={reason} epoch={}",
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
        self.initialization_until.store(
            monotonic_ms().saturating_add(INITIALIZATION_LEASE_MS),
            Ordering::SeqCst,
        );
        Ok(())
    }
    fn finish_initialization(
        &self,
        epoch: u64,
        serial: u64,
        explicit: bool,
        observed: u64,
    ) -> Result<(), String> {
        let _transition = self.admission_transition.lock().unwrap();
        if !self.initialization_current(epoch, serial, explicit)
            || !self.authorized.load(Ordering::SeqCst)
            || monotonic_ms() >= observed.saturating_add(PUBLIC_LEASE_MS)
            || monotonic_ms() >= self.initialization_until.load(Ordering::SeqCst)
        {
            return Err("stale_permission_initialization".into());
        }
        let mut st = self.state.lock().unwrap();
        st.enigo = "ready";
        st.shortcuts = "ready";
        st.targets = "ready";
        st.error = None;
        self.ready_epoch.store(epoch, Ordering::SeqCst);
        self.lease_until
            .store(observed.saturating_add(PUBLIC_LEASE_MS), Ordering::SeqCst);
        self.gate.store(true, Ordering::SeqCst);
        self.lease_recovery_pending.store(false, Ordering::SeqCst);
        self.recovery_attempts.store(0, Ordering::SeqCst);
        self.recovery_after.store(0, Ordering::SeqCst);
        self.initialization_until.store(0, Ordering::SeqCst);
        resume_native_blocking();
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
fn pause_native_blocking() {
    #[cfg(not(test))]
    handy_keys::set_blocking_enabled(false);
}
fn resume_native_blocking() {
    #[cfg(not(test))]
    handy_keys::set_blocking_enabled(true);
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
    // 维护是全局明确撤销，不能被旧 worker 的局部票据屏蔽。
    if !matches!(reason, "maintenance" | "maintenance_marker") {
        if let Some(ticket) = WORKER_TICKET.with(|current| current.get()) {
            lifecycle().fail_worker(ticket, reason, monotonic_ms());
            return;
        }
    }
    lifecycle().close(reason);
}
pub fn mark_shortcuts_ready(epoch: u64) {
    let s = lifecycle();
    let _transition = s.admission_transition.lock().unwrap();
    if s.epoch.load(Ordering::SeqCst) == epoch && s.authorized.load(Ordering::SeqCst) {
        s.state.lock().unwrap().shortcuts = "ready";
    }
}
pub fn mark_shortcuts_failed(epoch: u64, error: &str) {
    lifecycle().fail_shortcuts(epoch, error);
}
pub(crate) fn mark_shortcut_owner_failed(epoch: u64, error: &str, owner: &AtomicBool) {
    lifecycle().fail_shortcuts_for_owner(epoch, error, Some(owner));
}
pub(crate) fn retire_shortcut_owner(epoch: u64, owner: &AtomicBool) {
    lifecycle().retire_shortcut_owner(epoch, owner, pause_native_blocking);
}
/// 旧 Arc 只能停止自己；改变全局阻断必须与当前代校验处于同一临界区。
pub(crate) fn set_shortcut_blocking(epoch: u64, enabled: bool) {
    let s = lifecycle();
    let _transition = s.admission_transition.lock().unwrap();
    if s.epoch.load(Ordering::SeqCst) == epoch && (!enabled || s.gate.load(Ordering::SeqCst)) {
        if enabled {
            resume_native_blocking();
        } else {
            pause_native_blocking();
        }
    }
}
fn root(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    if let Some(profile) = crate::candidate_profile::current() {
        return profile
            .control_root
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
        let primary = ime.get("maintenance_marker_epoch");
        let legacy = ime.get("marker_epoch");
        !epoch.is_empty()
            && match (primary, legacy) {
                (Some(primary), Some(legacy)) => {
                    primary.as_str() == Some(epoch) && legacy.as_str() == Some(epoch)
                }
                (Some(primary), None) => primary.as_str() == Some(epoch),
                (None, Some(legacy)) => legacy.as_str() == Some(epoch),
                (None, None) => false,
            }
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
    json!({"epoch":s.epoch.load(Ordering::SeqCst),"gate_open":s.gate.load(Ordering::SeqCst),"permission":{"accessibility":state.accessibility,"input_monitoring":state.input_monitoring},"health":{"enigo":s.resource_health(state.enigo, monotonic_ms()),"shortcuts":s.resource_health(state.shortcuts, monotonic_ms()),"targets":s.resource_health(state.targets, monotonic_ms())},"probe_in_flight":s.busy.load(Ordering::SeqCst),"recovery_pending":s.public_recovery_pending(monotonic_ms()),"recovery_attempts":s.recovery_attempts.load(Ordering::SeqCst),"error_reason":s.public_error_reason(&state, monotonic_ms()),"worker":worker_health(s),"maintenance":maintenance,"maintenance_state":maintenance_state,"last_error":state.error,"components":{"background":{"component":"background","bundle_id":app.config().identifier,"state":background_state(s, &state)},"ime":ime}})
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
    let lifecycle = lifecycle();
    let (keys_needed, input_needed, targets_needed) = {
        let mut state = lifecycle.state.lock().unwrap();
        let needed = (
            state.shortcuts != "stopped",
            state.enigo != "stopped",
            state.targets != "stopped",
        );
        if needed.0 {
            state.shortcuts = "retiring";
        }
        if needed.1 {
            state.enigo = "retiring";
        }
        if needed.2 {
            state.targets = "retiring";
        }
        needed
    };
    // 已有成功退休回执的资源不重复销毁；未知结果仍必须重新取得成功回执。
    let keys = if keys_needed {
        crate::shortcut::retire_shortcuts(app)
    } else {
        Ok(())
    };
    let input = if input_needed {
        crate::input::retire_enigo(app)
    } else {
        Ok(())
    };
    #[cfg(target_os = "macos")]
    let targets = if targets_needed {
        crate::ime_target_broker::invalidate_all(app)
    } else {
        Ok(())
    };
    #[cfg(not(target_os = "macos"))]
    let targets: Result<(), String> = {
        let _ = targets_needed;
        Ok(())
    };
    let mut state = lifecycle.state.lock().unwrap();
    state.shortcuts = retirement_health(&keys);
    state.enigo = retirement_health(&input);
    state.targets = retirement_health(&targets);
    lifecycle
        .retired_epoch
        .store(lifecycle.epoch.load(Ordering::SeqCst), Ordering::SeqCst);
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
    let now = monotonic_ms();
    if !recover && !s.probe_due(now) {
        return;
    }
    if s.busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let explicit_pending = s.explicit_recovery_pending.swap(false, Ordering::SeqCst);
    let recover = recover || explicit_pending;
    let ticket = WorkerTicket {
        id: s.worker_id.fetch_add(1, Ordering::SeqCst) + 1,
        epoch: s.epoch.load(Ordering::SeqCst),
        close_serial: s.close_serial.load(Ordering::SeqCst),
    };
    s.worker_epoch.store(ticket.epoch, Ordering::SeqCst);
    s.worker_serial.store(ticket.close_serial, Ordering::SeqCst);
    s.worker_started.store(now, Ordering::SeqCst);
    s.worker_phase_started.store(now, Ordering::SeqCst);
    s.worker_slow.store(false, Ordering::SeqCst);
    s.worker_hard_timeout.store(false, Ordering::SeqCst);
    *s.worker_phase.lock().unwrap() = "probe";
    // 按启动时刻限频；慢 probe 完成后不再额外空等两秒，更不能启动并发 probe。
    s.next_probe_after
        .store(now.saturating_add(PROBE_INTERVAL_MS), Ordering::SeqCst);
    drop(transition);
    let app = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _scope = WorkerScope::enter(ticket);
        let phase = |value| {
            *s.worker_phase.lock().unwrap() = value;
            s.worker_phase_started
                .store(monotonic_ms(), Ordering::SeqCst);
        };
        let current = || {
            if s.worker_current(ticket) {
                Ok(())
            } else {
                Err("stale_permission_initialization".to_owned())
            }
        };
        let result = (|| -> Result<(), String> {
            if s.maintenance.load(Ordering::SeqCst) || marker_active(&app) {
                close_gate("maintenance_marker");
                phase("retire");
                return retire(&app);
            }
            let (ax, input) = probe();
            let observed = monotonic_ms();
            if !s.record_probe(ticket, ax, input) {
                return Err("stale_permission_probe".into());
            }
            if !ax {
                return Err("permission_required".into());
            }
            if s.gate.load(Ordering::SeqCst) {
                phase("verify_shortcuts");
                crate::shortcut::health_ready(&app, ticket.epoch)?;
                return s.refresh_ready(ticket, observed, monotonic_ms());
            }
            if !recover && !s.lease_recovery_allowed() {
                // 明确关闭后的同代清理只做一次；没有恢复计划时不每两秒重复退休。
                if s.retired_epoch.load(Ordering::SeqCst) != ticket.epoch {
                    phase("retire");
                    return retire(&app);
                }
                return Ok(());
            }
            phase("retire");
            retire(&app)?;
            current()?;
            if marker_active(&app) {
                return Err("maintenance_marker".into());
            }
            // 退休可能合法耗时数秒；创建新 native owner 前重新读取真实权限。
            phase("probe_after_retirement");
            let (ax, input) = probe();
            if !s.record_probe(ticket, ax, input) {
                return Err("stale_permission_probe".into());
            }
            if !ax {
                return Err("permission_required".into());
            }
            s.begin_initialization(ticket.epoch, ticket.close_serial, recover)?;
            {
                let mut state = s.state.lock().unwrap();
                state.enigo = "initializing";
                state.shortcuts = "initializing";
                state.targets = "initializing";
            }
            phase("initialize_input");
            crate::input::initialize_enigo(&app, ticket.epoch)?;
            current()?;
            phase("initialize_shortcuts");
            crate::shortcut::init_shortcuts(&app)?;
            current()?;
            phase("verify_shortcuts");
            crate::shortcut::health_ready(&app, ticket.epoch)?;
            // 构造许可不是业务准入；最终 probe 之前保持事件阻断关闭。
            set_shortcut_blocking(ticket.epoch, false);
            phase("final_probe");
            let (ax, input) = probe();
            let observed = monotonic_ms();
            if !s.record_probe(ticket, ax, input) {
                return Err("stale_permission_probe".into());
            }
            if !ax {
                return Err("permission_required".into());
            }
            if marker_active(&app) {
                return Err("maintenance_marker".into());
            }
            {
                let _worker = s.worker_transition.lock().unwrap();
                current()?;
                s.finish_initialization(ticket.epoch, ticket.close_serial, recover, observed)?;
            }
            log::info!(
                "input_permission_ready epoch={} recovery={}",
                ticket.epoch,
                if recover {
                    "explicit_or_startup"
                } else {
                    "automatic"
                }
            );
            app.manage(crate::commands::ShortcutsInitialized);
            Ok(())
        })();
        if let Err(error) = result {
            if matches!(error.as_str(), "maintenance" | "maintenance_marker") {
                close_gate(&error);
            } else {
                s.fail_worker(ticket, &error, monotonic_ms());
            }
            // 单飞仍属于本 worker。只清理真实资源，不发布旧代失败，不重放任何输入。
            let already_retired = *s.worker_phase.lock().unwrap() == "retire";
            if !already_retired && s.worker_may_retire_after_failure(ticket) {
                phase("retire_after_failure");
                if let Err(error) = retire(&app) {
                    log::warn!(
                        "input_permission_retirement_unconfirmed reason={}",
                        health_reason(Some(&error)).unwrap_or("unclassified_error")
                    );
                }
            }
        }
        {
            let _transition = s.worker_transition.lock().unwrap();
            if s.worker_id.load(Ordering::SeqCst) == ticket.id {
                *s.worker_phase.lock().unwrap() = "idle";
                s.busy.store(false, Ordering::SeqCst);
            }
            let _ = tx.send(());
        }
        publish_health(&app);
    });
    std::thread::spawn(move || {
        if rx.recv_timeout(Duration::from_millis(1500)).is_ok() {
            return;
        }
        let _transition = s.worker_transition.lock().unwrap();
        if s.record_slow_worker(ticket) {
            // 只记慢检查：真正准入期限仍由公开 3.5s / 构造 8s 租约与真实撤权控制。
            // 超时不取消 native、不释放单飞，也不改变任何新代的恢复意图。
            log::warn!(
                "input_permission_worker_slow worker={} epoch={}",
                ticket.id,
                ticket.epoch
            );
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
fn background_state(s: &Lifecycle, st: &State) -> &'static str {
    if s.gate.load(Ordering::SeqCst) {
        "ready"
    } else if s.worker_blocked(monotonic_ms()) {
        "restart_required"
    } else if s.public_recovery_pending(monotonic_ms()) {
        "recovering"
    } else if st.error.as_deref() == Some("recovery_exhausted")
        || st.enigo == "restart_required"
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
    } else if s.busy.load(Ordering::SeqCst) {
        "retiring"
    } else {
        "paused"
    }
}
fn worker_health(s: &Lifecycle) -> Value {
    let busy = s.busy.load(Ordering::SeqCst);
    json!({"id":s.worker_id.load(Ordering::SeqCst),"epoch":s.worker_epoch.load(Ordering::SeqCst),
        "close_serial":s.worker_serial.load(Ordering::SeqCst),"busy":busy,
        "phase":*s.worker_phase.lock().unwrap(),"slow":busy && s.worker_slow.load(Ordering::SeqCst),
        "blocked":s.worker_blocked(monotonic_ms()),
        "elapsed_ms":if busy {monotonic_ms().saturating_sub(s.worker_started.load(Ordering::SeqCst))} else {0}})
}
fn publish_health(app: &AppHandle) {
    let s = lifecycle();
    let st = s.state.lock().unwrap();
    let state = background_state(s, &st);
    let value = json!({"schema_version":1,"component":"background","pid":std::process::id(),"updated_at_ms":now_ms() as u64,"state":state,"maintenance_state":if state=="maintenance" {"active"} else {state},"permission_epoch":s.epoch.load(Ordering::SeqCst),
        "gate_open":s.gate.load(Ordering::SeqCst),"recovery_pending":s.public_recovery_pending(monotonic_ms()),
        "recovery_attempts":s.recovery_attempts.load(Ordering::SeqCst),
        "recovery_wait_ms":if s.lease_recovery_allowed() {s.recovery_after.load(Ordering::SeqCst).saturating_sub(monotonic_ms())} else {0},
        "error_reason":s.public_error_reason(&st, monotonic_ms()),"worker":worker_health(s),
        "health":{"enigo":s.resource_health(st.enigo, monotonic_ms()),"shortcuts":s.resource_health(st.shortcuts, monotonic_ms()),"targets":s.resource_health(st.targets, monotonic_ms())}});
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
    {
        let s = lifecycle();
        let _transition = s.admission_transition.lock().unwrap();
        s.maintenance.store(false, Ordering::SeqCst);
        s.auto_initialize.store(true, Ordering::SeqCst);
        s.close_serial.fetch_add(1, Ordering::SeqCst);
        s.explicit_recovery_pending.store(true, Ordering::SeqCst);
        s.lease_recovery_pending.store(false, Ordering::SeqCst);
        s.recovery_attempts.store(0, Ordering::SeqCst);
        s.recovery_after.store(0, Ordering::SeqCst);
    }
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
    std::thread::spawn(move || {
        let mut next_health = 0;
        loop {
            let now = monotonic_ms();
            if now >= next_health {
                if marker_active(&app) && !lifecycle().maintenance.load(Ordering::SeqCst) {
                    close_gate("maintenance_marker");
                }
                publish_health(&app);
                next_health = now.saturating_add(2_000);
            }
            launch(&app, false);
            std::thread::sleep(Duration::from_millis(200));
        }
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
        s.finish_initialization(recovery_epoch, serial, false, monotonic_ms())
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
            let _ = s.finish_initialization(epoch, serial, false, monotonic_ms());
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
        s.finish_initialization(epoch, serial, false, monotonic_ms())
            .unwrap();
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

    fn worker_ticket(s: &Lifecycle) -> WorkerTicket {
        s.busy.store(true, Ordering::SeqCst);
        WorkerTicket {
            id: s.worker_id.fetch_add(1, Ordering::SeqCst) + 1,
            epoch: s.epoch.load(Ordering::SeqCst),
            close_serial: s.close_serial.load(Ordering::SeqCst),
        }
    }

    #[test]
    fn stale_probe_and_watchdog_cannot_cancel_newer_recovery() {
        let s = ready_lifecycle();
        let old = worker_ticket(&s);
        s.expire_lease(101);
        let serial = s.close_serial.load(Ordering::SeqCst);
        let attempts = s.recovery_attempts.load(Ordering::SeqCst);
        assert!(!s.fail_worker(old, "stale_permission_probe", 102));
        assert!(s.record_slow_worker(old));
        assert!(s.lease_recovery_allowed());
        assert_eq!(s.close_serial.load(Ordering::SeqCst), serial);
        assert_eq!(s.recovery_attempts.load(Ordering::SeqCst), attempts);
        assert!(s.busy.load(Ordering::SeqCst));
    }

    #[test]
    fn worker_identity_and_close_serial_both_guard_error_publication() {
        for mismatch in ["worker", "serial"] {
            let s = ready_lifecycle();
            let old = worker_ticket(&s);
            if mismatch == "worker" {
                s.worker_id.fetch_add(1, Ordering::SeqCst);
            } else {
                s.close_serial.fetch_add(1, Ordering::SeqCst);
            }
            assert!(!s.fail_worker(old, "快捷键健康检查通道掉线", 10));
            assert!(s.gate.load(Ordering::SeqCst));
            assert!(!s.lease_recovery_allowed());
        }
    }

    #[test]
    fn known_transient_faults_have_five_bounded_retries_without_reset_on_stale_results() {
        let s = ready_lifecycle();
        for attempt in 0..=RECOVERY_DELAYS_MS.len() {
            let ticket = worker_ticket(&s);
            assert!(s.fail_worker(ticket, "快捷键健康检查通道掉线", 100));
            assert!(!s.gate.load(Ordering::SeqCst));
            assert!(!s.fail_worker(ticket, "stale_permission_probe", 101));
            if let Some(delay) = RECOVERY_DELAYS_MS.get(attempt) {
                assert!(!s.recovery_due(100 + delay - 1));
                assert!(s.recovery_due(100 + delay));
            } else {
                assert!(!s.lease_recovery_allowed());
                assert_eq!(
                    s.state.lock().unwrap().error.as_deref(),
                    Some("recovery_exhausted")
                );
            }
        }
        assert_eq!(s.recovery_attempts.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn denied_maintenance_settings_and_unknown_faults_never_schedule_recovery() {
        for reason in [
            "permission_required",
            "shortcut_settings_retirement_unconfirmed",
            "input_device_poisoned",
            "Shortcut cannot be empty",
            "unknown external error",
        ] {
            let s = ready_lifecycle();
            let ticket = worker_ticket(&s);
            assert!(s.fail_worker(ticket, reason, 1));
            assert!(!s.lease_recovery_allowed());
        }
        let s = ready_lifecycle();
        let ticket = worker_ticket(&s);
        s.close("maintenance");
        assert!(!s.fail_worker(ticket, "快捷键健康检查通道掉线", 1));
        assert!(!s.lease_recovery_allowed());
        assert!(!s.public_recovery_pending(1));
    }

    #[test]
    fn slow_initialization_keeps_business_closed_then_requires_fresh_ready_proof() {
        let s = ready_lifecycle();
        s.expire_lease(101);
        let ticket = worker_ticket(&s);
        s.begin_initialization(ticket.epoch, ticket.close_serial, false)
            .unwrap();
        let start = monotonic_ms();
        assert!(s.record_slow_worker(ticket));
        s.expire_lease(start + 2_000);
        assert!(s.authorized.load(Ordering::SeqCst));
        assert!(s.check(ticket.epoch).is_err());
        s.finish_initialization(ticket.epoch, ticket.close_serial, false, monotonic_ms())
            .unwrap();
        let deadline = s.lease_until.load(Ordering::SeqCst);
        assert!(s.check(ticket.epoch).is_ok());
        s.expire_lease(deadline + 1);
        assert!(s.check(ticket.epoch).is_err());
    }

    #[test]
    fn repeated_two_second_probes_do_not_revoke_a_valid_public_lease() {
        let s = ready_lifecycle();
        s.lease_until.store(PUBLIC_LEASE_MS, Ordering::SeqCst);
        for observed in [2_000, 4_000, 6_000] {
            let ticket = worker_ticket(&s);
            assert!(s.record_slow_worker(ticket));
            s.expire_lease(observed);
            assert!(s.gate.load(Ordering::SeqCst));
            s.refresh_ready(ticket, observed, observed).unwrap();
            assert_eq!(
                s.lease_until.load(Ordering::SeqCst),
                observed + PUBLIC_LEASE_MS
            );
            s.next_probe_after.store(observed, Ordering::SeqCst);
            s.busy.store(false, Ordering::SeqCst);
            assert!(s.probe_due(observed));
        }
    }

    #[test]
    fn construction_permission_is_bounded_and_late_ready_cannot_override_revocation() {
        for maintenance in [false, true] {
            let s = ready_lifecycle();
            s.expire_lease(101);
            let ticket = worker_ticket(&s);
            s.begin_initialization(ticket.epoch, ticket.close_serial, false)
                .unwrap();
            if maintenance {
                s.close("maintenance");
            } else {
                s.expire_lease(s.initialization_until.load(Ordering::SeqCst) + 1);
            }
            assert!(s
                .finish_initialization(ticket.epoch, ticket.close_serial, false, monotonic_ms())
                .is_err());
            assert!(!s.authorized.load(Ordering::SeqCst));
            assert!(!s.gate.load(Ordering::SeqCst));
            assert!(s.busy.load(Ordering::SeqCst));
            if !maintenance {
                assert!(!s.public_recovery_pending(monotonic_ms()));
                assert!(s.lease_recovery_allowed());
            }
        }
    }

    #[test]
    fn stopped_terminal_state_does_not_probe_or_retire_forever() {
        let s = ready_lifecycle();
        s.close("permission_required");
        s.retired_epoch
            .store(s.epoch.load(Ordering::SeqCst), Ordering::SeqCst);
        for now in [0, 2_000, 20_000] {
            assert!(!s.probe_due(now));
        }
        assert_eq!(background_state(&s, &s.state.lock().unwrap()), "paused");
        s.explicit_recovery_pending.store(true, Ordering::SeqCst);
        assert!(s.probe_due(20_000));
    }

    #[test]
    fn hard_stalled_worker_keeps_slot_but_does_not_report_endless_ui_recovery() {
        let s = ready_lifecycle();
        s.expire_lease(101);
        let ticket = worker_ticket(&s);
        s.worker_phase_started.store(100, Ordering::SeqCst);
        assert!(s.public_recovery_pending(101));
        assert!(!s.public_recovery_pending(100 + INITIALIZATION_LEASE_MS));
        assert_eq!(
            s.resource_health("retiring", 100 + INITIALIZATION_LEASE_MS),
            "restart_required"
        );
        assert!(s.lease_recovery_allowed());
        assert_eq!(s.worker_id.load(Ordering::SeqCst), ticket.id);
        assert!(s.busy.load(Ordering::SeqCst));
    }

    #[test]
    fn explicit_recovery_intent_survives_a_monitor_claiming_the_slot_first() {
        let s = ready_lifecycle();
        s.close("unknown error");
        let monitor = worker_ticket(&s);
        s.close_serial.fetch_add(1, Ordering::SeqCst);
        s.explicit_recovery_pending.store(true, Ordering::SeqCst);
        assert!(!s.fail_worker(monitor, "unknown error", 10));
        assert!(s.explicit_recovery_pending.load(Ordering::SeqCst));
        s.busy.store(false, Ordering::SeqCst);
        assert!(s.probe_due(10));
        assert!(s.explicit_recovery_pending.swap(false, Ordering::SeqCst));
    }

    #[test]
    fn explicit_recheck_racing_a_ready_probe_cannot_retire_live_resources() {
        let s = ready_lifecycle();
        let monitor = worker_ticket(&s);
        s.close_serial.fetch_add(1, Ordering::SeqCst);
        s.explicit_recovery_pending.store(true, Ordering::SeqCst);
        assert!(!s.record_probe(monitor, true, true));
        assert!(!s.fail_worker(monitor, "stale_permission_probe", 1));
        assert!(!s.worker_may_retire_after_failure(monitor));
        assert!(s.gate.load(Ordering::SeqCst));
        s.busy.store(false, Ordering::SeqCst);
        assert!(s.probe_due(1));
    }

    #[test]
    fn retired_shortcut_arc_cannot_pause_or_fail_a_replacement_in_the_same_epoch() {
        let s = ready_lifecycle();
        let old = AtomicBool::new(true);
        let pauses = AtomicU64::new(0);
        assert!(s.retire_shortcut_owner(1, &old, || {
            pauses.fetch_add(1, Ordering::SeqCst);
        }));
        let replacement = AtomicBool::new(true);
        assert!(!s.retire_shortcut_owner(1, &old, || {
            pauses.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(!s.fail_shortcuts_for_owner(1, "快捷键原生监听器意外退出", Some(&old)));
        assert_eq!(pauses.load(Ordering::SeqCst), 1);
        assert!(replacement.load(Ordering::SeqCst));
        assert!(s.gate.load(Ordering::SeqCst));
    }

    #[test]
    fn health_reasons_never_copy_untrusted_error_details() {
        assert_eq!(
            health_reason(Some("private fixture body /path/to/file")),
            Some("unclassified_error")
        );
        assert_eq!(
            health_reason(Some("快捷键健康检查通道掉线")),
            Some("shortcut_channel_unavailable")
        );
        assert_eq!(health_reason(None), None);
    }

    #[test]
    fn maintenance_ack_accepts_real_ime_payload_and_rejects_conflicting_legacy_fields() {
        let marker = json!({"active":true,"epoch":"fixture-maintenance"});
        for ime in [
            json!({"maintenance_marker_epoch":"fixture-maintenance"}),
            json!({"marker_epoch":"fixture-maintenance"}),
            json!({"maintenance_marker_epoch":"fixture-maintenance","marker_epoch":"fixture-maintenance"}),
        ] {
            assert!(maintenance_ack_matches(&ime, Some(&marker)));
        }
        for ime in [
            json!({"maintenance_marker_epoch":"old","marker_epoch":"fixture-maintenance"}),
            json!({"maintenance_marker_epoch":"fixture-maintenance","marker_epoch":"old"}),
            json!({"maintenance_marker_epoch":null,"marker_epoch":"fixture-maintenance"}),
        ] {
            assert!(!maintenance_ack_matches(&ime, Some(&marker)));
        }
    }
}
