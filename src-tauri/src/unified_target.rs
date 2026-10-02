//! macOS 输出目标的进程内租约。普通目标接口只读取 AX 身份、角色和选区元数据。
//! 独立的学习确认适配器仅在已授权的提交许可中读取有界精确范围，不读取全文。
//!
//! 必须在主线程建立、调用和销毁；主 CFRunLoop 必须持续运行。它不是按键路径 API。
//! 注册表、观察器及 AX 强引用不跨 IPC；外部只能使用不可猜测的 opaque ID。
//! Ready 仅证明提交前这一刻目标可核验，不代表文本已经写入，也不提供组合状态证明。
#![cfg(target_os = "macos")]

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void};
use std::io::Read;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Ref = *const c_void;
type AxCallback = unsafe extern "C" fn(Ref, Ref, Ref, *mut c_void);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> Ref;
    fn AXUIElementCreateApplication(pid: c_int) -> Ref;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementGetPid(element: Ref, pid: *mut c_int) -> c_int;
    fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> c_int;
    fn AXUIElementSetMessagingTimeout(element: Ref, timeout: f32) -> c_int;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: Ref,
        attribute: Ref,
        parameter: Ref,
        value: *mut Ref,
    ) -> c_int;
    fn AXValueCreate(kind: u32, value: *const c_void) -> Ref;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetType(value: Ref) -> u32;
    fn AXValueGetValue(value: Ref, kind: u32, result: *mut c_void) -> u8;
    fn AXObserverCreate(pid: c_int, callback: AxCallback, result: *mut Ref) -> c_int;
    fn AXObserverAddNotification(
        observer: Ref,
        element: Ref,
        name: Ref,
        context: *mut c_void,
    ) -> c_int;
    fn AXObserverGetRunLoopSource(observer: Ref) -> Ref;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFEqual(left: Ref, right: Ref) -> u8;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: Ref) -> isize;
    fn CFStringGetCharacters(value: Ref, range: Selection, buffer: *mut u16);
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(value: Ref, kind: c_int, buffer: *mut c_void) -> u8;
    fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
    fn CFRunLoopGetMain() -> Ref;
    fn CFRunLoopAddSource(loop_ref: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRemoveSource(loop_ref: Ref, source: Ref, mode: Ref);
    static kCFRunLoopCommonModes: Ref;
    static kCFBooleanTrue: Ref;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

unsafe extern "C" {
    fn pthread_main_np() -> c_int;
    fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int)
        -> c_int;
}

// SDK sys/proc_info.h: proc_bsdinfo / PROC_PIDTBSDINFO=3; MAXCOMLEN=16。
#[repr(C)]
#[derive(Default)]
struct BsdInfo {
    flags: u32,
    status: u32,
    xstatus: u32,
    pid: u32,
    ppid: u32,
    uid: u32,
    gid: u32,
    ruid: u32,
    rgid: u32,
    svuid: u32,
    svgid: u32,
    reserved: u32,
    comm: [u8; 16],
    name: [u8; 32],
    nfiles: u32,
    pgid: u32,
    jobc: u32,
    tdev: u32,
    tpgid: u32,
    nice: i32,
    start_sec: u64,
    start_usec: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub location: isize,
    pub length: isize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessInstance {
    pub pid: u32,
    pub start_seconds: u64,
    pub start_microseconds: u64,
}

/// API 无法证明时的具体原因；不得把这些情况转换为自动 paste。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingReason {
    NotMainThread,
    AccessibilityUnavailable,
    SecureInput,
    UnknownTarget,
    UnsupportedControl,
    UnobservableControl,
    ProcessChanged,
    FocusChanged,
    Edited,
    Destroyed,
    Expired,
    SuspendedByOwner,
    RegistryFull,
    EntropyUnavailable,
}

/// 不含 AX 指针或正文；只有本注册表持有的 ID 才能再次验证。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub opaque_id: String,
    pub process: ProcessInstance,
    pub selection: Selection,
    pub focus_generation: u64,
    pub edit_generation: u64,
}

