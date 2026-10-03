//! 原生目标注册表只存在于主线程；后台输出账本不持有 AX 指针。

use tauri::AppHandle;

#[cfg(target_os = "macos")]
use crate::unified_target::{PendingReason, TargetRegistry};
#[cfg(target_os = "macos")]
use std::cell::RefCell;

#[cfg(target_os = "macos")]
struct TargetState {
    registry: TargetRegistry,
    current: Option<String>,
}

#[cfg(target_os = "macos")]
thread_local! {static TARGETS:RefCell<Option<TargetState>>=const {RefCell::new(None)};}

/// 必须在自己的窗口取得焦点之前调用；失败不会回退为任意当前焦点。
#[cfg(target_os = "macos")]
pub fn capture_before_ui() {
    TARGETS.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let Ok(registry) = TargetRegistry::new() else {
                return;
            };
            *slot = Some(TargetState {
                registry,
                current: None,
            });
        }
        let Some(state) = slot.as_mut() else {
            return;
        };
        if let Some(id) = state.current.take() {
            let _ = state.registry.forget(&id);
        }
        if let Ok(snapshot) = state.registry.capture(std::time::Duration::from_secs(120)) {
            if state
                .registry
                .arm_owner_overlay(&snapshot.opaque_id)
                .is_ok()
            {
                state.current = Some(snapshot.opaque_id);
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn capture_before_ui() {}

#[cfg(target_os = "macos")]
pub fn current_target() -> Option<String> {
    TARGETS.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|state| state.current.clone())
    })
}
#[cfg(target_os = "macos")]
pub fn current_target_deadline() -> Option<u64> {
    TARGETS.with(|slot| {
        let slot = slot.borrow();
        let state = slot.as_ref()?;
        let id = state.current.as_deref()?;
        state.registry.deadline_unix_ms(id).ok()
    })
}
#[cfg(target_os = "macos")]
pub fn target_deadline(id: &str) -> Option<u64> {
    TARGETS.with(|slot| {
        let slot = slot.borrow();
        let state = slot.as_ref()?;
        state.registry.deadline_unix_ms(id).ok()
    })
}
#[cfg(not(target_os = "macos"))]
pub fn current_target() -> Option<String> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn current_target_deadline() -> Option<u64> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn target_deadline(_id: &str) -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub fn validate_target(id: &str) -> Result<(), String> {
    TARGETS.with(|slot| {
        let slot = slot.borrow();
        let state = slot.as_ref().ok_or_else(|| "unknown_target".to_owned())?;
        state
            .registry
            .validate(id)
            .map(|_| ())
            .map_err(|reason| match reason {
                PendingReason::SuspendedByOwner => "suspended_by_owner".to_owned(),
                _ => "target_unavailable_or_changed".to_owned(),
            })
    })
}
#[cfg(not(target_os = "macos"))]
pub fn validate_target(_id: &str) -> Result<(), String> {
    Err("target_adapter_unavailable".into())
}

pub fn forget_target(id: &str) {
    #[cfg(target_os = "macos")]
    TARGETS.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            let _ = state.registry.forget(id);
            if state.current.as_deref() == Some(id) {
                state.current = None;
            }
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = id;
}

/// 系统键盘布局可走平台路线；第三方输入法须由真实 IME 会话证明组合已结束。
#[cfg(target_os = "macos")]
pub fn platform_composition_clear() -> bool {
    use std::ffi::c_void;
    type CFRef = *const c_void;
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> CFRef;
        fn TISGetInputSourceProperty(source: CFRef, key: CFRef) -> CFRef;
        static kTISPropertyInputSourceType: CFRef;
        static kTISTypeKeyboardLayout: CFRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFEqual(a: CFRef, b: CFRef) -> u8;
        fn CFRelease(value: CFRef);
    }
    // SAFETY: Copy 返回拥有的引用，属性值只借用并在释放 source 前比较。
    unsafe {
        let source = TISCopyCurrentKeyboardInputSource();
        if source.is_null() {
            return false;
        }
        let kind = TISGetInputSourceProperty(source, kTISPropertyInputSourceType);
        let clear = !kind.is_null() && CFEqual(kind, kTISTypeKeyboardLayout) != 0;
        CFRelease(source);
        clear
    }
}
#[cfg(not(target_os = "macos"))]
pub fn platform_composition_clear() -> bool {
    false
}

/// 维护屏障在主线程调用；同时释放未成为current的旧浮窗目标。
pub fn invalidate_all_targets() {
    #[cfg(target_os = "macos")]
    TARGETS.with(|slot| {
        slot.borrow_mut().take();
    });
}

#[derive(Debug)]
pub(crate) enum MainThreadError {
    NotStarted,
    Unknown,
}

impl From<MainThreadError> for String {
    fn from(value: MainThreadError) -> Self {
        match value {
            MainThreadError::NotStarted => "output task cancelled before starting",
            MainThreadError::Unknown => "output task response is unknown",
        }
        .into()
    }
}

