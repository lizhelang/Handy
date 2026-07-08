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

查看当前 build、系统安装、运行进程、设置启动器和最新安装包是否同版本：

```bash
./macos/InputiaInputMethod/status.sh
```

如果 `systemMatchesBuild=false` 或 `runningMatchesBuild=false`，菜单栏里实际运行的仍是旧 Host；这时不要继续用当前菜单行为判断新版输入逻辑。`status.sh` 还会输出 `statusGuiSmokeBlockReasons=...` 和 `statusGuiSmokeReady=true|false`；只有 `statusGuiSmokeReady=true reason=none` 才能进入真实 TextEdit/Safari/Clipboard GUI smoke，否则只允许跑非 GUI/no-launch 验证。

验证最新 pkg 是否确实携带当前 build 和最新 postinstall：

```bash
./macos/InputiaInputMethod/verify-pkg.sh
```

`verify-pkg.sh` 只展开本地 pkg 到临时目录，不安装系统包。它会校验：

- pkg `PackageInfo` version 等于当前 build version。
- pkg 内 `postinstall` 与 `Packaging/scripts/postinstall` sha256 一致。
- pkg 内嵌 host app 的 CDHash 等于当前 build app。
- pkg 内嵌设置启动器版本等于当前 build 设置启动器。
- postinstall 仍包含用户级 host 清理、TIS select、TextInputMenuAgent/SystemUIServer 刷新等关键动作。

安装前一键非 GUI 验证：

```bash
./macos/InputiaInputMethod/verify-nongui.sh
```

`verify-nongui.sh` 是默认安全收口入口，不打开 TextEdit/Safari。它会串起脚本语法、pkg/build 对齐、`status.sh`、`smoke-preflight.sh` 的安全失败路径、快捷键自检、TextEdit 输入 smoke、TextEdit 代表性 `Command-A/C/V` smoke、Clipboard recall smoke、Safari typing smoke、Safari 代表性 `Command-A/C/V` smoke、Safari enter smoke 的 UI-disabled/TIS-not-ready/existing-app no-launch 门禁，确认这些早退路径不改剪贴板、不改当前输入源、不留下 GUI 进程；同时覆盖 `post-install-regression.sh` 非 GUI 模式、`post-install-regression.sh` 的 UI/TIS-not-ready no-launch 门禁、`await-system-install.sh` 短超时、无弹框管理员安装门禁、残留进程和 `/tmp/inputia-*` 临时文件检查。快捷键自检不是只测复制/粘贴：它要求所有含 `Command` 的 keyDown 组合透传，并覆盖常见 Apple `Command` 快捷键集合以及常见 AppKit command selector。默认不运行 `osacompile` 编译目标 App AppleScript，避免触发 TextEdit/Safari 字典加载；需要做本机 AppleScript 语法检查时可显式设置 `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1`。只有需要真实输入链验证且 `smoke-preflight.sh` 已报告 `smokePreflightReady=true` 时，才显式设置 `INPUTIA_RUN_UI_SMOKE=1` 跑 GUI smoke。

`verify-nongui.sh` 会使用 `/tmp/inputia-verify-nongui.lock` 防止两条聚合验证并发互相污染残留判断。活锁存在时会返回 rc=20，并输出 `nonGuiVerificationPassed=false reason=verify-already-running` 与 `verifyLockOwnerPid=...`；pid 不存在的 stale lock 会自动清理并继续验证。不要手工删除仍在运行的 owner pid 对应锁；异常中断后再次运行脚本即可自愈。

只读 TIS readiness 诊断：

```bash
./macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
```

`tis-readiness.sh` 不切换输入源、不打开 GUI App，只报告待测 app CDHash、TIS Hans mode 的 icon 是否指向待测 app、是否 enabled/selectable/selected，以及当前输入源。它适合在 Safari/TextEdit 已被用户打开、`smoke-preflight.sh` 会提前停下时，单独确认系统安装和 TIS 状态是否已经足够支持真实 GUI smoke。

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