thread_local! { static QUERY_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) }; }
/// A deadline bounds the total query sequence as well as the per-message AX timeout.
pub fn with_query_budget<T>(budget: Duration, action: impl FnOnce() -> T) -> T {
    struct Restore(Option<Instant>);
    impl Drop for Restore {
        fn drop(&mut self) {
            QUERY_DEADLINE.with(|slot| slot.set(self.0));
        }
    }
    let _restore = Restore(QUERY_DEADLINE.with(|slot| slot.replace(Some(Instant::now() + budget))));
    action()
}
fn query_budget() -> Result<(), PendingReason> {
    if QUERY_DEADLINE.with(|slot| {
        slot.get()
            .is_some_and(|deadline| Instant::now() >= deadline)
    }) {
        Err(PendingReason::Expired)
    } else {
        Ok(())
    }
}

struct Owned(Ref);

impl Owned {
    fn from_created(value: Ref) -> Result<Self, PendingReason> {
        if value.is_null() {
            Err(PendingReason::UnknownTarget)
        } else {
            Ok(Self(value))
        }
    }

    fn attribute(&self, name: &'static [u8]) -> Result<Self, PendingReason> {
        query_budget()?;
        let key = cf_string(name)?;
        let mut value = ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(self.0, key.0, &mut value) };
        if status != 0 {
            // 仅记录固定属性名和系统错误码，不记录窗口、字段或正文内容。
            log::debug!(
                "unified_target_attribute_failed attribute={} status={}",
                String::from_utf8_lossy(name).trim_end_matches('\0'),
                status
            );
            // 防御不符合 Copy 合同的失败返回，仍释放可能分配的值。
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return Err(PendingReason::UnknownTarget);
        }
        let value = Self::from_created(value)?;
        query_budget()?;
        Ok(value)
    }

    fn ax_attribute(&self, name: &'static [u8]) -> Result<Self, PendingReason> {
        let value = self.attribute(name)?;
        if unsafe { CFGetTypeID(value.0) != AXUIElementGetTypeID() } {
            return Err(PendingReason::UnknownTarget);
        }
        if unsafe { AXUIElementSetMessagingTimeout(value.0, 0.05) } != 0 {
            return Err(PendingReason::UnknownTarget);
        }
        Ok(value)
    }

    fn equals(&self, other: &Self) -> bool {
        unsafe { CFEqual(self.0, other.0) != 0 }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

fn cf_string(value: &'static [u8]) -> Result<Owned, PendingReason> {
    debug_assert_eq!(value.last(), Some(&0));
    Owned::from_created(unsafe {
        CFStringCreateWithCString(ptr::null(), value.as_ptr().cast(), 0x0800_0100)
    })
}

fn main_thread() -> Result<(), PendingReason> {
    if unsafe { pthread_main_np() } == 1 {
        Ok(())
    } else {
        Err(PendingReason::NotMainThread)
    }
}

fn privacy_gate() -> Result<(), PendingReason> {
    if unsafe { IsSecureEventInputEnabled() } != 0 {
        return Err(PendingReason::SecureInput);
    }
    // Broker queries already carry a lifecycle permission proof; avoid synchronous TCC here.
    // Standalone legacy callers retain their passive permission check.
    if QUERY_DEADLINE.with(|slot| slot.get().is_none()) && unsafe { AXIsProcessTrusted() } == 0 {
        return Err(PendingReason::AccessibilityUnavailable);
    }
    Ok(())
}

pub fn process_instance(pid: u32) -> Result<ProcessInstance, PendingReason> {
    if pid == 0 || pid > c_int::MAX as u32 {
        return Err(PendingReason::ProcessChanged);
    }
    let mut info = BsdInfo::default();
    let size = std::mem::size_of::<BsdInfo>() as c_int;
    let bytes =
        unsafe { proc_pidinfo(pid as c_int, 3, 0, (&mut info as *mut BsdInfo).cast(), size) };
    if bytes != size || info.pid != pid || info.start_sec == 0 {
        return Err(PendingReason::ProcessChanged);
    }
    Ok(ProcessInstance {
        pid,
        start_seconds: info.start_sec,
        start_microseconds: info.start_usec,
    })
}

fn pid(element: &Owned) -> Result<u32, PendingReason> {
    let mut result = 0;
    if unsafe { AXUIElementGetPid(element.0, &mut result) } != 0 || result <= 0 {
        return Err(PendingReason::UnknownTarget);
    }
    Ok(result as u32)
}

fn focused_app() -> Result<Owned, PendingReason> {
    let system = Owned::from_created(unsafe { AXUIElementCreateSystemWide() })?;
    // Broker owns passive AX access in this process; bound the system-wide call too.
    if unsafe { AXUIElementSetMessagingTimeout(system.0, 0.05) } != 0 {
        return Err(PendingReason::UnknownTarget);
    }
    // This also establishes the process AX messaging default; child objects retain explicit limits.
    // 同步 AX 请求只用于启动/提交阶段；不应从输入法 handleEvent 调用。
    system.ax_attribute(b"AXFocusedApplication\0")
}

/// Derive the source from the actual foreground AX application, never a client PID.
pub fn foreground_source(source: &str) -> Result<(ProcessInstance, String), PendingReason> {
    main_thread()?;
    query_budget()?;
    privacy_gate()?;
    let focused = focused_app()?;
    let actual_pid = pid(&focused)?;
    let application =
        NSRunningApplication::runningApplicationWithProcessIdentifier(actual_pid as i32)
            .ok_or(PendingReason::UnknownTarget)?;
    if application
        .bundleIdentifier()
        .map(|bundle| bundle.to_string())
        .as_deref()
        != Some(source)
    {
        return Err(PendingReason::FocusChanged);
    }
    let window = focused.ax_attribute(b"AXFocusedWindow\0")?;
    let title = window.attribute(b"AXTitle\0")?;
    if unsafe { CFGetTypeID(title.0) != CFStringGetTypeID() || CFStringGetLength(title.0) > 16_384 }
    {
        return Err(PendingReason::UnknownTarget);
    }
    let mut bytes = [0u8; 65_537];
    if unsafe {
        CFStringGetCString(
            title.0,
            bytes.as_mut_ptr().cast(),
            bytes.len() as isize,
            0x0800_0100,
        )
    } == 0
    {
        return Err(PendingReason::UnknownTarget);
    }
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(PendingReason::UnknownTarget)?;
    let title = std::str::from_utf8(&bytes[..end]).map_err(|_| PendingReason::UnknownTarget)?;
    query_budget()?;
    Ok((process_instance(actual_pid)?, title.to_owned()))
}

fn selection(element: &Owned) -> Result<Selection, PendingReason> {
    let value = element.attribute(b"AXSelectedTextRange\0")?;
    let mut result = Selection::default();
    if unsafe {
        CFGetTypeID(value.0) != AXValueGetTypeID()
            || AXValueGetType(value.0) != 4
            || AXValueGetValue(value.0, 4, (&mut result as *mut Selection).cast()) == 0
    } || result.location < 0
        || result.location == isize::MAX
        || result.length < 0
        || result.location.checked_add(result.length).is_none()
    {
        return Err(PendingReason::UnsupportedControl);
    }
    Ok(result)
}

fn check_control(element: &Owned) -> Result<(), PendingReason> {
    let subrole = element.attribute(b"AXSubrole\0")?;
    if subrole.equals(&cf_string(b"AXSecureTextField\0")?) {
        return Err(PendingReason::SecureInput);
    }
    let role = element.attribute(b"AXRole\0")?;
    if ![
        b"AXTextField\0".as_slice(),
        b"AXTextArea\0",
        b"AXComboBox\0",
    ]
    .iter()
    .any(|name| cf_string(name).is_ok_and(|expected| role.equals(&expected)))
    {
        return Err(PendingReason::UnsupportedControl);
    }
    let enabled = element.attribute(b"AXEnabled\0")?;
    if unsafe { CFEqual(enabled.0, kCFBooleanTrue) } == 0 {
        return Err(PendingReason::UnsupportedControl);
    }
    Ok(())
}

struct Observation {
    app: Owned,
    field: Owned,
    window: Owned,
    owner_pid: u32,
    activation: Arc<Activation>,
    focus_generation: Cell<u64>,
    edit_generation: Cell<u64>,
    invalid: Cell<Option<PendingReason>>,
}

struct Activation {
    target_pid: u32,
    owner_pid: u32,
    overlay_armed: AtomicBool,
    invalid: AtomicBool,
}

impl Activation {
    fn activated(&self, actual_pid: Option<u32>) {
        let permitted = actual_pid == Some(self.target_pid)
            || (actual_pid == Some(self.owner_pid) && self.overlay_armed.load(Ordering::SeqCst));
        if !permitted {
            self.invalid.store(true, Ordering::SeqCst);
        }
    }
}

struct WorkspaceObservation {
    center: Retained<NSNotificationCenter>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl WorkspaceObservation {
    fn new(activation: Arc<Activation>) -> Self {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        Self::attach(center, activation)
    }

    fn attach(center: Retained<NSNotificationCenter>, activation: Arc<Activation>) -> Self {
        // 只捕获 Send+Sync 原子状态；读取事件当时的 App 而非回调时的前台 App。
        let block = RcBlock::new(move |notification: std::ptr::NonNull<NSNotification>| {
            let actual_pid = unsafe { notification.as_ref() }
                .userInfo()
                .and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }))
                .and_then(|object| object.downcast::<NSRunningApplication>().ok())
                .map(|app| app.processIdentifier() as u32);
            activation.activated(actual_pid);
        });
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        Self { center, observer }
    }
}

