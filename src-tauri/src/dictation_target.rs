//! 普通口述默认在实际发键时使用当前应用的输入位置，不要求 AX 字段。
//! 可选模式只固定开始录音的进程实例；不跟踪中途切换、不自动拉回应用、没有字段租约期限。

use crate::settings::DictationTargetMode;
use crate::unified_target::{process_instance, ProcessInstance};
use objc2_app_kit::NSWorkspace;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}
unsafe extern "C" {
    fn pthread_main_np() -> i32;
}

/// 模式在开始录音时冻结；普通应用身份不能当作原窗口或 AX 字段证明。
#[derive(Clone)]
pub(crate) enum DictationTarget {
    CurrentCursor,
    RecordingApplication(ProcessInstance),
}

fn require_main_thread() -> Result<(), String> {
    if unsafe { pthread_main_np() } != 1 {
        return Err("dictation_target_requires_main_thread".into());
    }
    Ok(())
}

fn foreground_pid() -> Result<u32, String> {
    let application = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or("dictation_foreground_unavailable")?;
    u32::try_from(application.processIdentifier())
        .map_err(|_| "dictation_foreground_unavailable".into())
}

fn foreground_process() -> Result<ProcessInstance, String> {
    process_instance(foreground_pid()?).map_err(|_| "dictation_process_unavailable".into())
}

fn validate_current_app(current_pid: u32, owner_pid: u32, secure: bool) -> Result<(), String> {
    if secure {
        return Err("dictation_secure_input".into());
    }
    if current_pid == owner_pid {
        return Err("dictation_foreground_is_inputia".into());
    }
    Ok(())
}

impl DictationTarget {
    /// 开始时独立检查安全输入；默认模式不读取前台，固定应用模式在主线程捕获进程身份。
    pub(crate) fn capture(mode: DictationTargetMode) -> Result<Self, String> {
        Self::capture_with(mode, unsafe { IsSecureEventInputEnabled() } != 0, || {
            require_main_thread()?;
            foreground_process()
        })
    }

    fn capture_with(
        mode: DictationTargetMode,
        secure: bool,
        capture: impl FnOnce() -> Result<ProcessInstance, String>,
    ) -> Result<Self, String> {
        // 捕获失败保留在本次上下文内，之后关闭安全输入也不补取目标或重放本次转写。
        if secure {
            return Err("dictation_secure_input".into());
        }
        match mode {
            DictationTargetMode::CurrentCursor => Ok(Self::CurrentCursor),
            DictationTargetMode::RecordingApplication => capture().map(Self::RecordingApplication),
        }
    }

    fn validate_foreground(
        &self,
        current: ProcessInstance,
        owner_pid: u32,
        secure: bool,
    ) -> Result<(), String> {
        validate_current_app(current.pid, owner_pid, secure)?;
        if let Self::RecordingApplication(origin) = self {
            if current != *origin {
                return Err("dictation_recording_application_not_foreground".into());
            }
        }
        Ok(())
    }

    fn validate_with(
        &self,
        pid: u32,
        owner_pid: u32,
        secure: bool,
        process: impl FnOnce(u32) -> Result<ProcessInstance, String>,
    ) -> Result<(), String> {
        validate_current_app(pid, owner_pid, secure)?;
        match self {
            Self::CurrentCursor => Ok(()),
            Self::RecordingApplication(_) => {
                self.validate_foreground(process(pid)?, owner_pid, secure)
            }
        }
    }

