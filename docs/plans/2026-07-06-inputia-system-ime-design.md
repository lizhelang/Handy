# Inputia 系统级输入法第一阶段设计

日期：2026-07-06

## 目标

Inputia 是一个本地优先的统一输入层。第一阶段先跑通 macOS 系统级输入法 MVP，但核心架构不能绑定到 macOS，也不能把 Rime、librime 或任何平台输入法框架当成核心大脑。

第一阶段必须证明：

- Inputia 可以作为 macOS 系统输入法被系统发现和启用。
- 普通 App 中可以英文直通输入。
- 普通 App 中可以中文拼音输入。
- 至少一种双拼方案可以输入中文，首选自然码或小鹤其中一种，具体以引擎 spike 结果确定。
- 候选词支持显示、数字选择、分页、退格、清空和上屏。
- Shift 可以切换中英文，设置中可以关闭或改键。
- 中文模式下可以选择始终使用英文标点。
- 打字历史、语音历史、剪贴板历史能影响候选排序。
- 用户打过的新词能进入语音热词或自定义词候选。
- 敏感 App 默认不读不学。
- 所有数据本地保存，不依赖服务器。

## 证据

### Apple InputMethodKit

官方 SDK 头文件确认 macOS 输入法应由 InputMethodKit 承载：

- `IMKServer` 是输入法进程与系统/客户端通信的服务器，每个输入法进程创建一个。
- `IMKInputController` 为每个输入会话创建 controller，接收 key event，维护 composition，并把 marked text 或 committed text 交给客户端。
- `IMKCandidates` 提供系统候选窗能力，包括候选数据、选择键、分页、候选选择回调。
- `TextInputSources.h` 确认 application bundle 输入法安装在 `<domain>/Library/Input Methods/`，`TISInputSourceID` 对 input method 通常与 Bundle ID 相同。

本机证据：

- `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk/System/Library/Frameworks/InputMethodKit.framework/Headers/IMKServer.h`
- `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk/System/Library/Frameworks/InputMethodKit.framework/Headers/IMKInputController.h`
- `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk/System/Library/Frameworks/InputMethodKit.framework/Headers/IMKCandidates.h`
- `/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk/System/Library/Frameworks/Carbon.framework/Versions/A/Frameworks/HIToolbox.framework/Versions/A/Headers/TextInputSources.h`

### Squirrel / Rime 成熟实现

`/tmp/inputia-squirrel` 显示 Squirrel 的成熟路线是：

`NSEvent -> IMKInputController.handle -> process_key -> get_commit/get_context -> client.insertText / client.setMarkedText -> candidate panel`

可复用结论：

- macOS Host 应是原生 accessory app，而不是 Tauri overlay。
- Host 只负责系统会话、事件转换、预编辑、候选窗、上屏和偏好入口。
- 每个输入会话独立管理 session；全局引擎生命周期由 app delegate 或等价全局服务管理。
- 不能随意吞事件。错误返回 `true` 会吃掉宿主 App 快捷键；错误返回 `false` 会导致重复输入。
- App bundle id 可作为上下文和隐私规则输入，但读取/上报必须默认保守。

### librime 可替换性

librime 是成熟的中文输入候选生成引擎，支持拼音、双拼、标点、schema、用户词典和跨平台前端。`/tmp/inputia-librime-sample` 证明 librime 可以通过 API 初始化、创建 session、处理 key sequence、读取 commit/context/menu。

本机 spike 进一步验证：`spikes/inputia-rime/build/rime_probe luna_pinyin_simp ni` 可返回 `你/拟/尼/泥/呢`，`ni{space}` 可 commit `你`。小鹤双拼 `double_pinyin_flypy` 已验证 `vsgo` 可返回 `中国` 候选并 commit。因此第一版 `Chinese Engine Adapter` 可以优先接 librime，但正式打包仍需单独处理 schema 数据、许可证和签名。

`crates/inputia-rime` 已把这个 spike 正式化为 Rust adapter：动态加载 librime dylib，输出 `inputia-core::Candidate`，并实现 `ChineseEngine` trait。验证结果：

