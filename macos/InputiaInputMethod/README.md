# Inputia macOS InputMethodKit Host

这是 Inputia 第一阶段的 macOS 系统输入法 Host 骨架。它只负责系统输入法进程、`IMKServer`、`IMKInputController`、候选窗入口和诊断命令；拼音/双拼、记忆排序和隐私学习仍然属于可替换的 Inputia Core / Engine / Runtime 层。

命名迁移：早期原型误写成 `Iputia`。当前源码、bundle id、C ABI、crate 名和安装包统一改为 `Inputia` / `com.inputia.inputmethod.Inputia`。系统里如果已经装过旧 `/Library/Input Methods/IputiaInputMethod.app`，新的 system/pkg 安装脚本会先 unregister 并移除旧 typo 版，避免菜单栏同时出现两个输入法。

当前 Host enabled/selectable 阻塞的证据与验证顺序见：

```text
macos/InputiaInputMethod/HOST_BLOCKER_RUNBOOK.md
```

构建：

```bash
./macos/InputiaInputMethod/build.sh
```

`build.sh` 会先构建 `crates/inputia-capi` release staticlib，再把 Swift Host 和 Rust C ABI 链进 `InputiaInputMethod.app`。

构建时会运行 `prepare-rime-data.sh`，把 Squirrel 的基础 Rime shared data 复制进 build，并打包到 `InputiaInputMethod.app/Contents/Resources/RimeData`。当前内置方案：

```text
中文全拼            luna_pinyin_simp
自然码双拼          double_pinyin
小鹤双拼            double_pinyin_flypy
搜狗双拼            double_pinyin_sogou
国标双拼            guobiao_bispell
微软双拼            double_pinyin_mspy
智能 ABC 双拼       double_pinyin_abc
拼音加加双拼        double_pinyin_pyjj
四通双拼            double_pinyin_st
```

方案来源：自然码/智能 ABC/小鹤/微软/拼音加加/四通来自 Rime 官方 `rime-double-pinyin`；搜狗双拼先采用 rime-ice 的键位映射并保留朙月拼音词典；国标双拼来自 `rime-guobiao-quick`。

设置页里的“中英切换”会写入 `input_mode_toggle_shortcut`，当前支持：

```text
Shift            shift
Control + Space  control_space
关闭             none
```

旧字段 `shift_toggle_enabled` 继续保留给 Core/CAPI 兼容：当 shortcut 为 `shift` 时为 `true`，其它值为 `false`。

自检：

```bash
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --self-check
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-self-check
```

验证 build 或已安装 app：

```bash
./macos/InputiaInputMethod/verify-system.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
./macos/InputiaInputMethod/verify-system.sh /Library/Input\ Methods/InputiaInputMethod.app
```

普通开发默认验证：

```bash
./macos/InputiaInputMethod/dev-fast.sh
```

`dev-fast.sh` 是候选词、双拼、快捷键、设置 UI 等日常开发的默认入口。它只跑 build、Rust tests、Swift/bridge self-check、Rime probe、router/shortcut self-check；不打开菜单栏、不打开 GUI App、不切换系统输入源、不检查公证。

其中 `inputia-candidate-panel-self-check` 会在不打开候选窗的情况下验证候选窗格式化：默认横向单行，展开态按候选换行，第 7 个候选可见，并且最多显示 9 个候选。

其中 `--bridge-direct-session-self-check` 会额外用临时 `double_pinyin` settings session 验证长串双拼组合的短语候选排在单字前面，并验证选择单字后剩余 composition 和候选继续保留，避免把这类问题误归到 GUI 候选窗。

`dev-fast.sh` 还会运行 `rime-latency-self-check.sh`，用 `persistent_session_probe` 比较冷 evaluate 和持久增量 session 的同一输入前缀。默认只做非 GUI、非系统输入源的宽松性能防回退；阈值可用 `INPUTIA_RIME_LATENCY_MAX_INCREMENTAL_MS` 和 `INPUTIA_RIME_LATENCY_MIN_SPEEDUP` 覆盖。

`dev-fast.sh` 的 Rime probe 是带期望值的矩阵，不只打印一个自然码样例。它会覆盖：

```text
double_pinyin + nillem     -> 第一候选 你来
double_pinyin + mlle       -> 第一候选 买了
double_pinyin_sogou + mlle -> 第一候选 买了
guobiao_bispell + mlle     -> 候选包含 买了，但不要求第一候选
guobiao_bispell + mkle     -> 第一候选 买了
```

这组用例专门锁住“候选窗只剩 raw 字母”和“国标双拼键位与自然码/搜狗不一致”这两类容易混淆的问题。

