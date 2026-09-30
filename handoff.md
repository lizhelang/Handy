# Inputia macOS 交接记录

更新时间：2026-07-16（Asia/Shanghai）

## 交接结论

Inputia v47 已经安装到系统目录，但**目前不可作为可用输入法**。最新用户反馈是“已经装上了，但是用不了”。当前问题优先级最高，后续开发者不要继续改 Rime、候选窗、词库、排序或设置页，先恢复 macOS 系统输入源的正式启用、选择和 Host 键盘事件链路。

当前不是“安装包没替换”的问题，也不能把 `TISEnableInputSource == noErr`、`enabled=true`、或“所有输入法”可见当成成功。必须以可选择、实际 Host 启动、普通 App 能输入为准。

## 工作目录与边界

- 唯一开发目录：`/Users/lzl/FILE/github/Handy-inputia-v44`
- 分支：`codex/inputia-v44-mainline`
- 基线提交：`f084121f`
- **不要**在旧工作树 `/Users/lzl/FILE/github/Handy` 上继续 Inputia 开发；它是 Handy 主项目，不是当前 Inputia 实现源。
- 本地构建必须使用 Rust 1.96：

  ```bash
  INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/dev-fast.sh
  ```

## 当前安装事实

已执行并通过：

```bash
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/build-pkg.sh
./macos/InputiaInputMethod/verify-pkg.sh
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/install-system.sh
```

安装结果：

- 系统 Host：`/Library/Input Methods/InputiaInputMethod.app`
- 设置 App：`/Applications/Inputia 设置.app`
- 构建版本：`47`
- Host CDHash：`d9eb3e86105288a722d3ba4a52f00346dfbcbcfe`
- `status.sh` 已确认 `systemMatchesBuild=true`、`targetMatchesBuild=true`、设置 App 也匹配该 Host。
- 当前输入源仍是微信输入法：`com.tencent.inputmethod.wetype.pinyin`。

因此“重新安装最新版本”已经完成；不能输入是 TIS/IMK 接入状态没有闭环，而不是仍在运行 v40 的问题。

## 当前 TIS 证据

请先运行，避免依据菜单栏截图或旧输出判断：

```bash
cd /Users/lzl/FILE/github/Handy-inputia-v44
INPUTIA_APP='/Library/Input Methods/InputiaInputMethod.app' \
INPUTIA_TIS_REQUIRE_APP_MATCH=1 \
./macos/InputiaInputMethod/build/inputia-tis-tool --dump

./macos/InputiaInputMethod/build/inputia-tis-tool --dump-current-input-source
./macos/InputiaInputMethod/status.sh
```

2026-07-16 最近一次 dump 的关键状态：

- `includeAllInstalled=false` 中出现 **两条**相同 `com.inputia.inputmethod.Inputia.Hans`。
- 两条 `Hans` 都报告 `enabled=true`、`selectable=true`，但都 `selected=false`。
- 父 source `com.inputia.inputmethod.Inputia` 也出现，`selectable=false`。
- `Hant` 为 installed、`enabled=false`。
- 当前 source 是微信输入法，而不是 Inputia。
- 此前对 `Hans` 调用 `TISSelectInputSource` 已出现 `-50 / paramErr`；对父 source 选择也会失败，因为 parent 本身不可选。
- `TISEnableInputSource` 曾返回 `0/noErr`，但并没有可靠地让输入源成为可选择的当前输入法。

这正是 Apple Text Input Source Services 所述的状态机问题：input mode 可选择前，它本身和父 input method 都要在正确的 enabled 状态；仅凭注册和单项 enable 返回码不足以证明状态已落盘。

## 必须遵守的验收标准

以下全满足前，不能说“Inputia 修好了”：

1. System Settings 的“已添加输入法”列表里只有一个 Inputia（名称就是 `Inputia`，不要附加“简体”）。
2. `TISCreateInputSourceList(nil, false)` 精确枚举到一个 `com.inputia.inputmethod.Inputia.Hans`，没有同 bundle/mode 的重复项。
3. 该 `Hans` 的 `kTISPropertyInputSourceIsEnabled == true` 与 `kTISPropertyInputSourceIsSelectCapable == true`。
4. `TISSelectInputSource(Hans)` 返回 `0/noErr`，`--dump-current-input-source` 显示 `Inputia.Hans`。
5. 选择后系统实际启动 **v47** Host，而不是手工启动一个脱离 TIS 的副本。
6. 用户在普通应用可输入：至少 Safari/Chrome 与 Codex/微信之一；中文模式有候选，英文和快捷键不被吞。
7. 选择和输入成功后再做一次 `status.sh`，确认 `runningMatchesBuild=true`。