- `luna_pinyin_simp ni` 返回 `你/拟/尼/泥/呢`。
- `double_pinyin_flypy vsgo` 返回 `中国/种过/种果/忠果/中`。
- `double_pinyin_flypy vsgo{space}` 返回 `commit=中国`。
- `InputiaCore<RimeEngine>` 已验证 `zhongguo` 和小鹤 `vsgo` 两条真实输入流：切到中文、候选显示、Core 分页、回到首页、空格上屏 `中国`。

### Inputia Core 状态机

`crates/inputia-core` 证明核心状态机可以脱离平台 Host 和 librime 类型独立测试。当前测试覆盖英文直通、Shift 切换、中文 composing、候选分页、数字选择、空格上屏、退格、清空、标点偏好、本地记忆重排、voice/clipboard completion、用户新词热词和敏感 App 不学习。`sqlite-memory` feature 已验证 `inputia_terms` / `inputia_events` 持久化、只读导入 Handy `history.db` / `clipboard.db`，并用 `/tmp` 临时库跑过本机真实导入 probe。

Inputia 只把 librime 放在 `Chinese Engine Adapter` 下：

- 输入核心不直接暴露 librime session。
- Ranker 不依赖 Rime 用户词库格式。
- Memory 不写入 Rime 内部状态作为唯一事实来源。
- 后续可以替换为自研拼音引擎、libpinyin、系统引擎桥接或多引擎组合。

### Handy Runtime 桥接层

`crates/inputia-handy-runtime` 已把 Handy 数据目录约定正式化：`history.db`、`clipboard.db` 和 Inputia 自己的 `inputia_memory.db`。runtime 不依赖 Tauri，只负责解析路径、打开 `SqliteMemory`、只读导入 Handy 语音历史和文本剪贴板历史。

本机真实数据 probe 使用 `/tmp/inputia-handy-runtime-import-probe-real.db` 作为目标库，读取 `$HOME/Library/Application Support/com.pais.handy`，结果为：

```text
history_imported=1203
clipboard_imported=1520
inputia_terms=2263
completion_count_for_prefix_中国=1
```

合成数据库测试同时验证了源 `history.db` / `clipboard.db` 行数导入前后不变，且 `com.1password.1password` 来源文本不会进入 completion。

### Windows / Linux 前瞻

Microsoft TSF 文档确认 Windows 侧核心概念是 text service、edit context、document manager、composition、candidate list UI。Linux 侧 IBus/Fcitx 都是“engine 处理 key event，更新 preedit/candidate，commit text”的模型。

因此 Inputia Core 的跨平台接口应抽象为：

- `begin_session(context)`
- `end_session(session_id)`
- `handle_key(session_id, key_event) -> InputOutcome`
- `commit_candidate(session_id, candidate_id)`
- `page_candidates(session_id, direction)`
- `reset(session_id)`
- `set_context(session_id, app_context)`

## 架构

```text
System Input Method Host
  macOS InputMethodKit first
  Windows TSF later
  Linux IBus/Fcitx later

Inputia Host Bridge
  converts platform key events and client context into stable Inputia events
  renders marked text, candidate UI, commit, cancel, paging

Inputia Input Core
  owns input mode, composing buffer, candidate state, command routing
  applies English mode, Chinese mode, Shift toggle, punctuation policy

Chinese Engine Adapter
  first adapter: librime spike
  contract: pinyin/shuangpin candidates in, normalized candidate list out

Inputia Memory
  typed text history
  Handy transcription history
  Handy clipboard history
  app/window/document context when allowed

Inputia Ranker
  local reranking over engine candidates, English completions, hot phrases
  source weights: typed > selected > voice > clipboard, with recency decay

Inputia Settings
  input mode, shuangpin scheme, candidate count, shortcuts, punctuation
  privacy learning, sensitive app exclusion, local data retention
```

