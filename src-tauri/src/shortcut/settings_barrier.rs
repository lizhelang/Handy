//! 快捷键配置的短期准入：待证明空闲时保留 release，确认空闲后才退休旧代数。
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};

#[derive(Default)]
struct Admission {
    generation: u64,
    pending: bool,
    frozen: bool,
    in_flight: usize,
}
impl Admission {
    fn capture(&self, is_press: bool) -> Option<u64> {
        (!self.frozen && !(self.pending && is_press)).then_some(self.generation)
    }
    fn admit(&mut self, generation: u64, is_press: bool) -> bool {
        if self.capture(is_press) != Some(generation) {
            return false;
        }
        self.in_flight += 1;
        true
    }
    fn begin(&mut self) -> Result<(), String> {
        if self.pending || self.in_flight != 0 {
            return Err("shortcut_settings_busy".into());
        }
        self.pending = true;
        Ok(())
    }
    fn freeze(&mut self) -> Result<(), String> {
        if self.in_flight != 0 {
            return Err("shortcut_settings_busy".into());
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("shortcut_generation_exhausted")?;
        self.frozen = true;
        Ok(())
    }
    fn finish(&mut self) {
        self.frozen = false;
        self.pending = false;
    }
}
static ADMISSION: OnceLock<Mutex<Admission>> = OnceLock::new();
fn admission() -> &'static Mutex<Admission> {
    ADMISSION.get_or_init(Mutex::default)
}

pub(crate) fn capture(is_press: bool) -> Option<u64> {
    let state = admission().lock().ok()?;
    state.capture(is_press)
}

pub(crate) struct EventLease;
impl Drop for EventLease {
    fn drop(&mut self) {
        if let Ok(mut state) = admission().lock() {
            state.in_flight = state.in_flight.saturating_sub(1);
        }
    }
}
/// 只在短临界区取得计数票据；不持锁等 IPC / Carbon / coordinator 回执。
pub(crate) fn admit(generation: u64, is_press: bool) -> Option<EventLease> {
    let mut state = admission().lock().ok()?;
    if !state.admit(generation, is_press) {
        return None;
    }
    Some(EventLease)
}

struct Pending;
impl Pending {
    fn begin() -> Result<Self, String> {
        let mut state = admission()
            .lock()
            .map_err(|_| "shortcut_admission_unavailable")?;
        state.begin()?;
        Ok(Self)
    }
    fn freeze(&self) -> Result<(), String> {
        let mut state = admission()
            .lock()
            .map_err(|_| "shortcut_admission_unavailable")?;
        state.freeze()
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut state) = admission().lock() {
            state.finish();
        }
    }
}

pub(crate) struct SettingsBarrier {
    // Drop 顺序：先发 coordinator release，再恢复 native 准入。
    _coordinator: crate::transcription_coordinator::SettingsIdleGuard,
    _pending: Pending,
}
impl SettingsBarrier {
    pub(crate) fn acquire(app: &AppHandle) -> Result<Self, String> {
        let pending = Pending::begin()?;
        #[cfg(target_os = "macos")]
        app.try_state::<crate::host_shortcut_broker::HostShortcutBroker>()
            .ok_or("shortcut_broker_unavailable")?
            .confirm_settings_idle()?;
        let coordinator = app
            .try_state::<crate::TranscriptionCoordinator>()
            .ok_or("shortcut_coordinator_unavailable")?
            .acquire_settings_idle()?;
        pending.freeze()?;
        super::cancel_registration::confirm_idle(app)?;
        Ok(Self {
            _coordinator: coordinator,
            _pending: pending,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_preserves_release_but_freeze_retires_old_queued_generation() {
        let mut state = Admission::default();
        let old = state.capture(true).unwrap();
        assert!(state.admit(old, true));
        assert!(state.begin().is_err()); // Host consume→actor 在途，不持锁也不能穿越。
        state.in_flight -= 1;
        state.begin().unwrap();
        assert!(state.capture(true).is_none());
        assert!(state.admit(old, false));
        assert!(state.freeze().is_err());
        state.in_flight -= 1;
        state.finish(); // 拒绝修改不退休原 release 代数。
        assert_eq!(state.capture(false), Some(old));
        state.begin().unwrap();
        state.freeze().unwrap();
        assert!(!state.admit(old, false));
        state.finish();
        assert!(!state.admit(old, true));
        assert!(state.capture(true).is_some_and(|next| next != old));
    }
}