这一步会写入 `/Library/Input Methods/InputiaInputMethod.app`，macOS 会要求管理员授权。安装脚本会清理旧 typo 路径 `/Library/Input Methods/IputiaInputMethod.app`。复制并校验 CDHash 后，脚本会先跑 `spctl --assess --type execute`；如果输出不是 accepted，脚本会打印 `systemInstallRequiredAction=sign-with-accepted-identity`、清理 Inputia 输入源偏好并停止，不再 register/enable/select。首次安装或替换后，如果 System Settings 的添加列表没有热刷新，按 ToyIMK/Squirrel/vChewing 对照经验，需要 logout/login 后再打开 System Settings > Keyboard > Text Input > Edit > Add。

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

输出：`macos/InputiaInputMethod/dist/InputiaInputMethod-v<version>-<cdhash>.pkg`。当前包是 `--nopayload` 形态，Host 被打成无 AppleDouble 的 `InputiaInputMethod.app.tar.gz` 放在 scripts 中，由 `postinstall` 解压到 `/Library/Input Methods` 后 register/enable/select；设置启动器被打成 `InputiaSettings.app.tar.gz`，安装到 `/Applications/Inputia 设置.app`。这样避免当前 macOS 环境把 `com.apple.provenance` xattr 编进 payload 文件列表。不要在 Installer 已经打开后重建同一个 pkg 路径；Installer 会检测 digest 变化并拒绝安装。

`postinstall` 使用 `/bin/zsh`，并对 LaunchServices 注册和 Inputia 自身的 register/enable/select 命令加了超时保护；如果当前 macOS 的 `syspolicyd`/YARA 状态再次让可执行启动卡住，Installer 不应无限停在“正在运行软件包脚本”。

当前已生成的本地包以 `build-pkg.sh` 或 `status.sh` 输出为准。不要沿用旧文档里的固定版本号；Installer 打开的包、`/Library/Input Methods` 里的 Host、菜单栏运行进程必须是同一个 CDHash，真实输入 smoke 才有意义。

安装前建议的非 GUI 验证顺序：

```bash
./macos/InputiaInputMethod/build.sh
./macos/InputiaInputMethod/build-pkg.sh
./macos/InputiaInputMethod/verify-pkg.sh
./macos/InputiaInputMethod/status.sh
./macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
./macos/InputiaInputMethod/smoke-preflight.sh
INPUTIA_RUN_UI_SMOKE=0 ./macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
./macos/InputiaInputMethod/verify-nongui.sh
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

`open-installer.sh` 会构建带 CDHash 的唯一 pkg 文件名，并打开刚生成的那一个，避免 Installer 缓存旧 digest。

`postinstall` 会打印 `inputiaInstalledVersion` / `inputiaInstalledCDHash`，并按 Squirrel 的路线 kill 旧 Host、register、以登录用户 enable/select，然后刷新 Text Input 菜单服务。

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
- `open-settings.sh` 会跳过和当前 build 版本不一致的旧设置启动器/旧 Host，并回退到当前 build 的设置页，避免本地调试时误开旧 UI。
- 用户截图里如果仍显示 `Iputia 简体`，说明系统当前选中的还是旧 typo 包；必须安装当前 `InputiaInputMethod-v<version>-<cdhash>.pkg` 或运行 system install，让菜单、设置窗口和 bundled RimeData 同步到新包。
- 第一版设置窗口写入 `~/Library/Application Support/Inputia/settings.json`。
- 当前可配置：输入方案、候选数量、Shift 切换、中文模式英文标点、本地记忆、隐私学习、敏感 App 排除列表。保存后 Host 会在 App/context 更新时热重载 settings。

候选窗视觉显示已用 build 版进程验收；分页热键还没有完全验收，继续看 `EVIDENCE.md` 的最新限制记录。

当前机器的最新事实以 `status.sh` 和 `EVIDENCE.md` 末尾为准。不要把上面的历史能力清单当成当前系统安装版已经匹配当前 build 的证据。