impl Drop for WorkspaceObservation {
    fn drop(&mut self) {
        unsafe { self.center.removeObserver((*self.observer).as_ref()) };
    }
}

impl Observation {
    fn invalidate(&self, reason: PendingReason) {
        // 饱和代数仍伴随不可恢复的 invalid，不允许溢出后重用旧 token。
        if reason == PendingReason::Edited {
            self.edit_generation
                .set(self.edit_generation.get().saturating_add(1));
        } else {
            self.focus_generation
                .set(self.focus_generation.get().saturating_add(1));
        }
        if self.invalid.get().is_none()
            || (self.invalid.get() == Some(PendingReason::Edited)
                && reason != PendingReason::Edited)
        {
            self.invalid.set(Some(reason));
        }
    }
}

unsafe extern "C" fn observe(_observer: Ref, element: Ref, name: Ref, context: *mut c_void) {
    // Box 地址固定，RunLoop source 在 Box 销毁前移除。回调和所有方法均在主线程。
    let state = unsafe { &*context.cast::<Observation>() };
    let matches = |expected| cf_string(expected).is_ok_and(|s| unsafe { CFEqual(name, s.0) != 0 });
    if matches(b"AXUIElementDestroyed\0") {
        state.invalidate(PendingReason::Destroyed);
    } else if matches(b"AXValueChanged\0") || matches(b"AXSelectedTextChanged\0") {
        state.invalidate(PendingReason::Edited);
    } else if matches(b"AXFocusedUIElementChanged\0") {
        // SDK 保证 element 是事件发生时的新焦点，而非处理回调时的焦点。
        // 因此 A→B→A 即使回调延迟到 A 也不会遗漏 B。
        if unsafe { CFEqual(element, state.field.0) } == 0 {
            let overlay_focus_loss = state.activation.overlay_armed.load(Ordering::SeqCst)
                && unsafe { CFEqual(element, state.app.0) } != 0
                && focused_app().and_then(|app| pid(&app)).ok() == Some(state.owner_pid);
            if !overlay_focus_loss {
                state.invalidate(PendingReason::FocusChanged);
            }
        }
    } else {
        // 保留事件时的窗口身份；同 App 快速切窗后切回也必须能失效。
        if unsafe { CFEqual(element, state.window.0) } == 0 {
            state.invalidate(PendingReason::FocusChanged);
            return;
        }
        let field_matches = state
            .app
            .ax_attribute(b"AXFocusedUIElement\0")
            .is_ok_and(|field| field.equals(&state.field));
        let window_matches = state
            .app
            .ax_attribute(b"AXFocusedWindow\0")
            .is_ok_and(|window| window.equals(&state.window));
        if !field_matches || !window_matches {
            state.invalidate(PendingReason::FocusChanged);
        }
    }
}

