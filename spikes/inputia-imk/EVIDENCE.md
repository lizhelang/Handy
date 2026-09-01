# Inputia IMK Spike 证据

日期：2026-07-06

## 本机环境

- SDK：`/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk`
- 用户级安装位置：`~/Library/Input Methods/InputiaIMKSpike.app`
- 产物：`spikes/inputia-imk/build/InputiaIMKSpike.app`

## 官方 API 约束

Apple SDK 头文件 `TextInputSources.h` 明确：

- `TISRegisterInputSource` 只注册位于 `/Library/Input Methods/` 或 `~/Library/Input Methods/` 的输入法 bundle。
- `TISSelectInputSource` 成功前，输入源必须 `selectable=true` 且 `enabled=true`。
- 如果输入源是 input mode，它的 parent input method 也必须 enabled。
- `TISCreateInputSourceList(..., includeAllInstalled=false)` 才代表当前已启用输入源列表；`includeAllInstalled=true` 会包含已安装但未启用输入源。

## 编译和短启动

命令：

```bash
./spikes/inputia-imk/build.sh
open -n spikes/inputia-imk/build/InputiaIMKSpike.app
pgrep -fl InputiaIMKSpike
pkill -x InputiaIMKSpike
```

关键结果：

```text
InputiaIMKSpike.app/Contents/Info.plist: OK
InputiaIMKSpike.app: valid on disk
InputiaIMKSpike.app: satisfies its Designated Requirement
.../InputiaIMKSpike.app/Contents/MacOS/InputiaIMKSpike
```

结论：

- Swift 单文件 Host 可以链接 `Cocoa` 和 `InputMethodKit`。
- 必须显式传 `-target "$(uname -m)-apple-macos13.0"`；否则 swiftc 在本机生成 `LC_BUILD_VERSION minos 28.0`，`open` 会失败。
- Host 进程可以启动并进入 `NSApplication.run()`。

## 注册和枚举

命令：

```bash
./spikes/inputia-imk/install-user.sh
"$HOME/Library/Input Methods/InputiaIMKSpike.app/Contents/MacOS/InputiaIMKSpike" --dump-inputia-sources
```

关键结果：

```text
Registered input source from /Users/lzl/Library/Input Methods/InputiaIMKSpike.app
Enable succeeds for dev.inputia.inputmethod.Spike
id=dev.inputia.inputmethod.Spike
bundle=dev.inputia.inputmethod.Spike
type=TISTypeKeyboardInputMethodWithoutModes
enabled=false
enableCapable=true
selectable=true
selected=false
matches=1
```

结论：

- 用户级 bundle 可以被 `TISRegisterInputSource` 注册，并能被 `includeAllInstalled=true` 枚举到。
- 当前 spike 已改为单一 input source，不使用 macOS `ComponentInputModeDict`。Inputia 的英文/拼音/双拼/语音/剪贴板模式应由 Inputia Core 内部管理，更符合统一输入层目标。

## 启用和选择失败

命令：

```bash
"$HOME/Library/Input Methods/InputiaIMKSpike.app/Contents/MacOS/InputiaIMKSpike" --dump-enabled-inputia-sources
"$HOME/Library/Input Methods/InputiaIMKSpike.app/Contents/MacOS/InputiaIMKSpike" --select-input-source
```

关键结果：

```text
--- enabled only ---
matches=0

--- select ---
Selecting from all installed source list
id=dev.inputia.inputmethod.Spike
type=TISTypeKeyboardInputMethodWithoutModes
enabled=false
selectable=true
selected=false
Select fails with -50 for dev.inputia.inputmethod.Spike
```

结论：

- `TISEnableInputSource` 返回 `noErr`，但当前用户的 enabled source list 仍不包含 Inputia。
- `TISSelectInputSource` 返回 `paramErr (-50)`，符合 Apple 文档：输入源未处于 enabled 状态时不能选择。
- 因此还不能验证真实 App 中的 key event、marked text、candidate panel 或 commit。

## 排除过的假设

已验证不是这些问题：

- **Swift class 名不可见**：`--self-check-classes` 中 `NSClassFromString("InputiaIMKSpike.InputiaSpikeInputController") == true`。
- **mode parent 未启用**：切换为 `TISTypeKeyboardInputMethodWithoutModes` 后仍不能 enabled/select。
- **缺少常见 input method 元数据**：补齐 icon key、key equivalent、bundle signature、character repertoire 后仍不能 enabled/select。
- **ad-hoc 签名唯一问题**：用本机 `Codexbar Local Code Signing Leaf v4` 重签后仍不能 enabled/select。
- **LaunchServices 缓存或进程未启动**：`lsregister -f` 并启动 Host 后仍不能 enabled/select。
- **直接写 `AppleEnabledInputSources` 可行**：追加 Inputia 偏好项并重启 `cfprefsd` 后 TIS enabled list 仍不包含 Inputia；该临时偏好项已删除。

## 与成熟实现对照

本机已安装的成熟输入法状态：

- `/Library/Input Methods/WeType.app`
- `/Library/Input Methods/DoubaoIme.app`
- `/Library/Input Methods/Squirrel.app`

对照结果：

- 微信输入法当前被用户启用时，`includeAllInstalled=false` 能同时枚举到 parent 和 mode，且 parent 为 `enabled=true`。
- 豆包/Squirrel 虽已安装，未在当前用户 enabled list 中时，`includeAllInstalled=true` 可显示部分 mode `enabled=true/selectable=true`，但 `TISSelectInputSource` 仍返回 `-50`。
- 所以判断“能否选择”的可靠标准是 `includeAllInstalled=false` 是否能枚举到该输入源，而不是 all-installed 列表里的静态 bool。

## 当前结论

当前 spike 已证明：

- 原生 IMK Host 能编译、签名、注册、枚举和短启动。
- Inputia 更适合暴露为单一 macOS input source，由 Inputia Core 管内部模式。
- 当前用户级自制 bundle 仍未能通过 TIS 真正进入 enabled source list。

下一步必须验证：

- 系统级 `/Library/Input Methods` 安装是否改变 enabled/select 行为。
- Developer ID / notarized 签名是否是现代 macOS 真启用第三方输入法的前提。
- 通过系统设置 UI 手动添加后，TIS enabled list、`IMKInputController.handle`、`insertText`、`IMKCandidates` 是否可用。

这些需要管理员权限、系统设置交互或正式签名，不应靠隐藏 defaults hack 当作产品方案。