`dev-fast.sh` 还会跑 Rime select probe，覆盖 `double_pinyin` 和 `double_pinyin_sogou` 下 `nillem` 的第 2 候选 `你`：选中后不应直接提交/清空，preedit 应继续显示 `你laiem`，下一段候选应保留 `来`。这条门禁专门锁住“长串输入先选一个字后，剩余字母和候选全部消失”的回归。

`dev-fast.sh` 也会跑安装链路纯逻辑自检：`INPUTIA_INSTALL_CHECK_SELF_CHECK=1 install-check.sh`、`INPUTIA_APPLY_CURRENT_HANDOFF_SELF_CHECK=1 apply-current-handoff.sh`、`INPUTIA_REPAIR_TIS_DUPLICATES_SELF_CHECK=1 repair-tis-duplicates.sh`。这些模式不读取 `/Library`、不查 TIS、不找 running host，也不会改系统输入源；只覆盖 `installCheckBlockReasons` / `installCheckRequiredAction`、安装交接清单 freshness、`installCheckRequiredActions` 到命令提示映射，以及管理员安装/修复 TIS duplicate 的动作链门禁。

安装链路变化后的验证：

```bash
./macos/InputiaInputMethod/install-check.sh
```

`install-check.sh` 只检查 `/Library/Input Methods/InputiaInputMethod.app`、`/Applications/Inputia 设置.app`、TIS enabled/selectable/current source、running host 是否与当前 build 对齐；默认不碰菜单栏、不做 GUI smoke readiness。设置启动器必须同时满足版本一致且 `InputiaExpectedHostCDHash` 等于当前 Host build CDHash，避免同版本旧 Host 被误打开。它还会只读检查 `build/install-handoff.txt` 是否由当前 source commit、干净 worktree、当前 build CDHash 和当前 pkg SHA 生成；若输出 `installHandoffCurrent=false`，先运行 `install-handoff.sh` 刷新交接清单，不要拿旧 pkg 去做管理员安装。

Host 和设置启动器的 build 产物会写入 `InputiaSourceCommit`、`InputiaSourceBranch`、`InputiaSourceDirty`。当 `CFBundleVersion` 相同但 CDHash 不同时，用这些字段判断系统安装版、设置启动器、当前 build 和远端主线到底对应哪次源码状态。

当 TIS 报告 `tis-duplicate-matches` 时，`install-check.sh` 会透出 `tis.targetSource.*` 明细，列出每条目标 `com.inputia.inputmethod.Inputia.Hans` source 来自 enabled list 还是 installed list，以及 type、iconURL、enabled/selectable/selected 状态。它还会输出 `tis.targetEnabledUniqueFingerprintCount` / `tis.targetInstalledUniqueFingerprintCount` 和 duplicate fingerprint，区分“缺少 source”“icon 指向旧 app”“多个不同 source 混在一起”和“同一目标 source 被系统缓存重复登记”。

失败时看 `installCheckBlockReasons`、`installCheckRequiredAction`、`installCheckRequiredActions`、`installCheckNextStep` 和 `installCheckNextCommand`。`installCheckRequiredAction` 是兼容旧脚本的首要动作；`installCheckRequiredActions` 是有序动作链；`installCheckCommand.*` 会把这条动作链展开成只读命令提示，脚本不会自动运行这些命令；`installCheckNextCommand` 是当前状态下最推荐的一条下一步命令。例如系统 app 或设置启动器不是当前 build 且没有非交互管理员权限时，如果 handoff 当前，会输出 `installCheckNextStep=apply-current-handoff` 和 `INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh`；running host 不是当前 build 时，会在动作链中追加 `restart-inputia-host-after-install`，不要把 TIS 已选中误判成当前代码正在运行。若 `TISCreateInputSourceList` 对同一个 Inputia mode 返回多个 enabled/installed 命中，会输出 `tis-duplicate-matches`，primary action 保持 `remove-duplicate-inputia-and-readd-once`，动作链追加 `run-repair-tis-duplicates`，不要把“菜单里能选”误判成单一干净注册态。

发布前或安装脚本变化后的完整验证：

```bash
./macos/InputiaInputMethod/release/full-check.sh
```

`release/full-check.sh` 是显式 opt-in 的重型入口，会跑 pkg/postinstall、公证 readiness、菜单栏 AXPress、TextEdit/Safari/Clipboard GUI smoke。它会为一次验证周期创建 `INPUTIA_MENU_READINESS_CACHE_FILE`，让菜单栏 AXPress 最多执行一次，后续子脚本只读 cache。