## 下一位开发者的最短排查路线

### 1. 先停止注册/修复脚本反复运行

不要反复执行 `TISRegisterInputSource`、`TISEnableInputSource`、`repair-tis-duplicates.sh` 或重启 `TextInputMenuAgent` 试运气。当前 machine state 已有重复 Hans；继续循环可能进一步污染 Input Sources 缓存。

尤其不要直接启动 `/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod` 并传 `--help` 等参数。Host 未定义普通 CLI 行为，会导致额外的 IMK server/进程，不能作为真实运行验证。

### 2. 走 macOS 正式 UI 完成一次“添加”

`install-system.sh` 本身明确输出：

```text
systemInstallTISReady=false reason=manual-add-required
```

这是当前安装脚本设计的一部分，而不是可以忽略的 warning。应通过 System Settings > Keyboard > Input Sources 的 UI：

1. 在已添加列表移除所有重复的 Inputia。
2. 通过 `+` > 简体中文 > Inputia 添加一次。
3. 完成后立即运行上面的 TIS dump，不要依赖菜单栏的视觉缓存。

操作 UI 时不要打开 TextEdit、Safari、Codex 或微信做自动化 smoke；这些会抢用户焦点。此轮只做输入源管理和 TIS 诊断。

### 3. 如果 UI 添加后仍然不可选择，做 known-good diff，而不是猜 plist

