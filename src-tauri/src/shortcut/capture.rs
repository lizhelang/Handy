//! 录制租约由进程内清理器持有；网页失联或结束回执丢失不会遗忘原生所有权。
use super::{settings_barrier::SettingsBarrier, settings_delta::RegistrationDelta};
use crate::settings::{self, AppSettings, KeyboardImplementation};
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const CAPTURE_LIFETIME: Duration = Duration::from_secs(60);
const END_WAIT: Duration = Duration::from_millis(1500);
struct Capture {
    token: String,
    epoch: u64,
    before: AppSettings,
    delta: RegistrationDelta,
    needs_rebuild: bool,
    _barrier: SettingsBarrier,
}
struct Lease {
    token: String,
    deadline: Instant,
    requested: bool,
    can_save: bool,
}
#[derive(Default)]
struct Cleanup {
    active: Option<Lease>,
    completed: VecDeque<(String, bool)>,
}
impl Cleanup {
    fn request(&mut self, token: &str) -> Result<Option<bool>, String> {
        if let Some((_, can_save)) = self.completed.iter().find(|(old, _)| old == token) {
            return Ok(Some(*can_save));
        }
        let active = self
            .active
            .as_mut()
            .filter(|lease| lease.token == token)
            .ok_or("shortcut_capture_token_mismatch")?;
        active.requested = true;
        Ok(None)
    }
    fn due(&mut self, token: &str, now: Instant) -> Option<bool> {
        let active = self.active.as_mut().filter(|lease| lease.token == token)?;
        if now >= active.deadline {
            active.requested = true;
            active.can_save = false;
        }
        active.requested.then_some(active.can_save)
    }
    fn complete_attempt(
        &mut self,
        token: &str,
        now: Instant,
        result: Result<bool, String>,
    ) -> bool {
        let Some(can_save) = self.due(token, now) else {
            return false;
        };
        let Ok(native_can_save) = result else {
            return false;
        };
        self.complete(token, can_save && native_can_save);
        true
    }
    fn complete(&mut self, token: &str, can_save: bool) {
        if self
            .active
            .as_ref()
            .is_some_and(|lease| lease.token == token)
        {
            self.active = None;
            self.completed.push_back((token.to_owned(), can_save));
            while self.completed.len() > 32 {
                self.completed.pop_front();
            }
        }
    }
}
#[derive(Default)]
struct Captures {
    current: Mutex<Option<Capture>>,
    cleanup: Mutex<Cleanup>,
    wake: Condvar,
}

pub(crate) fn ensure_none(app: &AppHandle) -> Result<(), String> {
    if let Some(slot) = app.try_state::<Captures>() {
        if slot
            .cleanup
            .lock()
            .map_err(|_| "shortcut_capture_unavailable")?
            .active
            .is_some()
        {
            return Err("shortcut_capture_busy".into());
        }
    }
    Ok(())
}