`release/full-check.sh` 在进入公证 readiness、菜单栏 AXPress 和 GUI smoke 前，会先运行 `install-check.sh` 并要求 `installCheckPassed=true`。如果系统目录、设置启动器、TIS duplicate 或 running host 还没有对齐当前 build，它会输出 `releaseFullCheckPassed=false reason=install-check-not-ready` 并停在只读诊断阶段，不会继续触碰菜单栏或 GUI。

`release/full-check.sh` 只有在 `install-check.sh` 和 `notarization-readiness.sh` 都通过后，才会导出 `INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1`、`INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1` 和 `INPUTIA_RUN_UI_SMOKE=1`。因此即使运行了 release/full-check，只要安装态或公证 readiness 没过，也不会把重型 opt-in 泄漏给前置检查。

`release/full-check.sh` 也提供离线门禁自检：

```bash
INPUTIA_FULL_CHECK_SELF_CHECK=1 ./macos/InputiaInputMethod/release/full-check.sh
```

这个模式只注入模拟输出，验证 install-check 失败、notarization 失败和成功路径的执行顺序；不会构建 pkg、不会查真实公证、不会打开菜单栏，也不会启动 GUI smoke。

`menu-readiness.sh` 和 `gui-smoke-readiness.sh` 本身也带硬门禁：直接运行时默认只输出 opt-in-required，不会打开菜单栏或读取 GUI readiness。只有 `release/full-check.sh` 或显式设置 `INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1` / `INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1` 的诊断才允许进入这些检查。

查看当前 build、系统安装、运行进程、设置启动器和最新安装包是否同版本：

```bash
./macos/InputiaInputMethod/status.sh
```

默认 `status.sh` 不打开菜单栏、不触碰 `TextInputMenuAgent`，也不做 GUI smoke readiness。需要把菜单栏呈现纳入状态时，显式设置 `INPUTIA_STATUS_INCLUDE_MENU_READINESS=1`；需要 GUI smoke readiness 时，再额外设置 `INPUTIA_STATUS_INCLUDE_GUI_SMOKE_READINESS=1`。

验证最新 pkg 是否确实携带当前 build 和最新 postinstall：

```bash
./macos/InputiaInputMethod/verify-pkg.sh
```

`verify-pkg.sh` 只展开本地 pkg 到临时目录，不安装系统包。它会校验：

- pkg `PackageInfo` version 等于当前 build version。
- pkg 内 `postinstall` 与 `Packaging/scripts/postinstall` sha256 一致。
- pkg 内嵌 host app 的 CDHash 等于当前 build app。
- pkg 内嵌设置启动器版本等于当前 build 设置启动器，且 `InputiaExpectedHostCDHash` 指向同一个内嵌 Host CDHash。
- postinstall 仍包含用户级 host 清理、TIS register、enabled/current TIS dump 和手动/显式修复提示；不得默认 TIS enable/select 或刷新菜单代理。

兼容入口：

```bash
./macos/InputiaInputMethod/verify-nongui.sh
```

`verify-nongui.sh` 现在默认委托到 `dev-fast.sh`，避免日常开发误跑旧的全量聚合验证。确实需要旧全量 non-GUI 聚合时，显式设置 `INPUTIA_VERIFY_NONGUI_FULL=1 ./macos/InputiaInputMethod/verify-nongui.sh`。旧 full 兼容模式也不会默认放行菜单栏或 GUI readiness；要把这些重型检查纳入旧聚合，必须额外显式设置 `INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1`、`INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1`、`INPUTIA_STATUS_INCLUDE_MENU_READINESS=1` 或 `INPUTIA_STATUS_INCLUDE_GUI_SMOKE_READINESS=1`。

旧 full 兼容模式如果进入菜单栏 readiness，会先设置或继承 `INPUTIA_MENU_READINESS_CACHE_FILE`，保证同一次验证周期里 `menu-readiness.sh` 的 AXPress 结果只采一次，后续子检查只读 cache。普通开发仍不应该打开这个 full 模式。

普通修候选词、双拼、快捷键、设置 UI 时，不要再把 `verify-nongui.sh` 当主入口；用 `dev-fast.sh`。重装或安装链路变化用 `install-check.sh`。发布前、pkg/postinstall/signing/notarization 或 GUI smoke 相关改动才用 `release/full-check.sh`。

`verify-nongui.sh` 会使用 `/tmp/inputia-verify-nongui.lock` 防止两条聚合验证并发互相污染残留判断。活锁存在时会返回 rc=20，并输出 `nonGuiVerificationPassed=false reason=verify-already-running` 与 `verifyLockOwnerPid=...`；pid 不存在的 stale lock 会自动清理并继续验证。不要手工删除仍在运行的 owner pid 对应锁；异常中断后再次运行脚本即可自愈。

