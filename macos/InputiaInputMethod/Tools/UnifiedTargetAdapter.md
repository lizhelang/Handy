# macOS 输出目标适配器：挂载合同与有限原生证据

本文件对应 `src-tauri/src/unified_target.rs`。当前交付为可编译的平台适配器；尚未接入 Handy 生命周期或输出调用，不能据此通过 P0/P3/A01/A02 的跨应用验收。

## 实现边界

- `TargetRegistry::new()` 只允许主线程，生成 256 bit 随机 registry nonce；每次 capture 获得不同 opaque ID。注册表不可 Send/Sync。最多 32 个目标，每个租约最多 120 秒，用单调时钟过期。
- `capture(ttl)` 在面板显示/语音开始**之前**读实际 AX focused application、focused UI element、focused window 的强引用；匹配真实 PID、内核 `proc_bsdinfo` 的进程启动秒/微秒，读取有效 selected-text range，检查角色、enabled、secure-text subrole 和 Secure Input。不读取 AXValue、selected text、标题或任何正文。
- AX observer 加入主 CFRunLoop 的 common modes；必须成功订阅 focused element、focused window、value change、selected-text change、destroyed。任一不可用返回 `UnobservableControl`，不能把无法观察的控件当可安全自动插入。
- focused-element callback 按 SDK 使用**事件当时**的新 AX element。不能只在回调到达时重读当前焦点，否则 A→B→A 会丢失 B。任何真实编辑/选区变化/控件销毁都会永久撤销该租约；返回原文本、原选区也不会恢复。
- `NSWorkspaceDidActivateApplicationNotification` 使用 userInfo 中的 `NSRunningApplication` 读取**事件当时**的 PID；因此从原 App 到 Handy、再到第三方 App、最后回原 App，第三方激活不会被返回原目标抹掉。它只捕获原子状态，通过主 NSOperationQueue 投递，不执行 AX 查询或读取正文。
- `arm_owner_overlay(id)` 只能在目标仍可验证时、展示自己面板之前调用。此后自己处于前台时 `validate` 返回 `SuspendedByOwner`；原结果仍保留，不强行激活旧 App。用户回原 App 时仍需核对同一个 AX 控件/窗口/选区/进程与观察状态。第三方激活会永久失效。AX 只报告失焦而无法归因时可能保守拒绝，应进入待插入。
- `validate(id)` 实时再查隐私、进程实例、AX 身份、选区及观察代数。它只证明派发前的目标检查，**不证明 IMK composition 状态，不证明文字上屏，也不能把 OS 查询和后续外部应用插入变成原子事务**。
- `forget(id)`、`prune()` 及 Drop 会释放强引用。Drop 先把 AX source 从主 RunLoop 移除，再释放 observer/refcon；Workspace observer 也注销。opaque ID 不持久恢复，进程重启后旧 ID 失效。
- 快照的 focus/edit generation 从该租约的 0 开始，观察变更将代数递增并永久失效；新 capture 用新的 opaque ID。没有返回恒定有效的占位分支。

## root 挂载位置

1. 在 `lib.rs` 加 `#[cfg(target_os = "macos")] mod unified_target;`；本子任务没有改共享文件。
2. 在主线程使用 `thread_local!` 的 `RefCell<Option<TargetRegistry>>` 建立 registry；Tauri 的后台命令以 `run_on_main_thread` 访问该 thread-local，不能把 registry 放进要求 Send+Sync 的全局 manager。
3. 将 capture 返回的 opaque ID 与控制器/Host 实例、会话 ID 绑定，后台只传这些不含正文的元数据。核心 `TargetToken.field_id` 可以使用 opaque ID；`process_start_id` 由 PID/启动秒/微秒编码；核心有效期还需在调用层记录，不替代注册表的单调时钟过期。
4. 原生派发前在主线程再次 `validate`，同时在输出协调层验证 composition、policy epoch、操作所有者与持久账本。跨 await 后必须再次验证，不能先 validate，再等待历史扫描/IPC/数据库工作，然后使用旧结果发按键。
5. 任意错误映射为待插入；`SuspendedByOwner` 可显示面板暂时持有焦点。只有输出所有者完成准备且账本记录 dispatched 后才能调用已有输出路径。此模块不发送事件、不修改剪贴板、不调用 insertText。
6. 注册表 prune 需要接入非按键路径的定时维护，以及会话取消/关闭时 forget。目标状态不能默认永远保留到 32 槽耗尽。