pub(crate) fn begin(
    app: &AppHandle,
    binding_id: String,
    backend: KeyboardImplementation,
) -> Result<String, String> {
    let started_at = Instant::now();
    let _change = super::settings_change_guard()?;
    ensure_none(app)?;
    let epoch = crate::input_permission::capture_epoch()?;
    let before = settings::get_settings(app);
    if before.keyboard_implementation != backend || !before.bindings.contains_key(&binding_id) {
        return Err("shortcut_capture_target_changed".into());
    }
    if backend == KeyboardImplementation::HandyKeys && crate::secure_input::is_enabled_now() {
        crate::secure_input::note_recorder_blocked(app);
        return Err("secure-input-active".into());
    }
    let token = uuid::Uuid::new_v4().to_string();
    let owned_app = app.clone();
    let owned_token = token.clone();
    let (start_worker, ready) = std::sync::mpsc::channel::<()>();
    // 在任何原生变更前确认清理线程存在；准备失败时关闭通道，空线程自动退出。
    std::thread::Builder::new()
        .name("shortcut-capture-cleanup".into())
        .spawn(move || {
            if ready.recv().is_ok() {
                cleanup_loop(owned_app, owned_token);
            }
        })
        .map_err(|_| "shortcut_capture_cleanup_unavailable")?;
    let barrier = SettingsBarrier::acquire(app)?;
    if crate::secure_input::pause_fallback_for_settings(app).is_err() {
        super::suspend_for_settings_failure(app);
        return Err("shortcut_capture_fallback_unconfirmed".into());
    }
    let delta = RegistrationDelta::suspend(&before);
    if delta.apply(app).is_err() {
        super::suspend_for_settings_failure(app);
        return Err("shortcut_capture_suspend_unconfirmed".into());
    }
    app.manage(Captures::default());
    let slot = app.state::<Captures>();
    *slot
        .current
        .lock()
        .map_err(|_| "shortcut_capture_unavailable")? = Some(Capture {
        token: token.clone(),
        epoch,
        before,
        delta,
        needs_rebuild: false,
        _barrier: barrier,
    });
    slot.cleanup
        .lock()
        .map_err(|_| "shortcut_capture_unavailable")?
        .active = Some(Lease {
        token: token.clone(),
        deadline: started_at + CAPTURE_LIFETIME,
        requested: false,
        can_save: true,
    });
    let started = backend != KeyboardImplementation::HandyKeys
        || super::handy_keys::current(app)
            .and_then(|state| state.start_recording(app, binding_id, token.clone()))
            .is_ok();
    if !started {
        let mut cleanup = slot
            .cleanup
            .lock()
            .map_err(|_| "shortcut_capture_unavailable")?;
        if let Some(active) = cleanup.active.as_mut() {
            active.requested = true;
            active.can_save = false;
        }
    }
    // 即使 token 尚未交付给网页，清理责任也已经在进程内。网页不必重试才能继续清理。
    if start_worker.send(()).is_err() {
        // 已创建线程异常退出：当前后台调用接管清理，不能返回一个无清理责任的租约。
        if let Ok(mut cleanup) = slot.cleanup.lock() {
            if let Some(active) = cleanup.active.as_mut() {
                active.requested = true;
                active.can_save = false;
            }
        }
        drop(_change);
        cleanup_loop(app.clone(), token);
        return Err("shortcut_capture_cleanup_unavailable".into());
    }
    if started {
        Ok(token)
    } else {
        Err("shortcut_capture_start_failed_cleanup_pending".into())
    }
}

/// true 才允许保存本次录制；false 表示已安全退休，但租约过期或权限已改变。
pub(crate) fn end(app: &AppHandle, token: String) -> Result<bool, String> {
    let slot = app
        .try_state::<Captures>()
        .ok_or("shortcut_capture_missing")?;
    let deadline = Instant::now() + END_WAIT;
    let mut cleanup = slot
        .cleanup
        .lock()
        .map_err(|_| "shortcut_capture_unavailable")?;
    if let Some(result) = cleanup.request(&token)? {
        return Ok(result);
    }
    slot.wake.notify_all();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("shortcut_capture_cleanup_pending".into());
        }
        cleanup = slot
            .wake
            .wait_timeout(cleanup, remaining)
            .map_err(|_| "shortcut_capture_unavailable")?
            .0;
        if let Some(result) = cleanup.request(&token)? {
            return Ok(result);
        }
    }
}

fn cleanup_loop(app: AppHandle, token: String) {
    let slot = app.state::<Captures>();
    let mut failures = 0usize;
    loop {
        let due = {
            let mut cleanup = match slot.cleanup.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if cleanup
                .active
                .as_ref()
                .is_none_or(|lease| lease.token != token)
            {
                return;
            }
            cleanup.due(&token, Instant::now())
        };
        if due.is_some() {
            let result = (|| {
                let _change = super::settings_change_guard()?;
                let mut current = slot
                    .current
                    .try_lock()
                    .map_err(|_| "shortcut_capture_busy")?;
                finish(&app, &mut current, &token)
            })();
            if let Ok(mut cleanup) = slot.cleanup.lock() {
                if cleanup.complete_attempt(&token, Instant::now(), result) {
                    slot.wake.notify_all();
                    return;
                }
            }
            failures = failures.saturating_add(1);
        }
        // 临时 busy/原生退出未完成可重试；持续失败仍持屏障且 end 返回 cleanup_pending。
        // 不记录快捷键内容，避免无界忙循环；窗口崩溃时60秒期限也会主动请求清理。
        let pause = if failures > 10 {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(100)
        };
        if let Ok(cleanup) = slot.cleanup.lock() {
            let _ = slot.wake.wait_timeout(cleanup, pause);
        }
    }
}