只读 TIS readiness 诊断：

```bash
./macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
```

`tis-readiness.sh` 不切换输入源、不打开 GUI App，只报告待测 app CDHash、TIS Hans mode 的 icon 是否指向待测 app、是否 enabled/selectable/selected，以及当前输入源。默认不打开菜单栏、不触碰 `TextInputMenuAgent`；需要菜单栏 AXPress 证据时显式设置 `INPUTIA_TIS_INCLUDE_MENU_READINESS=1`，并建议同时传入 `INPUTIA_MENU_READINESS_CACHE_FILE`。

TextEdit 真实输入 smoke：

```bash
./macos/InputiaInputMethod/smoke-textedit.sh
```

这个脚本会先比较 build app 和 `/Library/Input Methods` 中已安装 app 的 CDHash，并拒绝在旧系统包上误测。真实 GUI smoke 默认不会打开 TextEdit；必须显式设置 `INPUTIA_RUN_UI_SMOKE=1`。通过图形会话、TextEdit 未运行、TIS 当前源等前置检查后，它才会用 TextEdit 验证默认中文 `ni + Space`、Shift 切到英文 `ni + Space`、再 Shift 回中文。失败路径会自动退出脚本自己启动的 TextEdit；如果 TextEdit 在测试前已运行，默认直接拒绝。

TextEdit Command 快捷键 smoke：

```bash
./macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh
```

这个脚本用 TextEdit 验证代表性宿主 App `Command-A/C/V` 路径：先写入测试文本，执行全选、复制、清空、粘贴，再确认复制和粘贴结果都等于测试文本。更广泛的常用 `Command` 快捷键透传由非 GUI `inputia-shortcut-self-check` 和 `inputia-host-text-policy-self-check` 覆盖；GUI smoke 只负责证明真实宿主输入框里最容易回归的复制/粘贴路径。它同样默认不打开 TextEdit，必须显式设置 `INPUTIA_RUN_UI_SMOKE=1`，并要求系统安装版 CDHash、图形会话、TextEdit preflight 和 TIS 选择全部通过后才会改剪贴板或启动 AppleScript。失败/退出路径会恢复原剪贴板、恢复输入源并关闭脚本自己创建的 TextEdit 文档。

Safari Command 快捷键 smoke：

```bash
./macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
```

这个脚本用 Safari 输入框验证代表性 `Command-A/C/V` 仍由 Safari 处理：打开本地 `data:` 测试页，选中输入框内容后复制、清空、粘贴，再通过页面标题确认粘贴结果。更广泛的浏览器/系统 `Command` 快捷键透传由非 GUI 快捷键自检覆盖；GUI smoke 只跑最小真实浏览器输入链。它默认不打开 Safari，必须显式设置 `INPUTIA_RUN_UI_SMOKE=1`，并要求图形会话、Safari preflight、系统安装版 CDHash 和 TIS 选择都通过后才会改剪贴板或创建 Safari 窗口。失败/退出路径会恢复剪贴板、恢复输入源并按窗口 id 关闭脚本创建的测试窗口；如果 Safari 在测试前已运行，默认直接拒绝。

真实 GUI smoke 预检：

```bash
./macos/InputiaInputMethod/smoke-preflight.sh
```

`smoke-preflight.sh` 只读检查，不切换输入源、不打开 TextEdit/Safari。它会统一报告 CDHash、图形会话、TextEdit/Safari 是否已有用户进程、TIS Hans mode 是否指向待测 app，以及当前输入源。真实 GUI smoke 前先跑它；只有 `smokePreflightReady=true` 时才继续跑 TextEdit/Safari/Clipboard smoke。

等待管理员安装完成并自动收口：

```bash
./macos/InputiaInputMethod/await-system-install.sh
```

这个脚本会等待 `/Library/Input Methods/InputiaInputMethod.app` 的 CDHash 变成当前 build CDHash，同时观察 TIS 状态是否 ready：`spctl --assess --type execute` 是否接受当前 app、enabled list 是否出现目标 source、Hans mode 的 icon 是否指向目标 app、Hans 是否 enabled、当前输入源是否为目标 Hans。观察到系统 app 和 TIS ready 后，它会运行 `post-install-regression.sh`。如果签名未被系统接受，会报告 `signature-rejected` / `sign-with-accepted-identity` 并阻断 UI smoke，避免制造“允许后仍无法切换”的假启用状态。默认不会打开 TextEdit/Safari；只有显式设置 `INPUTIA_RUN_UI_SMOKE=1` 时才会进入真实 GUI smoke。