`crates/inputia-settings` 已把这层先落为本地 JSON 配置契约，并可由 C ABI/Host 读取。当前已经覆盖 schema、候选数量、Shift、标点、memory/privacy 开关和敏感 bundle id；设置 UI 还没有接入。

### macOS Host 骨架

`macos/InputiaInputMethod` 已新增正式 IMK Host skeleton，包含 `Info.plist`、Swift `IMKServer`/`IMKInputController` 入口、构建脚本、用户级安装/卸载诊断脚本。它已经验证：

- `swiftc` 可构建 `InputiaInputMethod.app`。
- `plutil` 和 `codesign --verify --deep --strict` 通过。
- `InputMethodServerControllerClass=InputiaInputMethod.InputiaInputController` 可被 `NSClassFromString` 找到。
- 早先 `LSBackgroundOnly=true` 时，用户级 `TISEnableInputSource` 返回 `noErr` 但 enabled list 仍没有 Inputia；现在已与本机 Squirrel 对齐为 `LSUIElement=true` / `LSBackgroundOnly=false`，并增加显式 `dev.inputia.inputmethod.Inputia.Hans` mode。
- 当前 Hans mode 在 all-installed list 中显示 `TISTypeKeyboardInputMode enabled=true/selectable=true`，但 `includeAllInstalled=false` 仍没有 Inputia，`TISSelectInputSource` 返回 `-50`；启动 IMKServer 后再 enable/select 结果相同。
- 已排除更多用户级变量：base+mode 同时 enable、`smSimpChinese` mode script、LaunchServices `lsregister -f`、直接写 `AppleEnabledInputSources` / `AppleSelectedInputSources`、本地自签、ad-hoc hardened runtime + entitlements，均不能让 Inputia 进入 `includeAllInstalled=false`。
- 本机已启用的微信输入法位于 `/Library/Input Methods/WeType.app`，Developer ID 签名，并在 enabled list 同时出现 base source 和 mode；当前用户对 `/Library/Input Methods` 不可写，因此系统级安装路径尚未验证。

因此当前真正未解的是 macOS 启用入口，而不是 Swift class、Info.plist 基本字段或 app-bundle 编译。

### Swift Host 到 Rust Core/Rime

`crates/inputia-capi` 已新增 C ABI，`macos/InputiaInputMethod` 的 build 脚本会先构建 Rust staticlib 再链接 Swift Host。Swift 侧 `InputiaRustBridge` 已能调用 Rust Core/Rime 并解析 JSON outcome。`--bridge-self-check` 验证：

```text
bridgeSelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=中国
commit=中国
```

这证明 IMK Host 进程已经不再只是英文直通骨架，而是能调用同一套 Inputia Core + Rime Adapter。剩余关键验证仍是系统输入法 enabled 后的真实宿主 App 事件。

C ABI 也已验证 `inputia_session_new_with_paths` 能通过临时 shared data 打开 `double_pinyin_flypy`，输入小鹤 `vsgo` 后首候选和 commit 都是 `中国`。测试中复现过并发初始化/析构 librime 的 SIGSEGV，已用测试锁串行化；正式 Host 需要单点管理 librime 生命周期和 session。

Swift Host 现在默认通过 `inputia_session_new_from_settings` 读取 `Application Support/Inputia/settings.json`，并派生 `rime/` 与 `inputia_memory.db`。`--bridge-settings-self-check` 证明 Host 可以从设置文件创建 session，默认候选数为 5，并完成 `zhongguo -> 中国`。`--bridge-memory-self-check` 使用临时库证明：学习剪贴板词 `种过` 后，输入 `zhongguo` 的首候选变为 `种过` 并可上屏。这把“打字/语音/剪贴板历史影响候选排序”的能力推进到了 Host 可调用层。Host 也已按 Squirrel 的成熟路径读取 `IMKTextInput.bundleIdentifier()` 并传给 Core；C ABI 测试证明 typed commit 在 `com.1password.1password` 上下文中不会被学习。Host 已接入 raw `NSEvent`：Command 快捷键放行，Space/Return/Backspace/Escape/方向键进入 Core，modifier-only Shift 在 key up 时切换中英文，候选窗最终选择会映射回 Core 的候选选择。