struct Lease {
    learning_field_instance: String,
    observer: Owned,
    _workspace: WorkspaceObservation,
    state: Box<Observation>,
    snapshot: TargetSnapshot,
    deadline: Instant,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // observer 先释放，再释放回调 refcon；移除整个 source 不依赖目标仍然活着。
        unsafe {
            CFRunLoopRemoveSource(
                CFRunLoopGetMain(),
                AXObserverGetRunLoopSource(self.observer.0),
                kCFRunLoopCommonModes,
            );
        }
    }
}

/// 有限容量、不可 Send/Sync 的主线程注册表。销毁/忘记即释放观察器和 AX 引用。
pub struct TargetRegistry {
    nonce: String,
    serial: u64,
    leases: BTreeMap<String, Lease>,
    _main_only: PhantomData<Rc<()>>,
}

impl TargetRegistry {
    pub fn new() -> Result<Self, PendingReason> {
        main_thread()?;
        let mut bytes = [0_u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|_| PendingReason::EntropyUnavailable)?;
        Ok(Self {
            nonce: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
            serial: 0,
            leases: BTreeMap::new(),
            _main_only: PhantomData,
        })
    }

    /// 在打开面板/开始语音之前捕获。ttl 上限 120 秒；过期必须重新捕获当前目标。
    pub fn capture(&mut self, ttl: Duration) -> Result<TargetSnapshot, PendingReason> {
        main_thread()?;
        privacy_gate()?;
        self.prune()?;
        if self.leases.len() >= 32 {
            return Err(PendingReason::RegistryFull);
        }
        let app = focused_app()?;
        let target_pid = pid(&app)?;
        if target_pid == std::process::id() {
            return Err(PendingReason::UnknownTarget);
        }
        let process = process_instance(target_pid)?;
        let field = app.ax_attribute(b"AXFocusedUIElement\0")?;
        if pid(&field)? != target_pid {
            return Err(PendingReason::UnknownTarget);
        }
        check_control(&field)?;
        let window = app.ax_attribute(b"AXFocusedWindow\0")?;
        let activation = Arc::new(Activation {
            target_pid,
            owner_pid: std::process::id(),
            overlay_armed: AtomicBool::new(false),
            invalid: AtomicBool::new(false),
        });
        let workspace = WorkspaceObservation::new(activation.clone());
        let mut state = Box::new(Observation {
            app,
            field,
            window,
            owner_pid: std::process::id(),
            activation,
            focus_generation: Cell::new(0),
            edit_generation: Cell::new(0),
            invalid: Cell::new(None),
        });
        let mut observer = ptr::null();
        if unsafe { AXObserverCreate(target_pid as c_int, observe, &mut observer) } != 0 {
            return Err(PendingReason::UnobservableControl);
        }
        let observer = Owned::from_created(observer)?;
        let context = (&mut *state as *mut Observation).cast();
        for (element, name) in [
            (state.app.0, b"AXFocusedUIElementChanged\0".as_slice()),
            (state.app.0, b"AXFocusedWindowChanged\0"),
            (state.field.0, b"AXValueChanged\0"),
            (state.field.0, b"AXSelectedTextChanged\0"),
            (state.field.0, b"AXUIElementDestroyed\0"),
        ] {
            let name = cf_string(name)?;
            if unsafe { AXObserverAddNotification(observer.0, element, name.0, context) } != 0 {
                return Err(PendingReason::UnobservableControl);
            }
        }
        let selected = selection(&state.field)?;
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or(PendingReason::RegistryFull)?;
        let snapshot = TargetSnapshot {
            opaque_id: format!("{}-{}", self.nonce, self.serial),
            process,
            selection: selected,
            focus_generation: 0,
            edit_generation: 0,
        };
        let learning_field_instance = self
            .leases
            .values()
            .find(|lease| {
                lease.snapshot.process == snapshot.process
                    && lease.state.focus_generation.get() == lease.snapshot.focus_generation
                    && !lease.state.activation.invalid.load(Ordering::SeqCst)
                    && lease
                        .state
                        .invalid
                        .get()
                        .is_none_or(|reason| reason == PendingReason::Edited)
                    && lease.state.field.equals(&state.field)
                    && lease.state.window.equals(&state.window)
            })
            .map(|lease| lease.learning_field_instance.clone())
            .unwrap_or_else(|| snapshot.opaque_id.clone());
        let lease = Lease {
            learning_field_instance,
            observer,
            _workspace: workspace,
            state,
            snapshot: snapshot.clone(),
            deadline: Instant::now() + ttl.min(Duration::from_secs(120)),
        };
        unsafe {
            CFRunLoopAddSource(
                CFRunLoopGetMain(),
                AXObserverGetRunLoopSource(lease.observer.0),
                kCFRunLoopCommonModes,
            )
        };
        self.leases.insert(snapshot.opaque_id.clone(), lease);
        // 安装观察期间发生的当前目标变化也必须拒绝。
        let result = self.validate(&snapshot.opaque_id);
        if result.is_err() {
            self.leases.remove(&snapshot.opaque_id);
        }
        result
    }