安装后回归：

```bash
./macos/InputiaInputMethod/post-install-regression.sh
```

默认情况下，这个脚本只做非 GUI 回归：系统目录、旧 typo 清理、用户级 host 冲突、bundle/resource/codesign、自检、bridge、快捷键、TIS readiness。快捷键回归包括“所有含 `Command` 的 keyDown 透传”和常见 AppKit command selector 透传。它会单独报告 `postInstallTISReady=true|false`，用于区分“bundle/bridge 正常”和“可以进入真实 GUI smoke”；若 `postInstallTISReady=false reason=signature-rejected`，下一步不是手动反复允许，而是用 `INPUTIA_CODESIGN_IDENTITY` 指向能被 `spctl` 接受的签名身份后重建/重装。如果设置 `INPUTIA_RUN_UI_SMOKE=1`，它会先确认 TextEdit/Safari 没有用户已有进程，并确认 `postInstallTISReady=true`，然后依次运行 TextEdit 输入、TextEdit 代表性 `Command-A/C/V`、Safari 输入源诊断、Safari typing、Safari 代表性 `Command-A/C/V`、Safari raw ASCII enter 和 Clipboard recall smoke；Safari 使用本地 `data:` 测试页，不向外部网站发送内容。

系统级安装诊断：

如果 Mac mini 还没有 MacBook 上的本地签名身份，先导入 `.p12`。脚本只在显式提供 `.p12` 密码时导入；缺密码会直接退出，不会继续触发 Keychain 交互框：

```bash
INPUTIA_P12_PASSWORD="..." \
  ./macos/InputiaInputMethod/import-signing-identity.sh

security find-identity -v -p codesigning | grep -F "Codexbar Local Code Signing Leaf v4"
```

```bash
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
  INPUTIA_CODESIGN_OPTIONS="--options runtime" \
  ./macos/InputiaInputMethod/install-system.sh
```

这一步会写入 `/Library/Input Methods/InputiaInputMethod.app`，macOS 会要求管理员授权。安装脚本会清理旧 typo 路径 `/Library/Input Methods/IputiaInputMethod.app`。复制并校验 CDHash 后，脚本会先跑 `spctl --assess --type execute`；如果输出不是 accepted，脚本会打印 `systemInstallRequiredAction=sign-with-accepted-identity` 并在 TIS 注册前停止，不手写或清理 HIToolbox 输入源偏好。首次安装或替换后，如果 System Settings 的添加列表没有热刷新，按 ToyIMK/Squirrel/vChewing 对照经验，需要 logout/login 后再打开 System Settings > Keyboard > Text Input > Edit > Add。

如果 `spctl` 仍然 rejected，先跑只读公证 readiness：

```bash
./macos/InputiaInputMethod/notarization-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
```

这个脚本会同时检查 `codesign --verify`、Authority 链、hardened runtime、`spctl`、`syspolicy_check`、Developer ID identity、`notarytool`/`stapler` 和 notarytool keychain profile。若输出 `notary-ticket-missing`、`missing-developer-id-application` 或 `missing-notarytool-profile`，下一步不是继续注册/启用 TIS，也不是跑 GUI smoke，而是导入可被 Gatekeeper 接受的 Developer ID Application identity 并完成 notarization/staple。

Developer ID identity 和 notarytool profile 都齐全后，用公证脚本处理已由 Developer ID Application 签名的 app：

```bash
INPUTIA_NOTARY_PROFILE="Inputia" \
  ./macos/InputiaInputMethod/notarize-app.sh \
  macos/InputiaInputMethod/build/InputiaInputMethod.app
```

`notarize-app.sh` 会先确认 app 已用 `Developer ID Application:` 签名、带 hardened runtime、`notarytool` profile 可用，然后才会 zip、`notarytool submit --wait`、`stapler staple`、`stapler validate` 和重新跑 `spctl --assess --type execute`。只想验证前置条件但不上传时使用：

```bash
INPUTIA_NOTARIZE_APP_PREFLIGHT_ONLY=1 \
  ./macos/InputiaInputMethod/notarize-app.sh \
  macos/InputiaInputMethod/build/InputiaInputMethod.app
```

如果要交付可双击安装的 `.pkg`，安装包本身也需要用 `Developer ID Installer` 签名并公证。`build-pkg.sh` 通过 `INPUTIA_PKG_SIGN_IDENTITY` 调用 `productsign`，随后用 pkg 公证脚本处理：