## 进程与代码组织

第一阶段建议在当前 Handy repo 内新增 Inputia 子系统，但不要把 Tauri 主窗口变成输入法 Host。

建议目录：

```text
src-tauri/src/inputia/
  core/
  memory/
  ranker/
  engine/
  settings/

crates/inputia-core/              # 已建立：平台无关 Core / Memory / Ranker
crates/inputia-handy-runtime/     # 已建立：Handy 数据目录和 Inputia memory 桥接
crates/inputia-rime/              # 已建立：动态加载 librime 的 Chinese Engine Adapter
crates/inputia-capi/              # 已建立：Swift Host 调 Rust Core/Rime 的 C ABI
crates/inputia-settings/          # 已建立：本地 settings.json 配置契约

macos/InputiaInputMethod/         # 原生 InputMethodKit Host
spikes/inputia-imk/               # 一次性平台 spike
```

第一阶段可以先把 Host 写成 Swift/Objective-C app target，因为 InputMethodKit 和候选窗/marked text 直接使用 Apple API 最可靠。Rust Core 通过 C ABI、Unix domain socket、localhost loopback 或嵌入静态库桥接。第一版先用最小可验证路径，不提前承诺桥接方式。

## 数据设计

现有 Handy 数据保持不变：

- `history.db`：语音转写历史。
- `clipboard.db`：剪贴板历史。
- `settings_store.json`：现有设置。
- `recordings/`、`clipboard_images/`：附件文件。

Inputia 新增：

- `inputia_memory.db`：Inputia 自己的学习与排序事实来源。
- `inputia_events`：记录本地学习事件，字段包含 source、text、app_bundle_id、privacy_decision、created_tick；敏感 App 排除事件不保存 text。
- `inputia_terms`：聚合词条、短语、英文 completion，字段包含 text、typed_count、voice_count、clipboard_count、last_used_tick。
- `inputia_app_policy`：敏感 App 默认排除和用户覆盖。

Memory 读取 Handy 历史时只做导入或查询，不直接改写 `history.db` / `clipboard.db` 的既有语义。

## 隐私默认值

默认不学习的场景：

- 密码管理器。
- 银行、支付、医疗、身份认证类 App。
- 浏览器隐私窗口。
- 终端中的密码提示、sudo、ssh passphrase 等无法可靠识别时，先不读取上下文。
- 任何用户显式加入排除列表的 bundle id。

第一阶段实现要满足：即使无法识别窗口标题或文档上下文，也必须能基于 bundle id 执行默认排除。无法确认安全时，不学习。

## 候选与排序

候选生成分两层：

1. Engine Adapter 生成基础候选：拼音/双拼候选、标点候选、基础纠错候选。
2. Ranker 做本地重排：根据 typed/voice/clipboard/memory source、当前 App、最近使用、选词确认、用户新词提升。

候选结构：

```text
Candidate {
  id
  text
  annotation
  source
  base_score
  memory_score
  final_score
  commit_text
}
```

输入核心只依赖这个结构，不依赖 librime 的候选对象。

## 标点策略

设置至少包含：

- `punctuation_mode`: `chinese` / `english` / `follow_input_mode`
- `force_english_punctuation_in_chinese`: bool
- `full_width_mode`: `off` / `punctuation_only` / `all`

第一阶段先实现“中文模式下始终英文标点”，这是用户明确要求，也是最容易验证的路径。

## Shift 切换

默认行为：

- 短按 Shift 切换中英文。
- 组合快捷键不触发切换。
- 按住 Shift 输入大写不应破坏英文大小写输入。
- 设置中可以关闭 Shift 切换或改为 Control/自定义键。

这部分必须通过 macOS key event spike 验证，因为 modifier-only 事件在输入法里容易受系统、远程桌面、键盘布局影响。

## 放弃方案