以 [macOS_IMKitSample_2021](https://github.com/ensan-hcl/macOS_IMKitSample_2021) 或 Apple NumberInput sample 为 baseline，逐项比对实际 build product：

- bundle id：当前应是 `com.inputia.inputmethod.Inputia`
- `NSPrincipalClass`：自定义 `NSApplication` 子类
- `LSBackgroundOnly=YES`
- `InputMethodConnectionName`
- `InputMethodServerControllerClass` 与 Swift `@objc(...)` runtime name
- macOS sandbox entitlement 与 `com.apple.security.temporary-exception.mach-register.global-name`
- `/Library/Input Methods` 安装路径、签名身份和嵌套资源
- `tsInputMethodCharacterRepertoireKey`、图标资源路径
- Host app `Info.plist` 和已安装 bundle 的 `Info.plist` 是否完全一致

每比较一项先保存证据；不要再随机微调单个 plist 字段。

### 4. 只在 TIS 成功后验证 Host 键盘事件

Host 无法被系统选择时，候选窗、Shift、中文输入、Rime session 的任何测试都没有意义。先让 TIS select 成功，再检查：

- Host 进程路径和版本是否是 v47。
- `InputiaInputController` 是否收到按键。
- `insertText`/`setMarkedText` 是否在真实 client 中生效。
- 若 Safari 成功而 Codex/微信失败，记录 client bundle、secure field、modifiers 和 Host 日志，不能泛化成“系统输入法已好”。

## 禁止的“修复”方式

- **禁止**把 `AppleEnabledInputSources` / `AppleSelectedInputSources` 的 `defaults write` 当作正式修复。它只能作为诊断材料，不能作为成功标准。
- **禁止**以 `TISEnableInputSource` 的返回值 `0` 作为成功判断。
- **禁止**以“所有输入法”列表能看到 Inputia 作为成功判断。
- **禁止**默认跑 menu-readiness、GUI smoke、TextEdit/Safari smoke。普通开发只跑 `dev-fast.sh`；菜单栏 AXPress 和 GUI smoke 必须显式 opt-in，一轮最多一次。
- **禁止**为了测试开大量 TextEdit 窗口或抢用户正在使用的输入焦点；用户此前明确反对。
- **禁止**在未恢复基础输入的情况下继续扩展 Rime、词库、候选 UI 或在线排名。

## 当前未提交代码状态

工作树当前有 6 个未提交文件，合计约 `987` 行新增、`102` 行删除。它们不应被随手丢弃或与 TIS 问题混为一谈：

| 模块 | 作用 | 交接注意事项 |
| --- | --- | --- |
| `crates/inputia-core/src/lib.rs` | 本地在线 ranker、候选反馈和特征排序 | 这是较大的功能改动；TIS 修复时不要顺手回滚。 |
| `crates/inputia-capi/src/lib.rs` | C API 将 App context 传给 ranker、记录候选选择反馈；繁体 Rime schema 选择 | 当前新增的繁体修复将 traditional mode 从 `luna_pinyin` 改为 `luna_pinyin_tw`。 |
| `InputiaHostTextPolicy.swift` | 安全输入/文本策略调整 | 与 SecurityAgent 安全英文直通相关，保持敏感上下文不学习。 |
| `main.swift` | Host 输入逻辑、诊断和 TIS helper | 这里包含 TIS 诊断/enable/select 代码；不要用诊断函数去手写 HIToolbox 偏好。 |
| `InputiaSecureDirectSelfCheck.swift` | secure direct 回归自检 | 对应 SecurityAgent/password client 的按键透传。 |
| `prepare-rime-data.sh` | Rime data 构建 | 新增 `luna_pinyin_tw` 到 schema list，保证繁体 schema 会部署。 |

本轮新增、已经验证的繁体问题修复：`luna_pinyin_simp` 在繁体模式下旧逻辑只改 config，仍会提交“中国”；改为 `luna_pinyin_tw` 并把它列入 `default.yaml` schema list 后，C API 自检得到“中國”。这和当前 TIS 无法选择是两个独立问题。

## 已通过的非 GUI 验证

```bash
cd /Users/lzl/FILE/github/Handy-inputia-v44
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/dev-fast.sh
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/build-pkg.sh
./macos/InputiaInputMethod/verify-pkg.sh
```

`dev-fast.sh` 已通过：

- Host build 和签名自检
- inputia-core 测试（43）
- Rime core flow 测试（6）
- schema smoke 测试（6）
- inputia-capi 测试（28）
- inputia-settings 测试（5）
- Swift self-check，包含繁体 `中國`
- Rime probe、router/shortcut self-check

`build-pkg.sh` 和 `verify-pkg.sh` 已通过。它们证明构建/包装正确，**不**证明 Input Sources UI 已正式启用。

## 常用命令

```bash
cd /Users/lzl/FILE/github/Handy-inputia-v44

# 默认开发验证：不碰菜单栏、GUI、系统输入源
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/dev-fast.sh

# 构建和包装
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/build.sh
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/build-pkg.sh
./macos/InputiaInputMethod/verify-pkg.sh

# 只读 TIS 状态
INPUTIA_APP='/Library/Input Methods/InputiaInputMethod.app' \
INPUTIA_TIS_REQUIRE_APP_MATCH=1 \
./macos/InputiaInputMethod/build/inputia-tis-tool --dump
./macos/InputiaInputMethod/build/inputia-tis-tool --dump-current-input-source
./macos/InputiaInputMethod/status.sh

# 仅安装链路变化后使用；会请求管理员权限并刷新输入法相关服务
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./macos/InputiaInputMethod/install-system.sh
```

## 参考资料

- [Apple Text Input Source Services Reference](https://leopard-adc.pepas.com/documentation/TextFonts/Reference/TextInputSourcesReference/TextInputSourcesReference.pdf)
- [Apple: Change Input Sources settings](https://support.apple.com/guide/mac-help/change-input-sources-settings-mchl84525d76/mac)
- [Apple QA1810](https://developer.apple.com/library/archive/qa/qa1810/_index.html)
- [macOS_IMKitSample_2021](https://github.com/ensan-hcl/macOS_IMKitSample_2021)
- [Squirrel / Rime](https://github.com/rime/squirrel)

## 给接手者的优先级

1. 恢复唯一、可选择的 `Inputia.Hans`，并拿到 TIS/Host/真实输入证据。
2. 只在该基础上排查“Safari 可以但 Codex/微信不可以”等 client 差异。
3. 再回到候选、长输入分段提交、下箭头网格导航、词库覆盖、图标和 UI 细节。

当前用户最关心的是“能用”，不是新的功能。不要在第一步没通过前交付视觉或词库改动。