```bash
INPUTIA_PKG_SIGN_IDENTITY="Developer ID Installer: <Name> (<TEAMID>)" \
  ./macos/InputiaInputMethod/build-pkg.sh

INPUTIA_NOTARY_PROFILE="Inputia" \
  ./macos/InputiaInputMethod/notarize-pkg.sh \
  macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
```

只检查安装包前置条件但不上传：

```bash
INPUTIA_NOTARIZE_PKG_PREFLIGHT_ONLY=1 \
  ./macos/InputiaInputMethod/notarize-pkg.sh \
  macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
```

注意：菜单栏实际输入链会按 macOS Text Input 的已安装 source 记录连接 `/Library/Input Methods` 里的 app。手动 `open macos/InputiaInputMethod/build/InputiaInputMethod.app` 只能做进程级诊断，不能代替系统目录安装；如果系统目录仍是旧 CDHash，TextEdit 仍会表现为旧 Host 行为。

安装脚本会打印 `sourceCDHash` / `destCDHash` 并要求二者相同，成功时输出 `systemInstallVerified=true`。安装后脚本会刷新 `TextInputMenuAgent` 和 `SystemUIServer`，再用 `verify-system.sh "/Library/Input Methods/InputiaInputMethod.app"` 做系统目录验证。

构建本地安装包：

```bash
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
  INPUTIA_CODESIGN_OPTIONS="--options runtime" \
  ./macos/InputiaInputMethod/build-pkg.sh
```

输出：`macos/InputiaInputMethod/dist/InputiaInputMethod-v<version>-<cdhash>.pkg`。当前包是 `--nopayload` 形态，Host 被打成无 AppleDouble 的 `InputiaInputMethod.app.tar.gz` 放在 scripts 中，由 `postinstall` 解压到 `/Library/Input Methods` 后 register 并 dump enabled/current TIS 状态；设置启动器被打成 `InputiaSettings.app.tar.gz`，安装到 `/Applications/Inputia 设置.app`。这样避免当前 macOS 环境把 `com.apple.provenance` xattr 编进 payload 文件列表。不要在 Installer 已经打开后重建同一个 pkg 路径；Installer 会检测 digest 变化并拒绝安装。

`postinstall` 使用 `/bin/zsh`，并对 LaunchServices 注册和 Inputia 自身的 register/dump 命令加了超时保护；如果当前 macOS 的 `syspolicyd`/YARA 状态再次让可执行启动卡住，Installer 不应无限停在“正在运行软件包脚本”。

当前已生成的本地包以 `build-pkg.sh` 或 `status.sh` 输出为准。不要沿用旧文档里的固定版本号；Installer 打开的包、`/Library/Input Methods` 里的 Host、菜单栏运行进程必须是同一个 CDHash，真实输入 smoke 才有意义。

安装前建议的分层验证顺序：

```bash
./macos/InputiaInputMethod/dev-fast.sh
# 只有安装/重装链路变更后：
./macos/InputiaInputMethod/install-check.sh
# 只有发布前或安装脚本变更后：
./macos/InputiaInputMethod/release/full-check.sh
```

安装后建议的收口顺序：

```bash
./macos/InputiaInputMethod/await-system-install.sh
INPUTIA_RUN_UI_SMOKE=1 ./macos/InputiaInputMethod/smoke-preflight.sh
INPUTIA_RUN_UI_SMOKE=1 ./macos/InputiaInputMethod/post-install-regression.sh
```

如果 `await-system-install.sh` 输出 `systemMatchesBuild=false`、`tis.enabledMatches=0`、`tis.hansIconMatchesApp=false` 或 `postInstallTISReady=false`，先解决系统安装/TIS 刷新问题，不要硬跑 GUI smoke。单独排查 TextEdit 行为时可以分别跑 `smoke-textedit.sh` 和 `smoke-textedit-command-shortcuts.sh`；单独排查 Safari 行为时可以分别跑 `smoke-safari-typing.sh`、`smoke-safari-command-shortcuts.sh` 和 `smoke-safari-enter.sh`，但仍必须遵守同一套 preflight/no-launch 门禁。

用户级安装：

```bash
./macos/InputiaInputMethod/install-user.sh
```

`install-user.sh` 会把当前 build 安装到 `~/Library/Input Methods` 和 `~/Applications`，并输出 `userInstallVerified=true` 证明文件 CDHash 匹配。这个结果只代表文件安装成功，不代表真实 GUI smoke 已可运行。脚本会继续输出 `userInstallTISReady=true|false`；只有 `true` 才说明 TIS Hans mode 指向用户级 app。若系统 `/Library/Input Methods/InputiaInputMethod.app` 仍是旧版本，macOS 可能继续把 Hans/Hant mode 解析到系统 app，此时会显示 `userInstallTISReady=false`，不要绕过 smoke 的 app-match 门禁。