### 放弃：Tauri overlay 伪装输入法

理由：不能作为系统输入法被启用，候选/上屏/marked text 与普通 App 的兼容性不可控，也无法满足 macOS 文本系统语义。

### 放弃：直接 fork Squirrel 作为 Inputia

理由：Squirrel 是 Rime 前端。Inputia 的产品核心是统一输入层与本地记忆，不是 Rime 配置发行版。可以学习 Squirrel 的 Host/事件路径，但不能让 Rime 成为不可替换大脑。

### 放弃：第一阶段自研完整拼音引擎

理由：候选质量、纠错、双拼、词典和分页都需要大量验证。第一阶段应先用成熟引擎跑通系统输入法链路，同时通过 Adapter 边界保留替换权。

## 第一阶段里程碑

1. `spikes/inputia-imk`：最小 InputMethodKit app 能编译、注册、枚举；当前卡点是用户级自制 bundle 未能进入 TIS enabled source list。
2. `macos/InputiaInputMethod`：正式 Host skeleton 已能编译、自检和用户级注册；当前卡点仍是 enabled source list 未出现 Inputia。下一步验证系统设置手动添加、系统级安装或正式签名/安装器。
3. Core crate：`crates/inputia-core` 已完成纯 Rust 单元测试，覆盖英文直通、中文 composing buffer、退格、清空、分页、候选选择、标点策略和 SQLite Memory/Ranker。
4. Engine adapter：librime 已能返回全拼候选和小鹤双拼候选；`crates/inputia-rime` 已建立动态加载 adapter，且已通过 `InputiaCore<RimeEngine>` 验证全拼/双拼候选、分页和上屏。继续验证正式打包策略、schema 数据和 userdb 单写入者策略。
5. Host bridge：`crates/inputia-capi` 和 `InputiaRustBridge.swift` 已证明 Swift Host 进程能调用 Rust Core/Rime，并得到 `中国` 候选和 commit；memory-enabled 自检已证明剪贴板记忆能把 `种过` 重排到首候选。真实候选窗仍需系统启用后验证。
6. Candidate spike：macOS Host 能显示候选、选择、分页、上屏。
7. Settings spike：`crates/inputia-settings` 和 `inputia_session_new_from_settings` 已完成配置契约与 Host 初始化链路；C ABI 测试已验证禁用 Shift、候选数量和中文标点偏好会生效。下一步是 Handy 设置页出现 Inputia 设置分组，能改 Shift 切换、双拼 schema、候选数量和英文标点策略。
8. Memory/runtime：SQLite 版本已证明 typed/voice/clipboard 信号会影响候选和 completion，并能只读导入 `history.db`、`clipboard.db`；`crates/inputia-handy-runtime` 已完成正式 App 数据目录桥接。下一步是在 `src-tauri` manager 或 macOS Host 中调用 runtime。
9. Privacy spike：敏感 bundle id 默认不学习，测试覆盖默认排除和用户覆盖。

## 验证策略

- 平台 API：用 SDK 头文件和官方文档核对；不凭记忆写 IMK/TSF/IBus/Fcitx 行为。
- 成熟实现：每个不确定行为至少对照 Squirrel/librime 或对应平台成熟实现。
- spike 优先：注册、modifier-only Shift、candidate window、marked text、context 读取都必须先做最小 spike。
- 单元测试：Core、Memory、Ranker 必须先能脱离 macOS Host 测。
- 集成测试：macOS Host 至少要在 TextEdit 或 Notes 中人工/脚本验证英文、拼音、候选和上屏。
- 隐私测试：敏感 App 规则用确定 bundle id 测，不依赖 UI 猜测。

## 当前结论

第一阶段正式实现前，必须先完成 `spikes/inputia-imk`。如果 spike 证明 InputMethodKit Host 可以可靠编译并暴露必需注册信息，再进入 Host MVP；如果注册或签名/安装模型被 macOS 当前版本卡住，先记录阻塞证据并评估是否需要 Xcode app target、Developer ID 签名或单独安装器。
