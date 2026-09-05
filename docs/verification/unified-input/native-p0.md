# P0 原生可行性、隔离验证与安装边界

记录时间：2026-09-05 08:13 北京时间。工作树基线：`b7d7db7047db270c949fea6e267a1aaa23d948ad`。本报告只说明实际完成的 P0 子项，不表示 P0 或 A01–A12 已通过。

## 已执行的证据

新增 `macos/InputiaInputMethod/Tools/UnifiedInputNativeProbe.swift`。它仅查询系统/权限元数据，在自己新建的 0700 临时目录中创建 0600 Unix socket，并启动自己的子进程验证对端身份。没有读取用户输入文本、剪贴板正文或窗口标题，没有申请权限、发按键、注册或切换输入源，也没有启动/控制用户文档。退出清理只覆盖本工具创建的唯一临时目录。

工具链：Apple M2 Max、12 核、32 GiB；运行系统 `Version 27.0 (Build 26A5425a)`；Swift 6.3.3；SDK `/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk`。工具链报告的默认 Target 为 `arm64-apple-macosx28.0`，不能把这个 SDK/编译器字符串当运行系统版本。

可复现构建（从仓库根执行，临时目录名由 mktemp 返回）：

```sh
probe_dir=$(mktemp -d /tmp/handy-unified-native.XXXXXX)
swiftc -parse-as-library \
  macos/InputiaInputMethod/Tools/UnifiedInputNativeProbe.swift \
  -framework AppKit -framework ApplicationServices -framework Carbon \
  -framework InputMethodKit -framework Security \
  -o "$probe_dir/UnifiedInputNativeProbe"
"$probe_dir/UnifiedInputNativeProbe" --rounds 100
```

本次可执行文件：`/tmp/handy-unified-native.ToanZr/UnifiedInputNativeProbe`（临时构建，不是安装包）。2026-09-05T00:13:37Z 的实际运行摘要：

```json
{
  "peer_rounds": 100,
  "peer_elapsed_seconds": 7.951361374987755,
  "all_same_user": true,
  "all_matching_child": true,
  "all_audit_ok": true,
  "all_lookup_ok": true,
  "all_unsigned_rejected": true,
  "all_child_exit_ok": true,
  "accessibility_trusted_for_probe": true,
  "input_monitoring_preflight_for_probe": true,
  "event_posting_preflight_for_probe": true,
  "secure_event_input_enabled": false,
  "imk_live_target_validated": false,
  "clipboard_source_coverage_validated": false
}
```

每轮 `getpeereid`、`LOCAL_PEERPID`、`LOCAL_PEERTOKEN` 均返回 0；audit token 为 32 字节。`SecCodeCopyGuestWithAttributes(kSecGuestAttributeAudit)`、`SecCodeCopyStaticCode`、签名信息查询、一般有效性检查均返回 0。子进程为 ad-hoc 签名，Team ID 为空；`anchor apple generic` requirement 检查返回 `-67050`，正确拒绝该签名。这里的 `all_unsigned_rejected` 是摘要字段名，实际含义是“所有无 Apple 信任链的 ad-hoc 探针被拒绝”，并非遍历了所有未签名程序。

追加编译门禁：同一源码增加 `-warnings-as-errors -target arm64-apple-macosx13.0` 编译退出 0，产物 `UnifiedInputNativeProbe-min13` 在本机一轮运行通过。它证明 macOS 13 deployment target 可编译，不替代在 macOS 13 实机运行验证。

100 轮证明 socket/内核对端身份/签名查询路径能在当前 Mac 运行。未运行输入事件，因此不能据此证明“连接断开不阻塞按键”，也不能把包含进程启动的 7.95 秒当 A09 性能数据。探针进程拥有权限不保证候选 Host 获得相同 TCC 授权。

初始诊断发现并已修正两处平台差异：Swift 签名信息接口要求先从 `SecCode` 取得 `SecStaticCode`；系统临时目录可能使 Unix socket 路径超过 Darwin 的 104 字节限制，改为 `/tmp/uip-<UUID>/peer.sock` 私有目录。产品 socket 路径同样必须限制字节数，不能直接拼接长工作区或 managed-profile 路径。

## 固定本地认证方案的建议