fn finish(app: &AppHandle, current: &mut Option<Capture>, token: &str) -> Result<bool, String> {
    let capture = current
        .as_mut()
        .filter(|capture| capture.token == token)
        .ok_or("shortcut_capture_token_mismatch")?;
    if capture.before.keyboard_implementation == KeyboardImplementation::HandyKeys {
        super::handy_keys::current(app)?.stop_recording()?;
    }
    if crate::input_permission::check_epoch(capture.epoch).is_err() {
        super::retire_configured_shortcuts_checked(app)?;
        current.take();
        return Ok(false);
    }
    let selected = settings::get_settings(app);
    if selected.keyboard_implementation != capture.before.keyboard_implementation
        || !RegistrationDelta::new(&capture.before, &selected).is_empty()
    {
        super::retire_configured_shortcuts_checked(app)?;
        current.take();
        return Ok(false);
    }
    if capture.needs_rebuild {
        // 不盲重放结果未知的 register：先真实退休整组件，再按已核设置重建。
        super::rebuild_configured_shortcuts(app)?;
    } else if capture.delta.restore(app).is_err()
        || crate::secure_input::resume_fallback_after_settings(app).is_err()
    {
        capture.needs_rebuild = true;
        return Err("shortcut_capture_restore_unconfirmed".into());
    }
    // 原生回执等待期间也可能撤权；仍在录制屏障内，先核原 epoch 才释放业务准入。
    if crate::input_permission::check_epoch(capture.epoch).is_err() {
        super::retire_configured_shortcuts_checked(app)?;
        current.take();
        return Ok(false);
    }
    current.take();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn active(now: Instant) -> Cleanup {
        Cleanup {
            active: Some(Lease {
                token: "first".into(),
                deadline: now + CAPTURE_LIFETIME,
                requested: false,
                can_save: true,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn start_failure_or_disappeared_ui_keeps_cleanup_responsibility_after_failed_attempt() {
        let now = Instant::now();
        let mut state = active(now);
        state.active.as_mut().unwrap().can_save = false;
        assert_eq!(state.request("first").unwrap(), None);
        // 首次原生 stop 失败没有 complete；后续 tick 仍保持原 token 的清理请求。
        assert_eq!(state.due("first", now), Some(false));
        assert!(!state.complete_attempt("first", now, Err("busy".into())));
        assert_eq!(
            state.due("first", now + Duration::from_millis(100)),
            Some(false)
        );
        assert!(state.complete_attempt("first", now + Duration::from_millis(100), Ok(true)));
        assert_eq!(state.request("first").unwrap(), Some(false));
    }
    #[test]
    fn lost_end_receipt_is_idempotent_and_cannot_end_new_capture() {
        let now = Instant::now();
        let mut state = active(now);
        state.request("first").unwrap();
        state.complete("first", true);
        state.active = Some(Lease {
            token: "second".into(),
            deadline: now + CAPTURE_LIFETIME,
            requested: false,
            can_save: true,
        });
        assert_eq!(state.request("first").unwrap(), Some(true));
        assert!(!state.active.as_ref().unwrap().requested);
        assert!(state.request("unknown").is_err());
    }
    #[test]
    fn native_cleanup_finishing_after_deadline_cannot_authorize_save() {
        let now = Instant::now();
        let mut state = active(now);
        state.request("first").unwrap();
        assert_eq!(state.due("first", now), Some(true));
        assert!(state.complete_attempt("first", now + CAPTURE_LIFETIME, Ok(true)));
        assert_eq!(state.request("first").unwrap(), Some(false));
    }
    #[test]
    fn lost_webview_has_bounded_lease_and_cannot_save_expired_capture() {
        let now = Instant::now();
        let mut state = active(now);
        assert_eq!(state.due("first", now), None);
        assert_eq!(state.due("first", now + CAPTURE_LIFETIME), Some(false));
        state.complete("first", false);
        assert_eq!(state.request("first").unwrap(), Some(false));
    }
}
