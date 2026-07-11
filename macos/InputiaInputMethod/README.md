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

如果 `systemMatchesBuild=false` 或 `runningMatchesBuild=false`，菜单栏里实际运行的仍是旧 Host；这时不要继续用当前菜单行为判断新版输入逻辑。

TextEdit 真实输入 smoke：

```bash
./macos/InputiaInputMethod/smoke-textedit.sh
```

这个脚本会先比较 build app 和 `/Library/Input Methods` 中已安装 app 的 CDHash，并拒绝在旧系统包上误测。通过前置检查后，它会用 TextEdit 验证默认中文 `ni + Space`、Shift 切到英文 `ni + Space`、再 Shift 回中文。

等待管理员安装完成并自动收口：

```bash
./macos/InputiaInputMethod/await-system-install.sh
```

这个脚本会等待 `/Library/Input Methods/InputiaInputMethod.app` 的 CDHash 变成当前 build CDHash；观察到替换后自动运行 `post-install-regression.sh`，验证系统目录、自检、TextEdit 真实输入 smoke、Safari 本地输入框 source context，并确认旧 typo 版 `IputiaInputMethod.app` 已被清掉。

安装后回归：

```bash
./macos/InputiaInputMethod/post-install-regression.sh
```

这个脚本会打开一个 Safari `data:` 本地测试页，只检查 Safari 新输入框实际保留的 Text Input Source，不向外部网站发送内容。

系统级安装诊断：

```bash
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
  INPUTIA_CODESIGN_OPTIONS="--options runtime" \
  ./macos/InputiaInputMethod/install-system.sh
```

这一步会写入 `/Library/Input Methods/InputiaInputMethod.app`，macOS 会要求管理员授权。安装脚本会清理旧 typo 路径 `/Library/Input Methods/IputiaInputMethod.app`。首次安装或替换后，如果 System Settings 的添加列表没有热刷新，按 ToyIMK/Squirrel/vChewing 对照经验，需要 logout/login 后再打开 System Settings > Keyboard > Text Input > Edit > Add。

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

当前机器已验证：

```text
includeAllInstalled=false 可枚举 Inputia
TISSelectInputSource=0
系统安装版本 v4，CDHash 61d7a3b826eb281e9b19e774d465ecc23e2548a7
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
- 安装 v7 包后还会出现 `/Applications/Inputia 设置.app`。这是独立设置启动器，会打开同一个 `Inputia 设置` 窗口，不依赖 `IMKInputController.menu()` 本轮是否被系统菜单展示。
- 如果输入法菜单当前没有显示设置项，可以直接运行 `./macos/InputiaInputMethod/open-settings.sh`；它会优先打开 `/Applications/Inputia 设置.app`，找不到启动器时再回退到 `InputiaInputMethod.app --open-settings`。
- `open-settings.sh` 会跳过和当前 build 版本不一致的旧设置启动器/旧 Host，并回退到当前 build 的设置页，避免本地调试时误开旧 UI。
- 用户截图里如果仍显示 `Iputia 简体`，说明系统当前选中的还是旧 typo 包；必须安装新版 `InputiaInputMethod-v7-*.pkg` 或运行 system install，让菜单、设置窗口和 bundled RimeData 同步到新包。
- 第一版设置窗口写入 `~/Library/Application Support/Inputia/settings.json`。
- 当前可配置：输入方案、候选数量、Shift 切换、中文模式英文标点、本地记忆、隐私学习、敏感 App 排除列表。保存后 Host 会在 App/context 更新时热重载 settings。

候选窗视觉显示已用 build 版进程验收；分页热键还没有完全验收，继续看 `EVIDENCE.md` 的最新限制记录。