1. 双端检查私有目录和 socket 的类型、属主、权限及符号链接，不能只在创建时 chmod。
2. 接受连接后先 `getpeereid` 验证有效 UID；用 `LOCAL_PEERTOKEN` 获取内核证明的进程实例身份。PID 只用于诊断，不作单独认证依据。
3. 用 audit token 查询 `SecCode`，验证明确的签名 requirement：可信 Apple Developer 签名链、产品 Team ID、对应 Handy/Inputia bundle ID 允许列表。当前探针只验证了 Apple anchor 的拒绝分支；完整发行 requirement 与合法签名候选的允许分支仍须执行。
4. 双向认证完成后才解析业务握手/profile/协议版本；禁止未认证客户端获得历史或词库。签名检查走后台队列，不进入输入法按键路径。
5. 当前日常 Inputia 安装为 ad-hoc，无 Team ID；开发例外只能在隔离开发 profile 显式开启，并固定预期 CDHash 与 profile，不得用“同 UID 即允许”作为发行默认。升级后的新 CDHash 需由可信安装过程更新，不允许握手自报后信任。

本机 SDK 第一手依据：`usr/include/sys/un.h` 中 `LOCAL_PEERPID/LOCAL_PEERTOKEN`；`Security.framework/Headers/SecCode.h` 中 `kSecGuestAttributeAudit`、`SecCodeCopyGuestWithAttributes`、`SecCodeCheckValidity`。原型已对上述 API 作实际调用，未冒称生产认证已接通。

## IMK 目标的可观测边界

现有 Host 在 `main.swift:301` 的 `activateServer` 设置 active controller，`deactivateServer` 清除并提交组合；`insertText` 是同步调用但返回值不包含宿主控件的文字提交回执。现有 `IMKServer` 已从 bundle 读取 ConnectionName 和 BundleIdentifier，可支持唯一测试 identity。

`IMKInputSession.h:94` 明确指出 `selectedRange` 等方法依赖客户端 TSMDocumentAccess 支持，部分应用会返回 NSNotFound。两个输入框也可能具有相同 selection range，不能用 range、bundle ID 或 controller 对象单独证明输入框没变。

实现须组合进程实例、Host/controller 实例、激活代数、可用的 AX focused-element 身份、选择区域及过期时间；任何缺失或矛盾都进入 pending_target。需要验证 AX 权限不足时的降级。Secure Input 与未知目标不得获取个性化结果。IMK 没有可据此断言“原文已落入应用控件”的统一回执，UI 必须区分已派发和已确认。

尚未完成 TextEdit、浏览器 textarea、Electron 编辑器的有效/失效目标测试；未完成同 App 输入框切换、组合冲突、Spaces、断线按键延迟、100 轮输出去重。以上保留为 P0/P3/P6 原生必验项。

## 候选 Host 能否与日常安装共存

平台支持用户级输入法目录。Apple SDK `TextInputSources.h:1199` 的 `TISRegisterInputSource` 文档明确接受 `/Library/Input Methods/` 或用户 `Library/Input Methods/` 中的 bundle，并可随后枚举注册的来源。独立 bundle/连接名/来源 ID 可以避免替换日常 Inputia；实际共存注册仍未运行，不能先标通过。

2026-09-05 只读安装检查：`/Library/Input Methods/InputiaInputMethod.app` 的 CFBundleVersion=50；仓库 Info.plist=40。安装版为 ad-hoc，identifier=`com.inputia.inputmethod.Inputia`，TeamIdentifier 未设置。不得把仓库旧构建覆盖这份较新安装。当前用户级 Input Methods 目录没有列出测试安装。

首次 P0 检查时尚无安全测试 profile：`InputiaRustBridge.swift:677–695` 固定写入 `Application Support/Inputia/rime`、`inputia_memory.db`、`settings.json`。因此只改 Info.plist 无法隔离数据。下方“隔离 profile 实施补充”记录本轮已完成的路径改造；完整候选包装仍须覆盖设置启动器、socket、签名和配对认证。

建议固定测试标识：

- Host bundle：`com.inputia.inputmethod.Inputia.UnifiedCandidate`。
- TIS parent/mode：上述 ID 及 `.Hans`。
- ConnectionName：`com.inputia.inputmethod.Inputia.UnifiedCandidate_Connection`。
- 显示名：`Inputia 融合候选测试`。
- 安装目标：`/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app`。
- 数据根：`/Users/lzl/Library/Application Support/HandyUnifiedCandidate/<run-id>/Inputia`；每次演练独立 run-id。
- 测试服务：配对 Handy 候选 profile，保留单独的短 socket 路径与签名/profile 校验。

## 安装及恢复操作单（准备说明，未执行）

执行前必须准备实际最终候选包、SHA-256、签名信息、profile 隔离测试证据及候选专用 TIS 工具。用户已要求系统安装先说明具体操作；本子任务没有安装授权，不注册/启用/选择任何来源。