    /// 仅在 Handy 即将展示自己的面板时调用；其他 App 激活不能被此声明豁免。
    pub fn arm_owner_overlay(&self, id: &str) -> Result<(), PendingReason> {
        self.validate(id)?;
        self.leases
            .get(id)
            .ok_or(PendingReason::UnknownTarget)?
            .state
            .activation
            .overlay_armed
            .store(true, Ordering::SeqCst);
        Ok(())
    }

    /// 必须在实际派发前同一个主线程任务里再次调用；不得把结果缓存后异步派发。
    pub fn validate(&self, id: &str) -> Result<TargetSnapshot, PendingReason> {
        self.validate_identity(id, false)
    }

    /// 返回注册表租约的绝对截止时间；实际派发前仍需重新调用 `validate`。
    pub fn deadline_unix_ms(&self, id: &str) -> Result<u64, PendingReason> {
        main_thread()?;
        let lease = self.leases.get(id).ok_or(PendingReason::UnknownTarget)?;
        let remaining = lease.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(PendingReason::Expired);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PendingReason::Expired)?;
        Ok(now
            .as_millis()
            .saturating_add(remaining.as_millis())
            .min(u64::MAX as u128) as u64)
    }

    /// 键入后的来源证明允许正文/选区变化，但必须仍是原字段、窗口和进程。
    pub fn validate_typed_field(&self, id: &str) -> Result<TargetSnapshot, PendingReason> {
        self.validate_identity(id, true)
    }

    fn validate_identity(
        &self,
        id: &str,
        allow_edits: bool,
    ) -> Result<TargetSnapshot, PendingReason> {
        main_thread()?;
        privacy_gate()?;
        let lease = self.leases.get(id).ok_or(PendingReason::UnknownTarget)?;
        if Instant::now() >= lease.deadline {
            return Err(PendingReason::Expired);
        }
        if let Some(reason) = lease.state.invalid.get() {
            if !allow_edits || reason != PendingReason::Edited {
                return Err(reason);
            }
        }
        if lease.state.activation.invalid.load(Ordering::SeqCst) {
            lease.state.invalidate(PendingReason::FocusChanged);
            return Err(PendingReason::FocusChanged);
        }
        if process_instance(lease.snapshot.process.pid)? != lease.snapshot.process {
            lease.state.invalidate(PendingReason::ProcessChanged);
            return Err(PendingReason::ProcessChanged);
        }
        if allow_edits && lease.state.focus_generation.get() != lease.snapshot.focus_generation {
            return Err(PendingReason::FocusChanged);
        }
        let app = focused_app()?;
        let actual_pid = pid(&app)?;
        if actual_pid == lease.state.owner_pid
            && lease.state.activation.overlay_armed.load(Ordering::SeqCst)
        {
            return Err(PendingReason::SuspendedByOwner);
        }
        if actual_pid != lease.snapshot.process.pid {
            lease.state.invalidate(PendingReason::FocusChanged);
            return Err(PendingReason::FocusChanged);
        }
        let field = app.ax_attribute(b"AXFocusedUIElement\0")?;
        let window = app.ax_attribute(b"AXFocusedWindow\0")?;
        if !field.equals(&lease.state.field) || !window.equals(&lease.state.window) {
            lease.state.invalidate(PendingReason::FocusChanged);
            return Err(PendingReason::FocusChanged);
        }
        check_control(&field)?;
        if !allow_edits && selection(&field)? != lease.snapshot.selection {
            lease.state.invalidate(PendingReason::Edited);
            return Err(PendingReason::Edited);
        }
        // AX 查询可能让主 RunLoop 处理回调；检查过程中失效不得被返回的旧快照掩盖。
        privacy_gate()?;
        if let Some(reason) = lease.state.invalid.get() {
            if !allow_edits || reason != PendingReason::Edited {
                return Err(reason);
            }
        }
        if lease.state.activation.invalid.load(Ordering::SeqCst) {
            return Err(PendingReason::FocusChanged);
        }
        Ok(lease.snapshot.clone())
    }

    pub fn forget(&mut self, id: &str) -> Result<(), PendingReason> {
        main_thread()?;
        self.leases.remove(id);
        Ok(())
    }

    pub fn prune(&mut self) -> Result<(), PendingReason> {
        main_thread()?;
        let now = Instant::now();
        self.leases.retain(|_, lease| now < lease.deadline);
        Ok(())
    }
}