    /// 每次副作用前检查当下前台；固定应用模式允许中途切走再切回。
    pub(crate) fn validate(&self) -> Result<(), String> {
        require_main_thread()?;
        self.validate_with(
            foreground_pid()?,
            std::process::id(),
            unsafe { IsSecureEventInputEnabled() } != 0,
            |pid| process_instance(pid).map_err(|_| "dictation_process_unavailable".into()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(pid: u32) -> ProcessInstance {
        ProcessInstance {
            pid,
            start_seconds: 100,
            start_microseconds: 12,
        }
    }

    #[test]
    fn current_cursor_does_not_capture_starting_app_and_uses_dispatch_foreground() {
        let target =
            DictationTarget::capture_with(DictationTargetMode::CurrentCursor, false, || {
                panic!("默认模式不得读取开始录音时的应用")
            })
            .unwrap();
        for current in [app(42), app(99), app(42)] {
            assert!(target.validate_foreground(current, 7, false).is_ok());
        }
    }

    #[test]
    fn both_modes_started_during_secure_input_stay_rejected_after_it_ends() {
        for mode in [
            DictationTargetMode::CurrentCursor,
            DictationTargetMode::RecordingApplication,
        ] {
            let target = DictationTarget::capture_with(mode, true, || {
                panic!("开始安全输入已拒绝，不读取应用或进程")
            });
            assert_eq!(
                target.and_then(|target| target.validate_with(42, 7, false, |_| Ok(app(42)))),
                Err("dictation_secure_input".into()),
            );
        }
    }

    #[test]
    fn current_cursor_does_not_require_process_creation_time_at_dispatch() {
        assert!(DictationTarget::CurrentCursor
            .validate_with(42, 7, false, |_| panic!("当前光标不读取进程创建时间"))
            .is_ok());
        assert_eq!(
            DictationTarget::RecordingApplication(app(42)).validate_with(42, 7, false, |_| Err(
                "dictation_process_unavailable".into()
            )),
            Err("dictation_process_unavailable".into()),
        );
    }

    #[test]
    fn recording_application_accepts_return_to_origin_but_never_redirects_another_app() {
        let target =
            DictationTarget::capture_with(DictationTargetMode::RecordingApplication, false, || {
                Ok(app(42))
            })
            .unwrap();
        assert!(target.validate_foreground(app(42), 7, false).is_ok());
        assert_eq!(
            target.validate_foreground(app(99), 7, false),
            Err("dictation_recording_application_not_foreground".into())
        );
        assert!(target.validate_foreground(app(42), 7, false).is_ok());
        assert!(target
            .validate_foreground(
                ProcessInstance {
                    start_seconds: 101,
                    ..app(42)
                },
                7,
                false,
            )
            .is_err());
    }

    #[test]
    fn target_mode_is_frozen_even_if_settings_change_after_capture() {
        let mut setting = DictationTargetMode::RecordingApplication;
        let target = DictationTarget::capture_with(setting, false, || Ok(app(42))).unwrap();
        setting = DictationTargetMode::CurrentCursor;
        assert!(target.validate_foreground(app(99), 7, false).is_err());
        let next = DictationTarget::capture_with(setting, false, || panic!("无须捕获")).unwrap();
        assert!(next.validate_foreground(app(99), 7, false).is_ok());
    }

    #[test]
    fn both_modes_reject_secure_input_and_inputia_itself() {
        for target in [
            DictationTarget::CurrentCursor,
            DictationTarget::RecordingApplication(app(7)),
        ] {
            assert_eq!(
                target.validate_foreground(app(7), 7, false),
                Err("dictation_foreground_is_inputia".into())
            );
            assert_eq!(
                target.validate_foreground(app(42), 7, true),
                Err("dictation_secure_input".into())
            );
        }
    }

    #[test]
    fn missing_starting_foreground_only_blocks_recording_application_mode() {
        assert!(
            DictationTarget::capture_with(DictationTargetMode::CurrentCursor, false, || {
                Err("dictation_foreground_unavailable".into())
            })
            .is_ok()
        );
        assert!(DictationTarget::capture_with(
            DictationTargetMode::RecordingApplication,
            false,
            || { Err("dictation_foreground_unavailable".into()) }
        )
        .is_err());
    }

    #[test]
    fn background_context_does_not_carry_thread_bound_cocoa_objects() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DictationTarget>();
    }
}
