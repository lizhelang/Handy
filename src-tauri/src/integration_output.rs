//! 原生目标注册表只存在于主线程；后台输出账本不持有 AX 指针。

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
#[cfg(not(target_os = "macos"))]
pub fn current_target() -> Option<String> {
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
