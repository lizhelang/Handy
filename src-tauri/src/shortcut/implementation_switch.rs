use crate::settings::KeyboardImplementation;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct ImplementationSwitch(Mutex<Phase>);

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Applying,
    AwaitingCleanup(KeyboardImplementation),
}

#[derive(Debug, PartialEq)]
pub(super) enum Begin {
    Unchanged,
    Fresh,
    ResumeCleanup,
}

impl ImplementationSwitch {
    // 仅保护阶段；绝不能持此锁调用 OS 注册或等待主线程。
    pub(super) fn begin(
        &self,
        current: KeyboardImplementation,
        requested: KeyboardImplementation,
    ) -> Result<Begin, String> {
        let mut phase = self.0.lock().map_err(|_| "快捷键切换状态锁不可用")?;
        let action = match *phase {
            Phase::Applying => return Err("快捷键切换仍在进行，请等待原请求返回".into()),
            Phase::AwaitingCleanup(target) if target != requested => {
                return Err(format!("快捷键切换尚未完成，请先重试 {target:?}"));
            }
            Phase::AwaitingCleanup(_) => Begin::ResumeCleanup,
            Phase::Idle if current == requested => return Ok(Begin::Unchanged),
            Phase::Idle => Begin::Fresh,
        };
        *phase = Phase::Applying;
        Ok(action)
    }

    pub(super) fn cleanup(
        &self,
        target: KeyboardImplementation,
        effect: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        // checked 清理可能失败或回执未知。仅成功回执授权后续 primary 注册；
        // 失败保留阶段，重试由 fallback 自己的资源账本收敛。
        if let Err(error) = effect() {
            *self.0.lock().map_err(|_| "快捷键切换状态锁不可用")? = Phase::AwaitingCleanup(target);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn complete(&self) -> Result<(), String> {
        *self.0.lock().map_err(|_| "快捷键切换状态锁不可用")? = Phase::Idle;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyboardImplementation::{HandyKeys, Tauri};

    #[test]
    fn changed_setting_does_not_hide_failed_cleanup_and_retry_registers_once() {
        let state = ImplementationSwitch::default();
        assert_eq!(state.begin(HandyKeys, Tauri).unwrap(), Begin::Fresh);
        let mut registered = 0;
        let mut cleanup_attempts = 0;
        let first = state.cleanup(Tauri, || {
            cleanup_attempts += 1;
            Err("清理回执未知".into())
        });
        if first.is_ok() {
            registered += 1;
        }
        assert_eq!(registered, 0);
        assert_eq!(state.begin(Tauri, Tauri).unwrap(), Begin::ResumeCleanup);
        state
            .cleanup(Tauri, || {
                cleanup_attempts += 1;
                Ok(())
            })
            .unwrap();
        registered += 1;
        state.complete().unwrap();
        assert_eq!((cleanup_attempts, registered), (2, 1));
        assert_eq!(state.begin(Tauri, Tauri).unwrap(), Begin::Unchanged);
    }

    #[test]
    fn repeated_failure_preserves_target_and_blocks_unrelated_switch() {
        let state = ImplementationSwitch::default();
        state.begin(HandyKeys, Tauri).unwrap();
        for _ in 0..3 {
            assert!(state.cleanup(Tauri, || Err("删除失败".into())).is_err());
            assert!(state.begin(Tauri, HandyKeys).is_err());
            assert_eq!(state.begin(Tauri, Tauri).unwrap(), Begin::ResumeCleanup);
        }
    }

    #[test]
    fn rollback_cleanup_has_a_retry_even_after_setting_was_reverted() {
        let state = ImplementationSwitch::default();
        state.begin(Tauri, HandyKeys).unwrap();
        assert!(state.cleanup(Tauri, || Err("回退清理失败".into())).is_err());
        assert_eq!(state.begin(Tauri, Tauri).unwrap(), Begin::ResumeCleanup);
        state.cleanup(Tauri, || Ok(())).unwrap();
        state.complete().unwrap();
        assert_eq!(state.begin(Tauri, HandyKeys).unwrap(), Begin::Fresh);
    }

    #[test]
    fn in_flight_request_cannot_be_replayed_or_hold_stage_lock_during_effect() {
        let state = ImplementationSwitch::default();
        state.begin(HandyKeys, Tauri).unwrap();
        state
            .cleanup(Tauri, || {
                assert!(state.begin(Tauri, Tauri).is_err());
                Ok(())
            })
            .unwrap();
        assert!(state.begin(Tauri, HandyKeys).is_err());
        state.complete().unwrap();
    }
}
