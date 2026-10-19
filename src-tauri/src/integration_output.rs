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

/// 系统键盘布局提供保守的无组合证明；第三方输入法不能仅凭选中状态得到此证明。
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

/// 自动回车的可选精确字段证明；普通听写粘贴不依赖此能力。
#[cfg(target_os = "macos")]
#[derive(Clone)]
pub(crate) struct AutoSubmitTarget(std::sync::Arc<VoiceTargetLease>);

#[cfg(target_os = "macos")]
impl AutoSubmitTarget {
    /// 仅在用户启用自动发送时于主线程捕获；失败不能阻断普通文本粘贴。
    pub(crate) fn capture(app: AppHandle) -> Option<Self> {
        capture_voice_target(app).map(Self)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        TARGETS.with(|slot| {
            let slot = slot.borrow();
            let state = slot.as_ref().ok_or("auto_submit_target_unavailable")?;
            state
                .registry
                .validate_typed_field(&self.0.id)
                .map_err(|reason| format!("auto_submit_target_{reason:?}"))?;
            if !platform_composition_clear() {
                return Err("auto_submit_composition_unverified".into());
            }
            Ok(())
        })
    }
}