/// 学习确认专用；调用方先验证已认证owner、当前设置和权限epoch，且保持主线程查询预算。
pub(crate) struct NativeLearningObserver<'a> {
    registry: &'a TargetRegistry,
    id: &'a str,
}
impl TargetRegistry {
    pub(crate) fn learning_field_instance(&self, id: &str) -> Result<String, PendingReason> {
        self.validate_typed_field(id)?;
        self.leases
            .get(id)
            .map(|lease| lease.learning_field_instance.clone())
            .ok_or(PendingReason::UnknownTarget)
    }
    pub(crate) fn learning_observer<'a>(&'a self, id: &'a str) -> NativeLearningObserver<'a> {
        NativeLearningObserver { registry: self, id }
    }
}
impl inputia_handy_runtime::memory_commit::LearningObserver for NativeLearningObserver<'_> {
    fn checkpoint(
        &mut self,
    ) -> Result<inputia_handy_runtime::memory_commit::FieldCheckpoint, &'static str> {
        use inputia_handy_runtime::memory_commit::{FieldCheckpoint, TextRange};
        query_budget().map_err(|_| "memory_target_timeout")?;
        self.registry
            .validate_typed_field(self.id)
            .map_err(|_| "memory_target_unverified")?;
        let lease = self
            .registry
            .leases
            .get(self.id)
            .ok_or("memory_target_unverified")?;
        let before_edit = lease.state.edit_generation.get();
        let selected = selection(&lease.state.field).map_err(|_| "memory_range_unavailable")?;
        let count = lease
            .state
            .field
            .attribute(b"AXNumberOfCharacters\0")
            .map_err(|_| "memory_range_unavailable")?;
        let mut length = 0i64;
        if unsafe {
            CFGetTypeID(count.0) != CFNumberGetTypeID()
                || CFNumberGetValue(count.0, 4, (&mut length as *mut i64).cast()) == 0
        } || length < 0
            || selected.location as u64 + selected.length as u64 > length as u64
        {
            return Err("memory_range_unavailable");
        }
        self.registry
            .validate_typed_field(self.id)
            .map_err(|_| "memory_target_unverified")?;
        if before_edit != lease.state.edit_generation.get()
            || selected != selection(&lease.state.field).map_err(|_| "memory_range_unavailable")?
        {
            return Err("memory_target_changed");
        }
        query_budget().map_err(|_| "memory_target_timeout")?;
        if before_edit != lease.state.edit_generation.get()
            || lease.state.activation.invalid.load(Ordering::SeqCst)
            || lease.state.focus_generation.get() != lease.snapshot.focus_generation
        {
            return Err("memory_target_changed");
        }
        Ok(FieldCheckpoint {
            field_instance: lease.learning_field_instance.clone(),
            selection: TextRange {
                location: selected.location as u64,
                length: selected.length as u64,
            },
            document_units: length as u64,
            focus_generation: lease.state.focus_generation.get(),
            edit_generation: before_edit,
        })
    }
    fn read_range(
        &mut self,
        range: inputia_handy_runtime::memory_commit::TextRange,
    ) -> Result<Vec<u16>, &'static str> {
        if range.length > inputia_handy_runtime::memory_commit::MAX_TEXT_BYTES as u64
            || range.end()? > isize::MAX as u64
        {
            return Err("memory_range_invalid");
        }
        let before = self.checkpoint()?;
        if range.end()? > before.document_units {
            return Err("memory_range_invalid");
        }
        let lease = self
            .registry
            .leases
            .get(self.id)
            .ok_or("memory_target_unverified")?;
        let native = Selection {
            location: range.location as isize,
            length: range.length as isize,
        };
        let parameter =
            Owned::from_created(unsafe { AXValueCreate(4, (&native as *const Selection).cast()) })
                .map_err(|_| "memory_range_unavailable")?;
        let attribute = cf_string(b"AXStringForRange\0").map_err(|_| "memory_range_unavailable")?;
        let mut value = ptr::null();
        query_budget().map_err(|_| "memory_target_timeout")?;
        let status = unsafe {
            AXUIElementCopyParameterizedAttributeValue(
                lease.state.field.0,
                attribute.0,
                parameter.0,
                &mut value,
            )
        };
        if status != 0 {
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return Err("memory_range_unsupported");
        }
        let value = Owned::from_created(value).map_err(|_| "memory_range_unavailable")?;
        if unsafe {
            CFGetTypeID(value.0) != CFStringGetTypeID()
                || CFStringGetLength(value.0) != native.length
        } {
            return Err("memory_range_invalid");
        }
        let mut units = vec![0u16; range.length as usize];
        unsafe {
            CFStringGetCharacters(
                value.0,
                Selection {
                    location: 0,
                    length: native.length,
                },
                units.as_mut_ptr(),
            )
        };
        if self.checkpoint()? != before {
            return Err("memory_target_changed");
        }
        query_budget().map_err(|_| "memory_target_timeout")?;
        Ok(units)
    }
}