1. 候选构建自检：校验所有 bundle/mode/ConnectionName 为测试标识；确定候选数据目录存在且为空或仅含合成副本；拒绝任何软链接指向日常 Inputia/Handy 数据。现有 `InputiaTISTool` 无参数默认会注册并切换，不能用无参数命令做只读检查。
2. 保存恢复证据：只读运行 TIS 工具 `--dump-current-input-source` 和 `--dump`，记录原选中来源和候选是否已存在；保存日常 bundle 的 hash、版本、签名元数据。若用户希望备份真实数据，使用一致性副本并另行纳入安装流程；本隔离测试不需要读取真实正文。
3. 在候选 profile 与原生测试权限得到确认后，将准备好的 bundle 用 `ditto` 复制到上述唯一用户级目标。若该目标已存在且不属于本次测试，停止覆盖并使用新的测试标识/目标。
4. 候选专用工具分别调用 `TISRegisterInputSource(candidateURL)`、`TISEnableInputSource(candidateParent/Mode)`；明确选中测试模式后才打开本次专用的合成 TextEdit 文档、浏览器页面及 Electron 文件。记录实际 OSStatus 与选中 ID。现有工具的 `INPUTIA_TIS_REQUIRE_APP_MATCH` 仍比较 `inputia.pdf`，而 plist 使用 `inputia-menu.pdf`，先修正候选工具的目标匹配，不可依赖错误 icon 路径通过检查。
5. 进行计划所需每 App 至少 20 轮输入和 10 轮焦点切换，记录匿名目标 token/operation ID/结果及只含合成内容的录像。任何找不到明确目标的自动输出都保留待插入。不要在用户真实聊天或文档做测试。
6. 恢复时先取消待输出并停止候选会话；通过 `TISSelectInputSource(originalSource)` 恢复步骤 2 保存的来源并核对。然后 `TISDisableInputSource(candidateMode)` 和 `TISDisableInputSource(candidateParent)`，只终止验证过路径/bundle ID 的候选进程。
7. 将候选安装 bundle 移入本次恢复包或废纸篓，保留合成数据/验收证据。日常安装和真实数据从未被替换，无需覆盖恢复。若系统保留注册缓存，保持候选禁用并如实记录；如需注销登录清理缓存，另向用户说明，不擅自注销当前会话。

候选专用 TIS 工具还需提供独立 register/enable/disable 命令，当前仓库工具只有 reset-enable 和 select，不能把“恢复方案已说明”标为“可执行恢复已验证”。此步骤由后续安装交付实现补齐。

## Rime 个性化遗忘