菜单图标：`inputia.pdf` 保持 Squirrel/系统输入法常见的 mode icon 文件路线，但资源本身已经换成 16x16、透明背景、无白色填充的文本 PDF，避免菜单里出现白色方块。

打开安装器：

```bash
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
  INPUTIA_CODESIGN_OPTIONS="--options runtime" \
  ./macos/InputiaInputMethod/open-installer.sh
```

`open-installer.sh` 会构建带 CDHash 的唯一 pkg 文件名，并打开刚生成的那一个，避免 Installer 缓存旧 digest。默认开发验证只运行它的 preflight 和 dry-run 自检，不会构建 pkg，也不会打开 Installer。

如果当前环境不能非交互取得管理员权限，先生成不抢前台的安装交接清单：

```bash
./macos/InputiaInputMethod/install-handoff.sh
```

`install-handoff.sh` 不会打开 Installer，也不会改系统输入源；它会重建并验证最新 pkg，然后在 `build/install-handoff.txt` 里写入 source branch/commit/upstream/dirty、pkg 路径、SHA256、`pkgVerificationPassed`、当前 build CDHash、系统已安装 CDHash、设置启动器的 `InputiaExpectedHostCDHash` 匹配状态、当前 `install-check` 的 block reasons / primary required action / ordered required actions、推荐的一条 `applyCurrentHandoffCommand`、低层管理员安装命令，以及安装后的 `await-system-install.sh` / `install-check.sh` 验证命令。若当前状态含 `tis-duplicate-matches`，交接清单会额外写入显式 opt-in 的 `INPUTIA_REPAIR_TIS_DUPLICATES=1 ./repair-tis-duplicates.sh`。这个 repair 脚本要求 `/Library/Input Methods/InputiaInputMethod.app` 已经等于当前 build；如果 `systemMatchesBuild=false`，它会拒绝修复并要求先完成管理员安装。交接清单里的通过标准会以 `successCriteria.*` 单独列出，避免和当前状态 key 混淆；实际 `install-check.sh` 输出必须到 `systemMatchesBuild=true`、`settingsMatchesBuild=true`、`installCheckBlockReasons=none`、`installCheckRequiredActions=none`、`installCheckTISDuplicateMatches=false`、`runningMatchesBuild=true`、`installCheckPassed=true` 才算当前 build 真正进入系统运行态。

如果要从当前交接清单继续完整安装链路，可在终端显式运行：

```bash
cd macos/InputiaInputMethod
INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh
```

`apply-current-handoff.sh` 会先确认 `install-handoff.txt` 属于当前 commit/pkg，然后按 `install-check` 的动作链执行：管理员安装当前 pkg、必要时显式修复 TIS duplicate、等待系统安装生效、最后重跑 `install-check.sh`。默认不打开 GUI、不触碰菜单栏；若没有非交互 sudo 且未设置 `INPUTIA_ALLOW_ADMIN_PROMPT=1`，它会退出并打印继续命令，而不是卡住。设置 `INPUTIA_ALLOW_ADMIN_PROMPT=1` 后，交互终端默认走 `sudo`，非交互环境默认走 `osascript ... with administrator privileges` 的 macOS 管理员授权弹窗；可用 `INPUTIA_ADMIN_PROMPT_MODE=sudo|osascript|auto` 强制选择。`osascript` 授权默认最多等待 300 秒，可用 `INPUTIA_ADMIN_PROMPT_TIMEOUT_SECONDS=0` 关闭超时；超时会输出 `applyCurrentHandoffPassed=false reason=admin-authorization-timeout` 和继续命令，并清掉挂起授权进程。管理员安装后如果 `install-check` 仍报告 `admin-install-current-handoff` 或 `run-install-handoff-and-admin-install`，脚本会输出 `applyCurrentHandoffPassed=false reason=admin-install-did-not-update-system` 并停止，不能继续修 TIS duplicate。最终 `install-check` 未通过时必须输出 `applyCurrentHandoffPassed=false reason=final-install-check-failed` 和下一步动作，不能把半安装态误报为成功。`INPUTIA_APPLY_CURRENT_HANDOFF_SELF_CHECK=1` 会用 test-only 分支验证完整成功动作链顺序、非交互管理员弹窗路径、授权超时路径和“安装后仍需管理员安装时不得继续 repair”的保护，不需要 sudo、不修系统 TIS、不等待真实安装。