/// 只查询自身 PID 和新建的自身 AX 代理，不触及用户窗口/输入源/剪贴板。
/// 独立 probe 使用它核验 FFI 布局、内核进程实例和 CF 对象身份。
pub fn metadata_self_check() -> Result<(), PendingReason> {
    main_thread()?;
    let instance = process_instance(std::process::id())?;
    if process_instance(instance.pid)? != instance || process_instance(0).is_ok() {
        return Err(PendingReason::ProcessChanged);
    }
    let first =
        Owned::from_created(unsafe { AXUIElementCreateApplication(instance.pid as c_int) })?;
    let second =
        Owned::from_created(unsafe { AXUIElementCreateApplication(instance.pid as c_int) })?;
    if !first.equals(&second) || pid(&first)? != instance.pid {
        return Err(PendingReason::UnknownTarget);
    }
    let registry = TargetRegistry::new()?;
    if registry.nonce.len() != 64 || !registry.leases.is_empty() {
        return Err(PendingReason::EntropyUnavailable);
    }
    let activation = Arc::new(Activation {
        target_pid: 42,
        owner_pid: instance.pid,
        overlay_armed: AtomicBool::new(true),
        invalid: AtomicBool::new(false),
    });
    activation.activated(Some(42));
    activation.activated(Some(instance.pid));
    activation.activated(Some(42));
    if activation.invalid.load(Ordering::SeqCst) {
        return Err(PendingReason::FocusChanged);
    }
    // 真正的 Foundation observer/block/dictionary 回调路径，局限在新建私有 center。
    let center = NSNotificationCenter::new();
    let watcher = WorkspaceObservation::attach(center.clone(), activation.clone());
    unsafe {
        center.postNotificationName_object_userInfo(
            NSWorkspaceDidActivateApplicationNotification,
            None,
            None,
        )
    };
    if !activation.invalid.load(Ordering::SeqCst) {
        return Err(PendingReason::UnobservableControl);
    }
    drop(watcher);
    activation.invalid.store(false, Ordering::SeqCst);
    unsafe {
        center.postNotificationName_object_userInfo(
            NSWorkspaceDidActivateApplicationNotification,
            None,
            None,
        )
    };
    if activation.invalid.load(Ordering::SeqCst) {
        return Err(PendingReason::UnobservableControl);
    }
    activation.activated(Some(7));
    activation.activated(Some(42));
    if !activation.invalid.load(Ordering::SeqCst) {
        return Err(PendingReason::FocusChanged);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activation(armed: bool) -> Activation {
        Activation {
            target_pid: 42,
            owner_pid: 24,
            overlay_armed: AtomicBool::new(armed),
            invalid: AtomicBool::new(false),
        }
    }

    #[test]
    fn unrelated_activation_cannot_be_erased_by_returning_to_target() {
        let state = activation(true);
        for pid in [42, 24, 99, 24, 42] {
            state.activated(Some(pid));
        }
        assert!(state.invalid.load(Ordering::SeqCst));
    }

    #[test]
    fn only_explicit_owner_overlay_is_tolerated() {
        let state = activation(true);
        for pid in [42, 24, 42] {
            state.activated(Some(pid));
        }
        assert!(!state.invalid.load(Ordering::SeqCst));
        let state = activation(false);
        state.activated(Some(24));
        assert!(state.invalid.load(Ordering::SeqCst));
    }

    #[test]
    fn unknown_activation_is_not_a_valid_target() {
        let state = activation(true);
        state.activated(None);
        assert!(state.invalid.load(Ordering::SeqCst));
    }

    #[test]
    fn native_process_identity_rejects_invalid_pid() {
        assert!(process_instance(0).is_err());
        assert!(process_instance(u32::MAX).is_err());
        assert_eq!(
            process_instance(std::process::id()),
            process_instance(std::process::id())
        );
    }
}