使用仓库已有 `objc2 0.6`、`block2 0.6`、`objc2-foundation 0.3`、`objc2-app-kit 0.3`，无新增依赖。CF/AX/proc FFI 根据本机 Apple SDK 声明实现；链接 ApplicationServices、CoreFoundation、Carbon。`AXUIElementSetMessagingTimeout` 仅用于非 system-wide 的返回 AX 对象；SDK 明确 system-wide timeout 会修改整个进程的默认值，故没有设置该全局值。外部 AX 仍是同步 API，启动/提交阶段可能等待，不能从输入法 `handleEvent` 热路径直接调用。

## 已运行的验证

命令（仓库根）：

```sh
bash macos/InputiaInputMethod/Tools/UnifiedTargetProbe.sh
```

脚本复用 `src-tauri/target/debug/deps` 中已有的唯一 `.rlib`，默认使用对应的 Rust 1.96.0，带 `-D warnings`；可用 `UNIFIED_TARGET_TOOLCHAIN` 和 `UNIFIED_TARGET_DEPS_DIR` 指向匹配的隔离编译依赖。它在 mktemp 目录编译，不覆盖日常安装或工作树产物。

实际输出：

```text
unified_target_metadata_self_check=pass external_focus_read=false text_read=false input_posted=false
test result: ok. 4 passed; 0 failed; 0 ignored
evidence_executables=/tmp/handy-unified-target.9Lhzl4
```

原生 self-check 实际调用 `proc_pidinfo` 查询本进程，核实二次取得的启动身份一致，核实 invalid PID 拒绝；创建本进程两个 AX application 代理，通过真实 CFEqual 与 AXUIElementGetPid 检查身份。另在全新私有 NSNotificationCenter 注册真实 Objective-C block observer、发送不含用户信息的通知，验证 fail-closed 回调和 removeObserver 后不再触发。没有向真正的 NSWorkspace notification center 注入合成事件。

4 个回归覆盖第三方激活后返回仍失效、只有显式 owner overlay 可豁免、未知激活失效和内核 PID 身份。这些事件序列为合成状态测试，不能宣称原生跨 App 场景已通过。

第一手 API 依据：

- 本机 macOS SDK `HIServices.framework/Headers/AXUIElement.h`：AX Copy 所有权、AXObserver/refcon、run-loop source、notification unsupported、MessagingTimeout 作用域。
- 同目录 `AXNotificationConstants.h`：`AXFocusedUIElementChanged` 的值是新焦点 UIElement 或无焦点时的 Application；`AXValueChanged`、`AXSelectedTextChanged`、`AXUIElementDestroyed`。
- SDK `usr/include/sys/proc_info.h`：`PROC_PIDTBSDINFO=3` 和 `proc_bsdinfo` 布局。
- 已安装 objc2 生成绑定 `NSWorkspace.rs`、`NSNotification.rs`、`NSRunningApplication.rs`：notificationCenter、激活通知 userInfo key、block 注册/移除与 processIdentifier。

## 必须后续验证的项目

真实候选进程 TCC、AX observer 通知支持/投递、同 App 多输入框、快速切换、overlay 失焦与恢复、编辑/选区改后还原、窗口销毁、进程重启、Secure Input 开关、组合冲突和通知延迟。需针对 TextEdit、浏览器 textarea、Electron 编辑器分别进行，仍不得用这里的私有 center 或代理 identity 检查替代。当前没有运行外部焦点 capture，没有进行测试输入，没有实际控件观察证据。

额外审查注意：AX/Workspace 事件由系统异步投递，验证后到外部派发之间仍可能发生竞争；调用方须遵守单所有者/未知回执不重试合同，并在真实原生验收中确认通知时序。不支持必要观察能力的应用目前进入 PendingTarget，应由 Inputia 的真实目标/组合会话证明或用户在当前控件重新触发输入补足，不能在未知目标时强行 paste。