本仓库 `crates/inputia-rime/src/lib.rs:1069` 的 `RimeApi` 仅定义到 `select_candidate_on_current_page`；没有暴露 delete API。上游 [librime rime_api.h](https://raw.githubusercontent.com/rime/librime/master/src/rime_api.h) 提供按 session/candidate index 删除的 API，要求运行时检查自描述结构长度和非空函数指针。上游 [rime_api_impl.h](https://raw.githubusercontent.com/rime/librime/master/src/rime_api_impl.h) 将其路由至当前会话 Context 的候选删除。它不是“传一个词就清除所有 schema/code 个性化”的 API。

只读 `nm -gU` 已在本机 `/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib` 发现：`_rime_get_api`、`__Z19RimeDeleteCandidatemm`、`__Z32RimeDeleteCandidateOnCurrentPagemm`。这证明本机二进制含删除实现符号，尚未证明对用户词典的持久化删除效果；没有加载或修改真实 Rime userdb。

具体接入：优先为由 Host 记录了 schema/拼写/候选来源的词提供精确撤销；后台串行任务使用隔离维护 session，重建候选并核对目标文本，依据运行时 API 长度检查调用 deletion，销毁维护 session 后重新打开副本验证加权撤销及基础候选保留。不得借用用户正在组合的 live session。若词有多个 schema/code 或历史记录不完整，候选删除不能证明全域遗忘，显示 `rime_personalization_pending`。

全域无法精准撤销时，按设计提供独立“重置 Rime 学习”：Host 停止所有 Rime sessions，备份并重建仅个性化 userdb；保留用户 schema、custom.yaml、显式词典与配置；同步目录里的旧快照也必须受最新忘记账本约束，不能恢复后自动同步回来。执行前用合成 profile 验证具体库文件清单、关闭锁、重启、重新部署和兼容 Host 重导入。重置会清除其他学习，必须作为明确用户动作，不能在点“忘记一个词”时静默全删。

当前 Rime 状态：API/本机符号存在；Rust 封装未实现；词级删除后重启持久性、跨 schema 全覆盖、重置与回滚均未验证。不能报告“所有输入引擎已忘记”。

## 仍待实测的剪贴板边界

本子任务没有读取系统剪贴板，未统计来源覆盖率或进行图片/文件/多格式恢复。原生来源与隐私标记观察必须在后续专用合成 App/页面执行，分别记录真实声明来源、前台 App 线索、unknown、敏感标记；不能把前台进程当可信来源。上述数据由 P0/P2/A06/A07 的测试另行提供。

## 隔离 profile 实施补充

2026-09-05 08:26 北京时间完成以下有限改造，未构建或安装 Host 应用：

- 新增 `Sources/InputiaInputMethod/InputiaProfile.swift`，候选 bundle ID 必须以 `.UnifiedCandidate` 结尾，并有真正的 plist Boolean `InputiaDevelopmentCandidate=true`。整数 1 或只满足一个条件均不成立。
- `INPUTIA_PROFILE_RUN_ID` 或已签名 plist `InputiaProfileRunID` 指定 run-id；两者同时存在必须相同。标识只允许 1–64 字节 ASCII 字母、数字、`-`、`_`。候选缺 run-id、生产构建注入 run-id、路径穿越、空白、超长和不合法 plist 类型均失败关闭，不回退日常目录。运行时初始化拒绝返回进程状态 78。
- 候选根固定为 Application Support 的 `HandyUnifiedCandidate/<run-id>/Inputia`，Handy 配对根固定到同 run-id 的 `Handy`。settings/memory/Rime/outbox/snapshots/policy/logs 均有明确 profile 路径，现有任意配置中的 memory/Rime 可写目录会重新绑定。加载设置、打开 Rust session、保存设置和导入前检查候选路径及现存路径祖先中的符号链接。
- `InputiaRustBridge`、`InputiaSettingsWindow`、`InputiaHandyMemorySync` 已改用统一路径；候选不会发现日常 Handy DB，显式传入日常或其他 profile 的历史路径也拒绝。候选 Rime shared data 只接受自己的 bundle 资源，不回退到日常 Inputia 安装的资源。正常日常 bundle 的默认路径保持原行为。
- `build.sh` 接受绝对路径 `INPUTIA_BUILD_DIR`，保留 `CARGO_TARGET_DIR`，从**本次 Cargo build 输出的 compiler-artifact JSON**取得唯一静态库路径，避免自定义 target 后误链接旧 `crate/target/release`。所有引用 bridge/settings/sync 的编译入口已加入 InputiaProfile，并新增 profile 自检构建产物。

实际验证：`UnifiedInputProfileSelfCheck.swift` 编译后执行 54 个检查通过；覆盖路径穿越、缺标识、生产注入、plist/env 冲突、所有写路径无日常泄漏、旧配置重新绑定、配对导入和符号链接拒绝。测试只创建并清理 `Library/Caches/InputiaProfileCheck-<UUID>` 中的合成目录与符号链接。

```text
unifiedInputProfileSelfCheck=true checks=54
swiftc -warnings-as-errors -target arm64-apple-macosx13.0: exit 0
四个受影响 Swift 文件联合 typecheck + warnings-as-errors: exit 0
zsh -n build.sh: exit 0
INPUTIA_BUILD_DIR=<隔离绝对路径> + BUILD_PREFLIGHT_SELF_CHECK: exit 0
INPUTIA_BUILD_DIR=/: exit 2
普通命令行 bundle 注入 INPUTIA_PROFILE_RUN_ID + --current: exit 78
```

构建/执行自检命令：

```sh
swiftc -warnings-as-errors -target arm64-apple-macosx13.0 -parse-as-library \
  macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaProfile.swift \
  macos/InputiaInputMethod/Tools/UnifiedInputProfileSelfCheck.swift \
  -o /tmp/handy-unified-native.ToanZr/UnifiedInputProfileSelfCheck
/tmp/handy-unified-native.ToanZr/UnifiedInputProfileSelfCheck
```

后续独立候选包装必须：在构建副本中写测试 Host 的 bundle/TIS/ConnectionName，再写 `InputiaDevelopmentCandidate` 与 `InputiaProfileRunID`，最后签名；SettingsLauncher 也需同 run-id 和配对 Host 定位。IMK 由系统启动，不能依赖终端环境变量继承，因此安装候选使用 plist 元数据。`INPUTIA_BUILD_DIR` 只隔离输出，并不自动更改产品 identity，不可直接把该参数构建的默认 bundle 当独立测试安装。

剩余限制：outbox/snapshots/policy/log 路径已定义，但新服务消费者仍须实际采用；Rime 当前日志设置为 stderr，未新增落盘日志。socket 的短路径与双向签名认证由通讯实现接入。没有进行 Host 完整链接、候选 bundle 生成/签名、真实 TIS 注册、实际 Rime 数据写入与迁移演练；尚不能标为“可安全安装已验证”。路径检查属于当前进程的拒绝规则，不能宣传为抵御同 UID 恶意进程实时替换文件的完整沙箱。