pub(crate) fn guarded_main_thread_call<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(crate::dispatch_gate::DispatchGate) -> T + Send + 'static,
) -> Result<T, MainThreadError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let gate = crate::dispatch_gate::DispatchGate::new(std::time::Duration::from_secs(3));
    let queued = gate.clone();
    app.run_on_main_thread(move || {
        if queued.start() {
            let _ = sender.send(work(queued));
        }
    })
    .map_err(|_| MainThreadError::NotStarted)?;
    receiver
        .recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| {
            if gate.cancel_pending() {
                MainThreadError::NotStarted
            } else {
                MainThreadError::Unknown
            }
        })
}

pub(crate) fn main_thread_call<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    guarded_main_thread_call(app, |_| work()).map_err(String::from)
}

/// 语音使用独立租约，打开历史浮窗不能覆盖正在录音的原字段。
#[cfg(target_os = "macos")]
fn capture_voice_target(app: AppHandle) -> Option<std::sync::Arc<VoiceTargetLease>> {
    TARGETS.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(TargetState {
                registry: TargetRegistry::new().ok()?,
                current: None,
            });
        }
        let state = slot.as_mut()?;
        let target = match state.registry.capture(std::time::Duration::from_secs(120)) {
            Ok(target) => target,
            Err(reason) => {
                log::debug!("platform_voice_target_capture_unavailable reason={reason:?}");
                return None;
            }
        };
        if state.registry.arm_owner_overlay(&target.opaque_id).is_err() {
            let _ = state.registry.forget(&target.opaque_id);
            return None;
        }
        Some(std::sync::Arc::new(VoiceTargetLease {
            app,
            id: target.opaque_id,
        }))
    })
}

#[cfg(target_os = "macos")]
struct VoiceTargetLease {
    app: AppHandle,
    id: String,
}

#[cfg(target_os = "macos")]
impl Drop for VoiceTargetLease {
    fn drop(&mut self) {
        let id = self.id.clone();
        // 释放失败最多保留到 registry 的 120 秒上限，不允许把新目标补给旧操作。
        let _ = self.app.run_on_main_thread(move || forget_target(&id));
    }
}

/// 由录音协调器每次 Start 创建，Stop 后移动给唯一输出 worker，不按快捷键名共享。
#[cfg(target_os = "macos")]
pub(crate) struct PlatformVoiceContext {
    target: Option<std::sync::Arc<VoiceTargetLease>>,
    policy_epoch: Option<u64>,
    permission_epoch: Result<u64, String>,
}

#[cfg(target_os = "macos")]
impl PlatformVoiceContext {
    pub(crate) fn capture(app: &AppHandle) -> Self {
        use tauri::Manager;
        let permission_epoch = crate::input_permission::capture_epoch();
        let target_app = app.clone();
        let target = main_thread_call(app, move || capture_voice_target(target_app))
            .ok()
            .flatten();
        let policy_epoch = app
            .try_state::<std::sync::Arc<crate::managers::integration::IntegrationManager>>()
            .and_then(|manager| manager.service.policy_epoch().ok());
        Self {
            target,
            policy_epoch,
            permission_epoch,
        }
    }
}

#[cfg(target_os = "macos")]
fn validate_voice_field(id: &str, allow_output_edits: bool) -> Result<(), String> {
    TARGETS.with(|slot| {
        let slot = slot.borrow();
        let state = slot.as_ref().ok_or("unknown_target")?;
        let result = if allow_output_edits {
            state.registry.validate_typed_field(id)
        } else {
            state.registry.validate(id)
        };
        result.map_err(|_| "target_unavailable_or_changed".to_owned())?;
        if !platform_composition_clear() {
            return Err("composition requires IME session".into());
        }
        Ok(())
    })
}