`postinstall` 会打印 `inputiaInstalledVersion` / `inputiaInstalledCDHash`，kill 旧 Host、清理旧用户级 Host、register 当前系统 Host，并 dump enabled/current TIS 状态。它默认不 enable/select，也不刷新菜单栏代理；重复输入源或手动添加问题用 `repair-tis-duplicates.sh` 或 System Settings 显式处理，避免安装脚本继续制造 HIToolbox 重复项。

用户级安装诊断：

```bash
./macos/InputiaInputMethod/install-user.sh
./macos/InputiaInputMethod/verify-system.sh "$HOME/Library/Input Methods/InputiaInputMethod.app"
```

删除安装：

```bash
./macos/InputiaInputMethod/uninstall-user.sh
./macos/InputiaInputMethod/uninstall-system.sh
```

注意：macOS enabled/selectable 入口已经打通；当前目标已推进到真实 IMK 事件链上的 Core/Rime/候选窗验证。

当前 Host 使用 `com.inputia.inputmethod.Inputia`，并通过 `ComponentInputModeDict` 暴露 `com.inputia.inputmethod.Inputia.Hans` 和 `com.inputia.inputmethod.Inputia.Hant`。诊断命令会 register parent，并按 Squirrel 的路线 enable/select primary mode。改名后当前机器如果仍未安装新 pkg，`includeAllInstalled=false` 只会枚举旧 `com.iputia...`；这不是新版 build 失败，而是系统目录还没迁移到 `/Library/Input Methods/InputiaInputMethod.app`。仍然不要把 all-installed 可见或 `TISEnableInputSource == noErr` 单独当成成功标准。

历史已验证能力包括：

```text
includeAllInstalled=false 可枚举 Inputia
TISSelectInputSource=0
TextEdit 英文直通可上屏
TextEdit 拼音 zhongguo + Space 可上屏 中国
TextEdit 拼音 zhongguo + 数字 1 可选择候选并上屏 中国
build 版自有候选窗可在 TextEdit 中显示候选条
build 版默认中文自检 ni + Space 可提交 你
物理键盘 Shift flagsChanged 可切换 mode=English / mode=Chinese
```

设置入口：

- 输入法菜单中有 `Inputia 设置...`，实现方式与 Squirrel 一样走 `IMKInputController.menu()`。
- 输入法菜单中有 `召回剪贴板`；也可以在 Inputia 激活时用 `Ctrl+Shift+V` 调出本地剪贴板候选。Host 会先检查 settings 的隐私开关和敏感 App 列表，允许时才读取当前系统剪贴板并学习为本地候选。
- 安装当前 `InputiaInputMethod-latest.pkg` 或 `build-pkg.sh` 输出的带 CDHash 包后，还会出现 `/Applications/Inputia 设置.app`。这是独立设置启动器，会打开同一个 `Inputia 设置` 窗口，不依赖 `IMKInputController.menu()` 本轮是否被系统菜单展示。
- 如果输入法菜单当前没有显示设置项，可以直接运行 `./macos/InputiaInputMethod/open-settings.sh`；它会优先打开 `/Applications/Inputia 设置.app`，找不到启动器时再回退到 `InputiaInputMethod.app --open-settings`。
- `open-settings.sh` 会跳过和当前 build 版本或 Host CDHash 不一致的旧设置启动器/旧 Host，并回退到当前 build 的设置页，避免本地调试时误开同版本旧 UI。这个选择逻辑已经纳入 `dev-fast.sh` 的 dry-run 自检，不会真的打开设置窗口。
- 用户截图里如果仍显示 `Iputia 简体`，说明系统当前选中的还是旧 typo 包；必须安装当前 `InputiaInputMethod-v<version>-<cdhash>.pkg` 或运行 system install，让菜单、设置窗口和 bundled RimeData 同步到新包。
- 第一版设置窗口写入 `~/Library/Application Support/Inputia/settings.json`。
- 当前可配置：输入方案、候选数量、Shift 切换、中文模式英文标点、本地记忆、隐私学习、敏感 App 排除列表。保存后 Host 会在 App/context 更新时热重载 settings。

候选窗视觉显示、默认 7 个候选、下箭头展开、上下翻页和 Command 修饰键透传都已纳入非 GUI `dev-fast.sh` 自检；真实宿主 GUI smoke 仍只在安装版 CDHash 对齐并显式 opt-in 后运行。

当前机器的最新事实以 `status.sh` 和 `EVIDENCE.md` 末尾为准。不要把上面的历史能力清单当成当前系统安装版已经匹配当前 build 的证据。