/// 普通语音与 Host/历史输出共用同一个账本和短期许可。
/// 只从后台调用；主线程仅执行目标核验和原生副作用，不等待 SQLite。
#[cfg(target_os = "macos")]
pub(crate) fn dispatch_saved_platform_result(
    app: AppHandle,
    context: Option<PlatformVoiceContext>,
    history_id: i64,
    text: String,
    is_cancelled: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<inputia_handy_runtime::output_ledger::OutputRecord, String> {
    use inputia_handy_runtime::output_ledger::{OutputAction, OutputOutcome, OutputState};
    use std::sync::Arc;
    use tauri::Manager;
    let service = app
        .try_state::<Arc<crate::managers::integration::IntegrationManager>>()
        .ok_or("unified history service unavailable")?
        .service
        .clone();
    let settings = crate::settings::get_settings(&app);
    let copy_only = settings.paste_method == crate::settings::PasteMethod::None
        && settings.clipboard_handling == crate::settings::ClipboardHandling::CopyToClipboard;
    let action = if copy_only {
        OutputAction::CopyPlainText
    } else {
        OutputAction::InsertText
    };
    let target = context.as_ref().and_then(|context| context.target.clone());
    let output = service.prepare_saved_platform_result(
        history_id,
        text.clone(),
        if copy_only {
            None
        } else {
            target.as_ref().map(|target| target.id.clone())
        },
        context.as_ref().and_then(|context| context.policy_epoch),
        action,
    )?;
    if output.state != OutputState::Prepared {
        return Ok(output);
    }
    let intent = output.intent;
    if is_cancelled() || settings.paste_method == crate::settings::PasteMethod::None && !copy_only {
        return service.finish_output(intent, OutputOutcome::Rejected);
    }
    let permission_epoch = context.and_then(|context| context.permission_epoch.ok());
    let Some(permission_epoch) = permission_epoch else {
        return service.finish_output(intent, OutputOutcome::PendingTarget);
    };
    if crate::input_permission::check_epoch(permission_epoch).is_err() {
        return service.finish_output(intent, OutputOutcome::PendingTarget);
    }
    if !copy_only {
        let ready_target = target.clone();
        let ready = main_thread_call(&app, move || {
            ready_target
                .as_ref()
                .is_some_and(|target| validate_voice_field(&target.id, false).is_ok())
        });
        if !matches!(ready, Ok(true)) {
            return service.finish_output(intent, OutputOutcome::PendingTarget);
        }
    }
    let Some(permit) = service.claim_output_with_permit(intent.clone())? else {
        return service
            .output_record(intent.operation_id)?
            .ok_or("output receipt unavailable".into());
    };
    let output_app = app.clone();
    let dispatched = guarded_main_thread_call(&app, move |gate| {
        let cancelled = is_cancelled.clone();
        let permission = permit.clone();
        let boundary_target = target.clone();
        let boundary = move |allow_output_edits| {
            crate::dispatch_gate::validate_output_boundary(
                || {
                    gate.check()?;
                    permission.check()?;
                    crate::input_permission::check_epoch(permission_epoch)?;
                    if cancelled() {
                        return Err("voice output cancelled".into());
                    }
                    Ok(())
                },
                || {
                    if copy_only {
                        return Ok(());
                    }
                    let target = boundary_target.as_ref().ok_or("unknown_target")?;
                    validate_voice_field(&target.id, allow_output_edits)
                },
            )
        };
        let final_text = if settings.append_trailing_space {
            format!("{text} ")
        } else {
            text
        };
        if copy_only {
            return crate::clipboard::copy_voice_text(&final_text, &output_app, &mut || {
                boundary(false)
            });
        }
        // 延迟补发 Enter 也必须仍属于本次字段/许可；回执未知不准换路线重派。
        let completion = boundary.clone();
        crate::clipboard::paste_voice_text(
            &final_text,
            &output_app,
            &settings,
            &mut || boundary(false),
            Box::new(move || completion(true)),
        )
    });
    let outcome = platform_output_outcome(dispatched, copy_only);
    service.finish_output(intent, outcome)
}

#[cfg(target_os = "macos")]
fn platform_output_outcome(
    result: Result<crate::paste_tx::HistoryPasteOutcome, MainThreadError>,
    copy_only: bool,
) -> inputia_handy_runtime::output_ledger::OutputOutcome {
    use crate::paste_tx::HistoryPasteOutcome;
    use inputia_handy_runtime::output_ledger::OutputOutcome;
    match result {
        Ok(HistoryPasteOutcome::Dispatched) if copy_only => OutputOutcome::Confirmed,
        Ok(HistoryPasteOutcome::Dispatched) => OutputOutcome::DispatchedOnly,
        Ok(HistoryPasteOutcome::NotDispatched(_)) | Err(MainThreadError::NotStarted) => {
            OutputOutcome::NotDispatchedPendingTarget
        }
        Ok(HistoryPasteOutcome::PossiblyDispatched(_)) | Err(MainThreadError::Unknown) => {
            OutputOutcome::Uncertain
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::paste_tx::HistoryPasteOutcome;
    use inputia_handy_runtime::output_ledger::OutputOutcome;

    #[test]
    fn platform_receipts_never_claim_an_application_accepted_text() {
        assert_eq!(
            platform_output_outcome(Ok(HistoryPasteOutcome::Dispatched), false),
            OutputOutcome::DispatchedOnly
        );
        assert_eq!(
            platform_output_outcome(Ok(HistoryPasteOutcome::Dispatched), true),
            OutputOutcome::Confirmed
        );
        for result in [
            Ok(HistoryPasteOutcome::PossiblyDispatched("fixture".into())),
            Err(MainThreadError::Unknown),
        ] {
            assert_eq!(
                platform_output_outcome(result, false),
                OutputOutcome::Uncertain
            );
        }
        for result in [
            Ok(HistoryPasteOutcome::NotDispatched("fixture".into())),
            Err(MainThreadError::NotStarted),
        ] {
            assert_eq!(
                platform_output_outcome(result, false),
                OutputOutcome::NotDispatchedPendingTarget
            );
        }
    }
}
