# Inputia macOS IMK Host 证据

## 2026-07-07 v6 设置入口稳定化与旧 Iputia 残留定位

用户截图里输入法菜单仍显示 `Iputia 简体`，且设置页里只看到“全拼/小鹤双拼”。本轮重新查了当前系统安装状态，结论是：

- `/Library/Input Methods/InputiaInputMethod.app` 不存在。
- `/Library/Input Methods/IputiaInputMethod.app` 仍存在，`CFBundleIdentifier=com.iputia.inputmethod.Iputia`，`CFBundleVersion=4`。
- 因此用户当前菜单里跑的是旧 typo 版 Host，不是新版 `Inputia` v5/v6 build；设置窗口和 bundled RimeData 都还是旧能力面。

参考实现/证据：

- Squirrel 当前 Swift 版也通过 `IMKInputController.menu()` 添加 `Settings...`、`Deploy` 等输入法菜单项；这条路线保留，但它依赖 macOS 已经为当前 client 拉起 controller。
- Squirrel 启动面还提供命令行维护入口，并由 app delegate 持有全局状态。Inputia 现在同样保留 `--open-settings`，并进一步增加独立设置启动器，避免用户只能靠 IMK 菜单项。

本轮实现：

- 新增 `macos/InputiaInputMethod/SettingsLauncher/main.swift` 和 `SettingsLauncher/Info.plist`，构建 `Inputia 设置.app`。
- `build.sh` 同时构建并签名：
  - `InputiaInputMethod.app`
  - `Inputia 设置.app`
- `build-pkg.sh` 的 nopayload pkg scripts 现在带两个 archive：
  - `InputiaInputMethod.app.tar.gz`
  - `InputiaSettings.app.tar.gz`
- `postinstall` 会把 Host 解压到 `/Library/Input Methods/InputiaInputMethod.app`，并把设置启动器解压到 `/Applications/Inputia 设置.app`；同时继续删除旧 `/Library/Input Methods/IputiaInputMethod.app`。
- `open-settings.sh` 优先打开 `/Applications/Inputia 设置.app`，然后才回退到 `InputiaInputMethod.app --open-settings`。
- `install-system.sh` / `install-user.sh` / uninstall / await 脚本同步切到 zsh，并纳入设置启动器安装/删除。
- `CFBundleVersion` 升到 `6`，避免继续混淆旧 v4/v5 安装状态。

验证：

```text
zsh -n build/install/uninstall/open/postinstall scripts
  OK

plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

./macos/InputiaInputMethod/build-pkg.sh
  InputiaInputMethod.app: valid on disk
  Inputia 设置.app: valid on disk
  dist pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v6-f57364931673.pkg
  appCDHash=f573649316734fa609a89f3854bb0c05fe4128a7
  payloadFiles=0

build Info.plist:
  Host CFBundleIdentifier=com.inputia.inputmethod.Inputia
  Host CFBundleVersion=6
  Settings CFBundleIdentifier=com.inputia.inputmethod.Inputia.Settings
  Settings CFBundleVersion=6

pkg expand:
  Scripts/postinstall
  Scripts/InputiaInputMethod.app.tar.gz
  Scripts/InputiaSettings.app.tar.gz

tar list:
  InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod
  InputiaInputMethod.app/Contents/Resources/RimeData/double_pinyin_sogou.schema.yaml
  InputiaInputMethod.app/Contents/Resources/RimeData/guobiao_bispell.schema.yaml
  Inputia 设置.app/Contents/MacOS/InputiaSettingsLauncher
  Inputia 设置.app/Contents/Resources/Inputia.icns

INPUTIA_INSTALL_ROOT=/tmp/inputia-v6-root INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=6
  inputiaInstalledCDHash=f573649316734fa609a89f3854bb0c05fe4128a7
  inputiaSettingsLauncherInstalled=true
  /tmp/inputia-v6-root/Library/Input Methods/InputiaInputMethod.app
  /tmp/inputia-v6-root/Applications/Inputia 设置.app
```

限制：

- 当前 Codex/macOS 环境直接执行 build `.app` 的 `--self-check` 仍会在启动阶段超时，没有诊断输出；这与前面记录的 `syspolicyd`/YARA 环境问题一致。本轮用 Swift 编译、codesign、plist、pkg 展开、临时 root postinstall 来证明安装包内容正确。
- 由于当前会话没有免密 sudo，不能直接替换 root-owned `/Library/Input Methods/IputiaInputMethod.app`。需要通过 v6 pkg 的系统安装授权完成实际迁移；安装后旧菜单项应从 `Iputia 简体` 变成 `Inputia 简体`。

### v6 Installer 失败 UI 根因与 v7 修复

用户授权安装 v6 后，Installer UI 显示“安装失败”。系统状态检查显示 Host 和设置启动器其实已经落地：

```text
/Library/Input Methods/InputiaInputMethod.app
  CFBundleVersion=6
  CDHash=f573649316734fa609a89f3854bb0c05fe4128a7

/Applications/Inputia 设置.app
  CFBundleVersion=6

/Library/Input Methods/IputiaInputMethod.app
  missing
```

`/var/log/install.log` 给出真实失败点：

```text
./postinstall: inputiaInstalledVersion=6
./postinstall: inputiaInstalledCDHash=f573649316734fa609a89f3854bb0c05fe4128a7
./postinstall: inputiaSettingsLauncherInstalled=true
./postinstall: run_with_timeout:19: read-only variable: status
Install Failed: Error Domain=PKInstallErrorDomain Code=112
```

根因：`postinstall` 已切到 zsh，但 `run_with_timeout()` 里使用了变量名 `status`；`$status` 是 zsh 的只读特殊变量。文件安装已完成，脚本在后续 register/enable/select 之前因为这个变量名退出，导致 Installer UI 判失败。

修复：

- `status` 改名为 `exit_status`。
- Host 和设置启动器版本升到 v7，避免继续混淆 v6 的失败安装记录。

验证：

```text
zsh -n postinstall/build/open/install/uninstall scripts
  OK

plutil -lint Info.plist SettingsLauncher/Info.plist
  OK

./macos/InputiaInputMethod/build-pkg.sh
  dist pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v7-161e70316e6e.pkg
  appCDHash=161e70316e6e4c21e6e07d6b3abe561a794365e3
  payloadFiles=0

pkg expand + tar list:
  InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod
  InputiaInputMethod.app/Contents/Resources/RimeData/double_pinyin_sogou.schema.yaml
  InputiaInputMethod.app/Contents/Resources/RimeData/guobiao_bispell.schema.yaml
  Inputia 设置.app/Contents/MacOS/InputiaSettingsLauncher

INPUTIA_INSTALL_ROOT=/tmp/inputia-v7-root postinstall
  inputiaInstalledVersion=7
  inputiaInstalledCDHash=161e70316e6e4c21e6e07d6b3abe561a794365e3
  inputiaSettingsLauncherInstalled=true
  postinstallStatus=0
```

## 2026-07-07 剪贴板召回入口

本轮目标是把“剪贴板历史影响候选”推进为输入层可触发的剪贴板召回入口，同时保留敏感 App 默认不读不学的隐私边界。

实现：

- `inputia-core` 新增 `LocalMemory::source_candidates` / `clipboard_candidates`，只返回 `MemorySource::Clipboard` 学到的词条，不混入语音或打字来源。
- `SqliteMemory` 暴露 `clipboard_candidates`，让已导入 Handy `clipboard.db` 或当前剪贴板学习到的内容可以被召回。
- `inputia-capi` 新增 `inputia_session_clipboard_candidates(session, limit)`，返回本地候选 JSON；同时保留已有 `inputia_session_learn(..., SOURCE_CLIPBOARD, ...)` 学习入口。
- `InputiaRustBridge` 新增：
  - `learnClipboard(text:bundleId:)`
  - `clipboardCandidates(limit:)`
  - `shouldReadClipboard(bundleId:)`
- `InputiaInputController.menu()` 增加 `召回剪贴板` 菜单项；Host 也支持 `Ctrl+Shift+V` 触发召回。
- Host 在读取 `NSPasteboard.general` 之前先调用 `shouldReadClipboard(bundleId:)`，只有 settings 中 `memory_enabled=true`、`privacy_learning_enabled=true`，且当前 bundle id 不在敏感列表、也不是 `unknown` 时才读取剪贴板。
- 召回候选使用现有 `InputiaCandidatePanel` 展示；数字键、Space、Return 可上屏候选，Escape/Delete 关闭召回。

隐私边界：

- 当前 App 如果是 `com.1password.1password` 等 settings 敏感列表项，Host 不读取剪贴板。
- 如果 IMK client 无法提供 bundle id，Host 也不读取剪贴板。
- 敏感 App 的过滤仍是 bundle id 粒度；Safari 隐私窗口这类窗口级识别还没有系统级可靠证据，本轮不假装已经完成。

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  16 passed
  includes clipboard_recall_returns_only_clipboard_terms

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  7 passed
  includes inputia_session_clipboard_candidates via capi_memory_reranks_candidates_and_respects_sensitive_apps

nm -gU crates/inputia-capi/target/release/libinputia_capi.a
  _inputia_session_clipboard_candidates
  _inputia_session_learn
  _inputia_session_voice_hotwords

./macos/InputiaInputMethod/build-pkg.sh
  appCDHash=879ddfeb5aa170e20819a359e96750e81ed054c6
  dist pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v5-879ddfeb5aa1.pkg

codesign --verify --deep --strict --verbose=2 macos/InputiaInputMethod/build/InputiaInputMethod.app
  valid on disk
  satisfies its Designated Requirement

plutil -lint macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/Info.plist
  OK

pkg expand + tar list:
  InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod
  InputiaInputMethod.app/Contents/Resources/inputia.pdf
  InputiaInputMethod.app/Contents/Resources/RimeData/default.yaml
  InputiaInputMethod.app/Contents/Resources/RimeData/double_pinyin_sogou.schema.yaml
  InputiaInputMethod.app/Contents/Resources/RimeData/guobiao_bispell.schema.yaml

INPUTIA_INSTALL_ROOT=/tmp/InputiaPostinstallRoot-clipboard INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=5
  inputiaInstalledCDHash=879ddfeb5aa170e20819a359e96750e81ed054c6
```

限制：

- `.bundle` 方式运行 `--bridge-clipboard-recall-self-check` / `--bridge-clipboard-privacy-self-check` 本轮再次卡在可执行启动阶段，无诊断输出；进程已中止。此前同类现象已定位为当前 Codex/macOS 环境的 `syspolicyd` / YARA 偶发卡死，不作为剪贴板逻辑失败证据。
- 还没有在已安装系统输入法进程中用真实菜单/快捷键完成 UI smoke；需要安装 `v5-879ddfeb5aa1` 后继续跑真实 App 验收。

## 2026-07-07 双拼方案扩展与设置入口兜底

用户反馈设置里只能选“全拼”和“小鹤双拼”，并且输入法菜单里的设置入口有时不出现。结论拆分如下：

- 不是底层只能支持两个方案；此前只是 Settings UI 和打包的 RimeData 只接了最小两项。
- 截图里菜单仍显示 `Iputia 简体`，说明系统当前还在使用旧 typo 安装包；新版 `Inputia` 包未替换前，菜单、设置入口和 RimeData 都会继续表现为旧包状态。
- `IMKInputController.menu()` 只有在 macOS 已经把 Inputia controller 拉起后才稳定出现；因此需要保留菜单项，同时增加一个独立 `--open-settings` / `open-settings.sh` 入口，避免用户被菜单状态卡住。

本轮实现：

- `InputiaSettingsWindow.swift` 的输入方案列表扩展为：中文全拼、自然码双拼、小鹤双拼、搜狗双拼、国标双拼、微软双拼、智能 ABC 双拼、拼音加加双拼、四通双拼。
- 新增 `prepare-rime-data.sh`，构建时把 Squirrel 的基础 Rime shared data 复制到 build RimeData，并补齐上述双拼 schema。
- `build.sh` 会把生成后的 `RimeData` 打进 `InputiaInputMethod.app/Contents/Resources/RimeData`。
- 默认 settings 会写入 bundled `RimeData` 路径；旧 settings 缺少 `rime_shared_data_dir` 时会被补齐，避免仍然走系统外部 RimeData。
- 新增 `open-settings.sh` 和 app 参数 `--open-settings`，可直接打开 `Inputia 设置` 窗口，不依赖输入法菜单项是否被当前 App 展示。
- pkg `postinstall` 从 `/bin/bash` 改成 `/bin/zsh`，并为 `lsregister`、`--register-input-source`、`--enable-input-source`、`--select-input-source` 加超时保护，避免 Installer 无限卡在“正在运行软件包脚本”。
- `build.sh`、`prepare-rime-data.sh`、`build-pkg.sh`、`open-installer.sh`、`open-settings.sh` 改为 `/bin/zsh`，避免当前环境里 `/bin/bash` 偶发卡在 dyld/syspolicyd 阶段。
- `inputia.pdf` 从旧白色方块 PDF 换成 16x16 透明背景文本 PDF，保留与 Squirrel/系统输入法一致的 `tsInputModeMenuIconFileKey` 路线，同时避免菜单里出现突兀白块。

方案来源：

- Rime 官方 `rime/rime-double-pinyin` 已收录自然码、智能 ABC、小鹤、微软、拼音加加、四通双拼，并依赖朙月拼音：<https://github.com/rime/rime-double-pinyin>
- 搜狗双拼不在 Rime 官方 double-pinyin 配方里；Inputia 先使用 iDvel/rime-ice 的搜狗双拼键位映射，保留朙月拼音词典，避免在 MVP 阶段引入 rime-ice Lua/词库依赖：<https://github.com/iDvel/rime-ice/blob/main/double_pinyin_sogou.schema.yaml>
- 国标双拼使用 `baopaau/rime-guobiao-quick` 的 `guobiao_bispell` schema：<https://github.com/baopaau/rime-guobiao-quick/blob/main/guobiao_bispell.schema.yaml>
- 国标双拼对应国家标准为 `GB/T 34947-2017 信息技术 汉语拼音双拼和三拼输入通用要求`，当前状态为现行：<https://openstd.samr.gov.cn/bzgk/std/newGbInfo?hcno=C653EF9094722AB37666AEAC31C5BE08>

构建产物验证：

```text
build CFBundleVersion=5
build bundleIdentifier=com.inputia.inputmethod.Inputia
build CDHash=879ddfeb5aa170e20819a359e96750e81ed054c6
dist pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v5-879ddfeb5aa1.pkg
pkg signature=no signature
pkg payloadFiles=0
```

打包的 RimeData 已包含：

```text
default.yaml
double_pinyin.schema.yaml
double_pinyin_abc.schema.yaml
double_pinyin_flypy.schema.yaml
double_pinyin_mspy.schema.yaml
double_pinyin_pyjj.schema.yaml
double_pinyin_sogou.schema.yaml
double_pinyin_st.schema.yaml
guobiao_bispell.schema.yaml
luna_pinyin_simp.schema.yaml
```

`default.yaml` 的 schema list 已包含：

```text
luna_pinyin_simp
double_pinyin
double_pinyin_flypy
double_pinyin_sogou
guobiao_bispell
double_pinyin_mspy
double_pinyin_abc
double_pinyin_pyjj
double_pinyin_st
```

Rime smoke：

```text
double_pinyin vsgo -> 中国
double_pinyin_flypy vsgo -> 中国
double_pinyin_mspy vsgo -> 中国
double_pinyin_sogou vsgo -> 中国
guobiao_bispell vsgo -> 中国
double_pinyin_abc vsgo -> no candidate for this key sequence, because ABC layout differs
```

单测/脚本验证：

```text
cargo test --manifest-path crates/inputia-settings/Cargo.toml  # 3 passed
cargo test --manifest-path crates/inputia-rime/Cargo.toml      # 3 passed
cargo test --manifest-path crates/inputia-capi/Cargo.toml      # 7 passed
zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/prepare-rime-data.sh macos/InputiaInputMethod/build-pkg.sh macos/InputiaInputMethod/open-installer.sh macos/InputiaInputMethod/open-settings.sh macos/InputiaInputMethod/Packaging/scripts/postinstall
./macos/InputiaInputMethod/build-pkg.sh
  appCDHash=879ddfeb5aa170e20819a359e96750e81ed054c6
  dist pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v5-879ddfeb5aa1.pkg
plutil -lint macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/Info.plist
codesign --verify --deep --strict --verbose=2 macos/InputiaInputMethod/build/InputiaInputMethod.app
pkg expand + tar list: pkg 内包含新增 schema
pkg expand: pkg 内 postinstall shebang 为 #!/bin/zsh
INPUTIA_INSTALL_ROOT=/tmp/InputiaPostinstallRoot-v5 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=5
  inputiaInstalledCDHash=879ddfeb5aa170e20819a359e96750e81ed054c6
pkg icon:
  pixelWidth=16 pixelHeight=16
  textPdf=true
  hasWhiteFill=false
  hasBlackFill=true
```

当前 Codex/macOS 环境限制：

- 本机日志出现 `syspolicyd` / YARA 扫描错误和 `Terminating process due to Malware rejection`；直接执行 `.app` 或 `/bin/bash` 有时会卡在 dyld 启动阶段。
- 这类现象与近期 Codex Desktop 触发 macOS `syspolicyd`/YARA/FD 问题的公开报告相符：<https://github.com/openai/codex/issues/28071>
- 为避免把环境问题误判为 Inputia 逻辑问题，`verify-system.sh` 已改为对原 `.app` 做 plist/resource/codesign 检查，但通过临时 `.bundle` 副本运行诊断命令，并为易卡命令加 timeout。
- 早先 `build-pkg.sh` 因 `/bin/bash` 启动卡住未能完整跑完；本轮已把打包链切到 zsh，`./macos/InputiaInputMethod/build-pkg.sh` 已能直接生成 `v5-879ddfeb5aa1` pkg，并确认包内 `postinstall` 是 zsh + timeout 版本。

## 2026-07-07 命名修正：Iputia -> Inputia

用户确认产品名早期误写，正确名称应为 `Inputia`。

当前源码已完成命名迁移：

```text
macos/InputiaInputMethod
crates/inputia-core
crates/inputia-rime
crates/inputia-capi
crates/inputia-settings
crates/inputia-handy-runtime
CFBundleIdentifier=com.inputia.inputmethod.Inputia
TISInputSourceID=com.inputia.inputmethod.Inputia
InputModeID=com.inputia.inputmethod.Inputia.Hans
InputMethodConnectionName=com.inputia.inputmethod.Inputia_Connection
InputMethodServerControllerClass=InputiaInputMethod.InputiaInputController
C ABI prefix=inputia_
```

安装迁移策略：

- 新安装路径为 `/Library/Input Methods/InputiaInputMethod.app`。
- 旧 typo 版路径 `/Library/Input Methods/IputiaInputMethod.app` 只作为 legacy migration 处理；system/pkg/user 安装脚本会 unregister 并删除它，避免系统菜单同时出现旧名和新名。
- 当前机器的系统目录如果仍显示 `com.iputia.inputmethod.Iputia.Hans`，说明尚未安装新版 Inputia 包，不是新版 build 失败。

本轮改名验证：

```text
./macos/InputiaInputMethod/build.sh
./macos/InputiaInputMethod/verify-system.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
cargo test --manifest-path crates/inputia-core/Cargo.toml            # 15 passed
cargo test --manifest-path crates/inputia-rime/Cargo.toml            # 3 passed
cargo test --manifest-path crates/inputia-capi/Cargo.toml            # 7 passed
cargo test --manifest-path crates/inputia-settings/Cargo.toml        # 3 passed
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml   # 3 passed
```

安装后回归脚本：

```text
./macos/InputiaInputMethod/post-install-regression.sh
```

该脚本会在管理员安装新版 Inputia 后检查：

- `/Library/Input Methods/InputiaInputMethod.app` 存在且可执行。
- 旧 typo 版 `/Library/Input Methods/IputiaInputMethod.app` 不再存在。
- `verify-system.sh` 通过 plist、资源、codesign、controller class、自检、桥接层自检。
- `smoke-textedit.sh` 验证 TextEdit 真实 IMK 输入链：默认中文、Shift 到英文、Shift 回中文。
- `diagnose-safari-input-source.sh` 打开 Safari 本地 `data:` 测试页，只检查新输入框实际保留的 Text Input Source，不向外部网站发送内容。

新包：

```text
macos/InputiaInputMethod/dist/InputiaInputMethod-v5-5ad492c6f04b.pkg
appCDHash=5ad492c6f04b8934f07b8c63a20074f167a1962d
```

当前系统状态：

```text
build bundleIdentifier=com.inputia.inputmethod.Inputia
build CFBundleVersion=5
build CDHash=5ad492c6f04b8934f07b8c63a20074f167a1962d
TIS includeAllInstalled=false matches for com.inputia.inputmethod.Inputia = 0
current source = com.iputia.inputmethod.Iputia.Hans
```

解释：新版 Inputia build/pkg 已就绪，但尚未通过管理员授权安装到 `/Library/Input Methods/InputiaInputMethod.app`，所以 macOS Text Input Services 当前仍只认识旧 typo 版 `Iputia 简体`。这不是新版 build 失败；安装新版包后再用 `post-install-regression.sh` 验证。

日期：2026-07-06

## 2026-07-06 菜单图标过大 / 不能中文输入 / Shift 反馈复盘

用户截图中菜单栏已出现 `Inputia 简体`，但 mode icon 显示成过大的白色方块，TextEdit 中 `ni` / `zhongguo` 仍按英文直通。这一轮只处理 macOS Host 真实菜单运行路径，不扩展 Rime/Core 业务能力。

根因拆分：

- 图标：旧 Host 把 `Resources/inputia.pdf` 配成 mode menu icon；该 PDF 实际是白色方块，导致菜单里出现比系统内置 `拼` 图标更突兀的白块。
- 中文默认模式：Host 曾用“启动时模拟一次 Shift”进入中文，这是翻转式状态控制，不适合作为初始状态；已改为 C ABI 显式 `inputia_session_set_input_mode(..., Chinese)`。
- 真实输入路径：build 版自检通过，但 TextEdit 按键没有进入手动启动的 build Host。`INPUTIA_DEBUG_EVENTS=/tmp/inputia-smoke/events.log` 运行 build Host 后，TextEdit `ni + Space` 仍输出 `ni `，且事件日志为空，说明 macOS Text Input 实际仍按已安装 source 记录连接 `/Library/Input Methods/InputiaInputMethod.app`，不是手动打开的 build app。
- 系统目录仍是旧包：`/Library/Input Methods/InputiaInputMethod.app` 仍为 `CFBundleVersion=1`，CDHash `90af63b5e0a49345a7d2df3f3e11a4faa2299b95`，签名时间 21:16，资源中仍有 `inputia.pdf`。当前 build 包为 `CFBundleVersion=2`，CDHash `bfbb0ab03dc7558babada1fb2f606d2b92b53425`，签名时间 22:15，sealed resources 只有 4 个文件，不再包含 `inputia.pdf`。
- 临时取消旧 `/Library` LaunchServices 记录、注册 build app 后，`TISCreateInputSourceList` 的 `kTISPropertyIconImageURL` 仍指向 `/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf`。这说明 TIS 对已安装输入源仍绑定系统目录，不能靠手动注册 build app 绕过管理员安装。

本轮修复：

- `Info.plist` 升到 `CFBundleShortVersionString=0.0.2` / `CFBundleVersion=2`，降低 LaunchServices/Text Input 缓存继续命中旧包的概率。
- 删除 `Resources/inputia.pdf`，mode 菜单图标只保留 `TISIconLabels`，让 macOS 使用文字标签生成菜单图标。
- `InputiaCore` 增加显式 `set_mode(InputMode)`；`inputia-capi` 增加 `inputia_session_set_input_mode`；Swift Host 的 `makeDefault()` 改为显式设置中文模式，不再用 Shift 翻转模拟初始中文。
- 增加 `--bridge-default-chinese-self-check`，用与 Host 相同的 `makeDefault()` 路径验证默认中文；`verify-system.sh` 会在二进制支持该命令时运行它，不支持时跳过，避免验证旧系统包时误启动 Host 并挂起。
- `install-system.sh` 增加 source/dest CDHash 对比，复制后必须输出 `systemInstallVerified=true`；`postinstall` 增加 `inputiaInstalledVersion` / `inputiaInstalledCDHash` 输出。两条安装路径都会在 register/enable/select 后刷新 `TextInputMenuAgent` 和 `SystemUIServer`。
- 增加 `smoke-textedit.sh`：先要求 build app 与 `/Library/Input Methods` 中的系统安装包 CDHash 一致，并且系统包不包含 `inputia.pdf`，再自动跑 TextEdit 真实按键 smoke。
- 增加 `await-system-install.sh`：等待 `/Library/Input Methods` 的 CDHash 变成当前 build CDHash；观察到替换后自动运行 `verify-system.sh` 和 `smoke-textedit.sh`。

验证结果：

```text
crates/inputia-core cargo test: 15 passed
crates/inputia-capi cargo test: 7 passed
swiftc -typecheck: OK
build resources:
  Inputia.icns
  en.lproj/InfoPlist.strings
  zh-Hans.lproj/InfoPlist.strings
  zh-Hant.lproj/InfoPlist.strings
build verify:
  CFBundleVersion=2
  CDHash=bfbb0ab03dc7558babada1fb2f606d2b92b53425
  bridgeDefaultChineseSelfCheck=true
  mode=Chinese
  firstCandidate=你
  commit=你
dist pkg:
  macos/InputiaInputMethod/dist/InputiaInputMethod-v2-bfbb0ab03dc7.pkg
  payloadFiles=0
  appCDHash=bfbb0ab03dc7558babada1fb2f606d2b92b53425
pkg expand:
  pkgAppCDHash=bfbb0ab03dc7558babada1fb2f606d2b92b53425
  Scripts/postinstall contains inputiaInstalledVersion and TextInputMenuAgent/SystemUIServer refresh
  Scripts/InputiaInputMethod.app.tar.gz resources do not include inputia.pdf
postinstall temp-root:
  inputiaInstalledVersion=2
  inputiaInstalledCDHash=bfbb0ab03dc7558babada1fb2f606d2b92b53425
  resources do not include inputia.pdf
smoke-textedit on current system install:
  expectedCDHash=bfbb0ab03dc7558babada1fb2f606d2b92b53425
  actualCDHash=90af63b5e0a49345a7d2df3f3e11a4faa2299b95
  textEditSmokeReady=false reason=cdhash-mismatch
await-system-install short timeout:
  expectedCDHash=bfbb0ab03dc7558babada1fb2f606d2b92b53425
  actualVersion=1 actualCDHash=90af63b5e0a49345a7d2df3f3e11a4faa2299b95
  systemInstallObserved=false reason=timeout
Installer UI:
  old fixed-path Installer failed because package digest changed after the window was opened
  log: enqueued digest is mismatched or has been swapped
  new flow opens dist/InputiaInputMethod-v2-bfbb0ab03dc7.pkg
  waiting at Install button before administrator authentication
```

真实系统安装边界：

```text
install-system.sh output:
  systemInstallNeedsAdmin=true
  macOS 管理员授权窗口未返回；安装进程已中止，避免后台挂起。

/Library/Input Methods/InputiaInputMethod.app:
  CFBundleVersion=1
  CDHash=90af63b5e0a49345a7d2df3f3e11a4faa2299b95
  resources include inputia.pdf
  bridgeDefaultChineseSelfCheck=skipped
```

因此当前代码/build/pkg 已修复用户截图中暴露的 Host 问题，但菜单栏实际体验只有在管理员授权替换 `/Library/Input Methods/InputiaInputMethod.app` 后才能切到新二进制。替换后必须重新 `killall TextInputMenuAgent` / `killall SystemUIServer`，再验证系统包 CDHash 等于当前 build CDHash，最后用 TextEdit 真实按键验证 `ni + Space -> 你`。

## 2026-07-06 Host enabled/selectable 阻塞复盘

本轮只处理 macOS IMK Host 是否能进入 enabled/selectable，不继续扩展 Core、Rime、Settings。

已重新查证：

- Apple Text Input Source Services：mode-enabled input method 的 parent 不直接 selectable；visible input mode 必须进入 enabled list 后才能 select，否则 `TISSelectInputSource` 返回 `paramErr/-50`。
- Squirrel：使用 `ComponentInputModeDict` 暴露 `Hans/Hant` modes；postinstall 顺序是 register，然后以登录用户 enable/select primary mode。
- ToyIMK：首次安装后需要 logout/login，再到 System Settings 添加输入源。

本机现状：

- `macos/InputiaInputMethod/build/InputiaInputMethod.app` 是最新版 Host baseline，包含 `Inputia.icns` 和 3 份 `InfoPlist.strings`；`codesign --verify --deep --strict` 通过，controller class 自检通过。
- `/Library/Input Methods/InputiaInputMethod.app` 已通过本地 pkg 替换为最新版，签名时间为 20:56，sealed resources 变为 5 个文件，曾包含 `Inputia.icns`、`inputia.pdf` 和 3 份 `InfoPlist.strings`；后续菜单 mode 图标已改为只使用 `TISIconLabels`，不再使用旧白色 PDF。
- 替换后 TIS all-installed 中 `com.inputia.inputmethod.Inputia.Hans` 的本地化名称变为 `Inputia 简体`，`Hant` 变为 `Inputia 繁体`。
- 关闭 System Settings，并重启 `TextInputMenuAgent` / `SystemUIServer` 后，System Settings > Keyboard > Text Input > Add > 简体中文 列表中出现 `Inputia 简体`。
- 因此“所有输入法里可见但添加列表没有 Inputia”的入口阻塞已经定位为旧系统安装包和 System Settings/Text Input UI 缓存问题。
- 已在 System Settings 中添加 `Inputia 简体`。添加后 `includeAllInstalled=false` 能枚举到 parent 和 `com.inputia.inputmethod.Inputia.Hans`，两者 `enabled=true`，Hans mode `selectable=true/selected=true`。
- `TISSelectInputSource` 对 `com.inputia.inputmethod.Inputia.Hans` 返回 `selectStatus=0`。
- TextEdit smoke：当前输入源为 `Inputia 简体` 时，使用真实 `System Events keystroke "xyz"` 输入，TextEdit 文本区变为 `abcxyz`，`InputiaInputMethod` 进程日志出现 InputMethodKit `Inserting text`，证明按键事件进入系统 IMK 输入链并能上屏。
- TextEdit smoke 产生的自动保存文件 `~/Library/Mobile Documents/com~apple~TextEdit/Documents/未命名.rtf` 内容确认为 `abcxyz` 后已删除。

添加前 TIS 结果是：

```text
includeAllInstalled=false
matches=0

includeAllInstalled=true
id=com.inputia.inputmethod.Inputia.Hans
type=TISTypeKeyboardInputMode
languages=zh-Hans
enabled=true
selectable=true

selectStatus=-50
```

添加后 TIS 结果是：

```text
includeAllInstalled=false
id=com.inputia.inputmethod.Inputia
enabled=true
selectable=false

id=com.inputia.inputmethod.Inputia.Hans
name=Inputia 简体
enabled=true
selectable=true
selected=true

selectStatus=0
```

这说明 macOS enabled/selectable 入口阻塞已经解除。后续可以回到 Host 事件细化、候选窗和 Core/Rime/Settings 集成验证，但仍应按官方文档/成熟实现/spike 证据推进。

## 2026-07-06 Host 接入 Core/Rime 的系统级 smoke

已按强制流程补证据：

- Apple InputMethodKit / 本机 SDK header：
  - `IMKInputController` 每个输入会话一个 controller；可以通过 `handleEvent:client:` 接收真实 `NSEvent`，或通过 `inputText:client:` 接收 keybinding 后的文本。
  - `recognizedEvents` 默认只处理 key down；需要接收 modifier 事件时必须声明 `.flagsChanged`。
  - `IMKCandidates` 可以通过 `setCandidateData` 直接提供候选，`show` 显示候选窗；最终选择会回调 `candidateSelected`。
- Squirrel 对照：
  - `SquirrelInputController.handle(_:client:)` 处理 `.keyDown` 和 `.flagsChanged`，Command 快捷键交还宿主 App。
  - Squirrel 使用 `client.setMarkedText` 维护 inline preedit，使用候选面板展示候选，commit 时 `client.insertText`。

本轮实现：

- `build.sh` 现在先构建 `crates/inputia-capi` release staticlib，再把 `main.swift`、`InputiaRustBridge.swift` 和 `libinputia_capi.a` 链进 `InputiaInputMethod.app`。
- `main.swift` 的 `InputiaInputController` 不再是 Swift-only baseline：
  - 普通字符进入 `InputiaRustBridge.handle(character:)`。
  - Space/Return、Backspace、Escape、PageUp/PageDown 和数字候选选择进入 Core special/digit handler。
  - Command/Control/Option 快捷键交还宿主 App。
  - 单独 Shift press/release 切换中英文，Shift+字母不会触发切换。
  - Core outcome 有 `commit` 时调用 `insertText`，有 `composing` 时调用 `setMarkedText`，候选列表交给 `InputiaCandidatePanel` 显示。
- `InputiaCandidatePanel.swift` 新增一个最小自有候选窗：使用非激活 `NSPanel`，按 Squirrel/vChewing 的路线用 client line-height rect 定位，不再依赖本机不稳定的 `IMKCandidates` 显示状态。
- `verify-system.sh` 增加 bridge self-check：
  - `--bridge-self-check`
  - `--bridge-memory-self-check`
  - `--bridge-settings-self-check`

构建/单测验证：

```text
cargo test -p inputia-capi equivalent: crates/inputia-capi cargo test
result: 6 passed

crates/inputia-rime cargo test
result: 3 passed

InputiaInputMethod.app --bridge-self-check
bridgeSelfCheck=true
commit=中国

InputiaInputMethod.app --bridge-memory-self-check
bridgeMemorySelfCheck=true
commit=种过
```

系统安装验证：

```text
/Library/Input Methods/InputiaInputMethod.app executable size=3975808
Signed Time=Jul 6, 2026 at 21:16:55
CDHash=90af63b5e0a49345a7d2df3f3e11a4faa2299b95
bridgeSelfCheck=true
TIS current source=com.inputia.inputmethod.Inputia.Hans
selected=true
```

TextEdit 真实输入 smoke：

```text
current source = Inputia 简体
System Events: shift down/up, keystroke "zhongguo", key code 49
TextEdit value = 中国
InputiaInputMethod log = Setting marked text ... Inserting text
```

数字候选选择 smoke：

```text
TextEdit composition = zhongguo
System Events: keystroke "1"
TextEdit value = 中国
```

候选窗视觉 smoke：

```text
running process = macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod
System Events: shift down/up, keystroke "zhongguo"
fullscreen screenshot = /tmp/inputia-smoke/candidate-panel.png
visible candidate panel = 1 中国  2 种过  3 种果  4 忠果  5 中古
System Events: keystroke "1"
TextEdit value = 中国
```

已清理 TextEdit smoke 自动保存文件，删除前确认内容为 `中国`。

当前限制：

- 拼音候选生成、marked text、Space 上屏、候选窗视觉显示、数字选候选已通过真实 App smoke。
- 候选窗分页热键仍需要单独 smoke；当前只验证了候选窗显示和数字选择。
- 最新候选窗改动已在 build 版进程验证；`/Library/Input Methods/InputiaInputMethod.app` 仍是 21:16 的上一版系统安装，需走管理员安装流程后才会带上自有候选窗。

## 官方与成熟实现约束

已核对本机 Apple SDK：

- `InputMethodKit.framework/Headers/IMKServer.h`：每个输入法进程应创建一个 `IMKServer`，它负责客户端连接、输入会话和候选窗关联。
- `InputMethodKit.framework/Headers/IMKInputController.h`：每个输入会话对应一个 `IMKInputController`；事件入口可以是 `inputText`、`inputText:key:modifiers:` 或 `handleEvent`；返回值决定事件是否继续传给客户端。
- `InputMethodKit.framework/Headers/IMKCandidates.h`：`updateCandidates` 不会改变候选窗 visible state；`showCandidates` 要求调用方先把候选窗移动到合适位置。2026-07-06 TextEdit spike 中，`IMKCandidates` 路径已有候选数据、marked text 和 client attributes，但全屏视觉未显示候选窗，并出现 `IMKUIPanel ... canBecomeKeyWindow` 警告。
- `Carbon.framework/.../TextInputSources.h`：application bundle 输入法位于 `<domain>/Library/Input Methods/`；`TISInputSourceID` 对 input method 通常与 Bundle ID 相同；input mode ID 可以由 Bundle ID 和 mode suffix 组合。

已对照成熟 Swift IMKit 示例 `ensan-hcl/macOS_IMKitSample_2021`：

- Bundle ID 应包含 `.inputmethod.`。
- Info.plist 需要 `InputMethodConnectionName` 和 `InputMethodServerControllerClass`。
- Host 应创建 `IMKServer` 和 `IMKCandidates`。
- 诊断日志要用 `NSLog`，因为输入法通常是后台进程。

已对照成熟输入法 Squirrel/vChewing：

- Squirrel 使用自有 `NSPanel` 候选窗，`SquirrelInputController` 通过 `client.attributes(forCharacterIndex:lineHeightRectangle:)` 取得光标位置并把候选数据交给 panel。
- vChewing 也使用自有候选窗，并在项目说明中明确记录 `IMKCandidates` 有大量兼容性风险。
- 因此 Inputia 当前 Host baseline 采用 `InputiaCandidatePanel` 作为最小自有候选窗；`IMKCandidates` 只作为已验证失败的 spike 记录，不再作为第一阶段候选窗验收路径。

## 新增 Host 骨架

路径：`macos/InputiaInputMethod`

关键文件：

- `Info.plist`
- `Sources/InputiaInputMethod/main.swift`
- `Sources/InputiaInputMethod/InputiaRustBridge.swift`
- `build.sh`
- `install-user.sh`
- `uninstall-user.sh`

当前 bundle metadata：

```text
CFBundleIdentifier=com.inputia.inputmethod.Inputia
TISInputSourceID=com.inputia.inputmethod.Inputia
InputModeID=com.inputia.inputmethod.Inputia.Hans
InputMethodConnectionName=com.inputia.inputmethod.Inputia_Connection
InputMethodServerControllerClass=InputiaInputMethod.InputiaInputController
LSBackgroundOnly=false
LSUIElement=true
TISIntendedLanguage=zh-Hans
```

## 编译和 class 自检

命令：

```bash
./macos/InputiaInputMethod/build.sh
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --self-check
```

结果：

```text
InputiaInputMethod.app/Contents/Info.plist: OK
InputiaInputMethod.app: valid on disk
InputiaInputMethod.app: satisfies its Designated Requirement
bundleIdentifier=com.inputia.inputmethod.Inputia
connectionName=com.inputia.inputmethod.Inputia_Connection
InputMethodServerControllerClass=InputiaInputMethod.InputiaInputController
classFound=true
InputMethodServerDelegateClass=InputiaInputMethod.InputiaInputController
classFound=true
```

结论：

- Swift 6.3.3 / Xcode 26.6 可以编译正式 Host 骨架。
- `-target "$(uname -m)-apple-macos13.0"` 仍然必要，避免本机 Swift 默认生成 macOS 28.0 minos。
- 当前 Host 与本机 Squirrel 保持同类 app metadata：`LSUIElement=true` 且 `LSBackgroundOnly=false`。早先 `LSBackgroundOnly=true` 能枚举但不能进入 enabled list，保留为已排除方向继续验证。
- 当前 Info.plist 已增加显式 `ComponentInputModeDict`，包含 `com.inputia.inputmethod.Inputia.Hans`，与 Squirrel 的 base input method + visible input mode 结构对齐。
- 当前 mode script 使用 `smUnicodeScript`，input mode repertoire 使用 `Hans/Hant`，与 Squirrel 的可启用 input mode 结构保持一致。
- IMKServer 会查找的 controller/delegate class 能被 `NSClassFromString` 找到。
- 默认 build 现在使用 hardened runtime 签名选项和 `InputiaInputMethod.entitlements`。ad-hoc 输出为 `flags=0x10002(adhoc,runtime)`，entitlements 包含 `com.apple.security.app-sandbox=false` 和 `com.apple.security.cs.disable-library-validation=true`，为后续 Developer ID 签名和动态 librime 加载预留。

## Rust Core/Rime 桥接

Host 构建脚本现在会先构建 `crates/inputia-capi` 的 Rust staticlib，再把它链接进 `InputiaInputMethod.app`。Swift 侧通过 `InputiaRustBridge.swift` 调用 C ABI，`InputiaInputController.inputText` / `didCommandBySelector` 使用同一个 Rust outcome：

- 有 `commit` 时调用 `insertText`。
- 有 `composing` 时调用 `setMarkedText`。
- `candidates(_:)` 从 Rust outcome 的 `visible_candidates` 返回候选文本。
- 默认 session 通过 `Application Support/Inputia/settings.json` 创建；首次运行会派生 `Application Support/Inputia/rime` 和 `Application Support/Inputia/inputia_memory.db`。
- 设置文件会控制 schema、候选数量、Shift 切换、标点偏好、memory 开关和敏感 App 排除规则。
- `inputText` / `didCommandBySelector` 进入 Rust Core 前会读取 `IMKTextInput.bundleIdentifier()`，并通过 C ABI 设置当前 app context；这个做法来自 Squirrel 的成熟实现路径。
- `handle(_ event:client:)` 已接入 raw `NSEvent` 路径：Command 组合键直接交还宿主 App；Space/Return/Backspace/Escape/方向键映射到 Core；modifier-only Shift 在 key up 时切换中英文；普通 keyDown 保留 `event.characters`，以支持英文大小写。
- `candidateSelected(_:)` 已按当前可见候选文本映射回 Core 的数字候选选择，供 IMK 候选窗最终选择上屏。

命令：

```bash
./macos/InputiaInputMethod/build.sh
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-self-check
```

结果：

```text
bridgeSelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=中国
commit=中国
```

Memory-enabled 自检使用临时 memory db 先学习剪贴板词 `种过`，再输入 `zhongguo`：

```bash
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-memory-self-check
```

结果：

```text
bridgeMemorySelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=种过
commit=种过
```

Settings 自检使用 `/tmp/InputiaSettingsSelfCheck-*/settings.json`，验证 Host 可以从设置文件初始化 session：

```bash
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-settings-self-check
```

结果：

```text
bridgeSettingsSelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=中国
candidateCount=5
commit=中国
```

结论：

- Swift Host 已经能在进程内调用 Rust Core/Rime。
- Swift Host 已经能在进程内调用 SQLite memory/ranker，候选排序会受本地记忆影响。
- Swift Host 已经能从本地 `settings.json` 创建 session，而不是只靠硬编码候选数量和默认 schema。
- Swift Host 已经把宿主 App bundle id 传给 Core；C ABI 测试验证 typed commit 在 `com.1password.1password` 上下文中不会被学习。
- 早期此处还没有系统 enabled input source；后续已解除入口阻塞，并完成 TextEdit 真实 IMK 事件、上屏和候选窗 smoke，见本文顶部最新记录。

## 用户级 TIS 注册测试

命令：

```bash
./macos/InputiaInputMethod/install-user.sh
"$HOME/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod" --dump-enabled-input-source
"$HOME/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod" --select-input-source
./macos/InputiaInputMethod/uninstall-user.sh
```

结果：

```text
registerStatus=0
enableStatus=0
id=dev.inputia.inputmethod.Inputia.Hans
bundle=dev.inputia.inputmethod.Inputia
mode=dev.inputia.inputmethod.Inputia.Hans
type=TISTypeKeyboardInputMode
enabled=true
enableCapable=true
selectable=true
selected=false

--- enabled list ---
inputSourceFound=false

--- select ---
inputSourceFoundInEnabledList=false
selectStatus=-50
```

卸载验证：

```text
disableStatus=0
removed /Users/lzl/Library/Input Methods/InputiaInputMethod.app
```

结论：

- 正式 Host 骨架可以被 all-installed input source list 枚举。
- 增加显式 Hans mode 后，all-installed 里能看到 base source `TISTypeKeyboardInputMethodModeEnabled` 和 mode source `TISTypeKeyboardInputMode`。
- `TISEnableInputSource` 对 Hans mode 返回 `noErr`，all-installed 属性显示 `enabled=true/selectable=true`，但 `includeAllInstalled=false` 仍没有任何 Inputia 匹配项。
- install 脚本已改为同时对 base source 和 Hans mode 调用 `TISEnableInputSource`。base source 返回 `noErr` 但属性仍为 `enabled=false/selectable=false`；Hans mode 保持 `enabled=true/selectable=true`，enabled list 仍为空。
- 显式 `lsregister -f` 后再 register/enable/select，结果不变。
- 把 Inputia 临时写入 `AppleEnabledInputSources` 和 `AppleSelectedInputSources` 后，all-installed 里 Hans mode 会显示 `selected=true`，但 enabled list 仍为空，`TISSelectInputSource` 仍返回 `-50`，当前输入源仍保持微信输入法；测试结束后已恢复原 HIToolbox 偏好。
- 使用本机 `Codexbar Local Code Signing Leaf v4` 非 ad-hoc 签名后，结果不变。
- 使用 ad-hoc hardened runtime + entitlements 后，结果不变。
- 即使先 `open` 启动 IMKServer，再 enable/select，`TISSelectInputSource` 仍返回 `-50`，当前输入源保持微信输入法。
- 因为不在 enabled list，当前还不能验证真实 App 中的英文 commit、marked text、候选窗或上屏。

## 成熟输入法对照

本机成熟输入法均安装在系统级 `/Library/Input Methods`：

```text
/Library/Input Methods/Squirrel.app
/Library/Input Methods/WeType.app
/Library/Input Methods/DoubaoIme.app
```

签名对照：

- Squirrel：Developer ID Application，hardened runtime，Team ID `28HU5A7B46`，entitlements 包含 `disable-library-validation`。
- WeType：Developer ID Application，hardened runtime，Team ID `88L2Q4487U`，entitlements 包含 audio/network 等能力。
- DoubaoIme：Developer ID Application，hardened runtime，notarization ticket stapled，Team ID `96L78H6LMH`。
- Inputia：当前只能在用户目录做 ad-hoc 或本地自签测试；`/Library/Input Methods` 对当前用户不可写。

TIS 对照：

- 微信输入法在 `includeAllInstalled=false` 中同时出现 base source `TISTypeKeyboardInputMethodModeEnabled enabled=true` 和 pinyin mode `TISTypeKeyboardInputMode enabled=true/selectable=true/selected=true`。
- Inputia 用户级安装时，只在 `includeAllInstalled=true` 出现；Hans mode 可显示 `enabled=true/selectable=true`，但 base source 仍为 `enabled=false`，且 enabled list 为空。

## 当前阻塞面

还需要继续验证：

- 通过 System Settings 手动添加后，enabled list 是否出现 Inputia。
- 系统级 `/Library/Input Methods` 安装是否改变 enabled/select 行为；当前用户对该目录不可写，需要管理员安装路径。
- Developer ID / notarized 签名或安装器是否是现代 macOS 真启用第三方输入法的前提；本机只有本地自签 identity，不能等价验证 Developer ID。
- 进入 enabled list 后再验证 `inputText`、`handleEvent`、`insertText`、`IMKCandidates`。

## Squirrel/vChewing/ToyIMK 对照与当前结论

用户指出应直接学习鼠须管后，已拉取并核对：

- `rime/squirrel`
- `vChewing/vChewing-macOS`
- `eagleoflqj/toyimk`
- Apple Text Input Source Services Reference / 本机 SDK `TextInputSources.h`

关键证据：

- Apple TIS 文档明确约束：mode-enabled input method 的 parent input method 不是可选择对象；input mode 只有在自身 enabled 且 parent enabled 后才能被 `TISSelectInputSource` 选中，否则 `paramErr/-50`。
- Squirrel 的可见来源不是 parent，而是 `ComponentInputModeDict` 里的 `im.rime.inputmethod.Squirrel.Hans/Hant`；它的 postinstall 顺序是 register、enable mode、select mode。
- Squirrel 的 `InfoPlist.strings` 为 base id 和每个 mode id 都提供本地化名称，例如 `im.rime.inputmethod.Squirrel.Hans = 鼠须管`。
- vChewing 同样从 `ComponentInputModeDict.tsInputModeListKey` 动态读取所有 mode id，再调用 `TISEnableInputSource`；它的安装器遇到自动启用不完整时会提示用户到 System Settings 手动添加。
- ToyIMK 文档明确写着首次安装后需要 logout/login，然后再到 System Settings > Keyboard > Input Sources 添加。

本机验证：

```text
Inputia / all-installed:
id=com.inputia.inputmethod.Inputia
type=TISTypeKeyboardInputMethodModeEnabled
enabled=false
selectable=false

id=com.inputia.inputmethod.Inputia.Hans
type=TISTypeKeyboardInputMode
enabled=true
selectable=true

Inputia / enabled list:
matches=0

TISSelectInputSource:
selectStatus=-50
```

这与 Apple 文档一致：mode 看似 `enabled=true/selectable=true`，但 parent 没有进入 enabled list，因此不能 select。

已做的 Squirrel-style 调整：

- `Info.plist` 改为 `ComponentInputModeDict` 双 mode：`com.inputia.inputmethod.Inputia.Hans/Hant`。
- `LSBackgroundOnly=false`、`LSUIElement=true`、`NSPrincipalClass=NSApplication`，对齐 Squirrel/vChewing。
- 增加 `TISIconLabels`、`InfoPlist.strings`，让 Bundle 能解析 mode 本地化名。
- 诊断命令改为从 `ComponentInputModeDict` 动态读取 mode id，避免硬编码单个 mode。

结果：

- `Bundle.localizedString(..., table: "InfoPlist")` 能正确解析 Inputia mode 名称。
- 用全新 bundle id `com.inputia.inputmethod.InputiaProbe` 做用户级 probe 后，TIS all-installed 能显示 `InputiaProbe 简体/繁体`，说明本地化结构可行。
- 但 System Settings 添加列表仍不热加载用户级新 IMK app；搜索 `Inputia` 为空。
- 轻量刷新 `TextInputMenuAgent`、`SystemUIServer`、`System Settings` 后仍不出现。
- 本机系统级替换 `/Library/Input Methods/InputiaInputMethod.app` 需要管理员授权；第二次授权窗口未完成，未能把最新本地化 build 写入系统级路径。

当前最强假设：

1. 仅靠 `TISRegisterInputSource` / `TISEnableInputSource` 不能把第三方 mode-enabled input method 加入 enabled list；System Settings 手动添加或等价系统 UI 流程仍是必要入口。
2. System Settings 的“可添加输入源”列表对新安装的 IMK app 不一定热刷新。ToyIMK 的 logout/login 要求与本机现象一致。
3. 当前 Inputia 未出现在添加列表，剩余待验证变量是“最新 build 写入 `/Library/Input Methods` 后重新登录/重启是否出现”。这一步需要管理员安装或用户重新登录，不能再靠 Core/Rime/候选窗代码推进。

## 2026-07-06 后续 Host 安装改动

为减少与成熟输入法的非必要差异，已补充：

- app-level icon 改为真实 `Inputia.icns`；mode 菜单图标改为只使用 `TISIconLabels`，避免旧 `inputia.pdf` 白块被菜单当作过大的 mode icon。
- `install-system.sh` 改为使用 `/usr/bin/ditto --noextattr --noqtn` 复制完整 app bundle 到 `/Library/Input Methods`，减少扩展属性与 quarantine 变量。
- `install-system.sh` 复制后会重新 `lsregister -u/-f`，再 register input source。
- `install-system.sh` 会按当前 console user 执行 `--enable-input-source` 和 `--select-input-source`，对齐 Squirrel postinstall 里 root 安装、登录用户启用/选择的分层。
- `uninstall-system.sh` 增加 LaunchServices unregister 和管理员权限删除路径。
- 增加 `verify-system.sh`，一次性输出 bundle、resources、plist、codesign、class self-check、TIS all-installed/enabled、current source 和 LaunchServices 记录。

本地 build 验证：

```text
plutil -lint: OK
codesign --verify --deep --strict: OK
self-check classFound=true
build resources:
  Inputia.icns
  en.lproj/InfoPlist.strings
  zh-Hans.lproj/InfoPlist.strings
  zh-Hant.lproj/InfoPlist.strings
```

系统级安装验证现状：

- `/Library/Input Methods/InputiaInputMethod.app` 仍是旧系统包，只含 `inputia.pdf`，没有最新 `InfoPlist.strings` 和 `Inputia.icns`。
- 运行最新 `install-system.sh` 已进入 `systemInstallNeedsAdmin=true`，但 macOS 管理员授权窗口 90 秒未完成；命令已中止，避免后台挂起。
- 因此尚未验证“最新系统级包 + logout/login”是否能让 Inputia 出现在 System Settings 添加列表。

用户级脚本自测：

```text
install-user.sh:
  build OK
  ditto --noextattr --noqtn copy OK
  registerStatus=0
  enableStatus=0 for parent/Hans/Hant

verify-system.sh ~/Library/Input Methods/InputiaInputMethod.app:
  resources include Inputia.icns and InfoPlist.strings
  plutil OK
  codesign OK
  class self-check OK
  enabled list still matches=0

uninstall-user.sh:
  disableStatus=0
  removed ~/Library/Input Methods/InputiaInputMethod.app
```

自测后状态：

- `~/Library/Input Methods` 下没有 Inputia 副本。
- LaunchServices dump 不再列出 Inputia 用户级记录。
- TIS all-installed 仍能从系统级旧包看到 Inputia，但 enabled list 仍为空。

TIS 语言属性对照：

```text
Inputia.Hans languages=zh-Hans
Inputia.Hant languages=zh-Hant
Squirrel.Hans languages=zh-Hans
Squirrel.Hant languages=zh-Hant
Doubao pinyin languages=zh-Hans
WeType pinyin languages=zh-Hans
```

结论：Inputia mode 的语言归类与成熟输入法一致；当前系统级旧包的主要差异仍是缺少最新 `InfoPlist.strings`，导致 mode name 回退成 raw id。

安装包验证：

- 新增 `build-pkg.sh`。
- 初始 root payload pkg 会因为本机 `com.apple.provenance` xattr 被 `pkgbuild` 编出 AppleDouble `._*` 条目。
- 已改为 `--nopayload` 包：app 以 `COPYFILE_DISABLE=1 tar -czf InputiaInputMethod.app.tar.gz` 放入 scripts，`postinstall` 解压到 `/Library/Input Methods`。
- `pkgutil --payload-files dist/InputiaInputMethod.pkg` 输出为空：`payloadFiles=0`。
- 展开后的 `PackageInfo` 包含 `install-location="/" auth="root"`，安装时应由 macOS Installer 请求管理员授权。
- 展开包后用临时 root 跑 postinstall 自测通过：

```text
INPUTIA_INSTALL_ROOT=/tmp/InputiaPostinstallRoot INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
self-check classFound=true
resources:
  Inputia.icns
  en.lproj/InfoPlist.strings
  zh-Hans.lproj/InfoPlist.strings
  zh-Hant.lproj/InfoPlist.strings
```

注意：`.pkg` 仍未安装到真实 `/Library/Input Methods`，因为真实安装需要管理员授权。

非交互安装权限边界：

```text
/usr/sbin/installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod.pkg -target /
installer: Must be run as root to install this package.
```

结论：`dist/InputiaInputMethod.pkg` 的 `auth="root"` 生效；没有管理员授权时不能替换 `/Library/Input Methods/InputiaInputMethod.app`。这与当前阻塞一致，不是 Host 构建失败。

## 2026-07-07 v4 系统安装、物理 Shift 和设置入口

系统安装包：

```text
dist/InputiaInputMethod-v4-61d7a3b826eb.pkg
inputiaInstalledVersion=4
inputiaInstalledCDHash=61d7a3b826eb281e9b19e774d465ecc23e2548a7
registerStatus=0
selectStatus=0
selected=true
Installed "InputiaInputMethod-v4-61d7a3b826eb"
PackageKit: ----- End install -----
```

系统目录当前验证：

```text
/Library/Input Methods/InputiaInputMethod.app
CFBundleVersion=4
systemCDHash=61d7a3b826eb281e9b19e774d465ecc23e2548a7
includeAllInstalled=false:
  id=com.inputia.inputmethod.Inputia.Hans
  enabled=true
  selectable=true
  selected=true
  iconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
```

物理键盘 Shift 问题：

- 旧逻辑要求 modifier 集合除了 Shift 以外完全为空；真人键盘事件可能携带 CapsLock/fn/numericPad 等状态，导致裸 Shift 不触发。AppleScript 合成事件较干净，所以 smoke 曾经误通过。
- v4 改为只把 Command/Control/Option 视作阻断 modifier；Shift 按下后只要没有这些组合键，就 armed，松开时切换中英文。
- v4 打开 `INPUTIA_DEBUG_EVENTS=/tmp/inputia-events.log` 后，真实输入日志显示物理 Shift 已进入 IMK 并切换模式：

```text
flagsChanged keyCode=56 last=0 current=131072 hadShift=false hasShift=true blocking=false armed=false
flagsChanged keyCode=56 last=131072 current=0 hadShift=true hasShift=false blocking=false armed=true
apply ok=true consumed=true mode=Chinese composing= commit= candidates=

flagsChanged keyCode=56 last=0 current=131072 hadShift=false hasShift=true blocking=false armed=false
flagsChanged keyCode=56 last=131072 current=0 hadShift=true hasShift=false blocking=false armed=true
apply ok=true consumed=true mode=English composing= commit= candidates=
```

设置入口：

- 参考 Squirrel 的 `IMKInputController.menu()` 路径，v4 在输入法菜单中增加 `Inputia 设置...`。
- 设置窗口为原生 AppKit `NSWindowController`，写入现有配置契约：

```text
~/Library/Application Support/Inputia/settings.json
```

- 第一版可配置：输入方案、候选数量、Shift 切换、中文模式英文标点、本地记忆排序、隐私学习、敏感 App bundle id 列表。
- 设置写入后当前 Host 需要重建 session 才会完全生效；实时热更新还未做。

## 2026-07-07 Safari / 多 App 中 Shift 偶发失效

用户反馈：TextEdit 中物理 Shift 能切换，但 Safari/Codex/微信等 App 中有时不能切换，且重新测试时又可能恢复。

本机证据：

```text
defaults read com.apple.HIToolbox AppleGlobalTextInputProperties
{
    TextInputGlobalPropertyPerContextInput = 1;
}
```

这个值对应 macOS Input Sources 设置里的 “Automatically switch to a document's input source”。Apple 文档说明该选项会让某个文稿/文本上下文保存自己的 input source，并在之后回到该文稿时继续使用。实际现象符合这个机制：

- 在 TextEdit 前台，当前输入源保持 `com.inputia.inputmethod.Inputia.Hans`，物理 Shift 事件进入 Inputia。
- 切到微信或 Codex 时，当前输入源会变为 `com.tencent.inputmethod.wetype.pinyin`；这时按 Shift 不会进入 Inputia，因此不是 Inputia Core/Rime/候选窗问题。
- 新建 Safari 本地 `data:` 测试页时，曾观察到先全局选择 Inputia，再打开新页后当前输入源变为 `ABC`；在 Safari 输入框已聚焦后再次选择 Inputia，则 `com.apple.Safari` 的按键会进入 Inputia，并能拼出中文。
- 关闭该偏好后：

```text
defaults write com.apple.HIToolbox AppleGlobalTextInputProperties -dict TextInputGlobalPropertyPerContextInput -bool false
killall cfprefsd TextInputMenuAgent SystemUIServer
```

当前 Safari/TextEdit 前台检查都保持：

```text
id=com.inputia.inputmethod.Inputia.Hans
enabled=true
selectable=true
selected=true
```

外部资料：

- Apple Support `Change Input Sources settings on Mac` 明确说明 “Automatically switch to a document's input source” 会按文稿保存输入源。
- Squirrel README 写明首次安装后如果部分 App 打不出字，需要 logout/login；ToyIMK 也记录首次安装后要 logout/login 后再到系统设置添加输入源。这说明 macOS Text Services 对新 IMK app 和现有文本上下文有缓存/会话状态。
- StackOverflow 上也有同类 TIS 现象：菜单栏/状态看起来已切换，但当前文本上下文实际输入法没有立刻切过去。

Inputia 自身缺口：

- v4 安装版只有 mode-level `tsInputModeCharacterRepertoireKey`，没有顶层 `tsInputMethodCharacterRepertoireKey`。
- v5 已补齐顶层 `tsInputMethodCharacterRepertoireKey = Hans/Hant/Latn` 和 `tsInputMethodIconFileKey = inputia.pdf`，并把两个 mode 的 repertoire 也加入 `Latn`，更符合 Inputia “中英统一输入层”的能力声明。
- v5 已构建出 `dist/InputiaInputMethod-v5-a28d46eaf343.pkg`，但系统安装授权被取消，所以 `/Library/Input Methods/InputiaInputMethod.app` 当前仍是 v4。

当前判断：

1. Shift 本身在 Inputia 被选中时可用；Safari 能否切换取决于 Safari 当前输入框实际是否仍选择 Inputia。
2. 失效时先看菜单栏当前源；如果是 ABC/微信输入法，就不会进入 Inputia。
3. 需要完成 v5 系统安装、logout/login 或至少重启相关 App，清掉旧文本上下文缓存，再做 Safari 新窗口/新标签页回归。

本轮验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml       # 15 passed
cargo test --manifest-path crates/inputia-settings/Cargo.toml   # 3 passed
cargo test --manifest-path crates/inputia-capi/Cargo.toml       # 7 passed
build.sh                                                       # plist OK, codesign OK
```

## 2026-07-07 v11：菜单已出现但“打不了字”的根因与修复

用户反馈：菜单栏已经能选到 `Inputia 简体`，但实际输入框里“打不了字”。本轮把问题拆成两层验证：

1. Text Input Source 层：`Inputia.Hans` 已在 enabled list 中，`selectStatus=0`，`selected=true`。
2. Runtime 输入层：Host 是否收到按键、Core 是否给候选/提交、提交是否落到目标 App 输入框。

关键发现：

- 用户本机 `~/Library/Application Support/Inputia/settings.json` 在 11:20 左右被改成：

```json
"schema_id": "guobiao_bispell"
```

- 在该 schema 下，`ni + Space` 进入 Host，但 Rime 返回空候选，旧 Core 仍把空格消费掉，导致用户体感是“按键没反应/打不了字”。
- 这个问题不是 Safari 不发键，也不是 IMK Host 没启动；日志里 Safari 已经有：

```text
context bundle=com.apple.Safari
keyDown keyCode=45 chars=n
apply ok=true consumed=true mode=Chinese composing=n commit= candidates=
keyDown keyCode=34 chars=i
apply ok=true consumed=true mode=Chinese composing=ni commit= candidates=
keyDown keyCode=49 chars=space
apply ok=true consumed=true mode=Chinese composing=ni commit= candidates=
```

修复：

- `inputia-core`：中文模式下空组合按 Space 改为放行，不再吞掉普通空格。
- `inputia-core`：有组合但无候选时按 Space 改为提交原始组合串并清空组合，避免静默吞键。
- `inputia-core`：无可见候选时数字键进入组合串，避免所有数字都被当候选序号；这给国标/声调类 schema 留出基础空间。
- 本机设置切回 `luna_pinyin_simp`，恢复第一阶段默认全拼输入。

安全修复：

- 管理员授权窗口暴露 `com.apple.SecurityAgent` 会进入 Inputia Host。v11 将它加入默认敏感列表，并在 Host 层对敏感 App 直接 pass-through，避免消费密码框按键，也避免 debug log 记录敏感按键。
- 现有本机 settings 已补入：

```json
"com.apple.SecurityAgent"
```

安装结果：

```text
pkg: macos/InputiaInputMethod/dist/InputiaInputMethod-v11-ecdafdd26c30.pkg
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=11
CDHash=ecdafdd26c308bda1f26ddf0ab905ee2bb48d366
codesign --verify --deep --strict: OK
/Applications/Inputia 设置.app codesign verify: OK
```

验证结果：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml       # 19 passed
cargo test --manifest-path crates/inputia-settings/Cargo.toml   # 3 passed
zsh/bash script syntax checks                                  # OK
plist lint                                                     # OK
```

TextEdit 真实输入：

```text
Inputia.Hans selected=true
ni + Space -> 你
Shift -> English: ni + Space -> "ni "
Shift -> Chinese: ni + Space -> 你
```

Safari 真实输入框读回：

```text
expectedCDHash=ecdafdd26c308bda1f26ddf0ab905ee2bb48d366
actualCDHash=ecdafdd26c308bda1f26ddf0ab905ee2bb48d366
safariTypingTitle=VALUE:你
safariTypingResult=你
safariTypingExpectation=cjk
safariTypingSmokePassed=true
```

保留风险：

- v11 的 app bundle 自诊断命令（如 `--self-check`）在本机偶发卡住；实际系统 Host 输入链可用，后续验证优先使用独立 `build/inputia-tis-tool` 做 TIS 操作，再用真实 App smoke 验证输入。
- `guobiao_bispell` 来自第三方 Rime schema，且它的键位/声调行为不能按全拼 `ni` 断言。后续要为每个 schema 建独立 smoke，而不是只把 schema 放进设置列表。

## 2026-07-07 v13：回车原样上屏、候选消失路径与设置页按钮布局

用户反馈：中文输入状态下想临时输入英文时，按回车不能把当前拼写直接上屏；composition 留着下划线，继续打字时前面的内容会消失。设置页右下角按钮也会被长路径挤压，显示成很窄的蓝色残片。

根因拆分：

- Core 只有 `Space`，没有独立的 `Enter` 语义；Host 把 Return 和 Space 都映射到同一个提交候选入口。中文输入里用户按 Return 的预期应是提交原始 composition，例如 `ni` -> `ni`，而不是选首候选。
- Host 提交/取消 composition 时只更新内部状态和候选窗，未按当前 client 的 `markedRange()` 明确替换或清掉 marked text；部分 App 会继续保留下划线 inline session，下一次输入时覆盖旧 composition。
- 独立的 `/Applications/Inputia 设置.app` 没有打包 `RimeData`；它保存配置时可能无法写入 `rime_shared_data_dir`。Host 热重载后若回退到 Squirrel 原始 SharedSupport，则搜狗/国标等 Inputia 额外 schema 可能没有候选。
- 设置页 footer 使用固定宽度 stack，长 settings path 挤压右侧按钮，导致“保存”按钮可能被压到只剩一条蓝色窄条。

修复：

- `inputia-core` 新增 `Key::Enter`：中文模式 composition 非空时提交原始 composition 并清空候选；composition 为空时放行给 App。
- `inputia-capi` 新增 special key `KEY_ENTER=7`，并补 CAPI 回归测试。
- macOS Host 中 Return 调 `bridge.enter()`，Space 继续调 `bridge.space()`，两者不再混用。
- Host `apply()` 在提交时使用 `client.markedRange()` 作为 replacement range；取消 composition 且无 commit 时主动清掉 marked text，避免下划线残留。
- `InputiaSettingsWindow` 和 `InputiaRustBridge` 都把 `rime_shared_data_dir` 的空值/失效路径修正到已安装 Host 的 `/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData`。
- 设置页 footer 改为约束布局：状态路径低压缩优先级，`打开配置文件夹` 和 `保存` 固定贴右，不再被路径挤出窗口。
- `smoke-textedit.sh` 增加 `ni + Return -> ni` 验收步骤。
- Host/设置启动器版本升到 v13。

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  21 passed
  includes enter_commits_raw_composition_in_chinese_mode and enter_passes_through_without_composition

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  8 passed
  includes capi_enter_commits_raw_composition

INPUTIA_RIME_SHARED_DATA_DIR=macos/InputiaInputMethod/build/RimeData \
  cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  1 passed
  schemaSmokeEmojiErrorPresent=false

macos/InputiaInputMethod/build.sh
  InputiaInputMethod.app codesign verify OK
  Inputia 设置.app codesign verify OK
  build CFBundleVersion=13
  build CDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v13-db6a282f35b2.pkg
  payloadFiles=0
  appCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
```

限制：

- 初次运行 `install-system.sh` 时卡在 macOS 管理员授权路径，已中断；随后通过 v13 pkg 完成系统安装。
- 系统目录已替换为 v13：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
CDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
Inputia.Hans enabled=true selectable=true selected=true
```

真实 TextEdit / Safari smoke：

```text
defaultChineseResult=你
enterRawResult=ni
shiftEnglishResult=ni 
shiftChineseResult=你
textEditSmokePassed=true

safariTypingTitle=VALUE:你
safariTypingResult=你
safariTypingSmokePassed=true
```

事件日志证明 Return 走 v13 新语义：

```text
keyDown keyCode=36 chars=return
apply ok=true consumed=true mode=Chinese composing= commit=ni candidates=
```

## 2026-07-07 v14：Rime 多页候选与真实 Host smoke 覆盖扩展

本轮继续补第一阶段验收中的“候选词能正常显示、选择、分页和上屏”。

依据与 spike：

- Apple InputMethodKit SDK 头文件说明 IMK 可通过 `candidateSelected:` / selection keys 支持候选选择；当前 Inputia 使用自绘候选窗，因此数字选词和分页必须由 Host/Core 自己处理并验证。
- Rime/Squirrel shared data 的 `key_bindings` 使用 `Page_Down` / `Page_Up` 作为分页键。
- 最小 Rime spike 证明 librime `simulate_key_sequence` 接受 `{Page_Down}`：

```text
keys=ni
page=0 candidates=5
candidate[0]=你
candidate[1]=拟
candidate[2]=尼
candidate[3]=泥
candidate[4]=呢

keys=ni{Page_Down}
page=1 candidates=5
candidate[0]=妳
candidate[1]=妮
candidate[2]=腻
candidate[3]=逆
candidate[4]=倪
```

修复：

- `inputia-rime` 的 `ChineseEngine::candidates` 不再只返回 Rime 当前页；它会用 `{Page_Down}` 聚合最多 6 页候选，去重后按全局顺序设置 `base_score` 和候选 id。
- 这样 Core 的 `candidate_page_size`、数字选词、`PageDown/PageUp` 可以继续在统一 Input Core 层处理，不把 Rime 当核心大脑。
- `inputia-capi` 增加分页跨 Rime 页测试。
- `smoke-textedit.sh` 增加真实系统级验收步骤：
  - `ni + 3` 必须上屏非首选 CJK 候选。
  - `ni + PageDown + Space` 必须上屏非首选 CJK 候选。
- Host/设置启动器版本升到 v14。

验证：

```text
cargo test --manifest-path crates/inputia-rime/Cargo.toml -- --nocapture
  3 core_flow tests passed
  includes rime_engine_exposes_rime_second_page_to_core_paging_when_available
  schema smoke passed
  rimeEmojiErrorPresent=false

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  9 passed
  includes capi_paginates_across_rime_candidate_pages

cargo test --manifest-path crates/inputia-core/Cargo.toml
  21 passed

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v14-3202beee984a.pkg
  appCDHash=3202beee984a8a4c59593172f4f46484d13a7713
  build CFBundleVersion=14
```

包内容检查：

```text
pkg expanded app CFBundleVersion=14
codesign --verify --deep --strict: OK
RimeData includes luna_pinyin_simp, double_pinyin, double_pinyin_flypy, double_pinyin_sogou, guobiao_bispell, double_pinyin_mspy, double_pinyin_abc, double_pinyin_pyjj, double_pinyin_st
pkgGuobiaoEmojiPatch=true
```

限制：

- v14 pkg 已打开安装器，但本轮轮询 36 次后系统目录仍是 v13：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
CDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
```

- 因此新增的真实 `digitCandidateResult` / `pageCandidateResult` smoke 还没有在系统安装版 v14 上跑通。安装 v14 后需立即重跑：

```text
macos/InputiaInputMethod/smoke-textedit.sh
```

## 2026-07-07 v15：外接键盘 Enter、marked text 提交和设置页按钮布局

用户反馈：

- 设置页右下角按钮会被长路径挤压，显示成很窄的蓝色残片。
- 中文组合态下想临时输入英文，按回车有时不能把原始拼写上屏；候选/下划线残留后继续打字会覆盖前文。
- 双拼/全拼有时不出候选。

依据与定位：

- Apple `IMKTextInput` 文档说明 `insertText:replacementRange:` 的 replacement range 如果客户端不支持 TSMDocumentAccess 会被忽略；`markedRange` 是当前 inline session 的范围，marked text 会以下划线表示未完成转换。
- 鼠须管 `SquirrelInputController` 的提交路径使用 `client.insertText(string, replacementRange: .empty)` 并在 `rimeUpdate()` 中统一消费 commit、更新 marked text 和候选窗；它也长期持有 Rime session，而不是在每次候选查询时新建临时 Rime session。
- 本地 schema smoke 证明已打包的 `luna_pinyin_simp`、自然码、小鹤、搜狗、微软、ABC、拼音加加、四通、国标双拼均能在正确键序下产出“中国”；因此“候选不出”不是 schema 文件完全缺失，更像安装版本滞后、用户正在双拼方案里输入全拼键序、或 Host/marked text 状态处理不稳。
- 当前系统已安装版仍是 v13，build 版是 v15；用户侧问题必须先确保系统目录替换到最新 CDHash 后再做菜单栏真实 smoke。

修复：

- macOS Host 新增 `keyCodeKeypadEnter = 76`，把外接键盘/小键盘 Enter 也映射到 Core `Enter`，与内置 Return `36` 同样走“提交原始 composition”。
- Host 在 commit 非空且之前存在 composition 时，先显式清理 marked text，再用 `replacementRange = NSNotFound` 插入 commit，降低不同 App 对文档 replacement range 支持不一致时的下划线残留风险。
- `commitComposition(_:)` 也先清理 marked text 再插入原始 composition。
- 设置页底部改成两行：路径单独一行截断，按钮单独一行右对齐；窗口升到 620x600 并设置最小尺寸，避免保存按钮被路径挤压。
- `smoke-textedit.sh` 增加 `ni + keypad Enter -> ni` 验收。
- Host/设置启动器版本升到 v15。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-core/Cargo.toml
  21 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml capi_enter_commits_raw_composition -- --nocapture
  passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  9 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  bundled_rime_schemas_commit_zhongguo_when_available passed

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  build CFBundleVersion=15
  build CDHash=5326dd193f2d36551669e36bd1e69b274ab687a4

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --self-check
  controller class found

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-default-chinese-self-check
  bridgeDefaultChineseSelfCheck=true
  commit=你

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v15-5326dd193f2d.pkg
  pkg shasum256=7834ba4fa587f58cd5060428ced8a591977abff2c54d50cafbec493594162803
  appCDHash=5326dd193f2d36551669e36bd1e69b274ab687a4
```

包与安装脚本检查：

```text
pkgutil --expand-full InputiaInputMethod-v15-5326dd193f2d.pkg
  Scripts/postinstall
  Scripts/InputiaInputMethod.app.tar.gz
  Scripts/InputiaSettings.app.tar.gz

expanded app CFBundleVersion=15
expanded app CDHash=5326dd193f2d36551669e36bd1e69b274ab687a4
expanded settings CFBundleVersion=15
expanded settings CDHash=a82b42887178534e3763db7b68117b91367d2010

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v15 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=15
  inputiaInstalledCDHash=5326dd193f2d36551669e36bd1e69b274ab687a4
  inputiaSettingsLauncherInstalled=true
```

限制：

- 本机 `sudo -n installer -pkg ... -target /` 返回 `sudo: a password is required`，因此本轮不能非交互替换 `/Library/Input Methods`。
- 系统当前仍是 v13：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
```

- 安装 v15 后必须重跑：

```text
macos/InputiaInputMethod/smoke-textedit.sh
macos/InputiaInputMethod/verify-system.sh
```

## 2026-07-07 v16：Handy 语音/剪贴板历史导入 C API 与 Swift Bridge

本轮补第一阶段验收中的“打字历史、语音历史、剪贴板历史能影响候选排序”和“用户打过的新词能进入语音热词/自定义词候选”的桥接边界。

依据与定位：

- `inputia-core` 已有 `SqliteMemory::import_handy_history` / `import_handy_clipboard`：
  - `transcription_history` 优先导入 `post_processed_text`，为空时回退 `transcription_text`，作为 `MemorySource::Voice` 学习。
  - `clipboard_history` 只导入 `content_type = text` 的 `full_text` / `content_preview`，并优先使用行内 `source_app` 作为隐私上下文，作为 `MemorySource::Clipboard` 学习。
- Handy 当前表结构来自 `src-tauri/src/managers/history.rs` 和 `src-tauri/src/managers/clipboard.rs`，字段名与 Core 导入 SQL 对齐。
- 原缺口在 `inputia-capi` 和 Swift Host：Core 能导入，但系统输入法进程没有可调用入口。

修复：

- `inputia-capi` 新增：
  - `inputia_session_import_handy_history(session, history_db_path, bundle_id, limit)`
  - `inputia_session_import_handy_clipboard(session, clipboard_db_path, bundle_id, limit)`
  - 两者只返回 `{ ok, imported, error }`，不把语音/剪贴板正文放进 JSON 响应。
- `InputiaRustBridge` 新增：
  - `importHandyHistory(path:bundleId:limit:)`
  - `importHandyClipboard(path:bundleId:limit:)`
  - `voiceHotwords(limit:)`
- `inputia-capi` 测试新增真实 SQLite 源库导入用例：
  - Handy 语音历史导入后，`种过` 进入 voice hotwords，并把 `zhongguo` 候选重排到首位，候选来源为 `voice`。
  - Handy 剪贴板历史导入后，`种过` 进入 clipboard candidates，并把 `zhongguo` 候选重排到首位，候选来源为 `clipboard`。
  - `com.1password.1password` 来源的剪贴板行默认排除，不进入候选。
- Host/设置启动器版本升到 v16。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
  11 passed
  includes capi_imports_handy_history_into_voice_hotwords_and_ranking
  includes capi_imports_handy_clipboard_and_skips_sensitive_source_apps

cargo test --manifest-path crates/inputia-core/Cargo.toml
  21 passed

cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
  3 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  bundled_rime_schemas_commit_zhongguo_when_available passed

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  build CFBundleVersion=16
  build CDHash=d4403bb69d6ff4445686ed86a47378c6f0b08052

nm -gU build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod
  _inputia_session_import_handy_clipboard
  _inputia_session_import_handy_history
  _inputia_session_voice_hotwords
```

诊断自检说明：

- 系统当前仍有同 bundle id 的 `/Library/Input Methods/InputiaInputMethod.app` v13 进程在运行，直接执行 build app 的 `--self-check` 会超时/被系统进程状态干扰。
- 为避免杀掉用户当前输入法进程，本轮用临时复制的 diag app 改 bundle id 后验证 diagnostics：

```text
/tmp/InputiaInputMethod-diag.app/Contents/MacOS/InputiaInputMethod --self-check
  classFound=true for InputiaInputController

/tmp/InputiaInputMethod-diag.app/Contents/MacOS/InputiaInputMethod --bridge-default-chinese-self-check
  bridgeDefaultChineseSelfCheck=true
  commit=你

/tmp/InputiaInputMethod-diag.app/Contents/MacOS/InputiaInputMethod --bridge-memory-self-check
  bridgeMemorySelfCheck=true
  commit=种过
```

包与安装脚本检查：

```text
macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v16-d4403bb69d6f.pkg
  pkg shasum256=0806f5e0f66d5011597ab73748f4b913662bf5fe15ae8a0191dfaefedf8ec8e2
  appCDHash=d4403bb69d6ff4445686ed86a47378c6f0b08052

pkgutil --expand-full InputiaInputMethod-v16-d4403bb69d6f.pkg
  Scripts/postinstall
  Scripts/InputiaInputMethod.app.tar.gz
  Scripts/InputiaSettings.app.tar.gz

expanded app CFBundleVersion=16
expanded app CDHash=d4403bb69d6ff4445686ed86a47378c6f0b08052
expanded app exports import_handy_history/import_handy_clipboard/voice_hotwords symbols

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v16 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=16
  inputiaInstalledCDHash=d4403bb69d6ff4445686ed86a47378c6f0b08052
  inputiaSettingsLauncherInstalled=true
```

限制：

- 本机无无密码 sudo，不能非交互替换 `/Library/Input Methods`。
- 系统当前仍是 v13：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
```

- 安装 v16 后必须重跑：

```text
macos/InputiaInputMethod/verify-system.sh
macos/InputiaInputMethod/smoke-textedit.sh
```

## 2026-07-07 v17：Host marked text 提交、候选兜底与设置按钮布局

本轮针对用户实测反馈修三个直接可用性问题：

- 设置页底部“保存”按钮在某些窗口尺寸/布局状态下容易被挤成窄条。
- 中文模式下输入拼音/双拼后按 Return 期望直接提交英文 raw composing，但部分 App 中会留下下划线 marked text，后续输入可能覆盖掉前一段。
- Rime 对某些全拼/双拼中间态暂时不返回候选时，Host 完全隐藏候选窗，用户只看到输入框下划线，误以为输入法失灵。

依据：

- Apple IMK 文档中 `insertText:replacementRange:` 是输入法完成转换后向 client 发送最终文本的入口；`setMarkedText:selectionRange:replacementRange:` 表示仍在转换中的 marked text。
- 文档也说明 replacement range 在不支持 TSMDocumentAccess 的 client 中可能被忽略，因此本轮避免用“先清 marked range 再插入”的两步提交。
- 对照鼠须管 `SquirrelInputController.swift`，其提交路径是直接 `client.insertText(string, replacementRange: .empty)`，marked/preedit 显示路径才使用 `setMarkedText(...)`。

修复：

- `main.swift`
  - 提交候选、Return raw composing、deactivate commit 时，不再先 `clearMarkedText()` 再插入。
  - 改为直接 `insertText(..., replacementRange: NSNotFound)`，让 IMK/TextInput client 结束当前 marked session。
  - 当 composing 非空但 Rime 当前没有候选时，候选窗显示 raw composing 兜底项，避免“只有下划线、完全没候选反馈”。
- `InputiaSettingsWindow.swift`
  - 设置页内容放入 `NSScrollView`，窗口高度不够时滚动，而不是挤压底部控件。
  - 底部按钮改为固定宽度的“保存设置”和“打开配置文件夹”，移除会挤压按钮的 spacer。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-core/Cargo.toml
  21 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml
  5 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  11 passed
  includes capi_enter_commits_raw_composition

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  build CFBundleVersion=17
  build CDHash=3e4643da9d0e82d6ac815520d3cb7275bc594795

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v17-3e4643da9d0e.pkg
  pkg shasum256=372d375c72c3b933e17b1af0e60316ffcd3384d74a032b5fa9a5efb504343956
  appCDHash=3e4643da9d0e82d6ac815520d3cb7275bc594795

pkgutil --expand-full InputiaInputMethod-v17-3e4643da9d0e.pkg
  Scripts/postinstall
  Scripts/InputiaInputMethod.app.tar.gz
  Scripts/InputiaSettings.app.tar.gz

expanded app CFBundleVersion=17
expanded app CFBundleShortVersionString=0.0.17
expanded app CDHash=3e4643da9d0e82d6ac815520d3cb7275bc594795
expanded RimeData schema files include:
  luna_pinyin_simp
  double_pinyin
  double_pinyin_flypy
  double_pinyin_sogou
  guobiao_bispell
  double_pinyin_mspy
  double_pinyin_abc
  double_pinyin_pyjj
  double_pinyin_st

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v17-expand/Scripts/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_rime_schemas_commit_zhongguo_when_available passed
  covers full pinyin, natural double pinyin, flypy, sogou, guobiao, mspy, abc, pyjj, st

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v17 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=17
  inputiaInstalledCDHash=3e4643da9d0e82d6ac815520d3cb7275bc594795
  inputiaSettingsLauncherInstalled=true
```

本机限制：

- `/Library/Input Methods/InputiaInputMethod.app` 当前仍是 v13：

```text
CFBundleVersion=13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
```

- 本机没有无密码 sudo，本轮不能自动替换系统输入法。
- 从 Codex 进程直接执行 ad-hoc/本地证书签名的输入法 app 诊断参数会被 `amfid` 拒绝：

```text
AppleMobileFileIntegrityError Code=-423
The file is adhoc signed or signed by an unknown certificate chain
```

- 因此 v17 安装后仍必须在系统安装态补跑：

```text
macos/InputiaInputMethod/verify-system.sh
macos/InputiaInputMethod/smoke-textedit.sh
```

## 2026-07-07 v18：英文词学习与本地英文补全

本轮补第一阶段验收中的“英文输入包含英文补全”和“打字历史影响候选/热词”缺口。

依据：

- Apple `InputMethodKit` 文档说明输入法可以直接处理 `NSEvent`，并通过 `candidates(_:)` / `candidateSelected(_:)` 提供候选选择回调；因此英文补全可以由 Host 在不改变中文引擎的情况下独立显示与提交。
- Apple `InputMethodKit` 总览也把“输入法 Host 管 client 通信、候选窗、输入模式；转换引擎可以用任意语言实现”作为框架边界，这与 Inputia 的分层目标一致。
- 鼠须管 `SquirrelInputController.swift` 的成熟实现同样是 Host 处理事件、candidate selection 和 `insertText`，Rime 负责转换引擎，不把整套输入法状态封进 Rime。

设计选择：

- 英文模式继续保持直通输入，不把英文单词做成 marked text，避免破坏用户对英文输入的即时反馈。
- Host 在英文模式下维护一个轻量 `englishCompletionPrefix`：
  - 连续 ASCII 字母/数字/`_`/`-` 进入前缀。
  - 空格、回车、标点等词边界触发 `learnTyped(fullWord)`，只学习长度至少 2 且包含英文字母的词。
  - 前缀长度至少 2 时查询本地记忆里的英文补全候选。
  - 候选只显示 ASCII 词，大小写不敏感，来源标记为 `english_completion`。
  - 按 Tab 接受第一条候选，只插入“后缀”，不反向替换已输入前缀，从而避开跨 App replacement range 兼容风险。
- C API 新增 `inputia_session_completion_candidates(session, prefix, limit)`，Bridge 新增：
  - `learnTyped(text:bundleId:)`
  - `completionCandidates(prefix:limit:)`
  - `--bridge-english-completion-self-check`

修复文件：

- `crates/inputia-core/src/lib.rs`
  - `LocalMemory::english_completion_candidates`
  - `SqliteMemory::english_completion_candidates`
  - 新增 `english_completion_candidates_are_case_insensitive_and_memory_ranked` 测试
- `crates/inputia-capi/src/lib.rs`
  - 新增 `inputia_session_completion_candidates`
  - 新增 `capi_returns_english_completion_candidates_from_typed_memory` 测试
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaRustBridge.swift`
  - 新增 typed learn 和 completion query bridge
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/main.swift`
  - 英文前缀跟踪、词边界学习、候选显示、Tab 补齐后缀
- Host/设置启动器版本升到 v18。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-core/Cargo.toml
  22 passed
  includes english_completion_candidates_are_case_insensitive_and_memory_ranked

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  12 passed
  includes capi_returns_english_completion_candidates_from_typed_memory

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  3 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml
  5 passed

cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
  3 passed

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v18-c977648f46c0.pkg
  pkg shasum256=2994acbbabefed3da25a8637bb735c2a8246600e991df9a17d714dc79bcaedc1
  appCDHash=c977648f46c0e2166203cc6799ec23661ff633c7

pkgutil --expand-full InputiaInputMethod-v18-c977648f46c0.pkg
  expanded app CFBundleVersion=18
  expanded app CFBundleShortVersionString=0.0.18
  expanded app CDHash=c977648f46c0e2166203cc6799ec23661ff633c7

nm -gU build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod
  _inputia_session_completion_candidates
  _inputia_session_learn
  _inputia_session_clipboard_candidates

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v18 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=18
  inputiaInstalledCDHash=c977648f46c0e2166203cc6799ec23661ff633c7
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v18-expand/Scripts/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

限制：

- 英文补全当前使用 Tab 接受第一候选；数字选词仍保留给中文候选和剪贴板召回，避免英文输入数字时被抢键。
- 当前补全只插入后缀，不做跨 App 前缀替换，这是为了先保障兼容性。后续如果要做更接近微信输入法的补全 UI，需要单独验证 selected range / replacement range 在 Safari、TextEdit、Codex、微信等 App 的差异。
- 本机系统安装态仍需用户安装 v18 后再跑 `verify-system.sh` / `smoke-textedit.sh`；Codex 进程内直接执行 ad-hoc 输入法 app 仍受 `amfid` 拦截限制。

## 2026-07-07 v19：全角/半角输入设置

本轮补第一阶段验收中的“全角/半角相关设置”缺口。

依据：

- Microsoft 简体中文 IME 官方文档把 `Shift + Space` 定义为全角/半角字符宽度切换，把 `Ctrl + .` 定义为中文/英文标点切换；这说明字符宽度和标点偏好是两个独立状态。
- Apple 中文输入法文档提供“Convert Text to Full Width / Half Width”的 Latin 字符转换能力；Apple 日文输入法文档也把 full-width / half-width numerals 作为输入源设置。
- Inputia 当前 Shift 切换中英文还在做跨 App 稳定性验证，因此本轮只实现设置态全角/半角，不先抢 `Shift + Space` 快捷键，避免把两个风险混在一起。

设计选择：

- 新增设置字段：

```json
"character_width_preference": "half_width" | "full_width"
```

- 默认 `half_width`，旧 `settings.json` 缺字段时由 Swift Bridge patch 为 `half_width`，设置页 Swift 结构体用 optional decode，避免老配置 decode 失败。
- `full_width` 生效范围：
  - 英文模式直通输入：ASCII `!` 到 `~` 映射为 U+FF01 到 U+FF5E，空格映射为 U+3000。
  - 中文模式中按 Return 提交 raw composing 时同样映射，例如 `ni` -> `ｎｉ`。
- 不生效范围：
  - 中文候选词上屏不做全角转换。
  - “中文模式始终使用英文标点”仍保持半角英文标点，即使全角输入打开，也不把 `,` 改成 `，`，避免破坏该设置的字面承诺。

修复文件：

- `crates/inputia-core/src/lib.rs`
  - 新增 `CharacterWidthPreference`
  - `CoreSettings.character_width_preference`
  - 英文直通和 raw composing commit 的全角转换
  - 新增 3 个测试覆盖英文全角、raw composing 全角、英文标点保持半角
- `crates/inputia-settings/src/lib.rs`
  - 新增可序列化 `CharacterWidthPreference`
  - 默认半角，round-trip 测试覆盖全角
- `crates/inputia-capi/src/lib.rs`
  - 从 `InputiaSettings.character_width_preference` 映射到 Core
  - 新增 `capi_loads_full_width_setting_from_settings_file`
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaSettingsWindow.swift`
  - 设置页新增“使用全角输入”复选框
  - 旧配置 optional decode + sanitize 默认半角
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaRustBridge.swift`
  - 默认 settings writer 写入 `character_width_preference`
  - patch 旧 settings 时补 `half_width`
- Host/设置启动器版本升到 v19。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-core/Cargo.toml
  25 passed
  includes full_width_mode_translates_english_direct_input
  includes full_width_mode_translates_raw_composition_but_not_chinese_candidates
  includes english_punctuation_in_chinese_mode_stays_half_width_even_in_full_width_mode

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  3 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  13 passed
  includes capi_loads_full_width_setting_from_settings_file

cargo test --manifest-path crates/inputia-rime/Cargo.toml
  5 passed

cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
  3 passed

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  build CFBundleVersion=19
  build CFBundleShortVersionString=0.0.19
  build CDHash=cf2fb55fb85c536144ae0c4972433bdc9abfb052

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v19-cf2fb55fb85c.pkg
  pkg shasum256=c80f636b96e289079cd1aebb2b524cbb498347a4a03b70c6095ee867733b1e94
  appCDHash=cf2fb55fb85c536144ae0c4972433bdc9abfb052

pkgutil --expand-full InputiaInputMethod-v19-cf2fb55fb85c.pkg
  expanded app CFBundleVersion=19
  expanded app CFBundleShortVersionString=0.0.19
  expanded app CDHash=cf2fb55fb85c536144ae0c4972433bdc9abfb052
  expanded RimeData schema count=21

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v19 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=19
  inputiaInstalledCDHash=cf2fb55fb85c536144ae0c4972433bdc9abfb052
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v19-expand/Scripts/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

系统安装态：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
```

限制：

- v19 已构建并打包，但本机系统输入法仍是 v13；需要安装 v19 后再跑系统态验证。
- `Shift + Space` 快捷切换全角/半角还未实现；本轮只交付设置页控制，下一步可在 Host special-key 层单独做并验证与 Shift 中英文切换的冲突。

## 2026-07-07 v20：运行时标点/全半角切换与设置页按钮布局

本轮针对用户反馈：

- 中文输入中想临时打英文/标点时，状态切换不稳定，可能留下 marked text 下划线。
- `Shift + Space` / `Ctrl + .` 这类输入法级快捷键此前没有被 Host 正确拦截。
- 设置页底部按钮在不同窗口/字体状态下容易被压缩成异常形态。

依据：

- Microsoft 简体中文 IME 官方文档：
  - `Shift` 切换中英文模式。
  - `Shift + Space` 切换全角/半角字符宽度。
  - `Ctrl + .` 切换中文/英文标点。
  - 链接：https://support.microsoft.com/en-us/windows/hardware/input-devices/microsoft-simplified-chinese-ime
- Apple 中文输入法文档说明中文输入源下 Latin 字符可在 half-width / full-width 之间转换，默认 half-width。
  - 链接：https://support.apple.com/guide/chinese-input-method/enter-latin-characters-cim11841/mac

设计选择：

- Core 新增两个输入法专用 Key：
  - `TogglePunctuation`
  - `ToggleCharacterWidth`
- 两个运行时切换都会先清空未上屏 composition，避免切换后留下下划线 marked text，或后续输入把旧拼音串吞掉。
- Host 在通用 `command/control/option` 放行之前拦截：
  - `Ctrl + .` -> `togglePunctuationPreference()`
  - `Shift + Space` -> `toggleCharacterWidthPreference()`
- `Shift + Space` 不再落入普通空格逻辑，因此不会误触发候选上屏。
- `Ctrl + .` 不再被 control 修饰键分支提前返回。
- 设置页底部 footer 改为单行弹性布局：
  - 左侧状态路径可截断。
  - 右侧“打开配置文件夹”“保存设置”按钮保持 required hugging/compression。
  - 主按钮放最右侧，避免被压缩成小竖条。

修复文件：

- `crates/inputia-core/src/lib.rs`
  - 新增运行时标点/全半角切换 Key
  - 新增 3 个测试覆盖运行时标点切换、运行时全半角切换、切换清空未上屏 composition
- `crates/inputia-capi/src/lib.rs`
  - 新增 `KEY_TOGGLE_PUNCTUATION = 8`
  - 新增 `KEY_TOGGLE_CHARACTER_WIDTH = 9`
  - 新增 CAPI 运行时切换测试
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaRustBridge.swift`
  - 新增 `togglePunctuationPreference()`
  - 新增 `toggleCharacterWidthPreference()`
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/main.swift`
  - 新增 `Ctrl + .` 和 `Shift + Space` Host 级快捷键识别
  - 快捷键在修饰键放行和普通 Space 分支之前处理
- `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaSettingsWindow.swift`
  - 设置页 footer 按钮布局改为弹性、抗压缩布局
- Host/设置启动器版本升到 v20。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-core/Cargo.toml
  28 passed
  includes toggle_punctuation_switches_chinese_punctuation_runtime
  includes toggle_character_width_switches_direct_and_raw_input_runtime
  includes runtime_toggles_clear_uncommitted_composition

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  3 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  14 passed
  includes capi_toggles_punctuation_and_character_width_runtime

cargo test --manifest-path crates/inputia-rime/Cargo.toml
  5 passed

cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
  3 passed

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v20-1499344a72ea.pkg
  pkg shasum256=edc20f0c153616f5cf1eebf74bf1f72c9ad27f505c5e54fbe36acdc16b86681c
  appCDHash=1499344a72ea3a31ff464ee40d3158f01942c408

pkgutil --expand-full InputiaInputMethod-v20-1499344a72ea.pkg
  expanded app CFBundleVersion=20
  expanded app CFBundleShortVersionString=0.0.20
  expanded app CDHash=1499344a72ea3a31ff464ee40d3158f01942c408
  expanded RimeData schema count=30

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v20-final INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=20
  inputiaInstalledCDHash=1499344a72ea3a31ff464ee40d3158f01942c408
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v20-final-apps/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

系统安装态：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
sudo -n installer -pkg InputiaInputMethod-v20-1499344a72ea.pkg -target /
  sudo: a password is required
```

限制：

- v20 已构建并打包，但当前系统安装态仍是 v13；菜单栏里当前实际运行的仍可能是旧代码。
- Codex 不能在无密码 sudo 的情况下完成 `/Library/Input Methods` 系统安装；需要通过 macOS Installer UI 或带管理员授权的安装命令安装 v20。
- 需要安装 `macos/InputiaInputMethod/dist/InputiaInputMethod-v20-1499344a72ea.pkg` 后，再做真实系统态验证：
  - 菜单栏选择 Inputia。
  - TextEdit/Safari 输入中文候选。
  - `Ctrl + .` 在中文模式下切换英文/中文标点。
  - `Shift + Space` 切换全角/半角。

## 2026-07-07 v20 后续：Host 快捷键稳定自检与最终包

前一段 v20 包 `InputiaInputMethod-v20-1499344a72ea.pkg` 已被本段最终包取代。本段没有改变产品版本号，仍是 `CFBundleVersion=20` / `0.0.20`，但新增了 Host 快捷键源码级自检和脚本互调稳定性修正。

背景：

- `--host-shortcut-self-check` 直接运行 `.app` 在当前 Codex/macOS 环境仍会被系统杀掉，exit `137`，和此前 ad-hoc `.app` 诊断不稳定一致。
- 为避免把这个环境限制误判为 Host 逻辑失败，新增独立命令行工具 `build/inputia-shortcut-self-check`。
- 该工具和 IMK Host 编译同一份 `InputiaShortcutClassifier.swift`，但不启动 `.app` / `IMKServer`，因此可以稳定验证 Host 快捷键分类逻辑。
- 直接执行 `build.sh` 曾出现一次无输出 exit `137`；显式 `/bin/zsh macos/InputiaInputMethod/build.sh` 完整通过。为降低脚本入口受 provenance/系统安全链路影响的概率，脚本互调改为显式 `/bin/zsh`。

设计选择：

- 抽出共享 Swift 源码：
  - `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift`
- Host 和独立自检工具共用该 classifier：
  - `Ctrl + .` 命中标点切换
  - `Shift + Space` 命中全角/半角切换
  - `Ctrl + Shift + V` 保留给剪贴板召回
  - 带 `Command` / 额外 `Control` 的组合不误命中
- 新增工具：
  - `macos/InputiaInputMethod/Tools/InputiaShortcutSelfCheck.swift`
  - 构建输出：`macos/InputiaInputMethod/build/inputia-shortcut-self-check`
- `build-pkg.sh` / `install-system.sh` / `open-installer.sh` 中对内部脚本的调用改为显式 `/bin/zsh ...`。

验证：

```text
/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  ctrlPeriodPunctuation=true
  ctrlShiftPeriodRejected=true
  ctrlCommandPeriodRejected=true
  shiftSpaceCharacterWidth=true
  ctrlShiftSpaceRejected=true
  plainSpaceRejected=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

bash -n macos/InputiaInputMethod/verify-system.sh
  OK

zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/install-system.sh macos/InputiaInputMethod/open-installer.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK

/bin/zsh macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v20-3ad6fb1fbfd5.pkg
  pkg shasum256=576f54ac43472836403d756425980168517bbc7311d8909e7661582f2b32b0c7
  appCDHash=3ad6fb1fbfd50dd26ed549a5f2b2fa9401eca84c

pkgutil --expand-full InputiaInputMethod-v20-3ad6fb1fbfd5.pkg
  expanded app CFBundleVersion=20
  expanded app CFBundleShortVersionString=0.0.20
  expanded app CDHash=3ad6fb1fbfd50dd26ed549a5f2b2fa9401eca84c
  expanded RimeData schema count=30

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v20-shortcut INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=20
  inputiaInstalledCDHash=3ad6fb1fbfd50dd26ed549a5f2b2fa9401eca84c
  inputiaSettingsLauncherInstalled=true

cargo test --manifest-path crates/inputia-core/Cargo.toml
  28 passed

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  3 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  14 passed

cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
  3 passed

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v20-shortcut-apps/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

系统安装态仍未替换：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
sudo -n installer -pkg InputiaInputMethod-v20-3ad6fb1fbfd5.pkg -target /
  sudo: a password is required
```

当前应安装的最终包：

```text
macos/InputiaInputMethod/dist/InputiaInputMethod-v20-3ad6fb1fbfd5.pkg
```

## 2026-07-07 v22：拼音纠错提升、marked text 上屏修正、设置页按钮稳定

前一段本轮临时产物 `InputiaInputMethod-v21-5cf3debc01e0.pkg` 已被 v22 取代。v22 是本段最终包。

外部资料依据：

- Rime 官方 `SpellingAlgebra` 文档说明 `derive` 可用于常见错拼 / 模糊拼音，例如 `dagn -> dang`、`hoa -> hao`、`tain -> tian`、`zhonguo -> zhong guo`。
  - https://github.com/rime/home/wiki/SpellingAlgebra
- Rime 官方 `CustomizationGuide` 说明配置应通过 schema/custom patch 和重新部署进入 Rime 配置体系。
  - https://github.com/rime/home/wiki/CustomizationGuide

本地 spike：

```text
rime_probe luna_pinyin_simp zhonguo
  原始 Rime 能给出 中国，但不是第一候选。

rime_probe luna_pinyin_simp dagn
  第一候选 大概你，第二候选 当。

rime_probe luna_pinyin_simp hoa
  第一候选 后啊，第二候选 好。

rime_probe luna_pinyin_simp tain
  第一候选 太难，第二候选 天。
```

结论：Rime 已能产出部分纠错目标，但默认排序不稳定；本阶段采用 adapter 级别的候选提升，不把纠错规则写死进 Core，也不把 Rime 变成 Iputia 的核心大脑。

实现：

- `inputia-rime`
  - 新增 `RimeEngineConfig.spelling_correction`，默认开启，可由设置关闭。
  - 在调用原始 composing 前先探测少量纠错变体，并按候选文本去重合并。
  - 已覆盖常见错拼提升：`zhonguo -> 中国`、`dagn -> 当`、`hoa -> 好`、`tain -> 天`。
- `inputia-settings` / `inputia-capi`
  - 新增 `spelling_correction_enabled` 设置项，旧配置缺字段时默认 `true`。
  - CAPI 从 settings 文件读取后传给 Rime adapter。
- macOS Settings
  - 新增 `启用拼音纠错` 复选框。
  - 底部 footer 改为“路径一行、按钮一行”，避免长配置路径挤坏 `打开配置文件夹` / `保存设置` 按钮。
- macOS IMK Host
  - `apply(_:)` 中提交候选或 Enter 原文上屏时，改用当前 `markedRange()` 作为 replacement range。
  - `setMarkedText` 更新组合串时同样替换当前 marked range。
  - `commitComposition(_:)` 先走 `bridge.enter()` 清掉 Core 组合状态；如果 bridge 没返回 commit，再用 Host 本地 composing 兜底并 `bridge.escape()`。
  - 目的：修复部分 App 中“组合串下面还有下划线、按回车没真正进入输入框、下一次输入把前面覆盖掉”的 marked text 生命周期问题。

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  28 passed

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  3 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  15 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml
  unit 2 passed
  core_flow 3 passed
  schema_smoke 2 passed

plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

bash -n macos/InputiaInputMethod/verify-system.sh
  OK

zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/install-system.sh macos/InputiaInputMethod/open-installer.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  ctrlPeriodPunctuation=true
  ctrlShiftPeriodRejected=true
  ctrlCommandPeriodRejected=true
  shiftSpaceCharacterWidth=true
  ctrlShiftSpaceRejected=true
  plainSpaceRejected=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true
```

最终包：

```text
/bin/zsh macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v22-95fb72a0ca84.pkg
  pkg shasum256=cfe2cc507f6a730838e5487d224a1bd873409f587f383f35867cfa9de00fb958
  appCDHash=95fb72a0ca84ad91bd161786332ac4d2e8b2c937

pkgutil --expand-full InputiaInputMethod-v22-95fb72a0ca84.pkg
  expanded app CFBundleIdentifier=com.inputia.inputmethod.Inputia
  expanded app CFBundleVersion=22
  expanded app CFBundleShortVersionString=0.0.22
  expanded InputMethodConnectionName=com.inputia.inputmethod.Inputia_Connection
  expanded tsInputMethodCharacterRepertoireKey=Hans,Hant
  expanded settings CFBundleVersion=22
  expanded settings CFBundleShortVersionString=0.0.22
  expanded app codesign --verify --deep --strict passed
  expanded settings codesign --verify --deep --strict passed
  expanded app CDHash=95fb72a0ca84ad91bd161786332ac4d2e8b2c937
  expanded RimeData schema count=21

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v22.GM1uL8 INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=22
  inputiaInstalledCDHash=95fb72a0ca84ad91bd161786332ac4d2e8b2c937
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v22.ANFKaR/extracted/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_full_pinyin_promotes_spelling_corrections_when_available passed
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

包内 schema：

```text
bopomofo.schema.yaml
bopomofo_express.schema.yaml
bopomofo_tw.schema.yaml
cangjie5.schema.yaml
cangjie5_express.schema.yaml
double_pinyin.schema.yaml
double_pinyin_abc.schema.yaml
double_pinyin_flypy.schema.yaml
double_pinyin_mspy.schema.yaml
double_pinyin_pyjj.schema.yaml
double_pinyin_sogou.schema.yaml
double_pinyin_st.schema.yaml
guobiao_bispell.schema.yaml
luna_pinyin.schema.yaml
luna_pinyin_fluency.schema.yaml
luna_pinyin_simp.schema.yaml
luna_pinyin_tw.schema.yaml
luna_quanpin.schema.yaml
stroke.schema.yaml
terra_pinyin.schema.yaml
terra_pinyin_12345.schema.yaml
```

系统安装态仍未替换：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleIdentifier=com.inputia.inputmethod.Inputia
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
/Library/Input Methods/InputiaInputMethod.app CFBundleShortVersionString=0.0.13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374

sudo -n installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod-v22-95fb72a0ca84.pkg -target /
  sudo: a password is required
```

当前应安装的最终包：

```text
macos/InputiaInputMethod/dist/InputiaInputMethod-v22-95fb72a0ca84.pkg
```

限制：

- 当前 Codex 会话没有无交互管理员权限，不能直接替换 `/Library/Input Methods/InputiaInputMethod.app`，因此真实菜单栏 / Safari / TextEdit 系统态烟测仍未完成。
- 需要安装 v22 后再验证：
  - 菜单栏选择 Inputia。
  - 中文组合中按 Enter 是否直接上屏英文原文并清掉下划线。
  - 全拼输入 `zhonguo` / `dagn` / `hoa` / `tain` 是否优先给出纠错候选。
  - 搜狗双拼、国标双拼、自然码等 schema 是否能稳定显示候选。
  - 设置页底部按钮是否不再被配置路径挤压。

## 2026-07-07 v23：原始 composing 兜底候选可用数字 1 上屏

前一段最终包 `InputiaInputMethod-v22-95fb72a0ca84.pkg` 已被 v23 取代。v23 没有改变 Core/Rime 候选算法，只补 Host 的兜底候选交互。

背景：

- v22 中 Host 会在 Rime 暂时没有候选但仍有 composing 时，在候选窗显示 `[latestComposing]`，避免用户只看到输入框下划线。
- 但候选窗显示格式仍是 `1 composing`，如果用户按 `1`，旧 Host 会把 `1` 交给 Core，Core 在没有可见候选时会把数字当作 composing 的一部分。这和 UI 暗示不一致。
- Apple `NSTextInputClient.setMarkedText(_:selectedRange:replacementRange:)` 文档说明会替换指定文本范围；因此 Host 继续沿用 v22 的 `markedRange()` replacement 方案处理提交，避免 marked text 残留。
  - https://developer.apple.com/documentation/appkit/nstextinputclient/setmarkedtext%28_%3Aselectedrange%3Areplacementrange%3A%29

实现：

- `InputiaShortcutClassifier` 新增 `isDisplayedRawCompositionSelection(...)`：
  - 只有 `hasComposing == true`
  - 且 `hasCandidates == false`
  - 且无 Command/Control/Option
  - 且字符和忽略修饰键字符都为 `1`
  时才命中。
- `main.swift` 在普通字符分发前检查该状态，命中后调用 `bridge.enter()`，走同一条原文上屏和 marked range 替换路径。
- 独立 `inputia-shortcut-self-check` 和 app 内 `--host-shortcut-self-check` 同步覆盖该判断，避免 Host 逻辑和诊断工具漂移。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

bash -n macos/InputiaInputMethod/verify-system.sh
  OK

zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/install-system.sh macos/InputiaInputMethod/open-installer.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  ctrlPeriodPunctuation=true
  ctrlShiftPeriodRejected=true
  ctrlCommandPeriodRejected=true
  shiftSpaceCharacterWidth=true
  ctrlShiftSpaceRejected=true
  plainSpaceRejected=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true
  rawCompositionOneSelectsFallback=true
  rawCompositionTwoRejected=true
  rawCompositionOneRejectedWhenCandidatesExist=true
  rawCompositionOneRejectedWithCommand=true
```

最终包：

```text
/bin/zsh macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v23-a4e10dcdab22.pkg
  pkg shasum256=58c6c9637aad12f4b350ba2195b26292768e2579ee182aaa30a1bdbfef08ddd4
  appCDHash=a4e10dcdab2236bb3da42dfe02d336c8d0a28b82

pkgutil --expand-full InputiaInputMethod-v23-a4e10dcdab22.pkg
  expanded app CFBundleVersion=23
  expanded app CFBundleShortVersionString=0.0.23
  expanded settings CFBundleVersion=23
  expanded settings CFBundleShortVersionString=0.0.23
  expanded app codesign --verify --deep --strict passed
  expanded settings codesign --verify --deep --strict passed
  expanded app CDHash=a4e10dcdab2236bb3da42dfe02d336c8d0a28b82
  expanded RimeData schema count=21

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v23.JVKM5u INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=23
  inputiaInstalledCDHash=a4e10dcdab2236bb3da42dfe02d336c8d0a28b82
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v23.Jzaoar/extracted/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_full_pinyin_promotes_spelling_corrections_when_available passed
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

系统安装态仍未替换：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
/Library/Input Methods/InputiaInputMethod.app CFBundleShortVersionString=0.0.13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374

sudo -n installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod-v23-a4e10dcdab22.pkg -target /
  sudo: a password is required
```

当前应安装的最终包：

```text
macos/InputiaInputMethod/dist/InputiaInputMethod-v23-a4e10dcdab22.pkg
```

## 2026-07-07 v24：中英文切换支持改键

前一段最终包 `InputiaInputMethod-v23-a4e10dcdab22.pkg` 已被 v24 取代。v24 补齐第一阶段验收里的“Shift 快速切换，并在设置中允许用户关闭或改键”。

外部资料依据：

- Apple `NSEvent.keyCode` 文档说明 `keyCode` 是 key event 对应的 virtual key code。
  - https://developer.apple.com/documentation/appkit/nsevent/keycode
- Apple `NSEvent.ModifierFlags` 文档说明该结构表示 event 中的按键修饰状态。
  - https://developer.apple.com/documentation/appkit/nsevent/modifierflags-swift.struct

设计选择：

- 保留旧字段 `shift_toggle_enabled`，避免破坏已经存在的 Core/CAPI 兼容路径。
- 新增 `input_mode_toggle_shortcut`，可选：
  - `shift`
  - `control_space`
  - `none`
- `shift_toggle_enabled` 作为兼容派生字段：shortcut 为 `shift` 时写 `true`，其它值写 `false`。
- 旧配置如果只有 `shift_toggle_enabled=false`，会迁移为 `input_mode_toggle_shortcut=none`。
- Core 新增 `Key::ToggleInputMode`，表示“Host 已经判定某个配置快捷键命中，直接切换模式”。它不依赖 `shift_toggle_enabled`。
- `Key::Shift` 继续保留旧语义，只在 `shift_toggle_enabled=true` 时切换。
- Host 负责读取设置并判断快捷键：
  - `shift`：使用 `flagsChanged`，仅单独按下/释放 Shift 命中。
  - `control_space`：使用 `keyDown`，仅 `Control + Space` 命中。
  - `none`：都不命中。

实现：

- `crates/inputia-settings`
  - 新增 `InputModeToggleShortcut` enum。
  - 新增字段 `input_mode_toggle_shortcut`。
  - 保存/读取时同步维护 `shift_toggle_enabled`。
- `crates/inputia-core`
  - 新增 `Key::ToggleInputMode`。
- `crates/inputia-capi`
  - 新增 special key `KEY_TOGGLE_INPUT_MODE = 10`。
- `macos/InputiaInputMethod`
  - 设置页把旧复选框升级为“中英切换”下拉：`Shift` / `Control + Space` / `关闭`。
  - `InputiaShortcutClassifier` 新增 Shift/Control+Space 输入模式切换判断。
  - `InputiaRustBridge.toggleInputMode()` 改走显式 `KEY_TOGGLE_INPUT_MODE`，不再伪装成 `KEY_SHIFT`。
  - `ensureSettingsFile` 自动给旧 JSON 补 `input_mode_toggle_shortcut`。

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  29 passed

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  4 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  16 passed
  includes capi_explicit_input_mode_toggle_supports_remapped_shortcut

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  shiftInputModeArmsWhenConfigured=true
  shiftInputModeRejectedWhenDisabled=true
  shiftInputModeReleaseTogglesWhenArmed=true
  controlSpaceInputModeTogglesWhenConfigured=true
  controlSpaceInputModeRejectedWhenShiftConfigured=true
```

静态检查：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

bash -n macos/InputiaInputMethod/verify-system.sh
  OK

zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/install-system.sh macos/InputiaInputMethod/open-installer.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK
```

最终包：

```text
/bin/zsh macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v24-55a750502bb1.pkg
  pkg shasum256=3e90a58d6965e05f9b4a0e715cc4473eca8dd45d50518eb18de19edabfa97e8a
  appCDHash=55a750502bb10ad925c95c5319fdde885029032a

pkgutil --expand-full InputiaInputMethod-v24-55a750502bb1.pkg
  expanded app CFBundleVersion=24
  expanded app CFBundleShortVersionString=0.0.24
  expanded settings CFBundleVersion=24
  expanded settings CFBundleShortVersionString=0.0.24
  expanded app codesign --verify --deep --strict passed
  expanded settings codesign --verify --deep --strict passed
  expanded app CDHash=55a750502bb10ad925c95c5319fdde885029032a
  expanded RimeData schema count=21

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v24.ur7vSy INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=24
  inputiaInstalledCDHash=55a750502bb10ad925c95c5319fdde885029032a
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v24.n8SbcV/extracted/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_full_pinyin_promotes_spelling_corrections_when_available passed
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

系统安装态仍未替换：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
/Library/Input Methods/InputiaInputMethod.app CFBundleShortVersionString=0.0.13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374

sudo -n installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod-v24-55a750502bb1.pkg -target /
  sudo: a password is required
```

当前应安装的最终包：

```text
macos/InputiaInputMethod/dist/InputiaInputMethod-v24-55a750502bb1.pkg
```

## 2026-07-07 v25：窗口上下文隐私判定与私密窗口不读不学

前一段最终包 `InputiaInputMethod-v24-55a750502bb1.pkg` 已被 v25 取代。v25 补强第一阶段验收里的“敏感 App 默认不读不学”，覆盖到窗口标题上下文，尤其是私密/无痕浏览窗口。

外部资料依据：

- Apple `NSWorkspace.frontmostApplication` 可取得前台应用。
  - https://developer.apple.com/documentation/appkit/nsworkspace/frontmostapplication
- Apple `CGWindowListCopyWindowInfo` 可取得当前用户会话里的窗口配置详情。
  - https://developer.apple.com/documentation/coregraphics/cgwindowlistcopywindowinfo%28_%3A_%3A%29
- Apple `kCGWindowOwnerPID` 是窗口所属进程 pid，`kCGWindowName` 是窗口名。
  - https://developer.apple.com/documentation/coregraphics/kcgwindowownerpid
  - https://developer.apple.com/documentation/coregraphics/kcgwindowname

设计选择：

- 不把窗口标题用于学习内容本身，只作为本地隐私排除判定。
- `AppContext` 原本已经有 `window_title` 字段，本段把 Host/CAPI 通路接通。
- Host 尝试用 `NSWorkspace.frontmostApplication` + `CGWindowListCopyWindowInfo` 找到当前宿主 app 的前台窗口标题。
- 如果拿不到窗口标题，回退到只按 bundle id 判断，不阻断普通输入链。
- 敏感窗口标题关键词先保守覆盖：
  - `private browsing`
  - `private window`
  - `incognito`
  - `隐私浏览`
  - `无痕`
  - `密码`
  - `password`
  - `银行`
  - `bank`
  - `医疗`
  - `medical`

实现：

- `inputia-core`
  - `AppContext::with_window_title(...)`
  - `AppPolicy` 新增 `sensitive_context_terms`
  - `excludes(...)` 同时检查 bundle id、window title、document id
- `inputia-capi`
  - 新增 `inputia_session_set_app_context_with_window(...)`
  - `inputia_session_learn(...)` 在 bundle id 与 session context 一致时沿用窗口上下文
- macOS Host
  - 新增 `InputiaAppContext`
  - `updateAppContext` / sensitive pass-through / 剪贴板读取 / 英文补全学习都使用同一份 bundle + window title
  - 剪贴板读取在 Swift 层先做窗口标题敏感判定，避免进入私密窗口后先读剪贴板再被 Rust 拦截
  - 新增独立 `inputia-bridge-privacy-self-check`，避免直接运行 `.app --bridge-clipboard-privacy-self-check` 被 macOS 杀掉 exit `137`

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed
  includes sensitive_window_contexts_do_not_learn

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  4 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed
  includes capi_window_contexts_can_block_learning

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true
  texteditAllowsClipboardRead=true
  onepasswordRejectsClipboardRead=true
  unknownRejectsClipboardRead=true
  privateWindowRejectsClipboardRead=true

direct build settings visual smoke
  command=build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --open-settings
  screenshot=/tmp/inputia-settings-v26-direct.png
  result=footer path is middle-truncated; both 打开配置文件夹 and 保存设置 buttons are fully visible
```

静态检查：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

bash -n macos/InputiaInputMethod/verify-system.sh
  OK

zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/install-system.sh macos/InputiaInputMethod/open-installer.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK
```

最终包：

```text
/bin/zsh macos/InputiaInputMethod/build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v25-598c392c8902.pkg
  pkg shasum256=8645d0d5deeb92d4ee03b89855effe4e86d3b566fcce40e15aa9a00e31fefcce
  appCDHash=598c392c8902e94003e4fbf42d81c28a49835323

pkgutil --expand-full InputiaInputMethod-v25-598c392c8902.pkg
  expanded app CFBundleVersion=25
  expanded app CFBundleShortVersionString=0.0.25
  expanded settings CFBundleVersion=25
  expanded settings CFBundleShortVersionString=0.0.25
  expanded app codesign --verify --deep --strict passed
  expanded settings codesign --verify --deep --strict passed
  expanded app CDHash=598c392c8902e94003e4fbf42d81c28a49835323
  expanded RimeData schema count=21

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-v25.K1cEqR INPUTIA_POSTINSTALL_SKIP_TIS=1 postinstall
  inputiaInstalledVersion=25
  inputiaInstalledCDHash=598c392c8902e94003e4fbf42d81c28a49835323
  inputiaSettingsLauncherInstalled=true

INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-pkg-v25.AwpGrg/extracted/InputiaInputMethod.app/Contents/Resources/RimeData \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke
  bundled_full_pinyin_promotes_spelling_corrections_when_available passed
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

系统安装态仍未替换：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
/Library/Input Methods/InputiaInputMethod.app CFBundleShortVersionString=0.0.13
systemCDHash=db6a282f35b2bf46dcea4a6df750a4c3c184f374

sudo -n installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod-v25-598c392c8902.pkg -target /
  sudo: a password is required
```

当前应安装的最终包：

```text
macos/InputiaInputMethod/dist/InputiaInputMethod-v25-598c392c8902.pkg
```
## 2026-07-07 v26：Host marked text 策略与设置按钮防挤压

用户反馈：

- 设置窗口右下角保存按钮有时被长路径挤成窄条。
- 中文组合态下按 Return 想直接提交英文/拼写串时，部分 App 会留下下划线 marked text；继续输入时旧 composition 可能被覆盖。
- 全拼/双拼有时不出候选。

依据：

- Apple `IMKTextInput` 文档说明 `insertText:replacementRange:` 用于发送 fully converted text；当要插入到当前 selection 时，replacement range 应使用 `NSNotFound`。同一文档说明如果 client 不支持 TSMDocumentAccess，replacement range 会被忽略。
- Apple `IMKTextInput` 文档说明 `markedRange` 是当前 inline session 的范围，marked text 以下划线表示尚未完成转换。
- 鼠须管 `SquirrelInputController.swift` 的成熟路径是 commit 时 `client.insertText(string, replacementRange: .empty)`，preedit 显示时 `client.setMarkedText(..., replacementRange: .empty)`，不拿 `client.markedRange()` 作为常规写入 range。
- 本地 `schema_smoke` 已用真实 librime 和打包 RimeData 验证全拼、自然码、小鹤、搜狗、微软、ABC、拼音加加、四通、国标双拼在正确键序下都能产出“中国”。因此本轮把“候选完全没有反馈”的主要修复点放在 Host 状态/显示链路，而不是继续随机改 schema。

修复：

- 新增 `InputiaHostTextPolicy.swift`：
  - commit 和 marked text 写入统一使用 `NSRange(location: NSNotFound, length: 0)`。
  - Rime 暂时没有候选但 composing 非空时，候选窗显示 raw composing 兜底项。
- `main.swift`：
  - `insertText` / `setMarkedText` 不再使用 `client.markedRange()` 作为 replacement range。
  - Return raw composing、候选上屏、英文直通 commit 都走统一 commit helper。
  - 敏感 App pass-through 清理内部状态时也同步清掉 marked text。
- `InputiaSettingsWindow.swift`：
  - footer 改成路径中间截断 + 两个固定宽度按钮的水平行。
  - “保存设置”和“打开配置文件夹”按钮设置 required hugging/compression，长路径不能再把按钮挤窄。
- 新增 `InputiaHostTextPolicySelfCheck.swift`，并接入 `build.sh`。
- Host/设置启动器版本升到 v26。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed
  covers luna_pinyin_simp, double_pinyin, double_pinyin_flypy, double_pinyin_sogou,
  guobiao_bispell, double_pinyin_mspy, double_pinyin_abc, double_pinyin_pyjj,
  double_pinyin_st

macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  replacementRangeLocationIsNSNotFound=true
  replacementRangeLengthIsZero=true
  rawComposingFallbackCandidate=true
  engineCandidatesPreferred=true
  emptyCompositionHasNoFallback=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true
  texteditAllowsClipboardRead=true
  onepasswordRejectsClipboardRead=true
  unknownRejectsClipboardRead=true
  privateWindowRejectsClipboardRead=true
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v26-c79a34831bc7.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=4c15abb64ce4112f1e864314359eca6067e3d36cdb558a9ca63bc2b9dcc2e54a
appCDHash=c79a34831bc74884800c3e090fab075c2a85940c
pkgScriptAppVersion=26
pkgScriptSettingsVersion=26
pkgScriptSchemaCount=21
postinstallDryRunVersion=26
```

系统安装状态：

```text
/Library/Input Methods/InputiaInputMethod.app CFBundleVersion=13
sudo -n installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod-v26-c79a34831bc7.pkg -target /
  sudo: a password is required
```

结论：v26 包已构建并离线验证；当前系统菜单栏里仍运行 v13，必须完成管理员安装后才能在真实 App 中验证这次 marked text/按钮修复。

## 2026-07-07 v27：安装版本诊断与设置入口防旧版误开

用户继续反馈：

- 菜单里能看到 Inputia，但实际打字/候选行为仍像旧版。
- 设置入口/设置窗口不稳定，容易打开到旧 UI 或被误判为新版。

根因证据：

- `status.sh` 新增后显示当前 build 已是 v27，但系统目录、设置启动器和运行进程全部仍是 v13。
- 这解释了为什么 v26 已修复的按钮布局和 marked text 行为，在菜单栏真实输入链路中仍不会生效。

修复：

- 新增 `status.sh`：
  - 打印 build Host 版本/CDHash。
  - 打印 `/Library/Input Methods/InputiaInputMethod.app` 版本/CDHash，以及 `systemMatchesBuild`。
  - 打印 `/Applications/Inputia 设置.app` 版本，以及 `systemSettingsMatchesBuildVersion`。
  - 打印当前运行的 `InputiaInputMethod` 进程路径、版本/CDHash，以及 `runningMatchesBuild`。
  - 打印 `dist/InputiaInputMethod-latest.pkg` 的 sha256 和大小。
- `SettingsLauncher/main.swift`：
  - 设置启动器只打开与自身 `CFBundleVersion` 相同的 Host。
  - 如果只找到旧 Host，弹窗明确列出启动器版本、候选 Host 版本和路径，不再静默打开旧设置页。
- `open-settings.sh`：
  - 跳过和当前 build 版本不一致的旧 `/Applications/Inputia 设置.app` 或旧 Host。
  - 本地调试时回退到当前 build 的 `--open-settings`，避免误开旧 UI。
- `README.md`：
  - 增加 `status.sh` 使用说明。
  - 移除旧的固定 v7 包说明，改为以 `build-pkg.sh` / `status.sh` 输出为准。
- Host/设置启动器版本升到 v27。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

zsh -n macos/InputiaInputMethod/status.sh macos/InputiaInputMethod/open-settings.sh \
  macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  build Host CFBundleVersion=27
  build Settings CFBundleVersion=27

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  replacementRangeLocationIsNSNotFound=true
  rawComposingFallbackCandidate=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed
  bundled_full_pinyin_promotes_spelling_corrections_when_available passed
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v27-08c49ce7173c.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=f18fb56b810f22e669927cc6ccbe9f356e71d089b355158e171fa31f891d891a
appCDHash=08c49ce7173c00710c02bf8c98ca1ebadc2f97de
pkgScriptAppVersion=27
pkgScriptSettingsVersion=27
pkgScriptSchemaCount=30
postinstallDryRunVersion=27
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=27
  buildCDHash=08c49ce7173c00710c02bf8c98ca1ebadc2f97de
  system host version=13
  system host cdhash=db6a282f35b2bf46dcea4a6df750a4c3c184f374
  systemMatchesBuild=false
  system settings launcher version=13
  systemSettingsMatchesBuildVersion=false
  running host pid=29694
  runningVersion=13
  runningMatchesBuild=false
  latest package sha256=f18fb56b810f22e669927cc6ccbe9f356e71d089b355158e171fa31f891d891a
```

结论：v27 build/package 已验证；真实菜单栏仍运行 v13。下一步必须完成管理员安装 v27，然后用 `status.sh` 确认 `systemMatchesBuild=true`、`runningMatchesBuild=true` 后，再做 Safari/TextEdit/其他 App 的真实输入 smoke。

## 2026-07-07 v28：`inputText` 路径补齐 Return/Space special key

用户现象继续聚焦在：

- 中文组合态中想直接输入英文/raw 拼写时，按 Return 有时不提交。
- 候选/组合态在不同 App 里表现不稳定。

依据：

- Apple `IMKServerInput` 文档说明输入法接收 text event 有多条入口；除了 `handleEvent:client:`，还存在 `inputText:client:` / `inputText:key:modifiers:client:` 一类 text-data 路线。
- 旧实现已经覆盖 `handle(_ event:)` 的 `keyCodeReturn` / `keyCodeKeypadEnter` / `keyCodeSpace`，但 `inputText(_ string:)` 只把所有 character 当普通文本送进 `bridge.handle(character:)`。
- 因此如果某些 client 把 Return 作为 `"\r"` / `"\n"` 通过 `inputText` 送入，Host 不会走 `bridge.enter()`，就可能复现“按回车不提交 raw composing”的现象。

修复：

- `InputiaShortcutClassifier.swift`：
  - 新增 `isInputTextEnter(_:)`，识别 `"\r"` 与 `"\n"`。
  - 新增 `shouldHandleInputTextSpace(_:hasComposing:)`，仅当当前存在 composition 时把 `inputText(" ")` 映射为 special Space；无 composition 时仍交给普通输入/系统默认路径。
- `InputiaInputController.inputText`：
  - `"\r"` / `"\n"` 走 `bridge.enter()`。
  - 有 composition 时的 `" "` 走 `bridge.space()`，确保 text-data 路线也能上屏首候选/raw composition。
  - 其它字符保持原来的 `bridge.handle(character:)`。
- `InputiaShortcutSelfCheck.swift` 和 Host 内置 `--host-shortcut-self-check` 同步增加覆盖。
- Host/设置启动器版本升到 v28。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  inputTextCarriageReturnIsEnter=true
  inputTextLineFeedIsEnter=true
  inputTextLetterIsNotEnter=true
  inputTextSpaceHandledWhenComposing=true
  inputTextSpacePassesThroughWithoutComposing=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  replacementRangeLocationIsNSNotFound=true
  rawComposingFallbackCandidate=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed
  includes enter_commits_raw_composition_in_chinese_mode
  includes space_commits_first_candidate
  includes space_commits_raw_composition_when_engine_has_no_candidates

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed
  includes capi_enter_commits_raw_composition
  includes capi_paginates_across_rime_candidate_pages

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed
  bundled_full_pinyin_promotes_spelling_corrections_when_available passed
  bundled_rime_schemas_commit_zhongguo_when_available passed
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v28-4b7183e7a811.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=9f0c6525728e13a092600f126cfc519de77b3ce04eb9b56a5de75749f4c3a6fc
appCDHash=4b7183e7a8115e67161117cdd699db753e09abe2
pkgScriptAppVersion=28
pkgScriptSettingsVersion=28
pkgScriptSchemaCount=30
postinstallDryRunVersion=28
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=28
  buildCDHash=4b7183e7a8115e67161117cdd699db753e09abe2
  system host version=13
  systemMatchesBuild=false
  system settings launcher version=13
  systemSettingsMatchesBuildVersion=false
  runningVersion=13
  runningMatchesBuild=false
```

结论：v28 已补齐 `inputText` text-data 路线的 Return/Space 处理，并离线验证；真实菜单栏仍运行 v13，安装 v28 前不能用当前系统输入表现判断该修复是否生效。

## 2026-07-07 v29：抽出 `inputText` 路由并做真实 Bridge 序列自检

继续推进原因：

- v28 已修复 `inputText` 的 Return/Space 路线，但最初只用 classifier 层自检覆盖。
- 为减少安装后才发现不同 App text-data 路线仍有偏差的风险，本轮把 Host 实际使用的 `inputText` 路由抽成可复用策略，并用真实 Rust bridge 跑完整序列。

修复：

- 新增 `InputiaInputTextRouter.swift`：
  - `"\r"` / `"\n"` -> `.enter`
  - 有 composition 时 `" "` -> `.space`
  - 无 composition 的 `" "` 与普通字母 -> `.character`
- `InputiaInputController.inputText` 改为调用同一个 `InputiaInputTextRouter.action(...)`，避免 Host 实现和测试逻辑分叉。
- 新增 `InputiaInputTextRouterSelfCheck.swift`，并接入 `build.sh`：
  - 用真实 `InputiaRustBridge` 跑 `ni\r`，验证提交 raw `ni`。
  - 用真实 `InputiaRustBridge` 跑 `ni\n`，验证提交 raw `ni`。
  - 用真实 `InputiaRustBridge` 跑 `ni `，验证提交候选 `你`。
  - 用真实 `InputiaRustBridge` 跑单独空格，验证无 composition 时不消费。
- Host/设置启动器版本升到 v29。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/build-pkg.sh \
  macos/InputiaInputMethod/status.sh macos/InputiaInputMethod/open-settings.sh \
  macos/InputiaInputMethod/Packaging/scripts/postinstall
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  inputTextCarriageReturnIsEnter=true
  inputTextLineFeedIsEnter=true
  inputTextSpaceHandledWhenComposing=true
  inputTextSpacePassesThroughWithoutComposing=true

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  carriageReturnCommitsRaw=true
  lineFeedCommitsRaw=true
  composingSpaceCommitsCandidate=true
  plainSpacePassesThrough=true
  routeEnterAction=true
  routeSpaceAction=true
  routePlainSpaceAction=true
  routeLetterAction=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  replacementRangeLocationIsNSNotFound=true
  rawComposingFallbackCandidate=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v29-af9f2b7c294d.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=0841ef86dcbf619b9f5c0414f44f6a67a3e5f9011db9d5100697cb591ae91723
appCDHash=af9f2b7c294d03d24f8e947dcbc1ecc33aa1afbf
pkgScriptAppVersion=29
pkgScriptSettingsVersion=29
pkgScriptSchemaCount=30
postinstallDryRunVersion=29
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=29
  buildCDHash=af9f2b7c294d03d24f8e947dcbc1ecc33aa1afbf
  system host version=13
  systemMatchesBuild=false
  system settings launcher version=13
  systemSettingsMatchesBuildVersion=false
  runningVersion=13
  runningMatchesBuild=false
```

结论：v29 已把 text-data 路由抽成单一策略，并用真实 Bridge 序列验证 Return/Space 行为；真实菜单栏仍运行 v13，必须安装 v29 后才能做 Safari/TextEdit 真实 smoke。

## 2026-07-07 v30：输入法菜单接入 Handy 本地语音输入入口

目标：

- 第一阶段目标包含“语音输入”与“统一输入系统”。此前已有 Handy 语音历史导入、语音热词和本地记忆排序，但输入法菜单本身还不能直接触发语音输入。

依据：

- Handy README 已公开本地 CLI 远程控制入口：
  - `/Applications/Handy.app/Contents/MacOS/Handy --toggle-transcription`
  - `handy --toggle-transcription`
- Handy Tauri single-instance 代码会把 `--toggle-transcription` 转发为 `signal_handle::send_transcription_input(app, "transcribe", "CLI")`。
- Apple `NSRunningApplication.runningApplications(withBundleIdentifier:)` 用于按 bundle id 判断 App 是否已运行。
- Apple `NSWorkspace.OpenConfiguration.arguments` 用于给启动的 App 传递 launch arguments。

修复：

- 新增 `InputiaVoiceInputLauncher.swift`：
  - 查找 Handy app：`INPUTIA_HANDY_APP`、LaunchServices bundle id `com.pais.handy`、`/Applications/Handy.app`、`~/Applications/Handy.app`。
  - 如果 Handy 已运行：直接执行 `Handy --toggle-transcription`，由 single-instance 转发给运行中的 Handy。
  - 如果 Handy 未运行：用 `NSWorkspace` 以 `--start-hidden` 隐藏启动 Handy，再延迟执行 `--toggle-transcription`。
  - 如果找不到 Handy：返回 `.missing`，由 Host 弹出明确提示。
- `InputiaInputController.menu()`：
  - 新增菜单项 `语音输入`。
  - 保留 `召回剪贴板` 和 `Inputia 设置...`。
- 新增 `InputiaVoiceInputLauncherSelfCheck.swift`，并接入 `build.sh`：
  - 验证候选 Handy app 路径优先级。
  - 验证运行中 Handy 走立即 toggle。
  - 验证冷启动 Handy 走 `--start-hidden` + 延迟 `--toggle-transcription`。
- Host/设置启动器版本升到 v30。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-voice-input-launcher-self-check
  voiceInputLauncherSelfCheck=true
  candidatesAreOrdered=true
  findsEnvApp=true
  missingWhenExecutableAbsent=true
  runningPlanTogglesImmediately=true
  coldPlanStartsHiddenThenToggles=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v30-1df40868b997.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=73e9f5728701dfe447b0fb0018aff1340576c735e80724d62c1274f02ec8256b
appCDHash=1df40868b9974430edf16f4b6834d5547da6ef9d
pkgScriptAppVersion=30
pkgScriptSettingsVersion=30
pkgScriptSchemaCount=30
postinstallDryRunVersion=30
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=30
  buildCDHash=1df40868b9974430edf16f4b6834d5547da6ef9d
  system host version=13
  systemMatchesBuild=false
  system settings launcher version=13
  systemSettingsMatchesBuildVersion=false
  runningVersion=13
  runningMatchesBuild=false
```

结论：v30 已把 Inputia 输入法菜单接入 Handy 本地语音输入触发入口；真实菜单栏仍运行 v13，必须安装 v30 后才能验证菜单项可见和真实语音触发。

## 2026-07-07 v31：修复设置页按钮压缩、raw 回车提交和空候选降级显示

用户现象：

- 设置页右下角按钮会被路径挤压成很窄的蓝色竖条。
- 中文输入组合中想输入英文时，按 Return 有时没有把 raw 拼写提交进输入框，后续继续输入会让原组合消失。
- 全拼和多个双拼方案偶发“不出候选”，尤其无候选时只剩下划线，交互不清楚。

根因与判断：

- 当前系统实际运行的仍是 `/Library/Input Methods/InputiaInputMethod.app` v13；源码和包的 v31 修复必须安装后才会出现在菜单栏和真实输入里。
- Apple IMKTextInput 文档说明 marked text 是客户端维护的 inline composition，`insertText` 是 fully converted text 提交通道，`setMarkedText` 是活动组合通道；不同 App 对 IMKTextInput 支持程度不同，因此 Host 侧必须在提交前同步自身状态，并在空候选时提供 raw fallback。
- 鼠须管 `SquirrelInputController` 的可行模式也是 commit 只走 `client.insertText(_, replacementRange: .empty)`，活动组合走 `client.setMarkedText(_, selectionRange:, replacementRange: .empty)`；Inputia 保持同类策略。
- RimeData 不是只支持全拼和小鹤。v31 smoke 覆盖 `luna_pinyin_simp`、自然码、小鹤、搜狗、国标、微软、智能 ABC、拼音加加、四通，均能返回“中国”候选并空格上屏。

修复：

- `InputiaSettingsWindow.swift`
  - footer 改为两行布局：配置路径独占一行，按钮行另起一行。
  - `保存设置`、`打开配置文件夹` 改为最小宽度约束，避免窗口或长路径把按钮压瘦。
- `main.swift`
  - `candidates(_:)` 统一走 `InputiaHostTextPolicy.candidatesForPanel`，无引擎候选但仍有 composition 时返回 raw composition fallback。
  - `candidateSelected(_:)` 支持选择 raw fallback，走 `bridge.enter()` 提交英文 raw 拼写。
  - `apply(_:)` 在 commit 前先同步 Host 的 `latestComposing/latestCandidates` 状态，再隐藏候选窗并 `insertText`，降低不同 App 查询状态时读到旧 composition 的概率。
- `InputiaHostTextPolicySelfCheck.swift`
  - 增加 raw fallback 可被选择的自检。
- v31 同时包含 v30 后续补丁：
  - 输入法菜单新增 `同步语音/剪贴板记忆`。
  - 设置页复用 `InputiaHandyMemorySync` 导入 Handy 语音历史和剪贴板历史。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

/bin/zsh macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk

macos/InputiaInputMethod/build/inputia-handy-memory-sync-self-check
  handyMemorySyncSelfCheck=true

macos/InputiaInputMethod/build/inputia-voice-input-launcher-self-check
  voiceInputLauncherSelfCheck=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  carriageReturnCommitsRaw=true
  lineFeedCommitsRaw=true
  composingSpaceCommitsCandidate=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  rawComposingFallbackCandidate=true
  rawFallbackCanBeSelected=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed

INPUTIA_RIME_SHARED_DATA_DIR=macos/InputiaInputMethod/build/RimeData cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed

manual schema matrix with core_flow_probe
  luna_pinyin_simp zhongguo -> 中国
  double_pinyin vsgo -> 中国
  double_pinyin_flypy vsgo -> 中国
  double_pinyin_sogou vsgo -> 中国
  guobiao_bispell vsgo -> 中国
  double_pinyin_mspy vsgo -> 中国
  double_pinyin_abc asgo -> 中国
  double_pinyin_pyjj vygo -> 中国
  double_pinyin_st aygo -> 中国
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v31-7ab08f1c5ea1.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=31b1604073a2d718c5f37a9109a594d9923e697fca03307c724129a58728b499
appCDHash=7ab08f1c5ea1442cd550004974ebd176777e4697
pkgScriptAppVersion=31
pkgScriptSettingsVersion=31
pkgScriptSchemaCount=30
postinstallDryRunVersion=31
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=31
  buildCDHash=7ab08f1c5ea1442cd550004974ebd176777e4697
  system host version=13
  systemMatchesBuild=false
  system settings launcher version=13
  systemSettingsMatchesBuildVersion=false
  runningVersion=13
  runningMatchesBuild=false
```

结论：v31 包已修复本轮源码问题并通过离线验证；真实菜单栏仍运行 v13，必须完成 v31 安装并重启/重载 InputiaInputMethod 后，才能验证 Safari/TextEdit 真实输入行为。

## 2026-07-07 v34 输入热路径、回车透传和全局快捷键修复

用户实测反馈：

- 打字时有卡顿，输入反应慢。
- 在账号/密码网页中输入完后按 Return，网页没有执行默认登录动作。
- `Command + ,` 在任何软件下都会打开 Inputia 设置，说明输入法 Host 抢占了普通 App 快捷键。

修复策略：

- 移除输入法菜单中 `Inputia 设置...` 的 `Command + ,` keyEquivalent；设置入口保留在输入法菜单和独立 `/Applications/Inputia 设置.app`，但不再注册全局菜单快捷键。
- `keyDown` 和 `inputText` 两条入口都在“无组合串”时透传 Return / Keypad Enter；只有存在中文组合串时，Return 才提交原始拼写。
- `inputText` 无组合串空格也直接透传，减少普通空格输入时的 Host/C API 往返。
- `insertText` 提交时使用当前 `markedRange()` 替换 marked text；默认 replacement range 改成 `{NSNotFound, NSNotFound}`，避免回车提交后 marked text 下划线残留并覆盖后续输入。
- Host 每键热路径增加缓存：
  - settings reload 检查节流到约 0.5s；
  - app/window context 扫描节流到约 0.75s；
  - app context 只有变化时才推给 Rust core；
  - sensitive app/window 判断按 context 缓存。
- 敏感窗口关键词补充登录场景：`登录`、`登陆`、`登入`、`账号`、`账户`、`验证码`、`身份验证`、`login`、`sign in`、`authentication`、`otp`、`2fa` 等。
- Rime 候选每次按键不再预扫 6 页，降为保留首屏 + 一次翻页所需的 2 页，减少每个字母触发的 librime session/evaluate 次数。

关键验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

zsh -n build.sh build-pkg.sh status.sh open-settings.sh verify-system.sh
  OK

./macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  codesign verify OK

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  replacementRangeLocationIsNSNotFound=true
  replacementRangeLengthIsNSNotFound=true
  commitUsesMarkedRangeWhenComposing=true

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  carriageReturnCommitsRaw=true
  lineFeedCommitsRaw=true
  routeEnterPassesThroughWithoutComposing=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-handy-memory-sync-self-check
  handyMemorySyncSelfCheck=true

macos/InputiaInputMethod/build/inputia-voice-input-launcher-self-check
  voiceInputLauncherSelfCheck=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml
  17 passed

INPUTIA_RIME_SHARED_DATA_DIR=macos/InputiaInputMethod/build/RimeData cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  2 passed
```

性能 smoke：

```text
before: core_flow_probe luna_pinyin_simp zhongguo 2
  real ~= 3.12s, includes process startup and librime initialization

after v34 cold compile/run:
  real ~= 2.02s

after v34 warm run:
  real ~= 1.44s
  after_input candidate[0]=中国
  after_page_down candidate[0]=种果
  commit=中国
```

最终包：

```text
pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v34-eb0b688bca00.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=411cc6f7840f4c708601a8eb77f97b51eda8f306d8ec31ac3c63d18b1eae1201
appCDHash=eb0b688bca000354aaebb6f9b3d75cd44e1dc307
pkgScriptHostVersion=34
pkgScriptSettingsVersion=34
pkgScriptSchemaCount=21
postinstallDryRunHostVersion=34
postinstallDryRunSettingsVersion=34
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=34
  buildCDHash=eb0b688bca000354aaebb6f9b3d75cd44e1dc307
  system host version=31
  systemMatchesBuild=false
  system settings launcher version=31
  systemSettingsMatchesBuildVersion=false
  runningVersion=31
  runningMatchesBuild=false
```

结论：v34 包已经离线验证通过；真实菜单栏仍在运行 v31，需要安装 v34 并重新加载 InputiaInputMethod 后，再验证 Safari/TextEdit 中空组合 Return 透传、`Command + ,` 不再打开 Inputia 设置、中文组合 Return 仍提交原始拼写、候选和翻页仍正常。

## 2026-07-07 v36 Rime 生命周期和 `mlle` raw fallback 修复

用户实测反馈：

- 中文输入偶发不出候选，只显示 raw 字母，例如候选窗只有 `1 mlle`。
- 用户以为当前为“国标双拼”，但本机实际 settings 为 `schema_id=double_pinyin`。
- 需要确认这不是拼音表缺词，也不是候选窗绘制问题。

关键事实：

- 当前系统 Host 已安装并运行 v35/v36 路径，设置文件实际为 `double_pinyin`。
- Rime probe 显示 schema 本身有候选：
  - `double_pinyin + mlle`：第一候选 `买了`。
  - `guobiao_bispell + mlle`：候选包含 `买了`，但第一候选为 `迷路了`。
  - `guobiao_bispell + mkle`：第一候选 `买了`。
- `guobiao_bispell.schema.yaml` 来自 `baopaau/rime-guobiao-quick`，README 标明是 GB/T 34947-2017 方案；该 schema 中 `ai` 映射到 `K`，所以国标方案下 `买了` 是 `mkle`，不是 `mlle`。
- 鼠须管源码对照：
  - `SquirrelApplicationDelegate.deploy()` 先 `shutdownRime()`，再 `startRime()`。
  - `shutdownRime()` 调用 `rimeAPI.finalize()`。
  - controller 只 `create_session()` / `destroy_session()`，不在单个 controller 生命周期里随意 initialize/finalize 全局 librime。

复现到的根因：

```text
cargo test --manifest-path crates/inputia-capi/Cargo.toml capi_new_settings_session_survives_previous_session_free -- --nocapture
  before fix: failed
  assertion left: Null, right: "买了"
```

也就是：旧代码在设置热重载时先创建新 Rime session，再释放旧 session；旧 session drop 会触发 librime 全局 `finalize()`，把刚初始化的新 runtime 也收掉，之后 Host 就可能只有 composing/raw fallback，没有正常 Rime 候选。

修复：

- `inputia-rime` 增加进程内 librime runtime 引用计数：相同 Rime data/user 路径的多个 `RimeEngine` 可同时存在，只有最后一个释放时才 `finalize()`。
- 如果已有活跃 runtime 但路径不同，拒绝重叠初始化，避免静默混用不同 RimeData。
- Swift `InputiaRustBridge.reloadSettings` 改为先释放旧 session，再打开新 settings session，和鼠须管的全局生命周期边界一致。
- 设置窗口中输入方案、快捷键、标点/全角/纠错/记忆/隐私复选项改为变更即保存，保留“保存设置”按钮，减少“界面看似改了但 settings.json 未变”的状态错觉。
- schema smoke 增加 `mlle/mkle` 用例，防止 raw 字母候选回归。

验证：

```text
plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  OK

zsh -n build.sh build-pkg.sh install-system.sh status.sh
  OK

INPUTIA_RIME_SHARED_DATA_DIR=/Library/Input\ Methods/InputiaInputMethod.app/Contents/Resources/RimeData \
  cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
  18 passed
  capi_new_settings_session_survives_previous_session_free ... ok

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

INPUTIA_RIME_SHARED_DATA_DIR=/Library/Input\ Methods/InputiaInputMethod.app/Contents/Resources/RimeData \
  cargo test --manifest-path crates/inputia-rime/Cargo.toml -- --nocapture
  7 passed

INPUTIA_RIME_SHARED_DATA_DIR=/Library/Input\ Methods/InputiaInputMethod.app/Contents/Resources/RimeData \
  cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  3 passed
  bundled_double_pinyin_schemas_expose_maile_candidates_when_available ... ok

./macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  codesign verify OK

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-bridge-privacy-self-check
  bridgePrivacySelfCheck=true

/Library/Input\ Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-settings-reload-self-check
  bridgeSettingsReloadSelfCheck=true
```

Rime probe：

```text
double_pinyin + mlle
  candidate[0]=买了

guobiao_bispell + mlle
  candidate[0]=迷路了
  candidate[1]=买了

guobiao_bispell + mkle
  candidate[0]=买了
```

真实 IMK 验证：

```text
TextEdit: mlle + Space
  result=买了

TextEdit smoke after install
  defaultChineseResult=你
  enterRawResult=ni
  keypadEnterRawResult=ni
  digitCandidateResult=妳
  pageCandidateResult=呢
  shiftEnglishResult=ni
  shiftChineseResult=你
  textEditSmokePassed=true

Safari input source context
  safariKeptInputia=true
  safariAcceptsFocusedInputiaSelection=true

Safari typing smoke
  safariTypingResult=你
  safariTypingSmokePassed=true
```

v36 安装状态：

```text
status.sh
  buildVersion=36
  buildCDHash=383b01bd0acdcb6ec34c7d6481e42925b9fddc03
  system host version=36
  systemMatchesBuild=true
  system settings launcher version=36
  systemSettingsMatchesBuildVersion=true
  runningVersion=36
  runningMatchesBuild=true

pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v36-383b01bd0acd.pkg
latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
sha256=fefffafa753c012df0482fb812157665569559d9183af8a389bc7fa522dd7a09
appCDHash=383b01bd0acdcb6ec34c7d6481e42925b9fddc03
postinstallDryRunHostVersion=36
postinstallDryRunSettingsVersion=36
postinstallDryRunGuobiaoSchemaPresent=true
```

已知非本轮目标缺口：

```text
post-install clipboard recall smoke
  clipboardRecallSmokePassed=false reason=result-mismatch
```

该缺口和本轮 `mlle` 候选 raw fallback 根因不同，后续应单独按剪贴板召回路径排查。

## 2026-07-07 v36 剪贴板召回 smoke 脚本修复

现象：

```text
post-install clipboard recall smoke
  clipboardRecallSmokePassed=false reason=result-mismatch
```

根因定位：

- 产品侧 Host 已能接收 Ctrl+Shift+V 并显示剪贴板候选；失败点在 smoke 脚本。
- 旧脚本先选择 Inputia，再 `launchctl setenv INPUTIA_DEBUG_EVENTS`，如果 Host 已经运行，调试环境不会进入现有进程，导致日志不可用。
- 旧 AppleScript 逐个 `key down control` / `key down shift` / `key code 9`，在一次复现里事件实际落到 Chrome，且 modifiers 显示成 Command，不是目标的 Control+Shift。
- 旧脚本没有确认 TextEdit 已经成为 frontmost，UI 自动化可能把按键发给用户当前前台 App。

修复：

- `smoke-clipboard-recall.sh` 在选择输入源前设置剪贴板内容和 `INPUTIA_DEBUG_EVENTS`。
- 默认 `INPUTIA_RESTART_HOST_FOR_DEBUG=1`，杀掉旧 Host 后让 macOS 重新拉起带调试环境的新 Host。
- AppleScript 明确等待 TextEdit 成为 frontmost；否则直接报错并输出实际前台 App。
- 快捷键改为 `key code 9 using {control down, shift down}`，避免逐个 modifier down/up 的不稳定事件序列。
- 保留 CDHash 检查，避免误测旧安装包。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh
  OK

INPUTIA_CLIPBOARD_SMOKE_TEXT='Inputia Clipboard Smoke Fixed Script' \
  ./macos/InputiaInputMethod/smoke-clipboard-recall.sh /Library/Input\ Methods/InputiaInputMethod.app
  expectedCDHash=383b01bd0acdcb6ec34c7d6481e42925b9fddc03
  actualCDHash=383b01bd0acdcb6ec34c7d6481e42925b9fddc03
  clipboardRecallExpected=Inputia Clipboard Smoke Fixed Script
  clipboardRecallResult=Inputia Clipboard Smoke Fixed Script
  clipboardRecallSmokePassed=true
```

真实 Host 事件证据：

```text
context bundle=com.apple.TextEdit
flagsChanged current=262144
flagsChanged current=393216
clipboardRecallShown count=7
keyDown keyCode=49 modifiers=0 chars=
clipboardRecallCommit index=0 text=Inputia Clipboard Smoke 1783417349
```

安装后完整回归：

```text
./macos/InputiaInputMethod/post-install-regression.sh /Library/Input\ Methods/InputiaInputMethod.app
  textEditSmokePassed=true
  safariKeptInputia=true
  safariAcceptsFocusedInputiaSelection=true
  safariTypingSmokePassed=true
  clipboardRecallSmokePassed=true
  postInstallRegressionPassed=true
```

结论：

- v36 Host 二进制不需要因为该问题重新安装；本轮修的是仓库内回归脚本。
- 剪贴板召回的真实 IMK 路径已验证：Ctrl+Shift+V 打开召回候选，Space 选择首项并上屏。
- 后续若该 smoke 再失败，优先看 `frontmost`、CDHash、`INPUTIA_DEBUG_EVENTS` 继承和事件日志，不再直接归因到剪贴板召回逻辑。

## 2026-07-07 v37 Safari raw ASCII Return 透传修复

用户现象：

- 在网页登录/表单里使用 Inputia 时，输入账号密码后按 Return，网页没有提交。
- 体感上像是 Inputia 把 Return 或其它 App 快捷键“接管太多”。

依据：

- Apple InputMethodKit 文档：`handleEvent` / command selector 返回 `YES` 代表输入法处理并消费事件，返回 `NO` 才交还给 client app；没有 composition 时 Return 应该放行。
- 鼠须管 `SquirrelInputController.handle(_:client:)` 采用同样边界：Command 快捷键直接 `return false`，普通 key 只有 Rime `process_key` 处理成功时才消费。

最小复现：

```text
Safari data: form
  Inputia 中文模式输入 abc
  press Return

v36 result:
  safariEnterTitle=VALUE:abc
  safariEnterSubmitted=false

v36 event log:
  keyDown keyCode=0 chars=a
  apply ok=true consumed=true mode=Chinese composing=a commit= candidates=啊,爱,唉
  keyDown keyCode=11 chars=b
  apply ok=true consumed=true mode=Chinese composing=ab commit= candidates=
  keyDown keyCode=8 chars=c
  apply ok=true consumed=true mode=Chinese composing=abc commit= candidates=
  keyDown keyCode=36 chars=\r
  apply ok=true consumed=true mode=Chinese composing= commit=abc candidates=
```

根因：

- `abc` 在中文模式下进入 Rime composition。
- 当 composition 是纯 ASCII raw fallback 且没有候选词时，Return 只负责提交 `abc`，Host 返回 `true` 消费了 Return。
- 对网页表单来说，这意味着文本已经进输入框，但同一个 Return 没有传回页面触发表单提交。

修复：

- 新增 `InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit`。
- 条件非常窄：`previousComposing` 非空、候选列表为空、commit 等于原始 composing、且 composing 是可打印 ASCII。
- `inputText`、`didCommand(by:)`、`handleKeyDown` 三条 Return 入口共用该策略。
- 满足条件时，Host 先插入 raw ASCII，再返回 `false` 让宿主 App 继续处理 Return。
- 有中文候选词、非 ASCII composition、commit 发生变化的正常中文上屏路径仍然返回原来的 handled 状态。

验证：

```text
./macos/InputiaInputMethod/build.sh
  build/InputiaInputMethod.app valid on disk
  build/Inputia 设置.app valid on disk
  codesign verify OK

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  asciiRawFallbackNewlinePassesThroughAfterCommit=true
  candidateCommitNewlineDoesNotPassThrough=true
  nonAsciiRawFallbackNewlineDoesNotPassThrough=true
  changedCommitNewlineDoesNotPassThrough=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true

INPUTIA_RIME_SHARED_DATA_DIR=/Library/Input\ Methods/InputiaInputMethod.app/Contents/Resources/RimeData \
  cargo test --manifest-path crates/inputia-capi/Cargo.toml capi_new_settings_session_survives_previous_session_free -- --nocapture
  1 passed

INPUTIA_RIME_SHARED_DATA_DIR=/Library/Input\ Methods/InputiaInputMethod.app/Contents/Resources/RimeData \
  cargo test --manifest-path crates/inputia-rime/Cargo.toml --test schema_smoke -- --nocapture
  3 passed
```

安装状态：

```text
install-system.sh
  sourceVersion=37
  sourceCDHash=7ff224fc97bc72856482624e32d9f11366d020d0
  destVersion=37
  destCDHash=7ff224fc97bc72856482624e32d9f11366d020d0
  systemInstallVerified=true
  enabled=true
  selectable=true
  selected=true

status.sh
  buildVersion=37
  system host version=37
  systemMatchesBuild=true
  runningVersion=37
  runningMatchesBuild=true
  system settings launcher version=37
  systemSettingsMatchesBuildVersion=true
```

真实输入 smoke：

```text
smoke-safari-enter.sh
  expectedCDHash=7ff224fc97bc72856482624e32d9f11366d020d0
  actualCDHash=7ff224fc97bc72856482624e32d9f11366d020d0
  safariEnterTitle=SUBMITTED:abc
  safariEnterExpected=SUBMITTED:abc
  safariEnterSmokePassed=true

smoke-textedit.sh
  defaultChineseResult=你
  enterRawResult=ni
  keypadEnterRawResult=ni
  digitCandidateResult=尼
  pageCandidateResult=呢
  shiftEnglishResult=ni
  shiftChineseResult=你
  textEditSmokePassed=true

diagnose-safari-input-source.sh
  safariKeptInputia=true
  safariAcceptsFocusedInputiaSelection=true

smoke-safari-typing.sh
  safariTypingResult=你
  safariTypingSmokePassed=true

smoke-clipboard-recall.sh
  clipboardRecallSmokePassed=true
```

打包：

```text
build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v37-7ff224fc97bc.pkg
  latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  sha256=734a93adf6543d01f6a536ccfc0843c47f48479056957f6753601f89d05b9e1b
  appCDHash=7ff224fc97bc72856482624e32d9f11366d020d0
```

验证脚本修正：

- `verify-system.sh` 原本复制 `/Library/Input Methods/InputiaInputMethod.app` 到临时 `.bundle`，v37 环境下该副本会卡住；复制为临时 `.app` 又会被系统 `Killed: 9`。
- 修正为直接验证真实安装路径的 executable，这才符合系统输入法验收目标。
- `lsregister -dump` 在当前机器上可能很慢，改为默认跳过；需要 LaunchServices 诊断时显式设置 `INPUTIA_VERIFY_LAUNCHSERVICES=1`。
- `post-install-regression.sh` 新增 `smoke-safari-enter.sh`，但当前 Codex shell 会偶发让外层 zsh/bash 包装器在第一行前被系统杀掉；因此本轮验收以分段 smoke 的当前输出为准。

结论：

- v37 已解决 Safari/网页表单中“纯英文 raw fallback 输入后第一个 Return 只上屏、不提交”的问题。
- 该修复不改变有候选词的中文 Return 上屏行为，也没有破坏 TextEdit、Safari 中文输入、Shift 中英切换、剪贴板召回。

## 2026-07-07 21:04 CST - v38 候选数量 7 + 下箭头展开/翻页

问题：

- 用户反馈候选词只有 5 个太少，希望默认显示 7 个。
- 中文组词时按下箭头没有展开更多候选，方向键事件漏回前台文本框，导致光标移动。

依据：

- Apple InputMethodKit `handleEvent:client:` / `didCommandBySelector:client:` 返回 handled 时事件由输入法处理；返回 not handled 会继续交给 client。
- 当前 Host 只处理 `PageDown/PageUp`，没有处理方向键或 `moveDown:` / `moveUp:` selector。
- 当前 Core 默认 `candidate_page_size=5`，用户本机 `~/Library/Application Support/Inputia/settings.json` 也保存了 `candidate_page_size=5`，因此只改默认值不会影响现有安装。

变更：

- `inputia-core` 默认 `candidate_page_size` 从 5 改为 7，并扩展分页单测桩，确认第一页 7 个候选后仍可翻到第二页。
- `inputia-settings` 与 macOS Settings 默认候选数同步改为 7。
- macOS Host 加入候选导航分类：
  - 有拼音组合串时，无修饰键的下箭头先展开候选窗为多行；展开后再次下箭头翻到下一页。
  - 有拼音组合串时，上箭头翻回上一页；在第一页且已展开时折叠候选窗。
  - 没有组合串，或带 Command/Control/Option/Shift 时，不接管方向键。
- 自绘候选窗支持 compact / expanded 两种布局，expanded 使用多行显示，避免单行截断。
- 本机当前配置更新为：

```text
~/Library/Application Support/Inputia/settings.json
  candidate_page_size=7
  input_mode_toggle_shortcut=shift
  schema_id=double_pinyin
```

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-settings/Cargo.toml
  4 passed

build.sh
  build/InputiaInputMethod.app Info.plist OK
  build/Inputia 设置.app Info.plist OK
  codesign verify OK
```

快捷键自检：

```text
inputia-shortcut-self-check
  shortcutSelfCheck=true
  candidateDownArrowExpandsWhenComposing=true
  candidateUpArrowPagesWhenComposing=true
  candidateDownArrowRejectedWithoutComposition=true
  candidateDownArrowRejectedWithCommand=true

InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  candidateDownArrowExpandsWhenComposing=true
  candidateUpArrowPagesWhenComposing=true
  candidateDownArrowRejectedWithoutComposition=true
  candidateDownArrowRejectedWithCommand=true
```

系统安装状态：

```text
install-system.sh
  sourceVersion=38
  sourceCDHash=c81d20a4935b022d86d486102a36047df8c8bfa7
  destVersion=38
  destCDHash=c81d20a4935b022d86d486102a36047df8c8bfa7
  systemInstallVerified=true
  enabled=true
  selectable=true
  selected=true

status.sh
  buildVersion=38
  system host version=38
  systemMatchesBuild=true
  runningVersion=38
  runningMatchesBuild=true
  system settings launcher version=38
  systemSettingsMatchesBuildVersion=true
```

真实 TextEdit 下箭头 smoke：

```text
输入 ni，下箭头，下箭头，空格
arrowCandidateResult=腻

debug event log:
  keyDown keyCode=45 chars=n
  apply ... mode=Chinese composing=n candidates=你,呢,那
  keyDown keyCode=34 chars=i
  apply ... mode=Chinese composing=ni candidates=你,尼,妳
  keyDown keyCode=125
  candidatePanelExpanded
  keyDown keyCode=125
  apply ... mode=Chinese composing=ni candidates=腻,逆,倪
  keyDown keyCode=49 chars=space
  apply ... mode=Chinese composing= commit=腻
```

打包：

```text
build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v38-c81d20a4935b.pkg
  latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  sha256=afbbf899fb06719f289d88b135c08567593da6b0cd14c9fa0a049f1bc572e79d
  appCDHash=c81d20a4935b022d86d486102a36047df8c8bfa7
```

测试纪律修正：

- TextEdit / Safari smoke 只能作为焦点受控时的证据；用户正在操作前台时，失败可能来自前台焦点被抢或输入源切走，不能直接推断为产品失败。
- TextEdit smoke 后必须关闭本轮创建的未命名测试文档，并尽量恢复原前台 App。
- 本轮已清理测试遗留 TextEdit 文档：关闭了 clipboard smoke、空文档、`你`、`腻` 等测试内容文档，不保存。
- `smoke-textedit.sh` 增加 shell 级 TextEdit preflight；默认发现 TextEdit 已运行就拒绝测试，避免碰用户文档。
- `smoke-textedit.sh` / `smoke-clipboard-recall.sh` 增加 EXIT trap；脚本自己启动的 TextEdit 在成功或失败时都执行 `quit saving no`。
- `smoke-textedit.sh` 对前台被抢新增 `focus-lost:<app>` 报错，失败归因到测试环境，不再直接当作输入法行为失败。
- 新版 `smoke-textedit.sh` 已纳入 `arrowCandidateResult`，覆盖下箭头展开/翻页提交路径。
- 清理修复后验证：`pgrep -fl TextEdit` 无输出，表示没有遗留 TextEdit 测试进程。

2026-07-07 21:23 CST 追加修正：

- `smoke-textedit.sh`、`smoke-clipboard-recall.sh`、`smoke-safari-typing.sh`、`smoke-safari-enter.sh` 默认不再打开前台 App。
- 真实 UI smoke 需要显式设置 `INPUTIA_RUN_UI_SMOKE=1`，否则脚本会输出 `reason=ui-smoke-disabled` 并退出。
- `post-install-regression.sh` 默认只跑非 UI 的系统验证；UI smoke 分段在 `INPUTIA_RUN_UI_SMOKE=1` 时才执行。
- 禁用路径验证：

```text
smoke-textedit.sh
  textEditSmokeReady=false reason=ui-smoke-disabled
  exit=14

smoke-clipboard-recall.sh
  clipboardRecallSmokeReady=false reason=ui-smoke-disabled
  exit=7

smoke-safari-typing.sh
  safariTypingSmokeReady=false reason=ui-smoke-disabled
  exit=7

smoke-safari-enter.sh
  safariEnterSmokeReady=false reason=ui-smoke-disabled
  exit=5
```

- 默认回归验证：

```text
post-install-regression.sh
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true
```

- 无残留验证：`pgrep -fl TextEdit` 和 `pgrep -x Safari` 均无输出。

残余观察：

- `smoke-textedit.sh` 完整脚本在本轮自动化中仍有焦点/输入源状态波动，曾在 `digit-select-candidate` 或 `enter-raw-composition` 步骤返回空文本；本轮没有把它作为 v38 成功标准。
- 目标修复的下箭头路径已由真实 Host 事件日志和 TextEdit 上屏结果验证。

2026-07-07 21:38 CST v39 修正：

触发问题：

- 用户确认下箭头展开/选择已可用后，仍报告双拼偶尔退化成裸字母、输入卡顿、回车/快捷键有时像被输入法接管。
- 当前不再继续打开 TextEdit/Safari 做前台 smoke，避免抢用户光标；本轮只做非 UI 证据。

参考与判断：

- Apple InputMethodKit 文档说明 `IMKInputController` 是输入法侧事件与文本输入 controller；`IMKServerInput` 路径要求输入法实现 `handle(_:client:)` 接收 Text Services Manager 转发的 `NSEvent`。这支持当前 Host 只在明确组合态/输入法快捷键里返回 handled，其他组合键放回 client。
- Rime 官方 `rime-double-pinyin` 仓库列出自然码、智能 ABC、小鹤、微软、拼音加加、四通双拼；Inputia 额外打包搜狗和国标 schema，所以这些 schema 必须由本地 smoke 锁住。
- 鼠须管 `SquirrelInputController.swift` 的成熟路径是一 controller 持有一个 librime session，通过 `process_key` 增量推进，再 `get_commit` / `get_context` 更新 UI。Inputia 当前为了保持 Core/Engine Adapter 可替换，仍使用按 composing 重建 Rime evaluation 的适配层；长期性能优化方向应转为持久 session adapter，但本轮先做低风险边界修复。

变更：

- `inputia-capi` 增加 effective spelling correction 规则：`spelling_correction_enabled` 只对全拼 schema 生效，双拼 schema 即使设置文件中保留 true，也不会执行全拼纠错变体查询。
- 设置页将“启用拼音纠错”改为“启用全拼纠错”；选择双拼方案时该选项禁用并保存为 false，减少双拼误导和额外查询。
- Host 设置热重载后会同步清空 Swift 侧组合态与候选窗，避免 Rust/Rime session 已重建但 `latestComposing` 仍是旧值，导致下一次按键/回车用旧组合态解释新会话输出。
- `prepare-rime-data.sh` 增加本地 fallback：GitHub raw 下载失败时，优先复用上一版 build RimeData、已安装 Inputia RimeData 或资源目录中的 schema；只有本地无 schema 时才失败。这符合本地优先输入法的构建要求。
- Bundle 版本升为 v39，避免同为 v38 但 CDHash 不同造成安装状态误判。

验证：

```text
cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
  19 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml -- --nocapture
  8 passed
  bundled_double_pinyin_schemas_expose_maile_candidates_when_available ok
  bundled_rime_schemas_commit_zhongguo_when_available ok
```

双拼候选探针：

```text
double_pinyin mlle page_size=7
  candidate[0]=买了
  commit=买了

guobiao_bispell mlle page_size=7
  candidate[0]=迷路了
  candidate[1]=买了

guobiao_bispell mkle page_size=7
  candidate[0]=买了
  commit=买了

double_pinyin_sogou mlle page_size=7
  candidate[0]=买了
  commit=买了
```

构建、安装、TIS：

```text
build.sh
  Info.plist OK
  codesign verify OK

install-system.sh
  sourceVersion=39
  sourceCDHash=a0f96036ce1f6c3926665d8363560fde933c0a39
  destVersion=39
  destCDHash=a0f96036ce1f6c3926665d8363560fde933c0a39
  systemInstallVerified=true
  enabled=true
  selectable=true
  selected=true

post-install-regression.sh
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true

status.sh
  buildVersion=39
  system host version=39
  systemMatchesBuild=true
  runningVersion=39
  runningCDHash=a0f96036ce1f6c3926665d8363560fde933c0a39
  runningMatchesBuild=true
```

打包：

```text
build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v39-a0f96036ce1f.pkg
  latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  sha256=5edf8bc1fb19133a633b94f3e381da7abf0dc17086813348481d42b1d7127e10
  appCDHash=a0f96036ce1f6c3926665d8363560fde933c0a39
```

无扰验证：

```text
pgrep -fl TextEdit
  no output
pgrep -x Safari
  no output
```

残余风险：

- v39 降低了双拼额外查询和设置热重载 stale state 风险，但还没有把 Rime adapter 改成鼠须管式持久 session；这仍是后续性能优化的主要方向。
- 未运行真实前台 UI smoke；用户可直接试 v39，自动化 UI smoke 仍需显式 `INPUTIA_RUN_UI_SMOKE=1`。

2026-07-07 21:47 CST v40 持久 Rime session：

触发问题：

- 用户反馈输入时反应慢；v39 证据已显示候选正确性基本成立，因此下一步聚焦 Rime 适配层性能，而不是继续调候选窗。

参考与结论：

- 鼠须管 `SquirrelInputController.swift` 使用一个 controller 持有一个 librime session，并通过增量按键推进 session。Inputia v39 的 `RimeEngine::evaluate()` 每次候选查询都会创建 session、选择 schema、模拟整个输入串、销毁 session；这是输入卡顿的主要结构性原因。
- 为保持 Iputia 分层要求，v40 没有把 Rime 变成 Core 大脑；改动限定在 `Chinese Engine Adapter` 层。`InputiaCore` 仍只面向 `ChineseEngine` trait 请求候选。

变更：

- `inputia-rime` 新增 `RimeEngine::evaluate_incremental(composing, page)`：
  - engine 内部持有一个 live librime session；
  - 输入串递增时只模拟新增 suffix；
  - 翻页时在同一 session 里模拟 `{Page_Down}` / `{Page_Up}`；
  - 输入串回退或跳变时清空当前 composition 后重放；
  - Drop 时销毁 live session，再释放全局 Rime runtime。
- 普通候选路径现在优先用 live session；拼音纠错候选仍走 cold `evaluate()`，避免把纠错变体和主输入 session 混在一起。
- 新增 `persistent_session_probe`，用于量化 cold evaluate 与 incremental evaluate 的差距。
- Bundle 版本升为 v40。

性能 spike：

```text
persistent_session_probe double_pinyin mlle 30
  prefixes=m,ml,mll,mlle
  cold_first=没
  incremental_first=没
  cold_ms=7790.932
  incremental_ms=59.045
  speedup=131.95x

persistent_session_probe luna_pinyin_simp zhongguo 20
  prefixes=z,zh,zho,zhon,zhong,zhongg,zhonggu,zhongguo
  cold_first=在
  incremental_first=在
  cold_ms=9361.647
  incremental_ms=68.502
  speedup=136.66x
```

候选一致性验证：

```text
cargo test --manifest-path crates/inputia-rime/Cargo.toml bundled_incremental_session_matches_cold_evaluate_when_available -- --nocapture
  bundled_incremental_session_matches_cold_evaluate_when_available ok

cargo test --manifest-path crates/inputia-rime/Cargo.toml -- --nocapture
  9 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
  19 passed

cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed
```

系统安装与默认回归：

```text
install-system.sh
  sourceVersion=40
  sourceCDHash=1b9371c94aae8867b4a208fd8a39f5f555defbe5
  destVersion=40
  destCDHash=1b9371c94aae8867b4a208fd8a39f5f555defbe5
  systemInstallVerified=true
  enabled=true
  selectable=true
  selected=true

post-install-regression.sh
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true

status.sh
  buildVersion=40
  system host version=40
  systemMatchesBuild=true
  runningVersion=40
  runningCDHash=1b9371c94aae8867b4a208fd8a39f5f555defbe5
  runningMatchesBuild=true
```

Swift/Host 自检：

```text
inputia-shortcut-self-check
  shortcutSelfCheck=true

inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  carriageReturnCommitsRaw=true
  composingSpaceCommitsCandidate=true
  plainSpacePassesThrough=true
```

打包：

```text
build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v40-1b9371c94aae.pkg
  latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  sha256=ef7da716bdd56443ef6ce7fc715d796fa0cdc262fbcb6214d4560367b25b45f8
  appCDHash=1b9371c94aae8867b4a208fd8a39f5f555defbe5
```

测试纪律：

- 本轮没有打开 TextEdit/Safari UI smoke。
- `pgrep -fl TextEdit` 和 `pgrep -x Safari` 在回归后均无输出。

## v41 Mac mini 接续：Rime fallback、CAPI 稳定性、GUI smoke 纪律与 TIS 阻塞

接续环境：

- 工作区：`/Users/minizl/services/Handy`。
- 图形会话：`consoleUser=lizhelang`，`uiElementsEnabled=true`，前台应用为 Codex。
- 接续前后均确认 `texteditResidual=false`，没有遗留 TextEdit 进程。

关键修复：

- `prepare-rime-data.sh` 在缺少 `/Library/Input Methods/Squirrel.app/Contents/SharedSupport` 时，继续使用官方 Rime package 数据；同时把 schema 获取顺序调整为先复用本地已有 RimeData，再访问网络，避免安装阶段因 `raw.githubusercontent.com` 超时卡住。
- `smoke-clipboard-recall.sh` 补齐与 TextEdit smoke 一致的纪律：TextEdit preflight、trap 清理、失败路径 `quit saving no`、前台丢失检测，以及召回前双 Escape 清空 IME 状态。
- `Info.plist` 中 Inputia 简体/繁体 mode 的 `tsInputModeScriptKey` 从 `smUnicodeScript` 改为成熟中文输入法一致的 `smSimpChinese` / `smTradChinese`。对照对象：本机 `/Library/Input Methods/WeType.app` 的拼音 mode 使用 `smSimpChinese`。

Rime / CAPI 验证：

```text
brew librime
  /opt/homebrew/lib/librime.1.dylib exists

prepare-rime-data local reuse
  reuseRimeData=... from=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/RimeData
  reuseRimeSchema=... from=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/RimeData
  rimeDataDir=/tmp/inputia-rimedata-local-reuse-test

cargo test --manifest-path crates/inputia-rime/Cargo.toml -- --nocapture
  10 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture --test-threads=1
  19 passed
```

说明：`inputia-capi` 需要串行测试。librime runtime 是进程级全局状态；并行测试会在不同临时 `user_data_dir` 之间互相污染，表现为 `failed to save config to stream` 和后续锁 poison。测试已使用共享稳定 Rime 用户目录，串行全量通过。

构建和桥接自检：

```text
build.sh
  InputiaInputMethod.app: valid on disk
  Inputia 设置.app: valid on disk
  warning: Rust std/object files built for newer macOS version 26.0 than linked 13.0

bridge self-check from build app
  bridgeSelfCheck=true
  consumed=true
  mode=Chinese
  composing=
  firstCandidate=在
  commit=中国
  bridgeSelfCheckHasMissingSquirrel=false

inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  carriageReturnCommitsRaw=true
  lineFeedCommitsRaw=true
  composingSpaceCommitsCandidate=true
  plainSpacePassesThrough=true
```

系统安装 / TIS 状态：

```text
install-system.sh
  first run copied v40 to /Library/Input Methods and status matched build
  subsequent run after Info.plist script-key fix failed at admin authorization:
  execution error: 管理员用户名或密码不正确。 (-60007)

status.sh after script-key fix
  buildCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  userMatchesBuild=true

installed system Info.plist
  tsInputModeScriptKey=smUnicodeScript
```

TIS 选择阻塞：

```text
/Library/Input Methods/InputiaInputMethod.app --register-input-source
  registerStatus=0

--enable-input-source
  enabledSourceAlreadyPresent=true
  id=com.inputia.inputmethod.Inputia.Hans
  enabled=true
  selectable=true
  selected=false

--select-input-source
  selectStatus=-50
  selected=false

--dump-current-input-source
  id=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true
```

解释：当前系统级包仍是旧 CDHash，且 mode script 仍为 `smUnicodeScript`。用户级安装虽然写入 `~/Library/Input Methods/InputiaInputMethod.app` 并与 build 匹配，但同 bundle id 下 TIS 仍解析到系统级 `/Library/Input Methods/InputiaInputMethod.app`，不能替代系统级安装做真实 TextEdit/Safari smoke。

GUI smoke 纪律验证：

```text
INPUTIA_RUN_UI_SMOKE=1 smoke-textedit.sh
  expectedCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  textEditSmokeReady=false reason=cdhash-mismatch
  texteditResidualAfterTextEditSmoke=false

INPUTIA_RUN_UI_SMOKE=1 smoke-clipboard-recall.sh
  expectedCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  clipboardRecallSmokeReady=false reason=cdhash-mismatch
  texteditResidualAfterClipboardSmoke=false
```

当前结论：

- 后端 Rime/CAPI/bridge 已在 Mac mini 当前环境通过；缺 Squirrel.app 时可使用 Homebrew librime + bundled RimeData。
- smoke 脚本现在会在系统包 CDHash 不匹配时拒绝误测，且不会留下 TextEdit。
- 真正的 TextEdit/Clipboard GUI smoke 仍未运行；原因是最新 `Info.plist` 修复无法无交互写入 root-owned `/Library/Input Methods/InputiaInputMethod.app`，且当前 TIS 仍不能选中旧系统包（`selectStatus=-50`）。
- 下一步需要管理员授权安装当前 build CDHash `8f7569a20246137b95a4d9f8553a7309ffb81350` 到 `/Library/Input Methods`，刷新 TextInputMenuAgent/SystemUIServer 后，再重新验证 `--select-input-source` 和 GUI smoke。

## v42 Mac mini 接续：安装链路加固与可安装包

本轮继续保留 v41 的阻塞判断：系统级 `/Library/Input Methods/InputiaInputMethod.app` 仍是旧 CDHash，不能运行真实 GUI smoke。目标是把下一次管理员安装和 smoke 验证链路收紧到可复现。

脚本加固：

```text
zsh -n install-system.sh
zsh -n prepare-rime-data.sh
zsh -n smoke-textedit.sh
bash -n smoke-clipboard-recall.sh
  all passed
```

- `install-system.sh` 的管理员授权复制分支改为走 `run_with_timeout 120 admin-copy ... osascript ...`，避免授权窗口或 `osascript` 桥接进程无限残留。
- `prepare-rime-data.sh` 验证为本地复用路径：`reuseRimeData` / `reuseRimeSchema` 从已安装系统 RimeData 或 build RimeData 复制，不需要访问 GitHub。

Info.plist / 包内容：

```text
plutil -lint Info.plist
  OK

build/pkg-scripts/InputiaInputMethod.app.tar.gz:
  tsInputModeScriptKey=smSimpChinese
  tsInputModeScriptKey=smTradChinese
  pkgAppCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
```

可安装包：

```text
build-pkg.sh
  pkg=macos/InputiaInputMethod/dist/InputiaInputMethod-v40-8f7569a20246.pkg
  latest=macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  appCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  payloadFiles=0
  pkgSignature=none
  latest sha256=167b0285157d7d905e55020d7de7420283df5856cc32b7e5e8824fbe26353880
  latest sizeBytes=6773084
```

非 GUI 验证：

```text
bridge self-check from build app
  bridgeSelfCheck=true
  consumed=true
  mode=Chinese
  composing=
  firstCandidate=在
  commit=中国
  missingSquirrelLog=false

inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  carriageReturnCommitsRaw=true
  lineFeedCommitsRaw=true
  composingSpaceCommitsCandidate=true
  plainSpacePassesThrough=true

cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture --test-threads=1
  19 passed
```

smoke gate / 清理纪律：

```text
status.sh
  buildCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  running=false

INPUTIA_RUN_UI_SMOKE=1 smoke-textedit.sh
  textEditSmokeReady=false reason=cdhash-mismatch
  texteditResidualAfterTextEditSmoke=false

INPUTIA_RUN_UI_SMOKE=1 smoke-clipboard-recall.sh
  clipboardRecallSmokeReady=false reason=cdhash-mismatch
  texteditResidualAfterClipboardSmoke=false

pgrep -fl 'osascript|TextEdit|InputiaInputMethod|smoke-textedit|smoke-clipboard-recall'
  no GUI/auth/smoke residual after cleanup
```

当前剩余工作：

- 用管理员授权安装 `dist/InputiaInputMethod-latest.pkg` 或等价写入当前 build 到 `/Library/Input Methods`。
- 安装后必须确认 `systemMatchesBuild=true` 且系统包 `tsInputModeScriptKey` 已是 `smSimpChinese` / `smTradChinese`。
- 再运行 `--register-input-source` / `--enable-input-source` / `--select-input-source`，期望 `selectStatus=0` 和当前 source 为 `Inputia 简体`。
- 之后才能运行 TextEdit 和 Clipboard GUI smoke；如果 Safari 当前由用户使用，仍应跳过 Safari smoke 或先做 preflight，避免抢焦点。

## v40 Mac mini 续修：smoke 选择前置门禁与权限修复

时间：2026-07-07 22:59 Asia/Shanghai。

本轮继续处理 Mac mini 上的 GUI smoke 纪律问题，重点是不在输入源未真正选中时打开 TextEdit，也不留下 TextEdit 残留。

实现变更：

- `build.sh`：
  - 显式 `umask 022`，避免当前 shell umask 导致 `.app` bundle 生成成 `700`。
  - 签名前归一化 app bundle 权限：目录 `755`、文件 `644`、主可执行 `755`。
  - 重新生成 `build/inputia-tis-tool`，并对 `InputiaTISTool.swift` 使用 `-parse-as-library`。
- `install-system.sh`：
  - 显式 `umask 022`。
  - 复制到 `/Library/Input Methods` 和 `/Applications` 后归一化权限，避免 root-owned app 变成不可读。
- `smoke-textedit.sh`：
  - 增加 `INPUTIA_SKIP_CDHASH_CHECK=1` 显式跳过 CDHash 门禁，和 clipboard smoke 对齐。
  - 在打开 TextEdit 前检查 TIS 选择日志，只有 `selectStatus=0` 且 `selected=true` 才继续；否则报 `textEditSmokeReady=false reason=input-source-not-selected`。
- `smoke-clipboard-recall.sh`：
  - 增加和 TextEdit smoke 一样的选择前置门禁。
  - AppleScript 内增加失败清理、前台校验和双 Escape 清空 IME 状态，避免 composition 污染与残留窗口。
- `inputia-capi` / `inputia-rime`：
  - CAPI 默认会话也使用 Inputia bundled RimeData fallback，不再只在 settings 路径使用。
  - CAPI 测试使用稳定 `/tmp/inputia-capi-rime-user`，避免进程级 librime runtime 持有已删除 tempdir。
  - CAPI 测试的记忆重排夹具改用当前官方 RimeData 稳定包含的“中国”，不再依赖旧 Squirrel 词库里可能存在的“种过”。

验证：

```text
cargo test --manifest-path crates/inputia-core/Cargo.toml
  30 passed

cargo test --manifest-path crates/inputia-rime/Cargo.toml -- --nocapture
  lib: 3 passed
  core_flow: 3 passed
  schema_smoke: 4 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
  19 passed

build.sh
  buildRc=0
  build/inputia-tis-tool exists and executable

bridge self-check
  bridgeRc=0
  bridgeSelfCheck=true
  commit=中国
  containsSquirrelMissing=false
  containsNoSuchFile=false

inputia-input-text-router-self-check
  routerRc=0
  inputTextRouterSelfCheck=true
```

系统安装 / TIS 当前状态：

```text
sudo -n true
  sudoNoPromptRc=1
  sudo: a password is required

status.sh
  buildVersion=40
  buildCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  user host removed

inputia-tis-tool --reset-enable after user-host cleanup
  matches=false before register
  parentEnableStatus=0 but parent enabled=false/selectable=false
  hansEnableStatus=0
  includeAllInstalled=false matches=1
  Hans enabled=true selectable=true selected=false
  selectStatus=-50
```

GUI smoke 纪律验证：

```text
INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  texteditSmokeRc=15
  textEditPreflight=not-running docs=0
  textEditSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  selected=false
  texteditResidual=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  clipboardSmokeRc=8
  textEditPreflight=not-running docs=0
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  selected=false
  texteditResidualAfterClipboard=false
```

当前结论：

- 后端、Rime、CAPI、bridge、router 在 Mac mini 当前环境可用。
- GUI smoke 脚本已修到“选择失败不打开 TextEdit”，满足本轮 smoke 纪律目标。
- 真正 TextEdit/Clipboard 上屏 smoke 仍未执行，当前阻塞是系统级输入源无法在本机自动选中：Hans mode 进入 enabled list 但 `TISSelectInputSource` 仍返回 `-50`。
- 系统目录仍是上一次成功安装的 v40（CDHash `8d4f473...`），最新 build CDHash `8f7569...` 需要管理员授权安装到 `/Library/Input Methods` 后再刷新 TextInputMenuAgent/SystemUIServer，并重新验证 `selected=true`。

## v40 Mac mini 续修：TIS 刷新复核与统一 guiSmokeReady

时间：2026-07-07 23:06 Asia/Shanghai。

本轮继续尝试在不打开 TextEdit 的前提下恢复系统级 TIS 选择，并把 GUI smoke 前置失败输出统一成 `guiSmokeReady=false`。

当前状态复核：

```text
status.sh
  buildVersion=40
  buildCDHash=8f7569a20246137b95a4d9f8553a7309ffb81350
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  user host absent
  running=false

--dump-current-input-source
  id=com.tencent.inputmethod.wetype.pinyin
  selected=true

--dump-source-prefix com.inputia.inputmethod.Inputia
  includeAllInstalled=false
    matches=0
  includeAllInstalled=true
    parent enabled=false selectable=false selected=false
    Hans enabled=true selectable=true selected=false
```

刷新尝试：

```text
osascript quit System Settings
killall TextInputMenuAgent
killall SystemUIServer
--register-input-source
  registerStatus=0
--enable-input-source
  enabledSourceAlreadyPresent=true
  Hans enabled=true selectable=true selected=false
--select-input-source
  selectStatus=-50
  selected=false
--dump-enabled-input-source
  inputSourceFound=false
--dump-current-input-source
  still com.tencent.inputmethod.wetype.pinyin selected=true
```

结论：刷新 Text Input UI 缓存后仍不能让 Inputia 进入 `includeAllInstalled=false` 的可选列表；系统级 GUI smoke 继续被 TIS `selectStatus=-50` 阻塞。

脚本改动：

- `smoke-textedit.sh` 和 `smoke-clipboard-recall.sh` 在这些前置失败时统一输出 `guiSmokeReady=false`：
  - `ui-smoke-disabled`
  - `textedit-already-running`
  - `input-source-not-selected`
- 这样自动化调用方可以在不解析具体 smoke 名称的情况下判断“不能打开 GUI 继续测试”。

验证：

```text
bash -n smoke-textedit.sh smoke-clipboard-recall.sh
  ok

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  texteditSmokeRc=15
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  texteditResidual=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  clipboardSmokeRc=8
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterClipboard=false
```

当前剩余风险：

- 真实 TextEdit / Clipboard 上屏 smoke 尚未执行，因为 Inputia 当前未被 TIS 选中。
- 需要在 Mac mini 上完成系统级输入源添加/选中：要么通过 System Settings 手动添加 `Inputia 简体`，要么管理员授权安装最新 build 后让 `includeAllInstalled=false` 能枚举到 Inputia，并使 `TISSelectInputSource` 返回 `0`。

## v40 Mac mini 续修：Safari smoke 前置门禁与测试窗口清理

时间：2026-07-07 23:08 Asia/Shanghai。

背景：TextEdit / Clipboard smoke 已经在输入源未选中时提前退出，但 Safari smoke 仍会在 `TISSelectInputSource` 失败后打开 Safari 测试页，存在抢焦点和残留窗口风险。

实现变更：

- `smoke-safari-typing.sh`
  - 支持 `INPUTIA_SKIP_CDHASH_CHECK=1`，和其它 GUI smoke 对齐。
  - 在打开 Safari 前检查选择日志；必须同时满足 `selectStatus=0` 和 `selected=true`。
  - 前置失败统一输出 `guiSmokeReady=false reason=...`。
  - AppleScript 增加 `waitForFrontmost` / `assertStillFrontmost`，并记录脚本创建的 Safari window id。
  - 成功或失败都会关闭脚本创建的测试窗口并恢复之前前台应用。
- `smoke-safari-enter.sh`
  - 增加同样的输入源选择门禁和 `guiSmokeReady=false` 输出。
  - 增加 `launchctl unsetenv INPUTIA_DEBUG_EVENTS` trap，失败路径也清理调试环境变量。
  - AppleScript 增加测试窗口关闭和前台恢复。

验证：

```text
bash -n smoke-safari-typing.sh smoke-safari-enter.sh
  ok

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  safariTypingRc=8
  guiSmokeReady=false reason=input-source-not-selected
  safariTypingSmokeReady=false reason=input-source-not-selected
  no osascript path entered

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  safariEnterRc=6
  guiSmokeReady=false reason=input-source-not-selected
  safariEnterSmokeReady=false reason=input-source-not-selected
  INPUTIA_DEBUG_EVENTS unset after exit

post-check
  no smoke-safari / osascript / bridge-self-check process remains
  texteditResidual=false
```

当前结论：

- TextEdit、Clipboard、Safari 三类 GUI smoke 现在都具备输入源选择前置门禁，当前 TIS 未选中 Inputia 时不会抢用户焦点。
- Safari 真实上屏 smoke 仍等待同一个 TIS 前置条件：`includeAllInstalled=false` 可枚举 Inputia 且 `TISSelectInputSource` 返回 `0` / `selected=true`。

## v41 Command 系统快捷键透传修复

时间：2026-07-07 23:25 Asia/Shanghai。

用户现象：

- 在 Inputia 输入法下 `Command-C` / `Command-V` 不能正常复制粘贴。
- 该问题不能只按单个快捷键逐一修补；常用电脑快捷键应该默认不被输入法接管。

依据：

- Apple 官方 Mac keyboard shortcuts 文档列出 `Command-X/C/V/Z/A/F/G/S/O/P/W/Q/H/M/Tab/Space/Comma` 等为系统或 App 常用快捷键。
- 项目既有 v37 证据已确认 IMK Host 边界：`handleEvent` / `didCommandBySelector` 返回 handled 会消费事件；不属于输入法明确处理范围的事件必须返回 not handled。
- 鼠须管成熟实现边界同样是 Command 组合直接交还宿主 App。

根因判断：

- `didCommand(by:)` 当前默认已经对未知 selector 返回 `false`。
- 风险点在 `handleKeyDown`：Inputia 快捷键、剪贴板召回、候选处理在通用 Command/Control/Option 透传判断之前执行。虽然已有若干分支各自拒绝 Command，但缺少统一的 Command 系统快捷键前置门禁，后续新增快捷键容易回归。

实现变更：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)`
  - 只要包含 `.command`，统一判定为系统/App 快捷键，输入法不得接管。
- `InputiaInputController.handleKeyDown`
  - 在剪贴板召回、候选窗口、标点切换、输入模式切换之前先执行 Command 透传。
- `inputia-shortcut-self-check` 和 `--host-shortcut-self-check`
  - 增加常用 Command 快捷键覆盖：
    - `Command-C/V/X/Z/Shift-Z/A/S/O/W/Q/F/G/,/Tab/Space/Option-Escape`
  - 保留 `Control-Shift-V` 作为 Inputia 剪贴板召回快捷键。
  - 覆盖 `Control-Shift-Command-V` 被拒绝，防止和系统/App Command 组合冲突。

验证：

```text
./macos/InputiaInputMethod/build.sh
  ok
  InputiaInputMethod.app: valid on disk
  InputiaInputMethod.app: satisfies its Designated Requirement

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-self-check
  bridgeSelfCheck=true
  mode=Chinese
  commit=中国

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  plainSpacePassesThrough=true
  routeEnterPassesThroughWithoutComposing=true

./macos/InputiaInputMethod/build-pkg.sh
  pkg=dist/InputiaInputMethod-v40-ad6b9124eccd.pkg
  appCDHash=ad6b9124eccd93e413a387b9667a7275d013c080
  package signature=no signature

shasum -a 256 dist/InputiaInputMethod-latest.pkg dist/InputiaInputMethod-v40-ad6b9124eccd.pkg
  0ef68d3cdcc9a2f4001cec9dfbae815cb7a0a6d60bbb0c7a531ef188cb9afe8b  both files
```

GUI smoke 门禁：

```text
INPUTIA_RUN_UI_SMOKE=1 smoke-textedit.sh
  expectedCDHash=ad6b9124eccd93e413a387b9667a7275d013c080
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  textEditSmokeReady=false reason=cdhash-mismatch

INPUTIA_RUN_UI_SMOKE=1 smoke-clipboard-recall.sh
  expectedCDHash=ad6b9124eccd93e413a387b9667a7275d013c080
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  clipboardRecallSmokeReady=false reason=cdhash-mismatch

post-check
  no osascript / TextEdit / InputiaInputMethod / smoke-textedit / smoke-clipboard-recall process remains
```

当前剩余风险：

- 真实 TextEdit 中 `Command-C/V` 仍未跑 GUI smoke，因为系统安装版还是旧 CDHash `8d4f473adcc2f7c093b5629b9b1e742dcba184f8`，当前构建是 `ad6b9124eccd93e413a387b9667a7275d013c080`。
- 安装或刷新系统输入源后，需要在真实 App 中补一轮复制/粘贴、撤销/重做、全选、保存/打开、查找、应用切换 smoke。

## v41 续修：AppKit selector 透传与用户级半截安装防护

时间：2026-07-07 23:42 Asia/Shanghai。

新增发现：

- `Command-C/V` 这类快捷键除了 `keyDown` 路径，也可能经由 IMK `didCommand(by:)` 进入 AppKit selector。
- 当前 `didCommand(by:)` 的 default 分支已返回 `false`，但 `copy:` / `paste:` 只是隐式放行；后续维护时容易误加分支并回归。
- TIS 诊断一度显示同一 bundle 同时有系统级和用户级父输入源记录，且用户级路径为 `/Users/minizl/Library/Input Methods/InputiaInputMethod.app`。磁盘状态复核后，用户级 host 当前不存在；此前 TIS 里的用户路径是缓存残影。
- `install-user.sh` 旧流程先创建目标目录再直接 `ditto`，失败时可能留下存在但缺少 `Contents/Info.plist` 的半截 `.app`，会污染 TIS / status 判断。

实现变更：

- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)`
  - 显式透传常见 AppKit selector：
    - `copy:` / `paste:` / `cut:` / `undo:` / `redo:` / `selectAll:`
    - `saveDocument:` / `openDocument:` / `performClose:` / `terminate:`
    - `find:` / `orderFrontFindPanel:` / `print:` / `hide:` / `showHelp:`
    - `pasteAsPlainText:` / `toggleContinuousSpellChecking:` 等。
- `InputiaInputController.didCommand(by:)`
  - 在 newline / delete / candidate selector 处理前先调用 App command 透传策略并返回 `false`。
  - `deleteBackward:`、`moveDown:`、`insertTab:` 不列入 App command，保留输入法内部处理。
- `InputiaHostTextPolicySelfCheck`
  - 增加上述 selector 透传覆盖。
- `install-user.sh`
  - 改为复制到 `mktemp` 临时目录，校验 CDHash、`Contents/Info.plist` 和 host 可执行文件后再 `mv` 到目标路径。
  - 失败时只清理临时目录，不留下半截用户级 input method app。
- `status.sh`
  - 存在 `.app` 但缺少 `Contents/Info.plist` 时输出 `validBundle=false reason=missing-info-plist`。
- `await-system-install.sh`
  - 状态行增加 `validBundle=` 字段，便于识别半截安装。

验证：

```text
zsh -n install-user.sh status.sh await-system-install.sh
  ok

./macos/InputiaInputMethod/build.sh
  ok
  InputiaInputMethod.app: valid on disk
  InputiaInputMethod.app: satisfies its Designated Requirement

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  deleteBackwardIsNotAppCommand=true
  moveDownIsNotAppCommand=true
  insertTabIsNotAppCommand=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-self-check
  bridgeSelfCheck=true
  mode=Chinese
  commit=中国

macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  routeEnterPassesThroughWithoutComposing=true
  plainSpacePassesThrough=true
```

GUI smoke 门禁：

```text
INPUTIA_RUN_UI_SMOKE=1 smoke-textedit.sh
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  textEditSmokeReady=false reason=cdhash-mismatch

INPUTIA_RUN_UI_SMOKE=1 smoke-clipboard-recall.sh
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  clipboardRecallSmokeReady=false reason=cdhash-mismatch

post-check
  no osascript / TextEdit / InputiaInputMethod / smoke-textedit / smoke-clipboard-recall process remains
```

打包与当前状态：

```text
./macos/InputiaInputMethod/build-pkg.sh
  pkg=dist/InputiaInputMethod-v41-a1a178efd821.pkg
  appCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  package signature=no signature

shasum -a 256 dist/InputiaInputMethod-latest.pkg dist/InputiaInputMethod-v41-a1a178efd821.pkg
  f4fcadbf0012c4e9ae8fd842948373047495efea9b184e15a8b1f118f700ca8d  both files

status.sh
  buildVersion=41
  buildCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  system host version=40 cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8 systemMatchesBuild=false
  user host exists=false
  running=false
  latest pkg sha256=f4fcadbf0012c4e9ae8fd842948373047495efea9b184e15a8b1f118f700ca8d
```

当前剩余风险：

- 系统安装版仍是 v40 / CDHash `8d4f473adcc2f7c093b5629b9b1e742dcba184f8`；真实 GUI smoke 仍不能跑，否则会验证旧版本。
- TIS 仍需要在安装 v41 后重新验证：父输入源必须 `enabled=true`，`TISSelectInputSource` 必须返回 `0` 且 `selected=true`。

## v41 续修：Clipboard smoke 失败路径不污染剪贴板

时间：2026-07-07 23:55 Asia/Shanghai。

背景：

- 当前系统安装版仍是旧 v40，GUI smoke 应该在 CDHash mismatch / UI disabled / input-source-not-selected 等前置失败时提前退出。
- 复核 `smoke-clipboard-recall.sh` 时发现：脚本会在确认 `INPUTIA_RUN_UI_SMOKE=1`、TextEdit preflight 和 TIS 选中成功之前写入 `pbcopy`。
- 这不会抢焦点，但会在失败路径污染用户剪贴板，不符合 Clipboard GUI smoke 清理纪律。

实现变更：

- `smoke-clipboard-recall.sh`
  - 新增 `ORIGINAL_CLIPBOARD` / `CLIPBOARD_CHANGED` 和 `restore_clipboard()`。
  - `cleanup_smoke` trap 里恢复原文本剪贴板，默认开启，可用 `INPUTIA_CLIPBOARD_SMOKE_RESTORE=0` 跳过。
  - 将 smoke 文本写入 `pbcopy` 移到 TIS 选择成功之后。
  - 因此这些失败路径不会写剪贴板：
    - `cdhash-mismatch`
    - `ui-smoke-disabled`
    - `textedit-already-running`
    - `input-source-not-selected`

验证：

```text
bash -n smoke-clipboard-recall.sh smoke-textedit.sh smoke-safari-typing.sh smoke-safari-enter.sh
  ok

INPUTIA_RUN_UI_SMOKE=0 smoke-clipboard-recall.sh build/InputiaInputMethod.app
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  guiSmokeReady=false reason=ui-smoke-disabled
  clipboardRecallSmokeReady=false reason=ui-smoke-disabled
  clipboardUnchanged=true

INPUTIA_RUN_UI_SMOKE=1 smoke-clipboard-recall.sh
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  clipboardRecallSmokeReady=false reason=cdhash-mismatch
  clipboardUnchanged=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  clipboardUnchanged=true

post-check
  no osascript / TextEdit / InputiaInputMethod / smoke-clipboard-recall / smoke-textedit process remains
```

状态复核：

```text
status.sh
  buildVersion=41
  buildCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  system host version=40 cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8 systemMatchesBuild=false
  user host exists=false
  running=false
  latest pkg sha256=887bae79b66e4645d2c034cd5eae99ecc24337154e7fb80fafcb03406d5a3af3
```

当前结论：

- Clipboard smoke 在无法安全运行 GUI 时不再改用户剪贴板。
- 真实 Clipboard recall 上屏 smoke 仍等待系统安装版更新到 v41 并解决 TIS `selectStatus=-50`。

## v41 续修：Safari smoke 不触碰已有 Safari 会话

时间：2026-07-08 00:07 Asia/Shanghai。

背景：

- Safari smoke 已经会关闭自己创建的窗口并恢复前台应用，但如果用户已有 Safari 会话，脚本仍会在同一个 App 里创建新窗口，存在抢前台和打扰用户会话的风险。
- 当前 Mac mini 上 Safari 正在运行，适合验证默认拒绝策略。

实现变更：

- `smoke-common.sh`
  - 新增 `inputia_require_process_not_running(process, ready_var, exit_code, reason, allow_env)`。
  - 默认检测到目标 GUI App 正在运行时输出：
    - `guiSmokeReady=false reason=...`
    - `<smokeReady>=false reason=...`
  - 可通过对应 allow env 显式放行。
- `smoke-safari-typing.sh`
  - CDHash 校验通过后、TIS 选择前检查 `Safari` 是否已运行。
  - 默认已运行时拒绝：`safariTypingSmokeReady=false reason=safari-already-running`。
  - 临时 URL 文件改为变量 `TEST_URL_FILE`，trap 清理。
- `smoke-safari-enter.sh`
  - 同样增加 Safari 已运行门禁。
  - 复用原 `cleanup_smoke`，同时清理 `INPUTIA_DEBUG_EVENTS` 和临时 URL 文件。
- 显式放行开关：
  - `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1`

验证：

```text
bash -n smoke-common.sh smoke-safari-typing.sh smoke-safari-enter.sh
  ok

pgrep -x Safari
  safariRunning=true

INPUTIA_RUN_UI_SMOKE=1 smoke-safari-typing.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  guiSmokeReady=false reason=cdhash-mismatch
  safariTypingSmokeReady=false reason=cdhash-mismatch

INPUTIA_RUN_UI_SMOKE=1 smoke-safari-enter.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  guiSmokeReady=false reason=cdhash-mismatch
  safariEnterSmokeReady=false reason=cdhash-mismatch

INPUTIA_RUN_UI_SMOKE=1 smoke-safari-typing.sh build/InputiaInputMethod.app
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  SafariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariTypingSmokeReady=false reason=safari-already-running

INPUTIA_RUN_UI_SMOKE=1 smoke-safari-enter.sh build/InputiaInputMethod.app
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  SafariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariEnterSmokeReady=false reason=safari-already-running

post-check
  /tmp/inputia-safari-typing-test.url absent
  /tmp/inputia-safari-enter-test.url absent
  no osascript / smoke-safari / InputiaInputMethod process remains
```

状态补充：

```text
status.sh
  buildVersion=41
  buildCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  build topLevelTISInputSourceID=absent
  system host version=40 cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8 systemMatchesBuild=false
  system host topLevelTISInputSourceID=com.inputia.inputmethod.Inputia
  tis includeAllInstalled=false matches=0
  tis includeAllInstalled=true matches=3
  user host exists=false
  running=false
```

当前结论：

- Safari smoke 现在默认不会在用户已有 Safari 会话上创建测试窗口。
- 真实 Safari smoke 仍等待两项前置条件：
  - 系统安装版更新到 v41。
  - Safari 未运行，或显式设置 `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1`。

## v41 续修：系统安装清理用户级 Inputia host 冲突

时间：2026-07-08 00:19 Asia/Shanghai。

背景：

- TIS 选择失败的关键风险之一是同一 bundle 同时存在系统级和用户级 Inputia host。
- 当前磁盘状态已经是 `user host exists=false`，但之前 TIS 曾出现用户级路径残影；如果后续安装时用户级 host 再次残留，`TISSelectInputSource` 可能继续返回 `-50`。
- 系统安装应该只保留 `/Library/Input Methods/InputiaInputMethod.app` 作为唯一 host；用户数据、设置和记忆不应被删除。

实现变更：

- `Packaging/scripts/postinstall`
  - 解析当前 console user 的 `NFSHomeDirectory`。
  - 注销并删除：
    - `~/Library/Input Methods/InputiaInputMethod.app`
    - `~/Library/Input Methods/IputiaInputMethod.app`
  - 只处理 Inputia host bundle，不碰用户配置、记忆数据或 settings app。
  - 输出 `inputiaUserHostRemoved=true path=...` 或 `inputiaUserHostRemoved=skipped`。
- `install-system.sh`
  - 与 pkg postinstall 对齐，系统脚本安装也会清同样的用户级 host/legacy host。
  - 输出 `userHostRemoved=true path=...` 或 `userHostRemoved=skipped`。
- `post-install-regression.sh`
  - 增加 `user host conflict` 检查。
  - 系统安装后若 `~/Library/Input Methods/InputiaInputMethod.app` 或历史拼写错误 host 仍存在，直接失败：
    - `userHostConflict=true`

验证：

```text
zsh -n Packaging/scripts/postinstall install-system.sh post-install-regression.sh await-system-install.sh
  ok

INPUTIA_RUN_UI_SMOKE=0 post-install-regression.sh build/InputiaInputMethod.app
  inputiaInstalled=true
  legacyIputiaPresent=false
  userHostConflict=false
  bridgeSelfCheck=true
  hostShortcutSelfCheck=true
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true

rg inputiaUserHostRemoved build/pkg-scripts/postinstall Packaging/scripts/postinstall
  build/pkg-scripts/postinstall contains inputiaUserHostRemoved=true
  Packaging/scripts/postinstall contains inputiaUserHostRemoved=true
```

打包：

```text
./macos/InputiaInputMethod/build-pkg.sh
  pkg=dist/InputiaInputMethod-v41-a1a178efd821.pkg
  appCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  package signature=no signature

shasum -a 256 dist/InputiaInputMethod-latest.pkg dist/InputiaInputMethod-v41-a1a178efd821.pkg
  405e670fb88e05117e3df82639792a79d3f8df27423a980574489dfc97ffb85d  both files
```

门禁与状态复核：

```text
INPUTIA_RUN_UI_SMOKE=1 smoke-textedit.sh
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  textEditSmokeReady=false reason=cdhash-mismatch

INPUTIA_RUN_UI_SMOKE=1 smoke-clipboard-recall.sh
  expectedCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  clipboardRecallSmokeReady=false reason=cdhash-mismatch

status.sh
  buildVersion=41
  buildCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  system host version=40 cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8 systemMatchesBuild=false
  user host exists=false
  latest pkg sha256=405e670fb88e05117e3df82639792a79d3f8df27423a980574489dfc97ffb85d
  running=false

post-check
  no osascript / TextEdit / InputiaInputMethod / post-install-regression / verify-system / smoke process remains
```

当前结论：

- v41 pkg 安装路径现在会主动消除用户级 Inputia host 冲突，降低 TIS 缓存/父输入源选择失败风险。
- 真实系统安装仍未执行；当前 `/Library/Input Methods/InputiaInputMethod.app` 仍是 v40，因此 GUI smoke 继续拒绝运行。

## v40 Mac mini 续修：TIS 诊断增强与待测包路径一致性

时间：2026-07-07 23:35 Asia/Shanghai。

背景：当前 GUI smoke 已能在 Inputia 未选中时提前退出，但 TIS 失败日志缺少来源路径，无法区分记录来自系统副本、用户副本或临时 spike 副本；同时 smoke 脚本虽然允许传入待测 app 路径，调用 `inputia-tis-tool` 时却未把该路径传给工具。

官方/成熟实现依据：

- Apple Text Input Sources 头文件说明：input mode 只有在自身和 parent input method 都 enabled 时才能被选择；`TISSelectInputSource` 失败条件包括 input source 未 enabled，且如果它是 input mode，parent 也必须 enabled。
- Apple 用户文档仍把系统设置的 Input Sources 列表作为已启用输入源列表；这与当前 `includeAllInstalled=false` / `selected=true` 门禁一致。
- 本机成熟实现 WeType 的 TIS 状态显示 parent `com.tencent.inputmethod.wetype` 为 `enabled=true selectable=false`，mode `com.tencent.inputmethod.wetype.pinyin` 为 `enabled=true selectable=true selected=true`；Inputia 的异常点是 parent 始终 `enabled=false`。

实现变更：

- `Tools/InputiaTISTool.swift` 的 `printSource` 增加：
  - `iconURL=`，使用 `kTISPropertyIconImageURL`。
  - `languages=`，使用 `kTISPropertyInputSourceLanguages`。
- `smoke-textedit.sh`、`smoke-clipboard-recall.sh`、`smoke-safari-typing.sh`、`smoke-safari-enter.sh`、`diagnose-safari-input-source.sh` 调用 `inputia-tis-tool` 时统一传入 `INPUTIA_APP="$APP"`，避免脚本传入待测 app 路径时仍注册默认 `/Library` 包。

最小 spike 与结论：

```text
install-user.sh
  status=0
  user host installed at ~/Library/Input Methods/InputiaInputMethod.app
  but TIS selection still fails

TIS dump with iconURL/languages
  parent Inputia enabled=false selectable=false selected=false
  Hans enabled=true selectable=true selected=false
  iconURL can point to /Library or ~/Library depending last registration path
  selectStatus remains -50

InputiaSpike plist spike
  copied build app to ~/Library/Input Methods/InputiaSpikeInputMethod.app
  changed bundle id to com.inputia.inputmethod.InputiaSpike
  removed top-level parent TISInputSourceID
  result: not sufficient; parent still enabled=false
  rejected as formal fix; temporary app removed
```

当前状态复核：

```text
status.sh
  buildVersion=40
  buildCDHash=d5d43a9f4290486255050ffebfd463a887f54b6a
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  userVersion=40
  userCDHash=0e204bfa4b949492f50ad97670c684a7aef79bb8
  userMatchesBuild=false
  running=false
```

验证：

```text
bash -n smoke-textedit.sh smoke-clipboard-recall.sh smoke-safari-typing.sh smoke-safari-enter.sh diagnose-safari-input-source.sh
  ok

swiftc Tools/InputiaTISTool.swift -parse-as-library -target arm64-apple-macos13.0 -framework Carbon
  ok

./macos/InputiaInputMethod/build.sh
  ok
  codesign verify ok
  nonfatal ld warnings: Rust std objects built for macOS 26.0 while linking deployment target 13.0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  texteditRc=15
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  texteditResidual=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  clipboardRc=8
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterClipboard=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  safariTypingRc=8
  guiSmokeReady=false reason=input-source-not-selected
  safariTypingSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterSafariTyping=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  safariEnterRc=6
  guiSmokeReady=false reason=input-source-not-selected
  safariEnterSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterSafariEnter=false

post-check
  no smoke-textedit / smoke-clipboard / smoke-safari / osascript / bridge-self-check process remains
  texteditResidual=false
```

当前结论：

- GUI smoke 纪律继续成立：Inputia 未选中时不会打开 TextEdit/Safari，也不会留下 TextEdit 残留。
- `inputia-tis-tool` 现在能在失败日志里显示来源 icon path 和 language，后续可以直接判断 TIS 记录来自哪个安装副本。
- 真实 TextEdit / Clipboard / Safari 上屏 smoke 仍被同一 TIS 条件阻塞：Inputia parent input method 不能进入 `enabled=true`，导致 Hans mode 虽 `enabled=true selectable=true` 但 `TISSelectInputSource` 返回 `-50`。
- 临时用户安装和 InputiaSpike plist spike 都不能替代系统设置/管理员安装后的真实启用流程；下一步应优先消除系统/用户同 bundle id 重复注册，并让 System Settings 中 Inputia parent 进入 enabled input sources，再运行真实 GUI smoke。

清理：

```text
removed ~/Library/Input Methods/InputiaInputMethod.app
removed ~/Applications/Inputia 设置.app
removed ~/Library/Input Methods/InputiaSpikeInputMethod.app

status.sh after cleanup
  buildVersion=40
  buildCDHash=d5d43a9f4290486255050ffebfd463a887f54b6a
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  user host absent
  user settings launcher absent
  running=false
```

## v41 Mac mini 续修：TIS 路径严格门禁与 parent ID 去歧义

时间：2026-07-07 23:55 Asia/Shanghai。

背景：v40 的 smoke 门禁已经能阻止未选中 Inputia 时打开 GUI，但当同一 bundle id 在 build、系统目录或用户目录存在多份副本时，TIS 可能返回旧副本记录。继续推进时需要保证测试脚本不会拿 build 包当目标、却实际注册/选择系统旧包。

依据：

- Apple Text Input Sources 头文件说明：mode-enabled parent input method 本身不直接 selectable，但必须 enabled；input mode 只有在自身和 parent 都 enabled 时才可选中。
- 本机成熟实现 WeType 的 Info.plist 形状：top-level 使用 `CFBundleIdentifier` 表示 parent，mode 内写 `TISInputSourceID`；没有 top-level `TISInputSourceID`。
- 当前 Inputia v40 的 top-level `TISInputSourceID=com.inputia.inputmethod.Inputia` 与 `CFBundleIdentifier` 重复，fresh install 未必失败，但在当前多副本/缓存调试状态下会增加歧义。

实现变更：

- `Tools/InputiaTISTool.swift`
  - 新增 `INPUTIA_TIS_REQUIRE_APP_MATCH=1` 严格模式。
  - 查找 source 时优先匹配 `INPUTIA_APP/Contents/Resources/inputia.pdf` 对应的 `iconURL`。
  - 严格模式下如果没有匹配待测 app 的 source，不再回退到同 id 的第一条旧记录。
- GUI smoke 和 Safari 诊断脚本调用 `inputia-tis-tool` 时统一设置：
  - `INPUTIA_APP="$APP"`
  - `INPUTIA_TIS_REQUIRE_APP_MATCH=1`
- `Info.plist`
  - `CFBundleVersion` / `CFBundleShortVersionString` 升到 `41` / `0.0.41`。
  - 删除 top-level `TISInputSourceID`，保留 Hans/Hant mode 内的 `TISInputSourceID`。
- `SettingsLauncher/Info.plist`
  - 版本同步升到 `41` / `0.0.41`。

最小 spike：

```text
InputiaToplessInputMethod.app
  copied build app to ~/Library/Input Methods
  only removed top-level TISInputSourceID
  result: current machine still resolves Hans mode to /Library old v40 record
  selectStatus=-50
  conclusion: deleting top-level key is not enough to repair the current cached/system state without installing v41 to /Library, but it matches mature implementation shape and removes an unnecessary parent-id duplicate for the next clean system install
  temporary app removed
```

验证：

```text
plutil -lint Info.plist SettingsLauncher/Info.plist build app plists
  OK

build InputiaInputMethod.app Info.plist
  CFBundleVersion=41
  top-level TISInputSourceID missing as expected

bash -n smoke-textedit.sh smoke-clipboard-recall.sh smoke-safari-typing.sh smoke-safari-enter.sh diagnose-safari-input-source.sh
  OK

./macos/InputiaInputMethod/build.sh
  OK
  codesign verify OK
  nonfatal ld warnings: Rust std objects built for macOS 26.0 while linking deployment target 13.0

INPUTIA_APP=build/InputiaInputMethod.app INPUTIA_TIS_REQUIRE_APP_MATCH=1 inputia-tis-tool
  registerStatus=0
  parentFound=false
  hansFound=false
  includeAllInstalled=false shows old /Library Hans only
  selectSourceFoundInEnabledList=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  texteditRc=15
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  texteditResidual=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh /absolute/path/to/build/InputiaInputMethod.app
  texteditBuildAbsRc=15
  guiSmokeReady=false reason=input-source-not-selected
  parentFound=false
  hansFound=false
  texteditResidualAfterBuildAbs=false

./macos/InputiaInputMethod/build-pkg.sh
  pkg=/Users/minizl/services/Handy/macos/InputiaInputMethod/dist/InputiaInputMethod-v41-a1a178efd821.pkg
  latest pkg updated
  appCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  payloadFiles=0
  pkgSignature=none

pkg expand latest
  InputiaInputMethod.app/Contents/Info.plist present
  InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod present
  packaged CFBundleVersion=41
  packaged top-level TISInputSourceID missing as expected

status.sh
  buildVersion=41
  buildCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  systemSettingsMatchesBuildVersion=false
  user host absent
  running=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  clipboardRc=8
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterClipboard=false
  debugEnvUnset=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  safariTypingRc=8
  guiSmokeReady=false reason=input-source-not-selected
  safariTypingSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterSafariTyping=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  first post-check saw transient texteditResidualAfterSafariEnter=true but process list showed no TextEdit process
  cleanup confirmed texteditResidualAfterCleanup=false
  retry:
    safariEnterRetryRc=6
    guiSmokeReady=false reason=input-source-not-selected
    safariEnterSmokeReady=false reason=input-source-not-selected
    texteditResidualAfterSafariEnterRetry=false
    debugEnvAfterSafariEnterRetryUnset=true
```

当前结论：

- v41 build/pkg 已准备好用于下一次系统授权安装。
- Smoke 门禁现在不仅要求 `selectStatus=0` / `selected=true`，还通过 TIS 工具严格避免“待测 build 包实际选择旧系统包”的误判。
- 真实 GUI 上屏 smoke 仍需先把 v41 安装到 `/Library/Input Methods/InputiaInputMethod.app`，并让 System Settings/TIS 中 parent input method 进入 `enabled=true`。

## v41 Mac mini 续修：GUI session preflight 固化

时间：2026-07-08 00:08 Asia/Shanghai。

背景：交接要求 GUI smoke 在 Mac mini 图形桌面继续验证；如果没有已登录且未锁屏的图形桌面，应报告 `guiSmokeReady=false`，不能硬跑 TextEdit/Safari。此前脚本只有 `INPUTIA_RUN_UI_SMOKE=1` 和 app/TIS 门禁，缺少统一图形会话 preflight。

实现变更：

- 新增 `smoke-common.sh`。
- `inputia_require_gui_session ready_var exit_code` 检查：
  - `/dev/console` 当前用户不是空、`root` 或 `_mbsetupuser`。
  - `IOConsoleUsers` 中有 `kCGSessionLoginDoneKey=Yes`。
  - session 未出现 `CGSSessionScreenIsLocked=Yes` / `kCGSSessionScreenIsLocked=Yes`。
  - `System Events` 可读取当前 frontmost app，且不是 `loginwindow`。
- `smoke-textedit.sh`、`smoke-clipboard-recall.sh`、`smoke-safari-typing.sh`、`smoke-safari-enter.sh` 在打开 TextEdit/Safari 或选择输入源前调用该 preflight。
- 保留 `INPUTIA_SKIP_GUI_SESSION_CHECK=1` 作为诊断逃生口，默认不跳过。

验证：

```text
bash -n smoke-common.sh smoke-textedit.sh smoke-clipboard-recall.sh smoke-safari-typing.sh smoke-safari-enter.sh
  OK

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  texteditRc=15
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  texteditResidual=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  clipboardRc=8
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterClipboard=false
  debugEnvUnset=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  safariTypingRc=8
  guiSmokeReady=false reason=input-source-not-selected
  safariTypingSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterSafariTyping=false

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  safariEnterRc=6
  guiSmokeReady=false reason=input-source-not-selected
  safariEnterSmokeReady=false reason=input-source-not-selected
  texteditResidualAfterSafariEnter=false
  debugEnvAfterSafariEnterUnset=true

status.sh
  buildVersion=41
  buildCDHash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  user host absent
  running=false
```

当前结论：

- GUI smoke 已具备三层前置门禁：人工启用开关、图形桌面可用性、Inputia TIS 选中状态。
- 在当前 v40 系统包未选中、v41 build 未安装到系统目录的状态下，四条 smoke 都不会打开 TextEdit/Safari。
- 真实上屏 smoke 仍等待 v41 系统安装和 TIS parent enabled。

## v41 Mac mini 续修：status TIS/plist 关键状态输出

时间：2026-07-07 23:28 CST。

背景：当前阻塞不是构建产物是否存在，而是 macOS TIS 仍从 `/Library/Input Methods/InputiaInputMethod.app` 旧系统包解析输入源，导致 build 包验证和系统输入源状态容易混淆。需要让 `status.sh` 直接报告 build/system/user 三处 app 的顶层 `TISInputSourceID`，以及 TIS 当前解析到的 Inputia source 明细。

实现变更：

- `status.sh` 对每个 app 输出 `topLevelTISInputSourceID`，缺失时显示 `absent`。
- `status.sh` 调用 `build/inputia-tis-tool --dump`，输出当前 TIS 中 Inputia source 的 id、bundle、mode、name、iconURL、languages、enabled、selectable、selected。
- `status.sh` 的 expected build app 也通过统一 app 打印路径输出，便于和系统安装版逐项对比。

验证：

```text
zsh -n macos/InputiaInputMethod/status.sh
  OK

bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh
  OK

./macos/InputiaInputMethod/status.sh
  expected build app:
    version=41
    shortVersion=0.0.41
    topLevelTISInputSourceID=absent
    cdhash=a1a178efd821aa2d21ded1f4def586c3b63eb700
  system host:
    version=40
    shortVersion=0.0.40
    topLevelTISInputSourceID=com.inputia.inputmethod.Inputia
    cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
    systemMatchesBuild=false
  system settings launcher:
    version=40
    shortVersion=0.0.40
    systemSettingsMatchesBuildVersion=false
  user host:
    exists=false
  tis sources:
    includeAllInstalled=false matches=0
    includeAllInstalled=true matches=3
    parent id=com.inputia.inputmethod.Inputia enabled=false selectable=false selected=false
    Hant enabled=false selectable=true selected=false
    Hans enabled=true selectable=true selected=false
    iconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  running=false

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true
```

当前结论：

- build/pkg 侧已经是 v41，并且 build 的顶层 `TISInputSourceID` 已移除。
- 系统安装侧仍是 v40，仍带旧顶层 `TISInputSourceID`，所以 TIS 明细继续指向 `/Library` 旧包是预期状态。
- TIS parent input method 仍为 `enabled=false`，Hans mode 虽 `enabled=true/selectable=true` 但未选中；这解释了 GUI smoke 的 `input-source-not-selected` 门禁结果。
- 本轮只增强诊断和 smoke 安全门禁，没有执行需要密码的系统安装，也没有留下 TextEdit/osascript/Inputia 残留。

## v41 Mac mini 续修：TextEdit preflight/cleanup 公共化

时间：2026-07-07 23:31 CST。

背景：`smoke-textedit.sh` 和 `smoke-clipboard-recall.sh` 都需要在打开 TextEdit 前确认它没有已运行实例，并在失败路径上自动退出测试启动的 TextEdit。此前两条脚本各自手写 preflight/cleanup，格式不完全一致，后续维护容易出现一条脚本修了、另一条脚本遗漏。

实现变更：

- `smoke-common.sh` 新增 `inputia_textedit_document_count`，通过 AppleScript 查询 TextEdit 当前文档数；未运行或查询失败时返回 `0`。
- `smoke-common.sh` 新增 `inputia_require_textedit_idle ready_var exit_code`：
  - 输出 `textEditPreflight=<state> docs=<count>`。
  - 默认拒绝已有 TextEdit 进程，返回对应 smoke 的 ready 变量和 exit code。
  - 仍保留 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1` 作为人工诊断逃生口。
- `smoke-common.sh` 新增 `inputia_cleanup_textedit_if_started`，仅在 preflight 判定 TextEdit 原本未运行时 quit saving no。
- `smoke-textedit.sh` 与 `smoke-clipboard-recall.sh` 改用公共 TextEdit preflight/cleanup，保持原业务验证和 exit code 不变。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh
  OK

zsh -n macos/InputiaInputMethod/status.sh
  OK

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  guiSmokeReady=false reason=ui-smoke-disabled
  textEditSmokeReady=false reason=ui-smoke-disabled
  texteditUiDisabledRc=14

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  guiSmokeReady=false reason=ui-smoke-disabled
  clipboardRecallSmokeReady=false reason=ui-smoke-disabled
  clipboardUiDisabledRc=7

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  texteditGateRc=15

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  clipboardGateRc=8

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true
```

当前结论：

- TextEdit/Clipboard smoke 的 TextEdit 预检和清理纪律已统一到公共层。
- 当前系统仍未安装 v41，TIS 选择返回 `selectStatus=-50`，因此两条 GUI smoke 仍在打开 TextEdit 前停止。
- 失败路径没有留下 TextEdit、osascript、Inputia host 或 `INPUTIA_DEBUG_EVENTS`。

## v41 Mac mini 续修：Safari preflight/cleanup 公共化

时间：2026-07-07 23:33 CST。

背景：Safari typing/enter 两条 smoke 已能拒绝已有 Safari，但如果真实 GUI 段进入后异常退出，trap 只清理临时 URL 或 debug env，没有统一兜底退出 smoke 自己启动的 Safari。为了避免测试残留窗口，同时不误关用户已有 Safari，需要和 TextEdit 一样记录 preflight 状态，并只在 Safari 原本未运行时由 trap 兜底退出。

实现变更：

- `smoke-common.sh` 新增 `inputia_require_safari_idle ready_var exit_code`：
  - 输出 `safariPreflight=running|not-running`。
  - 默认拒绝已有 Safari，返回对应 smoke 的 ready 变量和 exit code。
  - 保留 `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1` 作为人工诊断逃生口。
- `smoke-common.sh` 新增 `inputia_cleanup_safari_if_started`：
  - 仅当 preflight 判定 Safari 原本未运行时，trap 才 `tell application "Safari" to quit`。
- `smoke-safari-typing.sh` 和 `smoke-safari-enter.sh` 改用公共 Safari preflight，并在 trap 中调用公共 Safari cleanup。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh
  OK

zsh -n macos/InputiaInputMethod/status.sh
  OK

preflight residue before Safari smoke
  safariResidualBefore=true
  texteditResidualBefore=false
  osascriptResidualBefore=false
  debugEnvBeforeUnset=true

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  guiSmokeReady=false reason=ui-smoke-disabled
  safariTypingSmokeReady=false reason=ui-smoke-disabled
  safariTypingUiDisabledRc=7

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  guiSmokeReady=false reason=ui-smoke-disabled
  safariEnterSmokeReady=false reason=ui-smoke-disabled
  safariEnterUiDisabledRc=5

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  safariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariTypingSmokeReady=false reason=safari-already-running
  safariTypingExistingRc=9

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  safariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariEnterSmokeReady=false reason=safari-already-running
  safariEnterExistingRc=7

residue check
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true
  safariTypingTempResidual=false
  safariEnterTempResidual=false
```

当前结论：

- Safari smoke 现在和 TextEdit smoke 一样有“原本未运行才清理”的公共纪律。
- 当前桌面上 Safari 是用户已有进程，smoke 正确停在 `safari-already-running`，没有强行关闭用户浏览器。
- 因 Safari 已运行且系统安装仍是 v40，本轮没有进入真实 Safari 输入段；真实 Safari 上屏验证仍等待用户关闭 Safari、v41 系统安装完成、TIS parent enabled 后再跑。

## v41 Mac mini 续修：Safari 输入源诊断脚本门禁化

时间：2026-07-07 23:36 CST。

背景：`post-install-regression.sh` 在 UI smoke 阶段会调用 `diagnose-safari-input-source.sh`。该诊断脚本不打字，但会打开 Safari data: 测试页；此前它没有 `INPUTIA_RUN_UI_SMOKE`、图形会话、已有 Safari、TIS 选中状态和临时文件清理门禁，可能绕过正式 Safari smoke 的清理纪律。

实现变更：

- `diagnose-safari-input-source.sh` source `smoke-common.sh`。
- 新增 `GUI diagnosis gate`：
  - 未设置 `INPUTIA_RUN_UI_SMOKE=1` 时输出 `safariInputSourceDiagnosisReady=false reason=ui-smoke-disabled` 并退出。
  - 调用 `inputia_require_gui_session`。
  - 调用 `inputia_require_safari_idle`，默认拒绝已有 Safari。
- 在打开 Safari 测试页前，校验 `inputia-tis-tool`/host select log 中同时存在 `selectStatus=0` 和 `selected=true`；否则输出 `safariInputSourceDiagnosisReady=false reason=input-source-not-selected` 并退出。
- 增加 trap 清理：
  - `/tmp/inputia-safari-input-source-test.url`
  - `/tmp/inputia-hitoolbox-preference.txt`
  - 仅当 Safari 原本未运行时兜底退出 Safari。

验证：

```text
bash -n macos/InputiaInputMethod/diagnose-safari-input-source.sh \
  macos/InputiaInputMethod/smoke-common.sh
  OK

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  OK

diagnose-safari-input-source.sh
  GUI diagnosis gate:
    guiSmokeReady=false reason=ui-smoke-disabled
    safariInputSourceDiagnosisReady=false reason=ui-smoke-disabled
    diagnoseUiDisabledRc=10

INPUTIA_RUN_UI_SMOKE=1 diagnose-safari-input-source.sh
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  safariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariInputSourceDiagnosisReady=false reason=safari-already-running
  diagnoseSafariExistingRc=11

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 diagnose-safari-input-source.sh
  safariPreflight=running
  guiSmokeReady=false reason=input-source-not-selected
  safariInputSourceDiagnosisReady=false reason=input-source-not-selected
  selectStatus=-50
  diagnoseInputSourceGateRc=12
  safariStillRunning=true
  safariDiagTempResidual=false

residue check
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true
  safariDiagTempResidual=false
  hitoolboxTempResidual=false
```

当前结论：

- `post-install-regression.sh` 的 Safari 诊断分支现在和 Safari smoke 共用同一套 GUI/Safari 安全门禁。
- 当前 Safari 是用户已有进程，诊断脚本不会强行关闭或复用它。
- 即使人工允许已有 Safari，当前 v40/TIS 状态下也会因 `selectStatus=-50` 在打开测试页前停止。

## v41 Mac mini 续修：smoke 临时日志清理

时间：2026-07-07 23:39 CST。

背景：TextEdit/Safari/Clipboard smoke 已能避免 GUI 进程和窗口残留，但 select/debug 日志仍使用固定 `/tmp/inputia-*` 路径。失败时日志内容会打印到 stdout 作为证据，磁盘文件不应默认残留；需要保留现场时再显式开启。

实现变更：

- `smoke-common.sh` 新增 `inputia_cleanup_smoke_files`：
  - 默认删除传入的临时文件。
  - 设置 `INPUTIA_KEEP_SMOKE_LOGS=1` 时跳过删除并输出 `smokeTempCleanup=skipped`。
- `smoke-textedit.sh`：
  - `SELECT_LOG=/tmp/inputia-textedit-select.log`。
  - trap 清理 select log 和 smoke 自己启动的 TextEdit。
- `smoke-clipboard-recall.sh`：
  - `SELECT_LOG=/tmp/inputia-clipboard-recall-select.log`。
  - trap 清理 select log、debug event log，恢复剪贴板并清理 TextEdit。
- `smoke-safari-typing.sh`：
  - `SELECT_LOG=/tmp/inputia-safari-typing-select.log`。
  - trap 清理 select log、测试 URL，并按 preflight 状态清理 Safari。
- `smoke-safari-enter.sh`：
  - `SELECT_LOG=/tmp/inputia-safari-enter-select.log`。
  - trap 清理 select log、debug event log、测试 URL，并按 preflight 状态清理 Safari。
- `diagnose-safari-input-source.sh`：
  - `SOURCE_SELECT_LOG=/tmp/inputia-safari-source-select.log`。
  - `FOCUSED_SELECT_LOG=/tmp/inputia-safari-focused-select.log`。
  - trap 清理 source/focused select log、Safari 诊断 URL、HIToolbox 临时文件，并按 preflight 状态清理 Safari。

验证：

```text
find /tmp -maxdepth 1 -name 'inputia-*' -print | sort
  <empty>

bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/diagnose-safari-input-source.sh
  OK

zsh -n macos/InputiaInputMethod/post-install-regression.sh \
  macos/InputiaInputMethod/status.sh
  OK

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  selectCurrentID=com.tencent.inputmethod.wetype.pinyin
  selectCurrentMatchesTarget=false
  texteditGateRc=15

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  selectStatus=-50
  selectCurrentID=com.tencent.inputmethod.wetype.pinyin
  selectCurrentMatchesTarget=false
  clipboardGateRc=8

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh
  safariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariTypingSmokeReady=false reason=safari-already-running
  safariTypingGateRc=9

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-enter.sh
  safariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  safariEnterSmokeReady=false reason=safari-already-running
  safariEnterGateRc=7

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 diagnose-safari-input-source.sh
  safariPreflight=running
  guiSmokeReady=false reason=input-source-not-selected
  safariInputSourceDiagnosisReady=false reason=input-source-not-selected
  selectStatus=-50
  selectCurrentID=com.tencent.inputmethod.wetype.pinyin
  selectCurrentMatchesTarget=false
  diagnoseGateRc=12

post-check
  find /tmp -maxdepth 1 -name 'inputia-*' -print | sort
    <empty>
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  TIS parent enabled=false selected=false
  TIS Hans enabled=true selected=false
```

当前结论：

- smoke/diagnose 失败路径会把关键 select 证据打印到 stdout，但默认不再留下 `/tmp/inputia-*` 文件。
- 当前系统仍是 v40，TIS 选择失败仍是预期阻塞；本轮只收紧测试清理纪律，没有执行系统安装或真实上屏输入。

## v41 Mac mini 续修：post-install UI smoke 总入口预检

时间：2026-07-07 23:42 CST。

背景：单条 TextEdit/Safari/Clipboard smoke 已经有各自门禁，但 `post-install-regression.sh` 在 `INPUTIA_RUN_UI_SMOKE=1` 时会串行调用多条 UI smoke。若 Safari 或 TextEdit 在中途才被某条脚本发现为用户已有进程，总入口可能已经先跑过前面的 UI smoke。需要在 UI smoke 分段开始前做一次全局前置检查。

实现变更：

- `post-install-regression.sh` 新增 `require_ui_process_idle process allow reason`。
- `INPUTIA_RUN_UI_SMOKE=1` 时，在任何具体 UI smoke 前新增 `UI smoke preflight`：
  - 检查 `TextEdit`，默认已有进程则输出 `postInstallUiSmokeReady=false reason=textedit-already-running` 并退出。
  - 检查 `Safari`，默认已有进程则输出 `postInstallUiSmokeReady=false reason=safari-already-running` 并退出。
  - 保留 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1` / `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1` 作为人工诊断逃生口。
- 通过后才继续 TextEdit、Safari diagnosis、Safari typing、Safari enter、Clipboard recall。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  OK

bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/diagnose-safari-input-source.sh
  OK

INPUTIA_RUN_UI_SMOKE=0 post-install-regression.sh
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true
  postInstallUiDisabledRc=0

INPUTIA_RUN_UI_SMOKE=1 post-install-regression.sh
  UI smoke preflight:
    TextEditPreflight=not-running
    SafariPreflight=running
    guiSmokeReady=false reason=safari-already-running
    postInstallUiSmokeReady=false reason=safari-already-running
    postInstallUiPreflightSequentialRc=4

post-check
  find /tmp -maxdepth 1 -name 'inputia-*' -print | sort
    <empty>
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  TIS parent enabled=false selected=false
  TIS Hans enabled=true selected=false
```

当前结论：

- post-install 总入口现在不会在 Safari/TextEdit 已运行时先跑部分 UI smoke。
- 当前桌面 Safari 是用户已有进程，所以 UI smoke 总入口在任何 TextEdit/Safari 自动化前正确停止。
- 本轮没有打开 TextEdit/Safari 新窗口，没有系统安装，也没有改变 v40 系统安装状态。

## v41 Mac mini 续修：常用 Command 快捷键透传自检

时间：2026-07-07 23:39 CST。

背景：用户发现 `Command-C`/`Command-V` 在 Inputia 下不可用，判断输入法接管了复制/粘贴，并要求举一反三处理常用电脑快捷键。依据 Apple 官方 Mac 键盘快捷键文档，先把包含 `Command` 的通用系统/App 快捷键作为输入法不应接管的命令面处理；输入法自身继续使用不含 Command 的组合，例如 `Control-Shift-V` 剪贴板召回。

实现变更：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 对含 `Command` 的键盘事件默认透传。
- `handleKeyDown` 在剪贴板召回、候选处理、标点/中英切换之前先检查 Command 透传。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 显式透传 AppKit command selector，例如 `copy:`, `paste:`, `cut:`, `undo:`, `redo:`, `selectAll:`, `saveDocument:`, `openDocument:`, `performClose:`, `terminate:`, `find:`, `print:`, `hide:`, `showHelp:`, `pasteAsPlainText:`。
- `didCommand(by:)` 在 newline/delete/candidate selector 处理前先检查 App command 透传。

验证：

```text
xcrun swiftc -parse-as-library \
  macos/InputiaInputMethod/Tools/InputiaShortcutSelfCheck.swift \
  macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift \
  -o /tmp/inputia-shortcut-selfcheck && /tmp/inputia-shortcut-selfcheck
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

xcrun swiftc -parse-as-library \
  macos/InputiaInputMethod/Tools/InputiaHostTextPolicySelfCheck.swift \
  macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaHostTextPolicy.swift \
  -o /tmp/inputia-host-policy-selfcheck && /tmp/inputia-host-policy-selfcheck
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandhidePassesThrough=true
  appCommandshowHelpPassesThrough=true
  appCommandpasteAsPlainTextPassesThrough=true
```

当前结论：

- 常用 Command 系统/App 快捷键不再由 Inputia 接管。
- `Control-Shift-V` 仍保留给 Inputia 剪贴板召回；带 Command 的 `Control-Shift-Command-V` 会被拒绝，避免覆盖系统/App 组合键。

## v41 Mac mini 续修：TIS 选择门禁收紧到当前目标输入源

时间：2026-07-07 23:44 CST。

背景：TextEdit/Safari/Clipboard smoke 以前用 `selectStatus=0` 加任意 `selected=true` 判断输入源选择成功。TIS 日志中会同时 dump 多个 source，宽泛匹配可能把非目标 source 或旧状态误判为待测 Hans mode 已选中。为避免真实 GUI smoke 误入错误输入源，需要让 TIS 工具在选择后读取当前键盘输入源，并让 smoke 只接受“当前输入源正是目标 source”的结果。

实现变更：

- `InputiaTISTool.select` 在 `TISSelectInputSource` 后新增：
  - `selectExpectedID=<target>`
  - `selectCurrentID=<current>`
  - `selectCurrentMatchesTarget=true|false`
- host fallback 的 `--select-input-source` 路径同步输出上述三项。
- `smoke-common.sh` 新增 `inputia_select_input_source_or_exit`，统一执行 TIS 选择并要求：
  - `selectStatus=0`
  - `selectCurrentMatchesTarget=true`
- `smoke-textedit.sh`、`smoke-clipboard-recall.sh`、`smoke-safari-typing.sh`、`smoke-safari-enter.sh`、`diagnose-safari-input-source.sh` 改用公共选择门禁。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/diagnose-safari-input-source.sh
  OK

xcrun swiftc -parse-as-library macos/InputiaInputMethod/Tools/InputiaTISTool.swift \
  -framework Carbon -o /tmp/inputia-tis-tool-check
  OK

xcrun swiftc -parse-as-library ... Sources/InputiaInputMethod/main.swift ... \
  crates/inputia-capi/target/release/libinputia_capi.a \
  -framework Cocoa -framework InputMethodKit -o /tmp/inputia-host-compile-check
  OK

./macos/InputiaInputMethod/build.sh
  build OK
  appCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c

./macos/InputiaInputMethod/build-pkg.sh
  pkg=dist/InputiaInputMethod-v41-c64e1a5618cb.pkg
  latestSha256=898175b6154dc1322deeccfddfb5b3768ad79bf7833d980bb55fa313b208a0c3
  appCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  pkgSignature=none

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true

macos/InputiaInputMethod/status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  system host version=40
  systemMatchesBuild=false
  user host exists=false
  latest pkg sha256=898175b6154dc1322deeccfddfb5b3768ad79bf7833d980bb55fa313b208a0c3
  TIS includeAllInstalled=false matches=0
  TIS includeAllInstalled=true matches=3

INPUTIA_RUN_UI_SMOKE=0 post-install-regression.sh build/InputiaInputMethod.app
  userHostConflict=false
  bridgeSelfCheck=true
  hostShortcutSelfCheck=true
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-safari-typing.sh build/InputiaInputMethod.app
  guiSmokeReady=false reason=ui-smoke-disabled
  safariTypingSmokeReady=false reason=ui-smoke-disabled
  rc=7

smoke-textedit.sh
  expectedCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  textEditSmokeReady=false reason=cdhash-mismatch
  rc=2

smoke-clipboard-recall.sh
  expectedCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  clipboardRecallSmokeReady=false reason=cdhash-mismatch
  rc=2

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
```

当前结论：

- GUI smoke 的输入源选择门禁不再依赖宽泛 `selected=true`，必须证明当前键盘输入源就是目标 Hans mode。
- build/pkg 已刷新到 `c64e1a5618cb...`，但系统安装仍是 v40，因此真实 TextEdit/Safari/Clipboard GUI smoke 继续正确停在 cdhash/TIS 前置门禁。
- 本轮没有安装系统包，没有打开 TextEdit/Safari，没有留下残留进程。

## v41 Mac mini 续修：post-install regression 互斥锁

时间：2026-07-07 23:45 CST。

背景：并行运行 `post-install-regression.sh` 会同时触发 Rime bridge self-check，可能让 Rime userdb 出现 `LOCK: Resource temporarily unavailable`。这不是产品输入路径失败，但会污染回归证据。post-install 总入口应当串行，避免同一台机器上重复运行 regression 互相干扰。

实现变更：

- `post-install-regression.sh` 新增本地互斥锁：
  - 默认锁目录：`/tmp/inputia-post-install-regression.lock`。
  - 可通过 `INPUTIA_POST_INSTALL_LOCK_DIR` 覆盖。
  - 成功加锁后写入 `pid`，EXIT trap 删除锁目录。
  - 若锁目录存在且 `pid` 仍在运行，输出 `postInstallRegressionReady=false reason=already-running pid=<pid>` 并以 rc=5 退出。
  - 若锁目录存在但 pid 不存在，输出 `postInstallLockStale=true ...`，删除 stale lock 后继续。
  - 保留 `INPUTIA_SKIP_POST_INSTALL_LOCK=1` 作为诊断逃生口。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  OK

pre-created live lock
  postInstallRegressionReady=false reason=already-running pid=<shell-pid>
  lockedRegressionRc=5
  lock dir removed by test cleanup

INPUTIA_RUN_UI_SMOKE=0 post-install-regression.sh
  bridgeSelfCheck=true
  bridgeMemorySelfCheck=true
  bridgeSettingsSelfCheck=true
  bridgeDefaultChineseSelfCheck=true
  hostShortcutSelfCheck=true
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true
  unlockedRegressionRc=0

post-check
  find /tmp -maxdepth 1 \( -name 'inputia-*' -o -name 'inputia-post-install-regression.lock' \) -print | sort
    <empty>
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  TIS parent enabled=false selected=false
  TIS Hans enabled=true selected=false
```

当前结论：

- post-install regression 不再允许同机并发运行，避免自检之间抢 Rime userdb lock。
- 正常非 UI regression 仍通过，退出后不留下 lock 目录、`/tmp/inputia-*`、TextEdit、osascript、Inputia host 或 debug env。

## v41 Mac mini 续修：安装等待脚本增加 TIS ready 诊断

时间：2026-07-07 23:50 CST。

背景：真实 GUI smoke 的下一道门槛是系统安装 v41 后 TIS 是否刷新到目标 Inputia Hans mode。此前 `await-system-install.sh` 只等待目标 app CDHash 匹配；如果安装后 TIS 缓存仍指向旧包、enabled list 为空、或当前输入源没切到目标，脚本会直接进入 regression，后续失败原因不够集中。需要把“系统 app 已替换”和“TIS 已可用”拆开观察。

实现变更：

- `await-system-install.sh` 每轮输出：
  - `expectedTISModeID=com.inputia.inputmethod.Inputia.Hans`
  - `expectedTISIcon=<target>/Contents/Resources/inputia.pdf`
  - `tis.enabledMatches`
  - `tis.installedMatches`
  - `tis.hansIconMatchesApp`
  - `tis.hansEnabled`
  - `tis.hansSelected`
  - `tis.currentID`
  - `tis.currentMatchesTarget`
- 等待成功条件从“目标 app CDHash 匹配”收紧为：
  - 目标 app CDHash 等于 build CDHash。
  - TIS enabled list 中出现 Inputia source。
  - Hans mode 的 iconURL 指向目标 app。
  - Hans mode 为 enabled。
- 不存在的用户级 app 不再把 PlistBuddy 的 `File Doesn't Exist` 文本写入 version 字段。

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  OK

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
  macos/InputiaInputMethod/await-system-install.sh
  expectedVersion=41
  expectedCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  expectedTISModeID=com.inputia.inputmethod.Inputia.Hans
  expectedTISIcon=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  target.version=40
  target.matchesBuild=false
  user.exists=false
  user.version=
  running.exists=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  systemInstallObserved=false reason=timeout
  rc=2

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
  macos/InputiaInputMethod/await-system-install.sh \
  macos/InputiaInputMethod/build/InputiaInputMethod.app
  target.matchesBuild=true
  system.version=40
  system.matchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconMatchesApp=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  systemInstallObserved=false reason=timeout
  rc=2

macos/InputiaInputMethod/status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  system host version=40
  systemMatchesBuild=false
  user host exists=false
  TIS includeAllInstalled=false matches=0
  TIS includeAllInstalled=true matches=3
  running=false
```

当前结论：

- 安装等待脚本现在能区分 app 替换、TIS 注册/启用、Hans icon 指向和当前输入源状态。
- 当前系统仍是 v40，TIS enabled list 对 Inputia 为 0，当前源是微信输入法；因此继续不跑真实 GUI smoke。
- 本轮没有安装系统包，也没有启动 TextEdit/Safari。

## v41 Mac mini 续修：安装等待阶段输出 UI smoke preflight

时间：2026-07-07 23:47 CST。

背景：`await-system-install.sh` 已经能拆分 app 替换和 TIS ready，但安装完成后还会进入 `post-install-regression.sh`。如果用户已有 Safari/TextEdit 进程，post-install 总入口会拒绝 UI smoke；等待阶段也应提前输出这个状态，避免安装成功后才发现 GUI smoke 不会启动。该检查只读进程状态，不打开 GUI。

实现变更：

- `await-system-install.sh` 新增 `process_preflight` 和 `ui_smoke_status_line`。
- 每轮等待都输出：
  - `uiSmokeRequested=true|false`
  - `uiTextEditPreflight=running|not-running`（仅 UI smoke requested 时）
  - `uiSafariPreflight=running|not-running`（仅 UI smoke requested 时）
  - `uiSmokeWouldStart=true|false`
  - `uiSmokeBlockReason=<reason>`
- `INPUTIA_RUN_UI_SMOKE` 未启用时输出 `uiSmokeBlockReason=ui-smoke-disabled`。
- `INPUTIA_RUN_UI_SMOKE=1` 时，按 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING` / `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING` 判断是否会被已有进程挡住。

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  OK

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 await-system-install.sh
  target.version=40
  target.matchesBuild=false
  user.exists=false
  running.exists=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  uiSmokeRequested=false
  uiSmokeWouldStart=false
  uiSmokeBlockReason=ui-smoke-disabled
  systemInstallObserved=false reason=timeout
  awaitUiDisabledRc=2

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 await-system-install.sh
  target.version=40
  target.matchesBuild=false
  user.exists=false
  running.exists=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  uiSmokeRequested=true
  uiTextEditPreflight=not-running
  uiSafariPreflight=running
  uiSmokeWouldStart=false
  uiSmokeBlockReason=safari-already-running
  systemInstallObserved=false reason=timeout
  awaitUiRequestedRc=2

post-check
  find /tmp -maxdepth 1 \( -name 'inputia-*' -o -name 'inputia-post-install-regression.lock' \) -print | sort
    <empty>
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true
```

当前结论：

- 安装等待阶段现在会提前说明 UI smoke 是否会被 Safari/TextEdit 已有进程挡住。
- 当前 Mac mini 上 Safari 正在运行，所以即使未来安装完成并请求 UI smoke，等待输出也会提示 `uiSmokeWouldStart=false uiSmokeBlockReason=safari-already-running`。
- 本轮没有安装系统包，没有启动 TextEdit/Safari，没有留下临时文件或进程残留。

## v41 Mac mini 续修：post-install regression 增加 TIS GUI readiness 门禁

时间：2026-07-07 23:56 CST。

背景：`post-install-regression.sh` 在 `INPUTIA_RUN_UI_SMOKE=0` 时会跳过 GUI smoke，但此前非 UI 回归没有独立报告“当前 TIS 是否足以进入 GUI smoke”。这会让系统安装后状态不稳定时，难以区分“基础 bundle/bridge 正常”与“TextEdit/Safari 可以开始真实上屏验证”。需要在 regression 中增加只读 TIS readiness 段，并且当 `INPUTIA_RUN_UI_SMOKE=1` 时，如果 TIS 未 ready，应在打开 TextEdit/Safari 前失败。

实现变更：

- `post-install-regression.sh` 新增 `TIS readiness for GUI smoke` section，输出：
  - `expectedTISModeID`
  - `expectedTISIcon`
  - `tis.enabledMatches`
  - `tis.installedMatches`
  - `tis.hansIconURL`
  - `tis.hansIconMatchesApp`
  - `tis.hansEnabled`
  - `tis.hansSelected`
  - `postInstallTISReady`
- UI smoke 启用时，如果 `postInstallTISReady=false`，脚本输出：
  - `guiSmokeReady=false reason=tis-not-ready`
  - `postInstallUiSmokeReady=false reason=tis-not-ready`
  - exit `6`
- 该 readiness 检查只读 TIS 状态，不切换当前输入源。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh \
  macos/InputiaInputMethod/await-system-install.sh
  OK

INPUTIA_RUN_UI_SMOKE=0 \
  macos/InputiaInputMethod/post-install-regression.sh \
  macos/InputiaInputMethod/build/InputiaInputMethod.app
  inputiaInstalled=true
  legacyIputiaPresent=false
  userHostConflict=false
  bridgeSelfCheck=true
  hostShortcutSelfCheck=true
  expectedTISModeID=com.inputia.inputmethod.Inputia.Hans
  expectedTISIcon=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  tis.hansIconMatchesApp=false
  tis.hansEnabled=true
  tis.hansSelected=false
  postInstallTISReady=false
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true

INPUTIA_RUN_UI_SMOKE=1 \
  macos/InputiaInputMethod/post-install-regression.sh \
  macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallTISReady=false
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready
  rc=6

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariAppStillRunning=true
```

当前结论：

- post-install regression 现在把 “bundle/bridge 自检通过” 和 “TIS 已可进入 GUI smoke” 分开报告。
- 当前 build app 与系统 TIS 仍不一致：TIS Hans icon 指向 `/Library/Input Methods/InputiaInputMethod.app` 的 v40，而非 build app，所以 UI=1 时会在打开 TextEdit/Safari 前退出。
- Safari 进程为用户已有会话；本轮没有新开 TextEdit/Safari，也没有留下 Inputia 或 osascript 残留。

## v41 Mac mini 续修：post-install TIS readiness 补齐当前输入源

时间：2026-07-07 23:50 CST。

背景：`await-system-install.sh` 已输出 `tis.currentID` / `tis.currentMatchesTarget`，但 `post-install-regression.sh` 的 `TIS readiness for GUI smoke` 只报告 Hans mode 是否注册/启用/指向目标 app。为了让等待脚本和回归脚本的证据口径一致，post-install 也应报告当前键盘输入源，但不把“当前已选中”作为进入 GUI smoke 的必要条件，因为后续 smoke 会显式选择目标输入源。

实现变更：

- `post-install-regression.sh` 新增 `current_source_id`。
- `TIS readiness for GUI smoke` 新增只读输出：
  - `tis.currentID`
  - `tis.currentMatchesTarget`
- `postInstallTISReady` 判定保持不变：仍要求 enabled matches 非 0、Hans icon 指向目标 app、Hans enabled=true；不要求当前输入源已选中。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh \
  macos/InputiaInputMethod/await-system-install.sh
  OK

INPUTIA_RUN_UI_SMOKE=1 post-install-regression.sh build/InputiaInputMethod.app
  TIS readiness for GUI smoke:
    expectedTISModeID=com.inputia.inputmethod.Inputia.Hans
    expectedTISIcon=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/Resources/inputia.pdf
    tis.enabledMatches=0
    tis.installedMatches=3
    tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
    tis.hansIconMatchesApp=false
    tis.hansEnabled=true
    tis.hansSelected=false
    tis.currentID=com.tencent.inputmethod.wetype.pinyin
    tis.currentMatchesTarget=false
    postInstallTISReady=false
    guiSmokeReady=false reason=tis-not-ready
    postInstallUiSmokeReady=false reason=tis-not-ready
    postInstallBuildUiTisGateSequentialRc=6

INPUTIA_RUN_UI_SMOKE=0 post-install-regression.sh build/InputiaInputMethod.app
  TIS readiness for GUI smoke:
    tis.currentID=com.tencent.inputmethod.wetype.pinyin
    tis.currentMatchesTarget=false
    postInstallTISReady=false
  UI smoke:
    uiSmokeSkipped=true reason=disabled
    postInstallBuildUiDisabledExtractRc=0

post-check
  find /tmp -maxdepth 1 \( -name 'inputia-*' -o -name 'inputia-post-install-regression.lock' \) -print | sort
    <empty>
  safariStillRunning=true
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  debugEnvUnset=true

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  systemVersion=40
  systemCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  TIS includeAllInstalled=false matches=0
  TIS includeAllInstalled=true matches=3
```

当前结论：

- await 和 post-install 现在都会报告当前输入源是否已是目标 Hans mode。
- 当前输入源仍是微信输入法，且 build app 与系统 TIS icon 不一致；GUI smoke 仍会在打开 TextEdit/Safari 前停在 `tis-not-ready`。
- 本轮没有切换输入源、没有打开 GUI、没有留下临时文件或进程残留。

## v41 Mac mini 续修：pkg 内容一致性验证

时间：2026-07-08 00:02 CST。

背景：当前系统安装仍是 v40，下一步真实 smoke 需要安装 `dist/InputiaInputMethod-latest.pkg`。在安装前需要证明 pkg 内嵌的 app、settings launcher 和 postinstall 与当前 build/source 一致，避免安装后才发现 pkg 仍携带旧 postinstall 或旧 app archive。

实现变更：

- 新增 `verify-pkg.sh`，只读展开 pkg 到临时目录并验证：
  - `PackageInfo` version 等于 build `CFBundleVersion`。
  - pkg 内 `Scripts/postinstall` 可执行，且 sha256 等于 `Packaging/scripts/postinstall`。
  - postinstall 包含关键安装后动作：`INPUTIA_POSTINSTALL_SKIP_TIS`、用户级 host 清理、`inputia-select`、`TextInputMenuAgent` 和 `SystemUIServer` 刷新。
  - pkg 内 `InputiaInputMethod.app.tar.gz` 和 `InputiaSettings.app.tar.gz` 存在。
  - 解出的 host app version/CDHash 等于当前 build app。
  - 解出的 settings launcher version 等于当前 build settings launcher。
  - 解出的 host/settings 可执行文件存在。

验证：

```text
zsh -n macos/InputiaInputMethod/verify-pkg.sh
  OK

macos/InputiaInputMethod/verify-pkg.sh
  path=dist/InputiaInputMethod-latest.pkg
  sha256=898175b6154dc1322deeccfddfb5b3768ad79bf7833d980bb55fa313b208a0c3
  pkgSignature=none
  packageVersion=41
  buildVersion=41
  postinstallExecutable=true
  sourcePostinstallSHA256=8f442c9209e90db8c7c56727b11c6d07c3a851f71ea96f20728be4e0f5bfe7f8
  pkgPostinstallSHA256=8f442c9209e90db8c7c56727b11c6d07c3a851f71ea96f20728be4e0f5bfe7f8
  postinstallBehaviorChecks=true
  appArchivePresent=true
  settingsArchivePresent=true
  archiveAppVersion=41
  archiveAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  buildAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  archiveSettingsVersion=41
  buildSettingsVersion=41
  archiveExecutables=true
  pkgVerificationPassed=true

INPUTIA_RUN_UI_SMOKE=0 post-install-regression.sh build/InputiaInputMethod.app
  postInstallTISReady=false
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 await-system-install.sh
  target.version=40
  target.matchesBuild=false
  tis.enabledMatches=0
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  uiSmokeRequested=false
  uiSmokeWouldStart=false
  uiSmokeBlockReason=ui-smoke-disabled
  systemInstallObserved=false reason=timeout
  rc=2

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
```

当前结论：

- `InputiaInputMethod-latest.pkg` 已证明携带当前 v41 host app、settings launcher 和最新 postinstall。
- pkg 仍未签名，这是当前本地 MVP 包装状态；安装验证继续依赖 CDHash、version 和 postinstall 行为证据。
- 当前系统仍是 v40，本轮没有安装系统包，也没有启动 TextEdit/Safari。

## v41 Mac mini 续修：README 验证纪律更新

时间：2026-07-08 00:08 CST。

背景：`README.md` 仍有几处旧验证说明：安装后回归看起来像默认会打开 Safari，历史 v4/v7 包号容易被误读成当前事实。当前 smoke 纪律已经改为“默认非 GUI，显式 `INPUTIA_RUN_UI_SMOKE=1` 才打开 TextEdit/Safari”，并且安装前必须先验证 latest pkg 内容一致性，因此 README 需要同步。

实现变更：

- README 新增 `verify-pkg.sh` 说明，明确它只展开 pkg、不安装系统包，并验证 PackageInfo、postinstall sha、内嵌 app/settings 和关键 postinstall 行为。
- README 更新 `smoke-textedit.sh` 说明：默认不会打开 TextEdit；必须显式 `INPUTIA_RUN_UI_SMOKE=1`，并通过图形会话、TextEdit idle、TIS 当前源门禁后才跑真实输入。
- README 更新 `await-system-install.sh` 说明：等待 app CDHash 和 TIS ready，并默认不打开 TextEdit/Safari。
- README 更新 `post-install-regression.sh` 说明：默认只跑非 GUI 回归并报告 `postInstallTISReady`；只有显式 UI smoke 且 TIS ready 才打开 TextEdit/Safari 本地测试页。
- README 新增安装前/安装后建议验证顺序。
- README 清理旧 `v7` 固定包号，改为以当前 `InputiaInputMethod-v<version>-<cdhash>.pkg` / `latest.pkg` 为准。
- README 将“当前机器已验证”改为“历史已验证能力”，并提示当前事实以 `status.sh` 和 `EVIDENCE.md` 末尾为准。

验证：

```text
rg -n "v4|v7|会打开一个 Safari|InputiaInputMethod-v7|系统安装版本 v" \
  macos/InputiaInputMethod/README.md
  only local code-signing identity v4 mentions remain

zsh -n verify-pkg.sh await-system-install.sh post-install-regression.sh status.sh
  OK

verify-pkg.sh
  sha256=1c37f577e136157bb456f83e872f45d03037a711a6c14ef0a0c478d2f6842d16
  packageVersion=41
  buildVersion=41
  archiveAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  buildAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  pkgVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  running=false
  latest pkg sha256=1c37f577e136157bb456f83e872f45d03037a711a6c14ef0a0c478d2f6842d16

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
```

当前结论：

- README 已对齐当前非 GUI 默认验证纪律和 pkg 一致性验证入口。
- 当前 latest pkg sha256 为 `1c37f577e136157bb456f83e872f45d03037a711a6c14ef0a0c478d2f6842d16`，host CDHash 仍为 `c64e1a5618cb...`。
- 系统安装仍是 v40；真实 GUI smoke 仍等待 v41 系统安装和 TIS ready。

## v41 Mac mini 续修：真实 GUI smoke 只读预检入口

时间：2026-07-08 00:15 CST。

背景：TextEdit/Safari/Clipboard smoke 各自有前置门禁，但安装后手动组合命令仍容易误跑 GUI。需要一个只读预检入口，集中报告“现在是否允许进入真实 GUI smoke”，并且不切换输入源、不打开 TextEdit/Safari。

实现变更：

- 新增 `smoke-preflight.sh`：
  - 校验目标 app 可执行。
  - 校验目标 app CDHash 与当前 build app 一致。
  - 默认要求显式 `INPUTIA_RUN_UI_SMOKE=1`。
  - 检查图形会话已登录且未锁屏。
  - 检查 TextEdit/Safari 是否已有用户进程。
  - 只读检查 TIS Hans mode 是否出现在 enabled list、icon 是否指向目标 app、Hans 是否 enabled、当前输入源 ID。
  - 成功时输出 `guiSmokeReady=true` 和 `smokePreflightReady=true`。
- README 新增 `smoke-preflight.sh` 说明，并将它加入安装前/安装后建议验证顺序。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-preflight.sh \
  macos/InputiaInputMethod/smoke-common.sh
  OK

macos/InputiaInputMethod/smoke-preflight.sh
  app=/Library/Input Methods/InputiaInputMethod.app
  expectedCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  actualCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  guiSmokeReady=false reason=cdhash-mismatch
  smokePreflightReady=false reason=cdhash-mismatch
  rc=2

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  macos/InputiaInputMethod/smoke-preflight.sh \
  macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  safariPreflight=running
  expectedTISModeID=com.inputia.inputmethod.Inputia.Hans
  expectedTISIcon=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  tis.hansIconMatchesApp=false
  tis.hansEnabled=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  guiSmokeReady=false reason=tis-not-ready
  smokePreflightReady=false reason=tis-not-ready
  rc=8

rg -n "smoke-preflight|smokePreflightReady" README.md
  README references present

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
```

当前结论：

- 真实 GUI smoke 现在有独立只读预检入口；后续安装 v41 后，应先得到 `smokePreflightReady=true`，再运行 TextEdit/Safari/Clipboard smoke。
- 当前系统安装仍是 v40，且 TIS icon 指向系统旧包；预检会在打开 GUI 前停住。
- 本轮没有切换输入源，没有由 smoke 打开 TextEdit/Safari。

补充检查：

```text
final process check
  osascriptResidual=false
  inputiaRunning=false
  smokeResidual=false
  textEditRunning=true pid=48414 started=Tue Jul 7 23:57:05 2026
```

说明：最终检查时环境中已有 TextEdit 进程；本轮没有关闭用户窗口。后续真实 GUI smoke 仍会因为 `textedit-already-running` 默认拒绝，除非用户关闭 TextEdit 或显式设置 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1`。

## v41 Mac mini 续修：await TIS mode 配置对齐

时间：2026-07-07 23:54 CST。

背景：`post-install-regression.sh` 已支持通过 `INPUTIA_TIS_MODE_ID` 覆盖目标输入源 mode，但 `await-system-install.sh` 仍硬编码 `com.inputia.inputmethod.Inputia.Hans`。这会让安装等待阶段和安装后回归阶段在验证 Hant 或其它 mode 时产生配置漂移。

实现变更：

- `await-system-install.sh` 的 `TARGET_MODE_ID` 改为 `${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}`。
- 默认行为不变，仍以 Hans 作为当前 MVP 的目标 mode。
- 该变更只影响等待/诊断脚本的目标 mode 选择，不安装系统包，不启动 TextEdit/Safari。

验证：

```text
zsh -n await-system-install.sh post-install-regression.sh verify-pkg.sh status.sh build-pkg.sh
  OK

verify-pkg.sh
  sha256=1c37f577e136157bb456f83e872f45d03037a711a6c14ef0a0c478d2f6842d16
  packageVersion=41
  buildVersion=41
  archiveAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  buildAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  pkgVerificationPassed=true

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
INPUTIA_TIS_MODE_ID=com.inputia.inputmethod.Inputia.Hans await-system-install.sh
  expectedVersion=41
  expectedCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  expectedTISModeID=com.inputia.inputmethod.Inputia.Hans
  target.version=40
  target.matchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  uiSmokeRequested=false
  uiSmokeWouldStart=false
  uiSmokeBlockReason=ui-smoke-disabled
  systemInstallObserved=false reason=timeout
  rc=2

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  system host version=40
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  running=false
  latest pkg sha256=1c37f577e136157bb456f83e872f45d03037a711a6c14ef0a0c478d2f6842d16
```

当前结论：

- `await-system-install.sh` 与 `post-install-regression.sh` 的目标 TIS mode 配置已对齐。
- 当前 pkg 内容一致性仍通过，latest pkg 是 v41 / CDHash `c64e1a5618cb...`。
- 当前系统安装仍是 v40，未触发 GUI smoke；TextEdit/Safari 清理纪律仍等待 v41 安装后继续验证。

## v41 Mac mini 续修：TextEdit preflight 与 clipboard recall 状态门禁

时间：2026-07-07 23:59 CST。

背景：继续修 GUI smoke 测试纪律时，发现两个脚本级问题：

- `inputia_textedit_document_count` 使用 `tell application "TextEdit"` 查询文档数，即使先判断进程，也可能在 AppleScript 编译/执行时拉起 TextEdit；一次验证中还留下了卡住的 `osascript`。
- `smoke-clipboard-recall.sh` 在触发 `Ctrl+Shift+V` 后直接按 Space，不能区分“召回候选已显示后提交”和“Space 误提交了残留 composition/旧状态”。

实现变更：

- `smoke-common.sh`
  - `inputia_textedit_document_count` 改为通过 System Events 查询 `application process "TextEdit"` 的窗口数，不再直接 `tell application "TextEdit"`。
  - 查询由 Python `subprocess.run(..., timeout=2)` 包裹，避免 `osascript` 卡住后污染后续 smoke。
- `smoke-clipboard-recall.sh`
  - 默认 debug event log 改为 `/tmp/inputia-clipboard-recall-events.$$.log`，减少并发或旧日志污染。
  - AppleScript handler 调整为顶层定义，主流程放入 `on run argv`，已用 `osacompile` 验证。
  - 清空状态后先确认 TextEdit 文档仍为空，再触发 clipboard recall。
  - 按 Space 提交前等待本次 event log 中出现 `clipboardRecallShown`。
  - commit 事件校验从只看 `clipboardRecallCommit index=0` 收紧为同时匹配 `text=$expected`。

验证：

```text
bash -n smoke-common.sh smoke-clipboard-recall.sh smoke-textedit.sh
  OK

osacompile extracted smoke-clipboard-recall AppleScript
  osacompileRc=0

bash -c 'source smoke-common.sh; inputia_textedit_document_count; inputia_textedit_document_count'
  0
  0
  docCountRc=0

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh
  guiSmokeReady=false reason=ui-smoke-disabled
  clipboardRecallSmokeReady=false reason=ui-smoke-disabled
  rc=7

INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh
  guiSmokeReady=false reason=ui-smoke-disabled
  textEditSmokeReady=false reason=ui-smoke-disabled
  rc=14

verify-pkg.sh
  packageVersion=41
  buildVersion=41
  archiveAppCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  pkgVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=c64e1a5618cb5545f58b21af380a00379ad55c7c
  system host version=40
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  latest pkg sha256=1c37f577e136157bb456f83e872f45d03037a711a6c14ef0a0c478d2f6842d16

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 await-system-install.sh
  target.version=40
  target.matchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  uiSmokeRequested=false
  systemInstallObserved=false reason=timeout
  rc=2

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
```

当前结论：

- TextEdit preflight 不再通过 AppleScript 拉起 TextEdit，也不会留下卡住的 `osascript`。
- Clipboard recall smoke 现在必须先看到 `clipboardRecallShown` 才会按 Space，并校验 commit 事件属于本次 expected 文本。
- 当前系统仍是 v40，TIS 当前 dump 已降为 installed matches=0；真实 GUI smoke 仍应等待 v41 系统安装/TIS ready 后再跑。
- Safari 仍是既有运行进程；本轮未启动 Safari GUI smoke。

## v41 常用 macOS 快捷键穿透补强

背景：用户反馈在 Inputia 下 `Command-C` / `Command-V` 不能正常复制粘贴，并要求不要逐个依赖人工发现；常用电脑快捷键应默认不被输入法接管。依据 Apple 官方 Mac keyboard shortcuts 文档，`Command` 是 macOS 常用 App/System 快捷键主修饰键，复制/粘贴/剪切/撤销/全选/保存/打开/查找/打印/隐藏/切换 App/Spotlight/强制退出等都应交还宿主或系统处理。

实现变更：

- 保持 `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 对所有包含 `Command` 的 `keyDown` 事件返回穿透。
- 扩展 `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 的显式 selector 白名单，覆盖常见编辑、文件、窗口、查找、格式和浏览命令：
  - 编辑：`copy:`, `paste:`, `cut:`, `undo:`, `redo:`, `selectAll:`, `pasteAsPlainText:`
  - 文件/App：`newDocument:`, `openDocument:`, `saveDocument:`, `saveDocumentAs:`, `saveDocumentTo:`, `duplicateDocument:`, `print:`, `showPreferences:`, `terminate:`
  - 查找：`find:`, `findNext:`, `findPrevious:`, `orderFrontFindPanel:`
  - 窗口：`performClose:`, `miniaturize:`, `performMiniaturize:`, `performZoom:`, `toggleFullScreen:`, `toggleToolbarShown:`, `toggleSidebar:`
  - 格式：`toggleBold:`, `toggleItalic:`, `toggleUnderline:`
  - 浏览：`goBack:`, `goForward:`, `reload:`, `stopLoading:`
- 保留 Inputia 明确快捷键：
  - `Control-Shift-V`：剪贴板召回。
  - `Control-.`：中/英文标点切换。
  - `Shift-Space`：全/半角切换。
  - 可配置的 `Shift` 或 `Control-Space`：中英文输入模式切换。
- 仍拒绝带 `Command` 的 Inputia 变体，例如 `Control-Shift-Command-V`，避免覆盖系统/App 组合键。

验证：

```text
swiftc -parse InputiaHostTextPolicy.swift InputiaHostTextPolicySelfCheck.swift
  OK

swiftc -parse InputiaShortcutClassifier.swift InputiaShortcutSelfCheck.swift
  OK

build.sh
  rc=0
  注：仍有既有 ld warning：Rust std/object files built for newer macOS 26.0 than linked 13.0。

inputia-shortcut-self-check
  shortcutSelfCheck=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandOptionEscapePassThrough=true

inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandfindNextPassesThrough=true
  appCommandfindPreviousPassesThrough=true
  appCommandshowPreferencesPassesThrough=true
  appCommandnewDocumentPassesThrough=true
  appCommandsaveDocumentAsPassesThrough=true
  appCommandsaveDocumentToPassesThrough=true
  appCommandperformMiniaturizePassesThrough=true
  appCommandperformZoomPassesThrough=true
  appCommandtoggleFullScreenPassesThrough=true
  appCommandtoggleBoldPassesThrough=true
  appCommandtoggleItalicPassesThrough=true
  appCommandtoggleUnderlinePassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandgoForwardPassesThrough=true
  appCommandreloadPassesThrough=true
  appCommandstopLoadingPassesThrough=true

status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
```

当前结论：

- 代码层已把包含 `Command` 的快捷键统一穿透给系统/宿主 App，且把常见 AppKit command selector 显式列入穿透白名单。
- 这不是只修 `Command-C/V`；自检覆盖了复制、粘贴、剪切、撤销、重做、全选、保存、打开、关闭、退出、查找、偏好设置、窗口、格式、浏览导航等常见类别。
- 真实 GUI smoke 仍未运行，因为系统安装版与当前构建不一致，`systemMatchesBuild=false` 且 TIS matches=0；继续等待系统安装/TIS ready 后验证宿主 TextEdit/Safari 真实行为。

## v41 Mac mini 续修：无弹框安装门禁与 pkg 重新对齐

时间：2026-07-08 00:04 CST。

背景：当前 Mac mini 上 build app 已是 v41，但 `/Library/Input Methods/InputiaInputMethod.app` 仍是 v40；真实 GUI smoke 会因 CDHash mismatch 正确拒绝。继续推进时确认当前用户没有非交互管理员权限：

```text
user=lizhelang
inputMethodsWritable=false
sudoCached=false
```

因此不能在自动化里调用会弹系统密码框的安装路径，否则会破坏“不抢 GUI/不等待人工确认”的 smoke 纪律。

实现变更：

- `install-system.sh`
  - 新增 `INPUTIA_INSTALL_NO_ADMIN_PROMPT=1` 模式。
  - 当 `/Library/Input Methods` 不可写且 `sudo -n` 不可用时，立即输出：
    - `systemInstallNeedsAdmin=true`
    - `systemInstallReady=false reason=admin-required`
    - exit `12`
  - 该门禁已前移到 build、kill、LaunchServices unregister、user host cleanup 之前，避免无权限时仍产生副作用。
  - 若后续存在 sudo 缓存，则 no-prompt 模式会走 `sudo -n /bin/zsh -c "$copy_command"`，仍不会弹密码框。
- `verify-nongui.sh`
  - 新增 `admin install no-prompt gate`，当前机器无 sudo 缓存时要求 `install-system.sh` 返回 `12`。

验证中发现并处理：

- 一次 no-prompt 验证触发了 build，使当前 build app CDHash 从旧 `c64e1a5618cb...` 变为 `4a96b63c4c...`；旧 latest pkg 因 archive CDHash 仍是 `c64e...` 被 `verify-pkg.sh` 正确判定为 stale。
- 随后重新运行 `build-pkg.sh`，生成并验证当前包：
  - `dist/InputiaInputMethod-v41-4a96b63c4c30.pkg`
  - latest sha256=`af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef`
  - `pkgVerificationPassed=true`

验证：

```text
zsh -n install-system.sh
  OK

INPUTIA_INSTALL_NO_ADMIN_PROMPT=1 install-system.sh
  systemInstallNeedsAdmin=true
  systemInstallReady=false reason=admin-required
  rc=12
  no build output
  no userHostRemoved output

build-pkg.sh
  appCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  pkgVerificationPassed=true
  output=/Users/minizl/services/Handy/macos/InputiaInputMethod/dist/InputiaInputMethod-v41-4a96b63c4c30.pkg

verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  postInstallRegressionPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  nonGuiVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  system host version=40
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  latest pkg sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
```

当前结论：

- 当前自动化不会在无管理员权限时弹系统密码框，也不会在权限失败前 build/kill/清 user host。
- latest pkg 已重新对齐当前 build v41 / CDHash `4a96b63c4c...`。
- 系统安装仍是 v40，TIS matches 仍为 0；真实 TextEdit/Safari/Clipboard GUI smoke 仍不能跑，必须等系统安装 v41 且 TIS ready。
- Safari 仍是既有运行进程；本轮未启动或关闭 Safari。

## v41 Mac mini 续修：AppleScript 编译检查默认禁用

时间：2026-07-08 00:08 CST。

背景：上轮手工用 `osacompile` 抓到了 clipboard smoke 的 AppleScript handler 结构错误，因此尝试把 quoted AppleScript heredoc 编译检查纳入 `verify-nongui.sh`。但本轮实测发现，`osacompile` 编译包含 `tell application "TextEdit"` 的脚本时会拉起 TextEdit：

```text
texteditBefore=false
compile smoke-common -> texteditAfter=true
compile smoke-textedit -> texteditAfter=true
compile smoke-clipboard-recall -> texteditAfter=true
```

这违反默认非 GUI 验证不能打开 TextEdit 的纪律，所以不能作为默认门禁。

实现变更：

- `verify-nongui.sh`
  - 保留 `compile_quoted_applescript_blocks` helper，供显式诊断使用。
  - 默认不执行 `osacompile`，输出：
    - `appleScriptCompileSkipped=true reason=would-launch-target-apps`
  - 只有设置 `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1` 时才运行该 opt-in 检查。

验证：

```text
bash -n verify-nongui.sh
  OK

verify-nongui.sh
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  postInstallRegressionPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  nonGuiVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  latest pkg sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

residue check after verification
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
```

当前结论：

- 默认 `verify-nongui.sh` 再次满足“不打开 TextEdit”的纪律。
- AppleScript 编译检查保留为显式 opt-in 诊断，不能作为默认无 GUI 门禁。
- 当前仍未安装 v41 到系统目录，TIS matches 仍为 0；真实 GUI smoke 继续等待系统安装/TIS ready。

## v41 Mac mini 续修：UI-disabled smoke no-launch 门禁

时间：2026-07-08 00:10 CST。

背景：前面已经修过 `smoke-textedit.sh` / `smoke-clipboard-recall.sh` 在 `INPUTIA_RUN_UI_SMOKE` 未开启时应直接退出。但这个行为只靠单次手工验证，不能防止后续脚本改动再次在 disabled 路径拉起 TextEdit、留下 `osascript`，或启动 Inputia host。

实现变更：

- `verify-nongui.sh`
  - 新增 `process_running` / `assert_process_not_running` helper。
  - 新增 `ui-disabled smoke no-launch gates`：
    - 若 TextEdit 本来已运行，跳过该门禁，避免干扰用户已有窗口。
    - 否则运行：
      - `INPUTIA_SKIP_CDHASH_CHECK=1 smoke-textedit.sh`，期望 rc `14`
      - `INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh`，期望 rc `7`
    - 随后断言 `TextEdit`、`osascript`、`InputiaInputMethod` 均未运行。

验证：

```text
verify-nongui.sh
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  postInstallRegressionPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  nonGuiVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  latest pkg sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

final residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
```

当前结论：

- 默认非 GUI 验证现在覆盖 UI-disabled TextEdit/Clipboard smoke 不拉起 TextEdit、不留下 osascript、不启动 host。
- 当前系统目录仍是 v40，TIS matches 仍为 0；真实 GUI smoke 仍等待系统安装 v41 和 TIS ready。

## v41 Mac mini 续修：Safari UI-disabled no-launch 门禁

时间：2026-07-08 00:12 CST。

背景：默认非 GUI 验证已覆盖 TextEdit 和 Clipboard smoke 在 UI-disabled 路径下不拉起 GUI，但 Safari typing、Safari enter、Safari input-source diagnosis 还没有进入同一门禁。由于当前 Safari 是用户既有运行进程，验证不能关闭它，也不能把 `Safari=true` 当失败；但仍应证明 disabled 路径不会留下 `osascript` 或启动 Inputia host。

实现变更：

- `verify-nongui.sh`
  - 新增 `safari ui-disabled no-launch gates`。
  - 记录 `safariPreExisting=true/false`。
  - 运行以下 UI-disabled 路径并检查 rc：
    - `smoke-safari-typing.sh` -> rc `7`
    - `smoke-safari-enter.sh` -> rc `5`
    - `diagnose-safari-input-source.sh` -> rc `10`
  - 断言没有残留 `osascript` 和 `InputiaInputMethod`。
  - 仅当 `safariPreExisting=false` 时，才断言 Safari 未被拉起，避免干扰用户已有 Safari。

验证：

```text
verify-nongui.sh
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  textEditUiDisabled.rc=14
  clipboardUiDisabled.rc=7
  uiDisabledNoLaunchPassed=true
  safariPreExisting=true
  safariTypingUiDisabled.rc=7
  safariEnterUiDisabled.rc=5
  safariDiagnoseUiDisabled.rc=10
  safariUiDisabledNoLaunchPassed=true
  postInstallRegressionPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  nonGuiVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  latest pkg sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

final residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
```

当前结论：

- 默认非 GUI 验证现在覆盖 TextEdit、Clipboard、Safari typing、Safari enter、Safari diagnosis 的 UI-disabled no-launch 行为。
- 当前 Safari 是既有进程，本轮未启动或关闭 Safari。
- 系统目录仍是 v40，TIS matches 仍为 0；真实 GUI smoke 仍等待系统安装 v41 和 TIS ready。

## v41 Mac mini 续修：整体验证 no-new-GUI 与 tmp residue 门禁

时间：2026-07-08 00:14 CST。

背景：前面的 no-launch 门禁覆盖了具体 UI-disabled smoke，但 `verify-nongui.sh` 作为整体验证入口，还应该在整轮开始时记录 GUI App 初始状态，并在结束时统一证明没有新拉起 TextEdit/Safari、没有泄漏 smoke 临时文件。这样后续新增非 GUI 检查时也会被最终门禁兜住。

实现变更：

- `verify-nongui.sh`
  - 在验证开始时记录：
    - `textEditPreExisting`
    - `safariPreExisting`
  - 在最终 `residue` 段后新增整体验证断言：
    - 若 `textEditPreExisting=false`，结束时 TextEdit 必须仍未运行。
    - 若 `safariPreExisting=false`，结束时 Safari 必须仍未运行。
  - 新增 `/tmp` residue 检查，覆盖：
    - `inputia-*-select.log`
    - `inputia-safari-*-test.url`
    - `inputia-*-events.*.log`
    - `inputia-*-applescript.*`
    - `inputia-pkg-verify.*`
    - `inputia-post-install-regression.lock`
  - 通过时输出 `tmpResidue=false`。

验证：

```text
verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  safariPreExisting=true
  safariUiDisabledNoLaunchPassed=true
  postInstallRegressionPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  latest pkg sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

final residue check
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  tmp inputia residue=false
```

当前结论：

- `verify-nongui.sh` 现在有整轮 no-new-GUI 和 tmp residue 兜底门禁。
- 当前 Safari 仍是既有进程；TextEdit 没有被默认验证拉起。
- 系统目录仍是 v40，TIS matches 仍为 0；真实 GUI smoke 仍等待系统安装 v41 和 TIS ready。

## v41 Mac mini 续验：非 GUI 验证链路与只读 GUI preflight

时间：2026-07-08 CST。

背景：继续推进 GUI smoke 清理纪律和状态污染验证前，先确认当前机器状态仍不适合真实 GUI smoke：

- `/Library/Input Methods/InputiaInputMethod.app` 仍为 v40，当前 build 为 v41。
- TIS `includeAllInstalled=false/true` matches 均为 0。
- 当前 Safari 正在运行，不能让 Safari smoke 抢用户窗口。

验证：

```text
bash -n verify-nongui.sh
  OK

zsh -n status.sh verify-pkg.sh await-system-install.sh post-install-regression.sh install-system.sh
  OK

verify-nongui.sh
  syntaxOK=true
  appleScriptCompileOK=true file=smoke-common
  appleScriptCompileOK=true file=smoke-textedit
  appleScriptCompileOK=true file=smoke-clipboard-recall
  pkgVerificationPassed=true
  packageVersion=41
  buildVersion=41
  archiveAppCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  buildAppCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  postInstallRegressionPassed=true
  postInstallTISReady=false
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  nonGuiVerificationPassed=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-preflight.sh build/InputiaInputMethod.app
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  safariPreflight=running
  guiSmokeReady=false reason=safari-already-running
  smokePreflightReady=false reason=safari-already-running
  rc=7

process residue exact check
  InputiaInputMethod=false
  osascript=false
```

当前结论：

- 统一非 GUI 验证链路已稳定通过，覆盖脚本语法、AppleScript 编译、pkg/build 对齐、preflight 安全失败、post-install 非 GUI、await timeout、无弹框管理员门禁和残留检查。
- 只读 GUI preflight 在 Safari 已运行时正确拒绝，未打开 TextEdit/Safari，也未留下 `InputiaInputMethod` 或 `osascript` 残留。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍不应运行：系统安装不是当前 v41，TIS 不 ready，且 Safari 是用户既有运行进程。

## v41 Mac mini 续修：Safari AppleScript 可选静态编译与 no-launch 门禁

时间：2026-07-08 CST。

背景：审计 GUI smoke 清理纪律时发现 `verify-nongui.sh` 的 AppleScript 编译检查默认被保护开关跳过，避免 `osacompile` 触发 TextEdit/Safari 字典加载；同时 Safari smoke 里的 AppleScript 是 shell 插值生成的，不能用普通 heredoc 抽取方式直接验证。需要让需要时可验证生成后的 Safari AppleScript，但默认仍不碰 GUI。

实现变更：

- `verify-nongui.sh`
  - 新增 `compile_safari_applescript_block`，在 `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1` 时抽取 Safari smoke 的 unquoted `APPLESCRIPT` block。
  - 静态替换 `set testURL to "$url"` 为本地 `data:` URL。
  - 静态替换 `smoke-safari-typing.sh` 中的 `$keys` 为 `ni + Space` 键码片段，验证 shell 生成后的 AppleScript 语法形态。
  - 默认仍输出 `appleScriptCompileSkipped=true reason=would-launch-target-apps`，不运行 `osacompile`。
- `README.md`
  - 把 `verify-nongui.sh` 说明更新为默认安全收口入口。
  - 明确记录 TextEdit/Clipboard smoke 的 UI-disabled no-launch 门禁。
  - 记录 `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1` 是显式本机 AppleScript 语法检查开关。

验证：

```text
bash -n verify-nongui.sh
  OK

zsh -n smoke-safari-typing.sh smoke-safari-enter.sh
  OK

verify-nongui.sh
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  postInstallRegressionPassed=true
  postInstallTISReady=false
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 默认非 GUI 验证仍保持不启动 TextEdit/Safari 的纪律。
- TextEdit/Clipboard smoke 在 `INPUTIA_RUN_UI_SMOKE` 未显式打开时已经被验证为 no-launch 失败路径。
- Safari AppleScript 的生成后编译能力已接入可选开关；本轮未启用该开关，避免在用户 GUI 会话里触发目标 App 字典加载。

## v41 Mac mini 续修：Safari no-launch 与残留检测收紧

时间：2026-07-08 CST。

背景：继续复核 Safari smoke 清理纪律时，确认 `verify-nongui.sh` 已有 Safari UI-disabled no-launch gate，但 residue 检测没有覆盖 `diagnose-safari-input-source.sh`，且 status 阶段偶发看到 `InputiaInputMethod` 瞬态运行后立即退出，导致一次 false negative。

实现变更：

- `verify-nongui.sh`
  - residue 正则加入 `diagnose-safari-input-source.sh`，诊断脚本若卡住不会被漏报。
  - `status` 阶段若第一次未看到 `running=false`，等待 0.5 秒重读一次；第二次仍不是 `running=false` 才按 `inputia-host-running` 失败。
  - status 摘要同时打印 `running=true/false`，方便证据判断。
- `README.md`
  - `verify-nongui.sh` 说明从 TextEdit/Clipboard no-launch 扩展为 TextEdit/Clipboard/Safari no-launch。
  - 明确记录 `/tmp/inputia-*` 临时文件残留检查。

验证：

```text
bash -n verify-nongui.sh
  OK

zsh -n diagnose-safari-input-source.sh smoke-safari-typing.sh smoke-safari-enter.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  safariPreExisting=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallRegressionPassed=true
  postInstallTISReady=false
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

process residue exact check
  InputiaInputMethod=false
  osascript=false
```

当前结论：

- TextEdit、Clipboard、Safari 三类 smoke 在 UI-disabled 模式下都已由统一非 GUI 验证证明不会启动目标 App。
- 当前 Safari 是用户既有运行进程，验证只确认没有新增/残留 `InputiaInputMethod` 或 `osascript`，不关闭 Safari。
- 系统安装仍是 v40/TIS matches=0；真实 GUI smoke 继续禁跑。

## v41 Mac mini 续验：post-install UI/TIS no-launch 门禁

时间：2026-07-08 CST。

背景：`post-install-regression.sh` 是安装后收口入口。如果调用者显式设置 `INPUTIA_RUN_UI_SMOKE=1`，但 TIS 仍未 ready，脚本必须在打开 TextEdit/Safari 之前失败，而不是尝试真实 GUI smoke。当前系统仍为 v40，TIS matches=0，正好可验证该失败路径。

实现/验证要点：

- `verify-nongui.sh` 已接入 `post-install UI TIS gate`：
  - 以 `INPUTIA_RUN_UI_SMOKE=1 post-install-regression.sh build/InputiaInputMethod.app` 运行。
  - 当前预期 rc 为 `6`，原因是 `tis-not-ready`。
  - 随后断言没有启动 TextEdit、没有留下 `osascript`、没有留下 `InputiaInputMethod`。
- `status` 阶段保留 0.5 秒短重试：`post-install-regression.sh` 内的 `--bridge-*-self-check` 子进程可能短暂显示为 `running=true`，但应快速退出；长期存在仍会失败。

验证：

```text
bash -n verify-nongui.sh
  OK

zsh -n post-install-regression.sh status.sh await-system-install.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  safariPreExisting=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallRegressionPassed=true
  postInstallTISReady=false
  postInstallUiTisGate.rc=6
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final status
  running=false
  InputiaInputMethod process=false
  osascript process=false
```

当前结论：

- 安装后回归入口即使被显式要求跑 UI smoke，也会先检查 TIS readiness；当前 TIS 不 ready 时返回 rc 6 并不打开 GUI。
- Clipboard recall 仍只能等系统安装 v41/TIS ready 后做真实 TextEdit 验证；当前非 GUI 证据已覆盖 UI-disabled、TIS-not-ready 和残留清理门禁。

## v41 Mac mini 续修：post-install GUI 分支前置门禁

时间：2026-07-08 CST。

背景：上一轮非 GUI 验证覆盖了 TextEdit、Clipboard、Safari smoke 的 UI-disabled no-launch 路径，但 `post-install-regression.sh` 在显式请求 UI smoke 时仍需要先证明 TIS 与 GUI 会话都就绪，避免在系统安装未对齐时启动 TextEdit/Safari。当前 Mac mini 系统安装版仍是 v40，构建产物是 v41，TIS matches=0，所以本轮只验证 GUI 分支会在启动目标 App 前收口。

实现变更：

- `post-install-regression.sh`
  - 在 UI smoke 分支加入 `require_gui_session`，检查 console user、登录完成、锁屏状态、frontmost app 可读性和 loginwindow 前台状态。
  - 调用顺序放在 TIS 就绪之后；如果系统输入源未注册或未选中，先以 `tis-not-ready` 收口，不触碰 TextEdit/Safari。
- `verify-nongui.sh`
  - 新增 post-install UI TIS gate：以 `INPUTIA_RUN_UI_SMOKE=1` 调用 `post-install-regression.sh "$BUILD_APP"`，当前系统状态下预期 rc=6。
  - gate 后立即检查 TextEdit、osascript、InputiaInputMethod 无残留。
  - 最终 residue 检查跳过当前 shell wrapper 命令，避免把调用方 `/bin/zsh -c` 误判为测试脚本残留。

验证：

```text
zsh -n post-install-regression.sh
  OK

bash -n verify-nongui.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.rc=6
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready
  postInstallUiTisGateNoLaunchPassed=true
  postInstallRegressionPassed=true
  postInstallTISReady=false
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- post-install UI smoke 在系统输入源未就绪时会停在 TIS gate，且不会启动 TextEdit、osascript 或 InputiaInputMethod。
- 系统安装仍是 v40，构建和 pkg 已对齐到 v41；由于当前用户没有非交互 admin 权限，安装脚本 no-prompt gate 按预期返回 `admin-required`。
- Safari 是验证前已存在的用户进程；本轮没有关闭它，也没有新增 TextEdit/osascript/InputiaInputMethod 残留。

## v41 Mac mini 续修：Clipboard recall 文档清理断言

时间：2026-07-08 CST。

背景：`smoke-clipboard-recall.sh` 已经等待 `clipboardRecallShown` 并校验 `clipboardRecallCommit index=0 text=...`，但成功路径只比较上屏文本，没有像 TextEdit smoke 一样证明测试后 TextEdit 文档数回到基线。为了避免 recall 成功但残留空白窗口/文档，本轮把清理结果纳入脚本自身断言。

实现变更：

- `smoke-clipboard-recall.sh`
  - AppleScript 增加 `textEditDocumentCount()`。
  - recall 提交后先执行 `cleanupTextEdit(...)`，再返回 `textEditDocsBefore` 与 `textEditDocsAfter`。
  - shell 层把 metadata 从 `clipboardRecallResult` 中剥离，仍按原文本比较 recall 结果。
  - 当 `textEditDocsAfter > textEditDocsBefore` 时以 `reason=textedit-cleanup` 失败并输出事件日志尾部。

验证：

```text
bash -n smoke-clipboard-recall.sh verify-nongui.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  systemPreflight.rc=2 reason=cdhash-mismatch
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.rc=6
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- Clipboard recall 的真实 GUI 成功路径现在会同时证明上屏文本、事件日志和 TextEdit 文档清理。
- 默认非 GUI 验证仍不启动 TextEdit/Safari；当前 Safari 是既有用户进程。
- 真实 recall GUI smoke 仍需等系统安装 v41 并让 TIS 注册/选中后再运行。

## v41 Mac mini 续修：TextEdit 用例级 IME 状态清理

时间：2026-07-08 CST。

背景：Clipboard recall smoke 已经使用双 Escape 清理 IME 状态，并在输入前确认 TextEdit 文档为空；TextEdit 主 smoke 仍是在每个 case 前单次 Escape。为了降低前一轮 composition/candidate 状态污染对后续 case 的影响，本轮把 TextEdit smoke 的每个输入用例也统一到双 Escape 清理，并在真正打字前断言空文档未被残留状态污染。

实现变更：

- `smoke-textedit.sh`
  - 新增 AppleScript `clearInputiaState()`，连续发送两次 Escape 并等待状态稳定。
  - `runCase(...)` 在创建文档前和 TextEdit 前台确认后各清理一次 IME 状态。
  - `runCase(...)` 输入前确认 TextEdit 仍是前台，且 front document 文本仍为空；否则以 `state-clear-leaked-text` 报错。
  - `runShiftCase()` 使用同样的双 Escape 与空文档断言；否则以 `shift-state-clear-leaked-text` 报错。

验证：

```text
bash -n smoke-textedit.sh smoke-clipboard-recall.sh verify-nongui.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=3
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  clipboardUiTisGate.rc=8 reason=input-source-not-selected
  clipboardUiTisGateNoLaunchPassed=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  id=com.inputia.inputmethod.Inputia enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- TextEdit 主 smoke 和 Clipboard recall smoke 现在都在输入前主动清空 IME 状态，并在成功/失败路径上保护 TextEdit 文档清理。
- `verify-nongui.sh` 的 Clipboard UI TIS gate 证明：即使显式允许 UI smoke，只要输入源无法选中，脚本会在启动 TextEdit 前退出，并保持剪贴板内容不变。
- TIS 已能从系统 v40 安装看到 3 个 Inputia 条目，但 build v41 与 system v40 仍不一致，Hans 也未被选中；真实 GUI smoke 继续禁跑。

## v41 Mac mini 续修：TextEdit UI TIS gate no-launch

时间：2026-07-08 CST。

背景：Clipboard recall 已有 UI TIS gate，证明显式允许 UI smoke 时，如果 Inputia 输入源无法选中，会在打开 TextEdit 前退出并保持剪贴板不变。TextEdit 主 smoke 也应该有同等级的 no-launch gate，避免未来 TIS 注册异常时误打开 TextEdit。

实现变更：

- `verify-nongui.sh`
  - 在 UI-disabled no-launch section 后新增 `textEditUiTisGate`。
  - 以 `INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1` 调用 `smoke-textedit.sh "$BUILD_APP"`。
  - 当前状态下预期 `smoke-textedit.sh` 在 `input-source-not-selected` 处以 rc=15 退出。
  - 立即断言 TextEdit、osascript、InputiaInputMethod 均未运行，并输出 `textEditUiTisGateNoLaunchPassed=true`。

验证：

```text
bash -n verify-nongui.sh smoke-textedit.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=3
  status running=false
  status sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  textEditUiTisGate.rc=15 reason=input-source-not-selected
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGate.rc=8 reason=input-source-not-selected
  clipboardUiTisGateNoLaunchPassed=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  id=com.inputia.inputmethod.Inputia enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- TextEdit 主 smoke 和 Clipboard recall smoke 都已有“UI 被允许但 TIS 不能选中”的 no-launch gate。
- 这两条 gate 共同证明当前 TIS 异常状态不会打开 TextEdit、不会留下 osascript/InputiaInputMethod 残留；Clipboard gate 还额外证明剪贴板不会被改写。
- 真实 GUI smoke 的剩余前置仍是：安装 v41 到 `/Library/Input Methods` 并让 Hans 输入模式成为 enabled/selected。

## v41 Mac mini 续修：TIS gate 当前输入源不变断言

时间：2026-07-08 CST。

背景：TextEdit/Clipboard UI TIS gate 已经证明不会启动 TextEdit、不会留下进程残留，Clipboard gate 也证明不会改剪贴板。但这些 gate 会调用 TIS 注册/选择路径；即使选择失败，也需要明确证明验证过程不会把用户当前输入源从原输入法切走。

实现变更：

- `verify-nongui.sh`
  - 新增 `current_input_source_id()`，通过 build app 的 `--dump-current-input-source` 读取当前输入源 ID。
  - 新增 `assert_current_source_unchanged(label, before, after)`，打印 before/after 并在不一致时失败。
  - `textEditUiTisGate` 前后比对当前输入源。
  - `clipboardUiTisGate` 前后比对当前输入源；同时保留已有剪贴板前后比对。

验证：

```text
bash -n verify-nongui.sh
  OK

verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  tisReadinessBuild.tisReadiness=false
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=3
  textEditUiTisGate.rc=15 reason=input-source-not-selected
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGate.rc=8 reason=input-source-not-selected
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGateNoLaunchPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/current source/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  currentSource.id=com.tencent.inputmethod.wetype.pinyin
  currentSource.selected=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  id=com.inputia.inputmethod.Inputia enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- TextEdit/Clipboard TIS gate 现在同时证明：不启动 TextEdit、不留进程、不改剪贴板、不改变用户当前输入源。
- 当前输入源在验证后仍是 `com.tencent.inputmethod.wetype.pinyin`。
- 真实 GUI smoke 仍需系统安装 v41 并让 `com.inputia.inputmethod.Inputia.Hans` 成为 selected。

## v41 Mac mini 续修：post-install UI TIS gate 当前输入源不变

时间：2026-07-08 CST。

背景：TextEdit/Clipboard 的 UI TIS gate 已证明不会改变当前输入源。`post-install-regression.sh` 的 UI TIS gate 虽然理论上只做 readiness 检查，不启动 TextEdit/Safari，也不应选择输入源，但它会执行完整 bundle/self-check/TIS readiness 诊断；因此也需要纳入当前输入源不变断言。

实现变更：

- `verify-nongui.sh`
  - 在 `postInstallUiTisGate` 前后读取 `current_input_source_id()`。
  - 复用 `assert_current_source_unchanged "postInstallUiTisGate"`。
  - 保留原有 TextEdit、osascript、InputiaInputMethod no-residue 断言。

验证：

```text
bash -n verify-nongui.sh
  OK

verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  tisReadinessBuild.tisReadiness=false
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=3
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  postInstallUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/current source/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  currentSource.id=com.tencent.inputmethod.wetype.pinyin
  currentSource.selected=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  id=com.inputia.inputmethod.Inputia enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- TextEdit、Clipboard、post-install 三条 UI TIS gate 都证明不会改变当前输入源。
- 当前失败路径不会启动 TextEdit/Safari、不会留下 osascript/InputiaInputMethod、不会改剪贴板、不会改当前输入源。
- 真实 GUI smoke 的剩余前置没有变化：系统安装 v41 并选中 Hans 输入模式。

## v41 Mac mini 续修：Safari 既有进程 gate 与 residue 二次采样

时间：2026-07-08 CST。

背景：当前 Safari 是用户既有进程。真实 Safari smoke 不能抢这个窗口，也不能在 Safari 已运行时继续选择输入源或打开测试页；同时一次非 GUI 验证在最终 residue 采样时撞到了 `await-system-install.sh` 的短暂退出窗口，产生 false positive。需要把 Safari 既有进程失败路径纳入总验证，并让 residue 检查区分短暂退出与真实残留。

实现变更：

- `verify-nongui.sh`
  - 当 Safari 预先存在时，新增三条显式 UI gate：
    - `safariTypingExistingGate` 预期 rc=9，停在 `safari-already-running`。
    - `safariEnterExistingGate` 预期 rc=7，停在 `safari-already-running`。
    - `safariDiagnoseExistingGate` 预期 rc=11，停在 `safari-already-running`。
  - 三条 gate 都比对当前输入源 before/after，确认没有切换用户当前输入法。
  - 三条 gate 后继续断言没有 osascript/InputiaInputMethod 残留。
  - residue 检查抽成 `collect_residue()`；首次发现残留后等待 1 秒重采样，第二次仍存在才失败，避免短暂退出窗口造成误报。

验证：

```text
bash -n verify-nongui.sh
  OK

verify-nongui.sh
  safariPreExisting=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  safariTypingExistingGate.rc=9 reason=safari-already-running
  safariTypingExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterExistingGate.rc=7 reason=safari-already-running
  safariEnterExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseExistingGate.rc=11 reason=safari-already-running
  safariDiagnoseExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/current source/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  currentSource.id=com.tencent.inputmethod.wetype.pinyin
  currentSource.selected=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  id=com.inputia.inputmethod.Inputia enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- Safari typing、enter、diagnose 三条脚本在 Safari 已运行时都会停在 preflight，且不会选择输入源、不会打开测试页、不会改变当前输入源。
- residue 检查仍会抓真实残留，但会过滤掉短暂退出采样竞态。
- 真实 Safari GUI smoke 仍需在 v41 系统安装/TIS 选中完成后，并且 Safari 不为用户既有运行进程或显式允许复用时再跑。

## v41 Mac mini 续修：await/admin gate 当前输入源不变

时间：2026-07-08 CST。

背景：TextEdit、Clipboard、Safari、post-install 的 UI gate 已证明不改变当前输入源。剩余的 `await-system-install.sh` timeout gate 和 `install-system.sh` no-prompt admin gate 也会读取安装/TIS 状态；本轮把它们纳入同一条系统状态保护线。验证中还遇到一个外部 `InputiaInputMethod --diagnostics` 进程导致首次 status 看到 `running=true`，等该外部诊断自然退出后重跑通过；未杀进程。

实现变更：

- `verify-nongui.sh`
  - `awaitShort` 前后读取当前输入源，并调用 `assert_current_source_unchanged "awaitShort"`。
  - `installNoPrompt` 前后读取当前输入源，并调用 `assert_current_source_unchanged "installNoPrompt"`。
  - 保持 no-admin gate 不触发管理员提示、不执行安装副作用。

验证：

```text
bash -n verify-nongui.sh
  OK

verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=3
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  awaitShort.rc=2 reason=timeout
  awaitShort.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  awaitShort.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.rc=12 reason=admin-required
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue/current source/status sample
  texteditResidual=false
  osascriptResidual=false
  inputiaRunning=false
  safariRunning=true
  currentSource.id=com.tencent.inputmethod.wetype.pinyin
  currentSource.selected=true
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  id=com.inputia.inputmethod.Inputia enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant enabled=false selected=false
  id=com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef
```

当前结论：

- 所有当前可跑的非 GUI / preflight / TIS / admin gate 都证明不会改变用户当前输入源。
- no-admin install gate 仍按预期返回 `admin-required`，没有触发管理员提示。
- 真实 GUI smoke 仍需系统安装 v41 并选中 Hans 输入模式。

## v41 Mac mini 续验：Clipboard recall UI/TIS no-launch 与剪贴板不变门禁

时间：2026-07-08 CST。

背景：`smoke-clipboard-recall.sh` 的真实路径会设置 `INPUTIA_DEBUG_EVENTS`、选择输入源、写入测试剪贴板，然后打开 TextEdit 触发 `Ctrl+Shift+V`。在系统安装未对齐/TIS 未 ready 时，即使调用者显式设置 `INPUTIA_RUN_UI_SMOKE=1`，也必须停在输入源选择门禁之前，不能改剪贴板、不能打开 TextEdit。

实现变更：

- `verify-nongui.sh`
  - 在 TextEdit 原本未运行时新增 `clipboardUiTisGate`。
  - 运行 `INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_RESTART_HOST_FOR_DEBUG=0 smoke-clipboard-recall.sh "$BUILD_APP"`。
  - 当前预期 rc=`8`，即 `input-source-not-selected`。
  - 运行前后读取 `pbpaste`，确认剪贴板文本不变。
  - gate 后断言 TextEdit、osascript、InputiaInputMethod 均未运行。

验证：

```text
bash -n smoke-clipboard-recall.sh verify-nongui.sh
  OK

verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  buildPreflightUiDisabled.rc=4 reason=ui-smoke-disabled
  textEditUiDisabled.rc=14 reason=ui-smoke-disabled
  clipboardUiDisabled.rc=7 reason=ui-smoke-disabled
  uiDisabledNoLaunchPassed=true
  clipboardUiTisGate.rc=8
  clipboardUiTisGateNoLaunchPassed=true
  safariTypingUiDisabled.rc=7 reason=ui-smoke-disabled
  safariEnterUiDisabled.rc=5 reason=ui-smoke-disabled
  safariDiagnoseUiDisabled.rc=10 reason=ui-smoke-disabled
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.rc=6
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

clipboardUiTisGate selected diagnostics
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  selectSourceFoundInEnabledList=false
  selected=false

final status
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  Inputia TIS iconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  running=false
  InputiaInputMethod process=false
  osascript process=false
  tmp inputia residue=false
```

当前结论：

- Clipboard recall smoke 现在有三层安全证据：UI-disabled 不启动、UI-enabled 但 TIS 不 ready 不启动且不改剪贴板、真实成功路径会校验 TextEdit 文档清理。
- 当前 TIS 能在 all-installed 中看到旧系统安装的 Inputia sources，但 icon 仍指向 `/Library/Input Methods/InputiaInputMethod.app` 旧 v40，而不是当前 build；严格 gate 因此继续正确阻止真实 GUI smoke。

## v41 Mac mini 续验：常用 Command 快捷键整体透传与 TIS readiness 诊断

时间：2026-07-08 CST。

背景：用户发现 Inputia 下 `Command-C` / `Command-V` 不可用，判断复制/粘贴被输入法接管，并要求按常用电脑快捷键举一反三处理。依据 Apple 官方 Mac keyboard shortcuts 文档，`Command` 是 macOS 常用系统/App 快捷键主修饰键；复制、粘贴、剪切、撤销、重做、全选、保存、打开、关闭、退出、查找、打印、隐藏、窗口、格式和浏览导航等命令都应交还系统或宿主 App。Inputia 自身快捷键继续使用不含 Command 的组合，例如 `Control-Shift-V` 剪贴板召回。

实现边界：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)`
  - 对所有包含 `Command` 的 `keyDown` 事件返回透传。
- `InputiaInputController.handleKeyDown`
  - 在剪贴板召回、候选处理、标点/中英切换之前先检查 Command 透传。
- `InputiaShortcutClassifier.isClipboardRecall`
  - 保留 `Control-Shift-V`。
  - 明确拒绝 `Control-Shift-Command-V`，避免覆盖系统/App Command 组合。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)`
  - 显式放行常见 AppKit command selector，包括编辑、文件、窗口、查找、格式、帮助和浏览导航类命令。
- `InputiaInputController.didCommand(by:)`
  - 在 newline/delete/candidate selector 处理前先检查 App command selector 透传。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  tisReadinessBuild.appMatchesBuild=true
  tisReadinessBuild.tis.enabledMatches=0
  tisReadinessBuild.tis.installedMatches=3
  tisReadinessBuild.tis.hansIconMatchesApp=false
  tisReadinessBuild.tis.hansEnabled=true
  tisReadinessBuild.tis.hansSelected=false
  tisReadinessBuild.tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tisReadinessBuild.tisReadiness=false
  textEditUiTisGate.rc=15 reason=input-source-not-selected
  clipboardUiTisGate.rc=8 reason=input-source-not-selected
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

快捷键自检覆盖：

```text
commandCPassThrough=true
commandVPassThrough=true
commandXPassThrough=true
commandZPassThrough=true
commandShiftZPassThrough=true
commandAPassThrough=true
commandSPassThrough=true
commandOPassThrough=true
commandWPassThrough=true
commandQPassThrough=true
commandFPassThrough=true
commandGPassThrough=true
commandCommaPassThrough=true
commandTabPassThrough=true
commandSpacePassThrough=true
commandOptionEscapePassThrough=true
ctrlShiftVClipboardRecall=true
ctrlShiftCommandVRejected=true
appCommandcopyPassesThrough=true
appCommandpastePassesThrough=true
appCommandcutPassesThrough=true
appCommandundoPassesThrough=true
appCommandredoPassesThrough=true
appCommandselectAllPassesThrough=true
appCommandsaveDocumentPassesThrough=true
appCommandopenDocumentPassesThrough=true
appCommandperformClosePassesThrough=true
appCommandterminatePassesThrough=true
appCommandfindPassesThrough=true
appCommandfindNextPassesThrough=true
appCommandfindPreviousPassesThrough=true
appCommandshowPreferencesPassesThrough=true
appCommandnewDocumentPassesThrough=true
appCommandsaveDocumentAsPassesThrough=true
appCommandsaveDocumentToPassesThrough=true
appCommandperformMiniaturizePassesThrough=true
appCommandperformZoomPassesThrough=true
appCommandtoggleFullScreenPassesThrough=true
appCommandtoggleBoldPassesThrough=true
appCommandtoggleItalicPassesThrough=true
appCommandtoggleUnderlinePassesThrough=true
appCommandgoBackPassesThrough=true
appCommandgoForwardPassesThrough=true
appCommandreloadPassesThrough=true
appCommandstopLoadingPassesThrough=true
```

只读 TIS readiness 抽样：

```text
bash macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  appMatchesBuild=true
  expectedTISModeID=com.inputia.inputmethod.Inputia.Hans
  expectedTISIcon=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  tis.hansIconMatchesApp=false
  tis.hansEnabled=true
  tis.hansSelectable=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  tisReadiness=false
```

最终状态抽样：

```text
status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=3
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

residue
  InputiaInputMethod process=false
  osascript process=false
  tmp inputia residue=false
```

当前结论：

- `Command-C/V` 不再作为单点补丁处理；代码层已经把包含 `Command` 的键盘事件统一交还系统/宿主 App。
- AppKit selector 路径也显式放行常见 App 命令，避免复制/粘贴等从 `didCommand(by:)` 进入时被后续维护误接管。
- 非 GUI 验证通过，且失败门禁没有启动 TextEdit/Safari、没有残留 `osascript`/Inputia 进程、没有留下 `/tmp/inputia-*` smoke 临时文件。
- 真实 GUI smoke 仍未运行，原因是系统安装版还是 v40，当前 build 是 v41；TIS Hans source 仍指向 `/Library/Input Methods/InputiaInputMethod.app`，不是当前 build app。

## v41 Mac mini 续修：TIS gate 改为只选择不注册

时间：2026-07-08 00:44 CST

问题：

- 非 GUI 验证里的 TextEdit/Clipboard `INPUTIA_RUN_UI_SMOKE=1` TIS gate 本意是验证“输入源未选中时早退且不启动 GUI App”。
- 旧实现调用 `inputia-tis-tool` 无参数路径；该路径会执行 `TISRegisterInputSource` + `TISEnableInputSource` + `TISSelectInputSource`。
- 在 build app 不是系统安装版时，macOS 会把 build app 记录到用户域输入法位置，导致后续 `post-install-regression.sh` 报 `userHostConflict=true`。

依据：

- Apple Text Input Source Services / `NSTextInputContext.keyboardInputSources` 文档说明输入源由 input source identifier 表示。
- 成熟工具 `InputSourceSelector` 的模式是把 `select <InputSourceId>` 作为独立命令，选择已存在/已启用输入源；不把 list/current/select 与 register/enable 混在同一个未知参数路径。
- 因此 smoke gate 分成两类：真实 smoke 可以注册/启用/选择；非 GUI gate 只能尝试选择已启用且路径匹配的 Inputia source，失败即早退。

实现：

- `Tools/InputiaTISTool.swift`
  - 新增 `--help/-h`，只打印 usage，不触发任何 TIS 注册。
  - 非空未知参数只打印 `unknownCommand=...` + usage，不触发注册。
  - 新增 `--select-inputia-source-id <id>`：只从 enabled list 选择，并继续走 app/icon 匹配逻辑；不注册、不启用。
  - 保留无参数默认行为，供真实 smoke 选择 Inputia 使用。
- `smoke-common.sh`
  - 新增 `INPUTIA_TIS_SELECT_ONLY=1` 路径。
  - select-only 时调用 `--select-inputia-source-id ${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}`。
  - restore 用户原输入源仍使用通用 `--select-source-id`，只从 enabled list 选择，不注册。
- `verify-nongui.sh`
  - TextEdit/Clipboard TIS gate 显式传入 `INPUTIA_TIS_SELECT_ONLY=1`。

选择/放弃：

- 选择：gate 用只选择模式，确保失败门禁不修改 TIS 注册状态。
- 放弃：继续在 gate 里走无参数 register/enable/select；原因是会生成 `~/Library/Input Methods/InputiaInputMethod.app` 用户域 host，污染 post-install 回归。
- 放弃：gate 用通用 `--select-source-id`；原因是同一个 source id 可能来自旧系统安装版，必须要求 app/icon 匹配当前被测 app。

用户域冲突处理：

```text
发现：/Users/minizl/Library/Input Methods/InputiaInputMethod.app
version=41
CDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a

可逆移动：
userHostMoved=true
backup=/Users/minizl/Library/Input Methods/InputiaInputMethod.app.inputia-smoke-backup-20260708004302
```

spike 验证：

```text
inputia-tis-tool --help
  usage=inputia-tis-tool [--dump|--reset-enable|--select-inputia-source-id <id>|--select-source-id <id>]
  defaultAction=register-enable-select-inputia
  helpNoSideEffect=true before=com.tencent.inputmethod.wetype.pinyin after=com.tencent.inputmethod.wetype.pinyin

inputia-tis-tool --not-a-command
  unknownCommand=--not-a-command
  unknownNoSideEffect=true before=com.tencent.inputmethod.wetype.pinyin after=com.tencent.inputmethod.wetype.pinyin

INPUTIA_APP=build/InputiaInputMethod.app INPUTIA_TIS_REQUIRE_APP_MATCH=1 inputia-tis-tool --select-inputia-source-id com.inputia.inputmethod.Inputia.Hans
  selectSourceFoundInEnabledList=false
  selectOnlyNoCurrentSourceMutation=true before=com.tencent.inputmethod.wetype.pinyin after=com.tencent.inputmethod.wetype.pinyin
```

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  pkgVerificationPassed=true
  status.buildVersion=41
  status.systemMatchesBuild=false
  status.userMatchesBuild=false
  status.includeAllInstalled=false matches=0
  status.includeAllInstalled=true matches=0
  status.running=false
  tisReadinessBuild.tisReadiness=false
  textEditUiTisGate.rc=15
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGate.rc=8
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGateNoLaunchPassed=true
  safariTypingExistingGate.rc=9
  safariEnterExistingGate.rc=7
  safariDiagnoseExistingGate.rc=11
  safariExistingGateNoMutationPassed=true
  postInstall.userHostConflict=false
  postInstallRegressionPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  verifyNonGuiRc=0
```

当前结论：

- 非 GUI gate 已恢复为无注册副作用路径；TextEdit/Clipboard TIS gate 不再生成用户域 Inputia host。
- 当前输入源保持 `com.tencent.inputmethod.wetype.pinyin`。
- TextEdit 没有预存进程，Safari 是预存进程且 gate 没有关闭或接管它。
- 真实 GUI smoke 仍未运行；当前系统安装版仍是 v40，build/pkg 是 v41，TIS readiness=false。

## v41 Mac mini 续验：用户级安装不能绕过旧系统 TIS mode 记录

时间：2026-07-08 CST。

背景：系统安装 v41 需要管理员权限，当前 `install-system.sh` 的 no-prompt gate 返回 `admin-required`。为了判断能否不用管理员权限推进真实 GUI smoke，本轮按 Apple TIS register/enable/select 语义做用户级安装 spike：把 v41 安装到 `~/Library/Input Methods/InputiaInputMethod.app`，再观察 TIS Hans mode 是否指向用户级 app。严格规则仍然是：如果 mode icon 不指向待测 app，不允许运行真实 TextEdit/Clipboard/Safari smoke，避免误测旧系统包。

实现变更：

- `install-user.sh`
  - 安装后调用 `tis-readiness.sh "$DEST_APP"`。
  - 输出 `userInstallTIS: ...` 诊断。
  - 文件 CDHash 匹配仍输出 `userInstallVerified=true`。
  - 额外输出 `userInstallTISReady=true|false`，区分“文件安装成功”和“TIS 已可用于真实 GUI smoke”。
- `README.md`
  - 记录用户级安装只能证明文件安装，不能替代 TIS readiness。
  - 明确系统旧版本仍存在时可能出现 `userInstallTISReady=false`，不能绕过 smoke app-match 门禁。

验证与 spike：

```text
install-user.sh
  registerStatus=0
  enabledSourceAlreadyPresent=true
  userInstallTIS: app=/Users/minizl/Library/Input Methods/InputiaInputMethod.app
  userInstallTIS: appMatchesBuild=true
  userInstallTIS: expectedTISIcon=/Users/minizl/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  userInstallTIS: tis.enabledMatches=0
  userInstallTIS: tis.installedMatches=6
  userInstallTIS: tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  userInstallTIS: tis.hansIconMatchesApp=false
  userInstallTIS: tis.hansEnabled=true
  userInstallTIS: tis.hansSelected=false
  userInstallTIS: tis.currentID=com.tencent.inputmethod.wetype.pinyin
  userInstallTIS: tisReadiness=false
  userInstallVerified=true
  userInstallTISReady=false
```

TextEdit GUI smoke with user app:

```text
INPUTIA_RUN_UI_SMOKE=1 smoke-textedit.sh "$HOME/Library/Input Methods/InputiaInputMethod.app"
  expectedCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  actualCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  textEditSmokeReady=false reason=input-source-not-selected
  previousInputSourceID=com.tencent.inputmethod.wetype.pinyin
  inputSourceRestore=skipped reason=already-current
```

TIS reset/refresh spike:

```text
INPUTIA_APP="$HOME/Library/Input Methods/InputiaInputMethod.app" inputia-tis-tool --reset-enable
  beforeCurrentID=com.tencent.inputmethod.wetype.pinyin
  resetEnableRc=0
  afterCurrentID=com.tencent.inputmethod.wetype.pinyin
  readinessAfterReset.tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  readinessAfterReset.tis.hansIconMatchesApp=false
  readinessAfterReset.tisReadiness=false
```

结论：

- 用户级 v41 文件安装可以成功，但在系统 v40 仍存在时，macOS TIS 仍把 Hans/Hant mode 解析到系统 `/Library` app；用户级安装不能作为真实 GUI smoke 的替代前置。
- `smoke-textedit.sh` 的 app-match/input-source gate 正确阻止了误测旧系统包，且没有启动 TextEdit。
- `--reset-enable`、LaunchServices 注销/注册、`mdimport`、`TextInputMenuAgent` 重启都没有把 Hans/Hant mode 转到用户级 app。
- 当前外部 TIS 缓存被刷新后暂时显示 `includeAllInstalled=true matches=0`；系统 app 文件仍存在但不是 v41。下一步真实 GUI smoke 仍需要系统 v41 安装和 TIS 刷新，不能用用户级安装绕过。

最终非 GUI 验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=6
  textEditUiTisGate.rc=15 reason=input-source-not-selected
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.rc=8 reason=input-source-not-selected
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final sample after cleanup
  systemMatchesBuild=false
  userMatchesBuild=false
  userSettingsMatchesBuildVersion=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  currentInputSource=com.tencent.inputmethod.wetype.pinyin
  TextEdit=not-running
  Safari=running
  InputiaInputMethod process=false
  osascript process=false
  tmp inputia residue=false
```

## v41 Mac mini 续修：用户级安装输出瘦身与非 GUI 瞬态进程容错

时间：2026-07-08 CST。

背景：上一轮用管道抽样 `install-user.sh` 输出时，`build.sh` 的链接 warning 和完整 TIS dump 输出过大，外层 `rg`/head 类管道可能提前关闭，造成用户级安装状态不易判断。同时 `verify-nongui.sh` 的 status 阶段只等待 0.5s，偶发把短生命周期 `--bridge-*-self-check` 误判为 `InputiaInputMethod` 残留。

实现变更：

- `install-user.sh`
  - `build.sh` stdout/stderr 写入临时 `build.log`，构建失败时只打印最后 80 行。
  - 安装成功时输出 `userInstallBuild=true`。
  - `--dump-input-source` 改为摘要输出，仅保留 Inputia source 的 `id`、`iconURL`、`enabled`、`selected`。
  - 保留 `userInstallTISReady=true|false`，继续区分文件安装和 TIS readiness。
- `verify-nongui.sh`
  - `current_input_source_id()` 捕获诊断输出，失败时返回 `unknown`，避免 `set -euo pipefail` 在恢复/断言阶段打断整轮验证。
  - status 阶段从一次 `sleep 0.5` 改为最多 3s 轮询，降低 self-check 瞬态进程误报。

验证：

```text
zsh -n install-user.sh
  OK

install-user.sh
  userInstallBuild=true
  registerStatus=0
  enabledSourceAlreadyPresent=true
  userInstallTISDump: id=com.inputia.inputmethod.Inputia.Hans
  userInstallTISDump: iconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  userInstallTISDump: enabled=true
  userInstallTISDump: selected=false
  userInstallTIS: app=/Users/minizl/Library/Input Methods/InputiaInputMethod.app
  userInstallTIS: appMatchesBuild=true
  userInstallTIS: expectedTISIcon=/Users/minizl/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  userInstallTIS: tis.enabledMatches=0
  userInstallTIS: tis.installedMatches=6
  userInstallTIS: tis.hansIconURL=/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
  userInstallTIS: tis.hansIconMatchesApp=false
  userInstallTIS: tis.currentID=com.tencent.inputmethod.wetype.pinyin
  userInstallTIS: tisReadiness=false
  userInstallVerified=true
  userInstallTISReady=false
```

```text
bash -n verify-nongui.sh
  OK

bash verify-nongui.sh
  syntaxOK=true
  pkgVerificationPassed=true
  status buildVersion=41
  status buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  status systemMatchesBuild=false
  status userMatchesBuild=false
  status includeAllInstalled=false matches=0
  status includeAllInstalled=true matches=0
  status running=false
  textEditUiTisGate.rc=15 reason=input-source-not-selected
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.rc=8 reason=input-source-not-selected
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

最终状态抽样：

```text
status.sh
  buildVersion=41
  buildCDHash=4a96b63c4c300404a18d037a6ae2fa8bdd649b3a
  systemMatchesBuild=false
  userMatchesBuild=false
  userSettingsMatchesBuildVersion=false
  includeAllInstalled=false matches=0
  includeAllInstalled=true matches=0
  running=false
  sha256=af865196dd75b0a1b8e7745e101c26a32eb274ad45aebc8e596e8caf7d25c6ef

residue
  TextEdit=not-running
  Safari=running
  InputiaInputMethod process=false
  osascript process=false
  tmp inputia residue=false
```

当前结论：

- `install-user.sh` 现在能稳定输出小而明确的用户级安装/TIS 诊断，不再被链接 warning 和完整 TIS dump 淹没。
- 用户级安装仍不能用于真实 GUI smoke：`userInstallTISReady=false`，Hans mode 仍指向系统 v40。
- `verify-nongui.sh` 已能容忍短生命周期 self-check 进程，并完整通过非 GUI 收口。
- 真实 GUI smoke 的前置没有变化：需要系统 v41 安装和 TIS Hans mode 指向系统 v41 app。

## v41 Mac mini 收尾补记：select-only gate 验证后状态

时间：2026-07-08 00:44 CST。

补记：

- 本轮详细记录见上方 `v41 Mac mini 续修：TIS gate 改为只选择不注册`。
- 后续再次运行 `bash macos/InputiaInputMethod/verify-nongui.sh`，最终通过。
- 当前工作区没有提交；保留全部 dirty changes。

最终状态：

```text
verifyNonGuiRc=0
currentInputSource=com.tencent.inputmethod.wetype.pinyin
TextEdit process=false
InputiaInputMethod process=false
osascript process=false
tmp inputia residue=false
userHost=/Users/minizl/Library/Input Methods/InputiaInputMethod.app exists=false
userHostBackup=/Users/minizl/Library/Input Methods/InputiaInputMethod.app.inputia-smoke-backup-20260708004302
```

## v41 Mac mini 续修：GUI smoke 默认不注册 TIS

时间：2026-07-08 CST。

问题：

- 上一轮只把 TextEdit/Clipboard 的非 GUI gate 显式设置为 select-only。
- 但如果维护者或自动化直接运行 `INPUTIA_RUN_UI_SMOKE=1 smoke-*.sh build/InputiaInputMethod.app`，公共 helper 的默认路径仍可能走 `inputia-tis-tool` 无参数注册/启用/选择。
- 这会把 build app 写进用户域输入法记录，违背 smoke 脚本“验证前置状态，不负责安装/注册”的职责边界。

实现：

- `smoke-common.sh`
  - `inputia_select_input_source_or_exit` 默认只调用 `inputia-tis-tool --select-inputia-source-id ...`。
  - 只有显式设置 `INPUTIA_TIS_REGISTER_BEFORE_SELECT=1` 时，才允许走旧的 register/enable/select 路径。
  - 如果 TIS 工具缺失且未显式允许注册，直接输出 `selectSourceFoundInEnabledList=false` 和 `selectOnlyMissingTISTool=true`，不再回退到 host 自身的选择命令。
- `diagnose-safari-input-source.sh`
  - 第二次“Safari input focused 后选择 Inputia”的路径改为复用公共 helper，避免绕过默认 select-only 纪律。
- `verify-nongui.sh`
  - 移除 TextEdit/Clipboard gate 中的旧 `INPUTIA_TIS_SELECT_ONLY=1` 显式变量；验证现在覆盖默认行为。

选择/放弃：

- 选择：安装/注册属于 `install-*`、`await-system-install.sh` 或显式 spike；GUI smoke 默认只选择已经存在且 app/icon 匹配的输入源。
- 放弃：让 smoke 默认 register/enable/select；原因是这会把验证脚本变成状态变更脚本，曾经产生用户域 host conflict。
- 保留：`INPUTIA_TIS_REGISTER_BEFORE_SELECT=1` 作为显式调试逃生口，但默认验证链路不使用。

验证：

```text
bash -n smoke-common.sh smoke-textedit.sh smoke-clipboard-recall.sh smoke-safari-typing.sh smoke-safari-enter.sh diagnose-safari-input-source.sh verify-nongui.sh
  syntaxOK

bash macos/InputiaInputMethod/verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  pkgVerificationPassed=true
  status.userMatchesBuild=false
  status.running=false
  tisReadinessBuild.tis.enabledMatches=0
  tisReadinessBuild.tis.installedMatches=0
  textEditUiTisGate.rc=15
  textEditUiTisGate.selectSourceFoundInEnabledList=false
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGate.rc=8
  clipboardUiTisGate.selectSourceFoundInEnabledList=false
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGateNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstall.userHostConflict=false
  postInstallRegressionPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  verifyNonGuiRc=0
```

当前结论：

- GUI smoke 脚本现在默认不注册、不启用 TIS；直接以 build app 运行 smoke gate 也不会再生成用户域 Inputia host。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 仍需系统 v41 安装并通过 TIS readiness 后再跑。

## v41 Mac mini 续修：debug env 和 host restart 后移到 TIS 选择成功后

时间：2026-07-08 CST。

问题：

- `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 为捕获 debug events，会设置 `INPUTIA_DEBUG_EVENTS` 并按默认配置 `killall InputiaInputMethod`。
- 旧顺序是在 TIS 选择前设置 env/restart host；当 TIS 未 ready 时，脚本虽然不会打开 TextEdit/Safari，但仍可能对当前 host/env 产生不必要副作用。
- smoke 的失败 gate 应该尽量只读取状态并早退；debug env/host restart 属于真实 smoke 执行阶段，不应发生在 input source 未选中之前。

实现：

- `smoke-clipboard-recall.sh`
  - 先执行 `inputia_select_input_source_or_exit`。
  - 只有选择成功后才创建 event log、`launchctl setenv INPUTIA_DEBUG_EVENTS`、按需 `killall InputiaInputMethod`。
- `smoke-safari-enter.sh`
  - 同样把 event log/env/host restart 移到 TIS 选择成功之后。
- `verify-nongui.sh`
  - Clipboard TIS gate 不再传 `INPUTIA_RESTART_HOST_FOR_DEBUG=0`；默认 restart 配置也必须在 TIS 未 ready 时无副作用。

直接 gate 验证：

```text
INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 smoke-clipboard-recall.sh build/InputiaInputMethod.app
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  clipboardRecallSmokeReady=false reason=input-source-not-selected
  previousInputSourceID=com.tencent.inputmethod.wetype.pinyin
  selectSourceFoundInEnabledList=false
  inputSourceRestore=skipped reason=already-current
  rc=8
  clipboardUnchanged=true
  currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
```

完整验证：

```text
bash -n smoke-clipboard-recall.sh smoke-safari-enter.sh verify-nongui.sh
  syntaxOK

bash macos/InputiaInputMethod/verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  syntaxOK=true
  pkgVerificationPassed=true
  status.userMatchesBuild=false
  status.running=false
  textEditUiTisGate.rc=15
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.rc=8
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstall.userHostConflict=false
  postInstallRegressionPassed=true
  postInstallUiTisGate.rc=6 reason=tis-not-ready
  awaitShort.rc=2 reason=timeout
  installNoPrompt.rc=12 reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  verifyNonGuiRc=0
```

当前结论：

- Clipboard/Safari Enter 的 debug env 与 host restart 不再发生在 TIS 未 ready 的失败 gate 中。
- 非 GUI 验证覆盖默认 restart 配置；不再依赖 `INPUTIA_RESTART_HOST_FOR_DEBUG=0` 才能安全早退。

## v41 Mac mini 续修：非 GUI 验证新增 debug env 和用户域 host 断言

时间：2026-07-08 CST。

问题：

- 前几轮已经修了 smoke gate 的副作用，但 `verify-nongui.sh` 主要检查进程、当前输入源、剪贴板和 `/tmp` 残留。
- 仍缺两个关键断言：
  - `launchctl` 用户环境里的 `INPUTIA_DEBUG_EVENTS` 不能被失败 gate 改变。
  - `~/Library/Input Methods/InputiaInputMethod.app` / `IputiaInputMethod.app` 不能被失败 gate 创建。
- 这些正是此前用户域 host conflict 和 debug env 残留风险所在，应该成为回归门禁。

实现：

- `verify-nongui.sh`
  - 新增 `debug_events_env()`，读取 `launchctl getenv INPUTIA_DEBUG_EVENTS`。
  - 新增 `assert_debug_env_unchanged(label,before,after)`。
  - 新增 `assert_no_user_host(label)`。
  - TextEdit TIS gate、Clipboard TIS gate、Safari existing gates、post-install UI TIS gate、await short timeout、install no-prompt gate 都纳入 debug env 和 user host 断言。

包重建：

```text
zsh macos/InputiaInputMethod/build-pkg.sh
  pkgVerificationPassed=true
  appCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  latestPkgSHA256=9a42fbbf6afba3f5c3202696c4f67495b5ff16e452921f8e64b5c4cb5c235f56
```

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  pkgVerificationPassed=true
  status.buildCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  status.systemMatchesBuild=false
  status.userMatchesBuild=false
  status.running=false
  textEditUiTisGate.debugEnvBefore=unset
  textEditUiTisGate.debugEnvAfter=unset
  textEditUiTisGate.userHost=false
  clipboardUiTisGate.debugEnvBefore=unset
  clipboardUiTisGate.debugEnvAfter=unset
  clipboardUiTisGate.userHost=false
  safariTypingExistingGate.debugEnvBefore=unset
  safariTypingExistingGate.debugEnvAfter=unset
  safariTypingExistingGate.userHost=false
  safariEnterExistingGate.debugEnvBefore=unset
  safariEnterExistingGate.debugEnvAfter=unset
  safariEnterExistingGate.userHost=false
  safariDiagnoseExistingGate.debugEnvBefore=unset
  safariDiagnoseExistingGate.debugEnvAfter=unset
  safariDiagnoseExistingGate.userHost=false
  postInstallUiTisGate.debugEnvBefore=unset
  postInstallUiTisGate.debugEnvAfter=unset
  postInstallUiTisGate.userHost=false
  awaitShort.debugEnvBefore=unset
  awaitShort.debugEnvAfter=unset
  awaitShort.userHost=false
  installNoPrompt.debugEnvBefore=unset
  installNoPrompt.debugEnvAfter=unset
  installNoPrompt.userHost=false
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  verifyNonGuiRc=0
```

当前结论：

- 非 GUI 回归现在直接覆盖失败 gate 不改 debug env、不创建用户域 host。
- 当前 build/pkg 是 v41 / CDHash `593ceb737c717c27cc46cd159ff5027f5095df8c`；系统安装版仍是 v40，真实 GUI smoke 仍需系统 v41 安装后再跑。

## v41 Mac mini 续修：preflight 和 UI-disabled gate 纳入副作用断言

时间：2026-07-08 CST。

问题：

- 上一轮 `verify-nongui.sh` 已经覆盖 TIS gate、post-install/await/install gate 的 debug env 和用户域 host 断言。
- 但更早的 `smoke-preflight.sh` 失败路径，以及 `INPUTIA_RUN_UI_SMOKE!=1` 的 UI-disabled gate 仍只检查返回码和进程残留。
- 这些 gate 是最常被手动/自动化调用的入口；也应该证明“不设置 debug env、不创建用户域 host”。

实现：

- `verify-nongui.sh`
  - `systemPreflight` 增加 `debugEnvBefore/After` 和 `userHost=false` 断言。
  - `buildPreflightUiDisabled` 增加同样断言。
  - `textEditUiDisabled` / `clipboardUiDisabled` 增加同样断言。
  - `safariTypingUiDisabled` / `safariEnterUiDisabled` / `safariDiagnoseUiDisabled` 增加同样断言。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  pkgVerificationPassed=true
  status.buildCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  systemPreflight.rc=2
  systemPreflight.debugEnvBefore=unset
  systemPreflight.debugEnvAfter=unset
  systemPreflight.userHost=false
  buildPreflightUiDisabled.rc=4
  buildPreflightUiDisabled.debugEnvBefore=unset
  buildPreflightUiDisabled.debugEnvAfter=unset
  buildPreflightUiDisabled.userHost=false
  textEditUiDisabled.rc=14
  textEditUiDisabled.debugEnvBefore=unset
  textEditUiDisabled.debugEnvAfter=unset
  textEditUiDisabled.userHost=false
  clipboardUiDisabled.rc=7
  clipboardUiDisabled.debugEnvBefore=unset
  clipboardUiDisabled.debugEnvAfter=unset
  clipboardUiDisabled.userHost=false
  safariTypingUiDisabled.rc=7
  safariTypingUiDisabled.debugEnvBefore=unset
  safariTypingUiDisabled.debugEnvAfter=unset
  safariTypingUiDisabled.userHost=false
  safariEnterUiDisabled.rc=5
  safariEnterUiDisabled.debugEnvBefore=unset
  safariEnterUiDisabled.debugEnvAfter=unset
  safariEnterUiDisabled.userHost=false
  safariDiagnoseUiDisabled.rc=10
  safariDiagnoseUiDisabled.debugEnvBefore=unset
  safariDiagnoseUiDisabled.debugEnvAfter=unset
  safariDiagnoseUiDisabled.userHost=false
  textEditUiTisGate.debugEnvAfter=unset
  clipboardUiTisGate.debugEnvAfter=unset
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGate.debugEnvAfter=unset
  awaitShort.debugEnvAfter=unset
  installNoPrompt.debugEnvAfter=unset
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  verifyNonGuiRc=0
```

当前结论：

- preflight、UI-disabled、TIS gate、existing-app gate、post-install/await/install gates 现在都统一证明不会改 debug env、不会创建用户域 host。
- 真实 GUI smoke 仍需系统安装版更新到 v41 并通过 TIS readiness 后再执行。

## v41 Mac mini 续修：常用 Command 快捷键覆盖扩展

时间：2026-07-08 CST。

用户反馈：

- 在 Inputia 下 `Command-C` / `Command-V` 不能正常复制粘贴，判断复制/粘贴被输入法接管。
- 要求按常用电脑快捷键举一反三处理，不能靠用户逐个发现。

依据：

- Apple 官方 Mac keyboard shortcuts 文档把 `Command-C`、`Command-V`、`Command-X`、`Command-Z`、`Command-A`、`Command-S`、`Command-O`、`Command-P`、`Command-F`、`Command-H`、`Command-M`、`Command-Tab`、`Command-Space`、`Command-Option-Esc`、`Command-Control-Q`、`Command-Shift-3/4/5` 等归为常用系统/App 快捷键。
- 因此输入法 host 不应逐个接管这些组合；只要 `keyDown` 事件含 `Command`，默认应交回系统或宿主 App responder chain。

实现/选择：

- 保持行为策略：`InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 对所有包含 `.command` 的 `keyDown` 事件返回透传。
- 保持入口顺序：`InputiaInputController.handleKeyDown` 在剪贴板召回、候选导航、标点/中英切换之前先检查 Command 透传。
- 保持 Inputia 自身剪贴板召回为 `Control-Shift-V`，并显式拒绝 `Control-Shift-Command-V`，避免覆盖 App/System Command 组合。
- 扩展自检覆盖，而不是新增一串逐键业务逻辑：
  - 已有：复制、粘贴、剪切、撤销、重做、全选、保存、打开、关闭、退出、查找、偏好设置、切换 App、Spotlight、强制退出。
  - 新增：隐藏/最小化/打印/新建/标签页/选中查找项/斜体/重载/下载/清除样式/使用所选内容、数字视图、括号导航、Command+箭头、Command+Delete、锁屏、截图。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 继续作为 AppKit `didCommand(by:)` selector 路径的显式白名单，覆盖编辑、文件、窗口、查找、格式、帮助和浏览导航类命令。

验证：

```text
swiftc -parse InputiaShortcutSelfCheck.swift InputiaInputTextRouter.swift InputiaShortcutClassifier.swift
  parseOK

swiftc -parse main.swift InputiaHostTextPolicy.swift ... InputiaRustBridge.swift
  parseOK

build.sh
  buildAppCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  codesignVerify=true

build-pkg.sh
  pkgVerificationPassed=true
  archiveAppCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  latestPkgSHA256=9a42fbbf6afba3f5c3202696c4f67495b5ff16e452921f8e64b5c4cb5c235f56

inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandHPassThrough=true
  commandMPassThrough=true
  commandPPassThrough=true
  commandTPassThrough=true
  commandNPassThrough=true
  commandDPassThrough=true
  commandEPassThrough=true
  commandIPassThrough=true
  commandRPassThrough=true
  commandJPassThrough=true
  commandKPassThrough=true
  commandYPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandNumberPassThrough=true
  commandBracketPassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandControlQPassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftCommandVRejected=true

inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandhidePassesThrough=true
  appCommandshowHelpPassesThrough=true
  appCommandtoggleFullScreenPassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandgoForwardPassesThrough=true
  appCommandreloadPassesThrough=true
  appCommandstopLoadingPassesThrough=true

verify-nongui.sh
  status.buildCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  pkgVerificationPassed=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

顺手修复的验证问题：

- 首轮 `verify-nongui.sh` 在最终 residue 检查处曾因短生命周期 `post-install-regression.sh` / `verify-system.sh` 进程尚未完全退出而误报 `reason=residue`。
- 将 residue 收口从固定等待 1 秒改为最多 3 秒轮询；第二轮完整验证通过。

当前结论：

- `Command-C/V` 不再作为单点补丁；Inputia 对所有包含 `Command` 的 keyDown 事件统一透传。
- 自检已经覆盖 Apple 常用 Command 快捷键族和 AppKit selector 路径，后续改动如果重新接管这些快捷键会在非 GUI 验证中暴露。
- 当前系统安装版仍是 v40，build/pkg 是 v41 / CDHash `593ceb737c717c27cc46cd159ff5027f5095df8c`；真实 GUI smoke 仍需系统 v41 安装和 TIS readiness 后再跑。

## v41 Mac mini 续修：真实 smoke 的 debug env 恢复纪律

时间：2026-07-08 CST。

问题：

- `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 的真实路径会设置 `launchctl` 用户环境变量 `INPUTIA_DEBUG_EVENTS`，用于让 Inputia host 写事件日志。
- 旧清理逻辑直接 `launchctl unsetenv INPUTIA_DEBUG_EVENTS`。如果用户或外部诊断工具本来设置了这个 env，真实 smoke 结束会把原值丢掉。
- 另外，如果调用者通过 shell env 显式传入 `INPUTIA_DEBUG_EVENTS=/path/to/log`，旧清理逻辑会把该路径当作 smoke 临时文件删除。

实现：

- `smoke-common.sh`
  - 新增 `inputia_capture_debug_events_env`，在 smoke 修改 launchctl env 之前记录原值。
  - 新增 `inputia_restore_debug_events_env`，清理时恢复原值；原本 unset 才 unset。
- `smoke-clipboard-recall.sh`
  - trap 前捕获原始 debug env。
  - cleanup 和正常 osascript 结束后都调用 restore helper。
  - 若调用者显式提供 `INPUTIA_DEBUG_EVENTS`，不再删除该 event log 文件，只删除 smoke 自己的 select/restore log。
- `smoke-safari-enter.sh`
  - 同样恢复原始 debug env。
  - 显式提供的 event log 不再被 cleanup 删除。
- `verify-nongui.sh`
  - source 公共 helper。
  - 新增 `debug env restore self-check`：临时设置 sentinel，调用 capture/set-temp/restore，确认 restore 回 sentinel；最后通过全局 trap 恢复验证开始前的原始 env。

选择/放弃：

- 选择：把 launchctl env 视为用户会话状态，真实 smoke 修改后必须恢复。
- 放弃：继续简单 unset；原因是它只在“原本 unset”时正确，无法保护已有诊断配置。
- 选择：显式传入的 event log 归调用者所有，smoke 不删除。

验证：

```text
bash -n smoke-common.sh smoke-clipboard-recall.sh smoke-safari-enter.sh verify-nongui.sh
  syntaxOK

launchctl getenv INPUTIA_DEBUG_EVENTS
  before=unset

bash macos/InputiaInputMethod/verify-nongui.sh
  debugEnvRestoreExpected=/tmp/inputia-debug-env-original.6257
  debugEnvRestoreActual=/tmp/inputia-debug-env-original.6257
  debugEnvRestoreSelfCheck=true
  systemPreflight.debugEnvBefore=unset
  systemPreflight.debugEnvAfter=unset
  buildPreflightUiDisabled.debugEnvBefore=unset
  buildPreflightUiDisabled.debugEnvAfter=unset
  textEditUiDisabled.debugEnvBefore=unset
  textEditUiDisabled.debugEnvAfter=unset
  clipboardUiDisabled.debugEnvBefore=unset
  clipboardUiDisabled.debugEnvAfter=unset
  clipboardUiTisGate.debugEnvBefore=unset
  clipboardUiTisGate.debugEnvAfter=unset
  safariEnterUiDisabled.debugEnvBefore=unset
  safariEnterUiDisabled.debugEnvAfter=unset
  safariEnterExistingGate.debugEnvBefore=unset
  safariEnterExistingGate.debugEnvAfter=unset
  postInstallUiTisGate.debugEnvBefore=unset
  postInstallUiTisGate.debugEnvAfter=unset
  awaitShort.debugEnvBefore=unset
  awaitShort.debugEnvAfter=unset
  installNoPrompt.debugEnvBefore=unset
  installNoPrompt.debugEnvAfter=unset
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 真实 smoke 修改 `INPUTIA_DEBUG_EVENTS` 时现在会恢复原值，不再默认清空用户会话里的诊断 env。
- 非 GUI 验证已有 helper 自检，并继续证明失败 gate 不改 debug env、不启动 TextEdit、不创建用户域 host。
- 当前系统安装版仍是 v40；真实 GUI smoke 仍需系统 v41 安装和 TIS readiness 后再跑。

## v41 Mac mini 续修：目标 App 清理必须晚于实际打开阶段

时间：2026-07-08 CST。

问题：

- `smoke-common.sh` 旧的 `inputia_cleanup_textedit_if_started` / `inputia_cleanup_safari_if_started` 只看 preflight 是否为 `not-running`。
- 如果脚本在 UI disabled、TIS 选择失败、Safari/TextEdit preflight 失败等路径提前退出，理论上它还没有打开目标 App；但如果用户恰好在这段窗口打开 TextEdit/Safari，trap 仍可能误清理用户的 App。
- 当前目标是 GUI smoke 失败路径不能抢光标、不能残留、也不能误关用户自己的窗口。

实现：

- `smoke-common.sh`
  - `inputia_cleanup_textedit_if_started` 现在同时要求：
    - `INPUTIA_TEXTEDIT_PREFLIGHT=not-running`
    - `INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1`
  - `inputia_cleanup_safari_if_started` 现在同时要求：
    - `INPUTIA_SAFARI_PREFLIGHT=not-running`
    - `INPUTIA_SAFARI_CLEANUP_ALLOWED=1`
- `smoke-textedit.sh`
  - 只有在 TIS 选择成功、即将执行打开 TextEdit 的 AppleScript 前，才设置 `INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1`。
- `smoke-clipboard-recall.sh`
  - 剪贴板内容写入后、即将执行打开 TextEdit 的 AppleScript 前，才设置 `INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1`。
- `smoke-safari-typing.sh` / `smoke-safari-enter.sh`
  - 只有生成测试 URL 后、即将执行打开 Safari 的 AppleScript 前，才设置 `INPUTIA_SAFARI_CLEANUP_ALLOWED=1`。

选择/放弃：

- 选择：早退 gate 不拥有目标 App，因此不允许 trap 兜底 quit。
- 保留：真实 AppleScript 已进入目标 App 打开阶段后，shell trap 仍能兜底清理脚本启动的 App。
- 放弃：仅凭 preflight `not-running` 判断是否可以 quit；原因是这个条件不能区分脚本打开的 App 和用户在失败窗口中打开的 App。

验证：

```text
bash -n smoke-common.sh smoke-textedit.sh smoke-clipboard-recall.sh smoke-safari-typing.sh smoke-safari-enter.sh verify-nongui.sh
  syntaxOK

bash macos/InputiaInputMethod/verify-nongui.sh
  debugEnvRestoreSelfCheck=true
  textEditUiDisabled.rc=14
  clipboardUiDisabled.rc=7
  textEditUiTisGate.rc=15
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.rc=8
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariTypingExistingGate.rc=9
  safariEnterExistingGate.rc=7
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGate.rc=6
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- UI disabled、TIS not-ready、Safari 已运行等早退路径不再具备“退出目标 App”的权限。
- 真实 GUI smoke 进入打开目标 App 阶段后，仍保留兜底清理能力。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 仍需系统 v41 安装并通过 TIS readiness 后再跑。

## v41 Mac mini 续修：把清理权限协议纳入非 GUI 回归

时间：2026-07-08 CST。

问题：

- 上一轮为 TextEdit/Safari smoke 增加了 `INPUTIA_*_CLEANUP_ALLOWED`，但这个约束只靠人工检查。
- `diagnose-safari-input-source.sh` 也会在真实路径打开 Safari，并复用 `inputia_cleanup_safari_if_started`；如果不显式加入同一协议，真实诊断路径会失去兜底关闭脚本自启 Safari 的能力。
- 后续维护时如果某个脚本把 cleanup flag 放到 TIS gate 前，早退路径又可能恢复误清理用户 App 的风险。

实现：

- `diagnose-safari-input-source.sh`
  - 在生成本地测试 URL 后、调用打开 Safari 的 AppleScript 前设置 `INPUTIA_SAFARI_CLEANUP_ALLOWED=1`。
- `verify-nongui.sh`
  - 新增 `verify_cleanup_permission_contract`。
  - 静态检查 `smoke-common.sh` 中 TextEdit/Safari cleanup helper 必须依赖对应 `INPUTIA_*_CLEANUP_ALLOWED`。
  - 静态检查以下脚本的 cleanup marker 必须存在，且顺序必须是 `inputia_select_input_source_or_exit` 之后、目标 App AppleScript 之前：
    - `smoke-textedit.sh`
    - `smoke-clipboard-recall.sh`
    - `smoke-safari-typing.sh`
    - `smoke-safari-enter.sh`
    - `diagnose-safari-input-source.sh`

选择/放弃：

- 选择：用非 GUI 静态契约锁住清理权限时序，因为真实 race 难以稳定复现，且不能为了验证而打开/关闭用户 App。
- 放弃：只依赖完整 GUI smoke 验证；原因是当前系统 v40/TIS not-ready，真实 GUI smoke 不能安全运行。

验证：

```text
bash -n diagnose-safari-input-source.sh verify-nongui.sh
  syntaxOK

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  pkgVerificationPassed=true
  status.buildCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallTISBlockReason=missing-enabled-source
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  verifyNonGuiRc=0
```

当前结论：

- 所有会打开 TextEdit/Safari 且复用公共 cleanup helper 的 smoke/diagnose 脚本都纳入了 cleanup permission 协议。
- 非 GUI 验证现在会防止 cleanup flag 提前到 TIS gate 前，或在新脚本中遗漏对应 marker。
- 当前真实 GUI smoke 仍需系统 v41 安装和 TIS readiness 后再跑。

## v41 Mac mini 续修：TIS readiness 输出可机器读取的阻塞原因

时间：2026-07-08 CST。

问题：

- 真实 GUI smoke 目前被 TIS readiness gate 阻挡，但旧输出只有 `tis-not-ready`。
- 这不足以区分“未安装/未启用”、“图标匹配异常”、“简体中文源禁用”或其他状态，后续排查容易误把 TIS 不就绪当成 GUI smoke 脚本失败。

实现：

- `smoke-preflight.sh`
  - 新增 `tis.readinessBlockReason`。
  - 目前可输出 `missing-enabled-source`、`icon-mismatch`、`hans-disabled`、`unknown` 或 `none`。
- `post-install-regression.sh`
  - `postInstallTISReady=false` 时同步输出 `postInstallTISBlockReason`。
- `await-system-install.sh`
  - 等待循环的单行 TIS 状态追加 `tis.readinessBlockReason`。

选择/放弃：

- 选择：沿用现有 TIS/status 探针，只把决策原因结构化输出。
- 放弃：在 preflight 里自动 register/enable Inputia；原因是真实 smoke 默认必须保持 select-only，不应在失败 gate 中修改用户输入源状态。

验证：

```text
INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  textEditPreflight=not-running docs=0
  safariPreflight=running
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.hansIconURL=unknown
  tis.hansIconMatchesApp=false
  tis.hansEnabled=unknown
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  smokePreflightReady=false reason=tis-not-ready
  rc=8

INPUTIA_RUN_UI_SMOKE=1 \
  zsh macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallTISReady=false
  postInstallTISBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready
  rc=6

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
  zsh macos/InputiaInputMethod/await-system-install.sh
  tis.readinessBlockReason=missing-enabled-source
  systemInstallObserved=false reason=timeout
  rc=2

bash macos/InputiaInputMethod/verify-nongui.sh
  postInstallTISBlockReason=missing-enabled-source
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TIS 不就绪现在能明确报告为 `missing-enabled-source`，而不是只给出泛化 `tis-not-ready`。
- 当前 Mac mini 上系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 仍不能跑，直到系统安装版更新且 TIS 能在 enabled source 列表中被 select-only 找到。

## v41 Mac mini 续修：TIS tool 增加只读当前输入源输出

时间：2026-07-08 CST。

问题：

- 公共 smoke helper 已经按 `--dump-current-input-source` 读取当前输入源，用于选择前记录和失败后恢复。
- 但 `inputia-tis-tool` 实际只有 `--dump`、`--reset-enable` 和 select 命令。直接调用缺失命令会进入 unknown-command 分支，无法作为独立只读探针使用。
- 真实 GUI smoke 的清理纪律需要一个不会 register/enable/select 的当前输入源探针，避免验证或恢复路径误碰 TIS 注册状态。

实现：

- `Tools/InputiaTISTool.swift`
  - 新增 `--dump-current-input-source`。
  - 输出当前输入源的 `id`、`bundle`、`mode`、`name`、`category`、`type`、`iconURL`、`languages`、`enabled`、`selectable`、`selected` 等字段。
  - usage 中补充该只读命令。
- 默认无参行为保持不变，仍是 register/enable/select Inputia；真实 smoke 默认仍走 select-only。

选择/放弃：

- 选择：把 current-source 诊断放在 TIS tool 里，而不是继续依赖 host executable 的 CLI。
- 原因：TIS 诊断、选择和恢复都属于同一个系统输入源边界，统一在 `inputia-tis-tool` 中更容易保证“只读路径不注册、不启用、不选择”。
- 放弃：让 `--dump` 顺带打印当前输入源；原因是 `--dump` 现在语义是列出 Inputia 匹配源，拆开命令可以避免解析歧义。

验证：

```text
swiftc -parse-as-library -typecheck macos/InputiaInputMethod/Tools/InputiaTISTool.swift
  rc=0

zsh macos/InputiaInputMethod/build-pkg.sh
  pkgVerificationPassed=true
  appCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  latest pkg sha256=a4c70c7dc96e785e00c15a295cb671725d7c6aa78440dc93f509397806151878

macos/InputiaInputMethod/build/inputia-tis-tool --help
  usage=inputia-tis-tool [--dump|--dump-current-input-source|--reset-enable|--select-inputia-source-id <id>|--select-source-id <id>]
  defaultAction=register-enable-select-inputia

macos/InputiaInputMethod/build/inputia-tis-tool --dump-current-input-source
  id=com.tencent.inputmethod.wetype.pinyin
  bundle=com.tencent.inputmethod.wetype
  mode=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  rc=8

bash macos/InputiaInputMethod/verify-nongui.sh
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallTISBlockReason=missing-enabled-source
  awaitShort.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  awaitShort.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `inputia-tis-tool --dump-current-input-source` 现在是可独立调用的只读探针，能证明当前输入源仍是微信输入法并支持 smoke 失败路径的恢复验证。
- 新构建和 package 仍通过；真实 GUI smoke 仍被 `missing-enabled-source` 正确阻挡，没有触发 TextEdit/Safari/Clipboard 实际输入测试。

## v41 Mac mini 续修：current-source 查询统一走 TIS tool

时间：2026-07-08 CST。

问题：

- 上一轮虽然新增了 `inputia-tis-tool --dump-current-input-source`，但多个脚本仍通过 `InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --dump-current-input-source` 读取当前输入源。
- 这个 host CLI 路径会短暂出现 `InputiaInputMethod --bridge-...` 自检进程。它不是长期残留，但会干扰最终残留采样，也不符合“只读 TIS 诊断不启动 host”的 smoke 纪律。

实现：

- `smoke-common.sh`
  - `inputia_current_input_source_id` 新增可选 `tis_tool` 参数。
  - 选择前记录、恢复前后确认优先使用 TIS tool，只在缺工具时 fallback 到 host executable。
- `smoke-preflight.sh`、`tis-readiness.sh`、`await-system-install.sh`、`post-install-regression.sh`、`verify-nongui.sh`
  - 当前输入源查询优先走 `inputia-tis-tool --dump-current-input-source`。
- `diagnose-safari-input-source.sh`
  - 当前输入源打印和解析改为优先走 TIS tool。
- `tis-readiness.sh`
  - 补齐 `tis.readinessBlockReason`，与 preflight/post-install/await 输出保持一致。

选择/放弃：

- 选择：把只读当前输入源查询统一放在 TIS tool，减少 host 进程瞬态和误判。
- 放弃：在状态采样里继续调用 host CLI；原因是它会让“无 host 残留”的证明变弱。

验证：

```text
bash -n smoke-common.sh smoke-preflight.sh tis-readiness.sh verify-nongui.sh diagnose-safari-input-source.sh
zsh -n await-system-install.sh post-install-regression.sh
  syntaxOK

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  rc=8

bash macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  tisReadiness=false
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  tisReadinessBuild: tis.readinessBlockReason=missing-enabled-source
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  awaitShort.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  awaitShort.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 安全 gate、TIS readiness、post-install 和 await 的 current-source 诊断现在默认不启动 Inputia host。
- 当前系统状态仍是系统安装版 v40、build/pkg v41，Inputia TIS enabled/installed matches 为 0；真实 GUI smoke 继续正确停止在 readiness gate。

## v41 Mac mini 续修：clipboard smoke 的剪贴板恢复契约

时间：2026-07-08 CST。

问题：

- `smoke-clipboard-recall.sh` 的真实路径会写入系统剪贴板，因此恢复契约必须比“脚本最终通过”更早被验证。
- 之前 `verify-nongui.sh` 已在 TIS gate 验证 clipboard 不变，但 UI-disabled gate 只验证了 debug env 和进程残留，没有显式证明它没有触碰剪贴板。
- 静态 cleanup contract 已覆盖 TextEdit/Safari cleanup 权限，但没有覆盖 clipboard restore 的顺序约束。

实现：

- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 clipboard 静态契约：
    - 必须先定义基于 `CLIPBOARD_CHANGED=1` 的条件恢复。
    - 必须先读取 `ORIGINAL_CLIPBOARD`。
    - 再写入测试剪贴板。
    - 写入后才设置 `CLIPBOARD_CHANGED=1`。
  - `clipboardUiDisabled` gate 前后读取 `pbpaste`，确认 UI-disabled 路径不会改变系统剪贴板。

选择/放弃：

- 选择：在非 GUI 验证里同时做静态顺序检查和运行时不变性检查。
- 放弃：等真实 GUI smoke 才发现剪贴板恢复问题；原因是恢复纪律属于 smoke 安全前置条件，应该在 gate 层就能验证。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  rc=8

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardUiDisabled.rc=7
  clipboardUiDisabled.clipboardUnchanged=true
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.debugEnvBefore=unset
  clipboardUiTisGate.debugEnvAfter=unset
  clipboardUiTisGate.userHost=false
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard smoke 的 UI-disabled 和 TIS gate 都已证明不会污染系统剪贴板。
- 真实 clipboard recall 路径仍需系统安装版更新到 v41 且 TIS readiness 通过后再跑；当前正确停在 `missing-enabled-source`。

## v41 Mac mini 续修：TextEdit/Clipboard 状态清空与事件契约

时间：2026-07-08 CST。

问题：

- 原始交接要求继续修 GUI smoke 测试纪律，特别是 TextEdit 焦点丢失、IME composition 状态污染、下箭头候选展开/翻页、clipboard recall shown/commit。
- 这些逻辑在 smoke 脚本里存在，但之前非 GUI 验证没有把它们锁成契约；后续重构可能删除 `clearInputiaState`、`focus-lost` 检查、`arrowCandidateResult` 或 clipboard event 校验而不被发现。

实现：

- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 TextEdit 静态契约：
    - 必须有 `clearInputiaState()`。
    - 必须通过 Escape 清空状态。
    - 必须检查 `state-clear-leaked-text:`。
    - 必须保留 `focus-lost:` 断言。
    - 必须覆盖 `arrowCandidateResult` / `arrow-down-commit` / Down Arrow key code。
  - `cleanupPermissionContract` 新增 Clipboard 静态契约：
    - 必须有 `clearInputiaState()` 和 `state-clear-leaked-text:`。
    - 必须等待 `clipboardRecallShown`。
    - 必须检查 `clipboardRecallCommit index=0`。
    - 必须检查 `clipboardRecallCommit index=0 text=$expected`。

选择/放弃：

- 选择：把真实 GUI smoke 的关键纪律做成静态契约，先防止脚本退化。
- 放弃：在 TIS 未就绪时强行跑 TextEdit/Clipboard；原因是当前 select-only gate 已证明 Inputia 不在 enabled source 列表里，硬跑会污染用户 GUI 会话且不会得到有效 Inputia 行为证据。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  rc=8

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardUiDisabled.clipboardUnchanged=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TextEdit 的状态清空、焦点断言、下箭头候选覆盖，以及 Clipboard recall shown/commit/text 匹配，现在都被非 GUI 验证锁住。
- 真实 GUI smoke 仍等待系统安装/TIS readiness；当前仍正确停在 `missing-enabled-source`，没有触发 TextEdit/Clipboard 实际输入。

## v41 Mac mini 续修：debug event 与 host restart 时序契约

时间：2026-07-08 CST。

问题：

- `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 需要设置 `INPUTIA_DEBUG_EVENTS` 并重启 Inputia host，才能在真实 GUI smoke 中读取事件日志。
- 但这两个动作必须发生在 TIS select 成功之后。否则 TIS gate 失败时也可能污染 launchctl debug env 或重启 host。
- 之前运行时 gate 已证明 debug env 不变，但没有静态锁住“select 之前不得 setenv/killall”的顺序。

实现：

- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 debug-event smoke 静态契约，覆盖 `smoke-clipboard-recall.sh` 与 `smoke-safari-enter.sh`：
    - 必须先 `inputia_capture_debug_events_env`。
    - 必须安装 `trap cleanup_smoke EXIT`。
    - 必须先 `inputia_select_input_source_or_exit`。
    - 然后才能 `launchctl setenv INPUTIA_DEBUG_EVENTS`。
    - 最后才允许 `killall InputiaInputMethod`。
    - 显式传入的 event log 必须走 caller-owned cleanup 分支，生成的 event log 才能由 smoke 删除。

选择/放弃：

- 选择：静态检查时序，运行时 gate 验证不变性。
- 放弃：只依赖 TIS gate 的 debugEnvBefore/debugEnvAfter；原因是它能发现结果污染，但不能阻止未来重构把 setenv/killall 提前到 select 之前。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  rc=8

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  clipboardUiTisGate.debugEnvBefore=unset
  clipboardUiTisGate.debugEnvAfter=unset
  safariEnterExistingGate.debugEnvBefore=unset
  safariEnterExistingGate.debugEnvAfter=unset
  postInstallUiTisGate.debugEnvBefore=unset
  postInstallUiTisGate.debugEnvAfter=unset
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Debug event env 和 host restart 时序现在被非 GUI 验证锁住；TIS gate 失败不会先 setenv 或重启 host。
- 当前真实 GUI smoke 仍正确停止在 `missing-enabled-source`。

## v41 Mac mini 续修：Safari 诊断页按窗口 id 清理

时间：2026-07-08 01:16 CST。

问题：

- `diagnose-safari-input-source.sh` 在真实路径会打开 Safari 本地 `data:` 测试页。
- 脚本已经有 Safari preflight 和 cleanup permission gate，但诊断页自身没有记录窗口 id；如果 Safari 是脚本允许打开的目标，后续只能依赖公共 cleanup，不能精确关闭自己创建的窗口。
- 用户已经明确要求 smoke/诊断不能抢光标、不能残留窗口；Safari 诊断路径需要和 Safari typing/enter smoke 一样具备窗口级清理纪律。

实现：

- `diagnose-safari-input-source.sh`
  - 新增 `SAFARI_DIAGNOSE_WINDOW_ID`。
  - 打开 Safari 诊断页后立即 `return id of front window` 并记录 `safariDiagnoseWindowID=...`。
  - trap 清理阶段调用 `close_safari_diagnose_window`，只关闭该窗口 id。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 继续要求 Safari cleanup permission 只能在 TIS 选择成功后、即将打开目标 App 前设置。
  - 新增 Safari 窗口 id 静态契约：
    - `smoke-safari-typing.sh` 和 `smoke-safari-enter.sh` 必须捕获 `smokeWindowId` 并用 `close window id smokeWindowId` 清理。
    - `diagnose-safari-input-source.sh` 必须捕获 `SAFARI_DIAGNOSE_WINDOW_ID`，并用同一个窗口 id 清理。

选择/放弃：

- 选择：真实会打开 Safari 的路径都按“脚本创建哪个窗口，就只关哪个窗口”的规则约束。
- 放弃：只依赖 `inputia_cleanup_safari_if_started`；原因是它只能证明脚本自启动 Safari 时可退出 Safari，不能证明已运行 Safari 中的测试窗口不会残留。

验证：

```text
bash -n macos/InputiaInputMethod/diagnose-safari-input-source.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandHPassThrough=true
  commandMPassThrough=true
  commandPPassThrough=true
  commandTPassThrough=true
  commandNPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandOptionEscapePassThrough=true
  postInstallTISBlockReason=missing-enabled-source
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Command+C/V 以及 Apple 官方常用 Command 系列快捷键现在按类别整体放行，不再逐个靠用户发现。
- Safari 诊断和 Safari smoke 都具备窗口 id 清理契约；非 GUI 验证已把该契约固定下来。
- 当前 Mac mini 仍是系统安装版 v40、build/pkg v41，Inputia TIS `enabled/installed matches=0`；真实 GUI smoke 继续正确停止在 `missing-enabled-source`，未打开 TextEdit/Safari/Clipboard 输入测试。

## v41 Mac mini 续修：外部 debug event log 不再被 smoke 删除

时间：2026-07-08 01:20 CST。

问题：

- `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 已经在 cleanup 阶段保留调用方传入的 `INPUTIA_DEBUG_EVENTS` 文件，但启动阶段仍无条件 `/bin/rm -f "$EVENT_LOG"`。
- 这会让外部传入的诊断日志路径被删除再重建，和“调用方提供的日志不由 smoke 删除”的调试纪律不一致。
- clipboard recall 的状态污染排查依赖事件日志；外部日志应该可以被调用方稳定持有，smoke 只负责截断本轮内容。

实现：

- `smoke-common.sh`
  - 新增 `inputia_prepare_debug_event_log(event_log, event_log_provided)`。
  - 外部传入日志时使用 shell truncation `: >"$event_log"`，保留文件路径和调用方所有权。
  - smoke 自生成日志时仍删除旧临时文件，cleanup 阶段继续删除临时日志。
- `smoke-clipboard-recall.sh`
  - 使用 `inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"` 替代无条件删除。
- `smoke-safari-enter.sh`
  - 同样改为公共初始化函数。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态约束：
    - `smoke-common.sh` 必须提供公共 event log 初始化函数。
    - clipboard recall 和 Safari enter smoke 必须调用该函数。
    - 两个脚本不得出现无条件 `/bin/rm -f "$EVENT_LOG"`。
    - 初始化顺序必须在 TIS 选择之后、`launchctl setenv INPUTIA_DEBUG_EVENTS` 之前。

选择/放弃：

- 选择：调用方提供的日志只截断，不删除；smoke 自生成的临时日志仍由 smoke 清理。
- 放弃：继续无条件删除日志；原因是外部日志路径可能由上层验证器持有，删除会破坏后续排查和文件生命周期边界。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  rc=0

rg -n 'inputia_prepare_debug_event_log|rm -f "\$EVENT_LOG"|EVENT_LOG_PROVIDED|unconditional-event-log-delete' \
  macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  smoke-common.sh: inputia_prepare_debug_event_log()
  smoke-clipboard-recall.sh: inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"
  smoke-safari-enter.sh: inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"
  verify-nongui.sh: unconditional-event-log-delete contract present
  no target smoke script contains /bin/rm -f "$EVENT_LOG"

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardUiDisabled.clipboardUnchanged=true
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallTISBlockReason=missing-enabled-source
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- clipboard recall 和 Safari enter smoke 的外部 debug log 生命周期现在和 cleanup 行为一致：外部文件保留、临时文件清理。
- Clipboard recall smoke 仍具备 ESC 清状态、空文档断言、`clipboardRecallShown` 和 `clipboardRecallCommit index=0 text=$expected` 检查；真实路径仍需系统安装版更新到 v41 且 TIS readiness 通过后执行。

## v41 Mac mini 续验：debug event log 生命周期运行时自检

时间：2026-07-08 01:23 CST。

问题：

- 上一轮已把外部 `INPUTIA_DEBUG_EVENTS` 不删除改成静态合同，但静态合同只能证明脚本文本形状，不能证明 helper 的运行时行为。
- Clipboard recall 的 GUI smoke 依赖 event log 判断 `clipboardRecallShown` 和 `clipboardRecallCommit`；如果日志初始化行为漂移，会重新引入状态污染排查盲区。

实现：

- `verify-nongui.sh`
  - 新增 `debug event log lifecycle self-check`：
    - 创建 provided log，记录 inode。
    - 调用 `inputia_prepare_debug_event_log "$provided_event_log" "provided"`。
    - 断言 provided log 的 inode 不变、size 变为 0。
    - 创建 generated log，调用 `inputia_prepare_debug_event_log "$generated_event_log" ""`。
    - 断言 generated log 被删除。
  - 自检后显式清理两个 `/tmp/inputia-debug-event-*` 文件。

选择/放弃：

- 选择：在非 GUI 验证中加入真实文件系统行为检查，和静态合同互补。
- 放弃：只靠 `rg`/文本合同；原因是文件生命周期问题属于运行时副作用，应该用运行时证据锁住。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh macos/InputiaInputMethod/smoke-common.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogProvidedInodeBefore=51290050
  debugEventLogProvidedInodeAfter=51290050
  debugEventLogProvidedSizeAfter=0
  debugEventLogLifecycleSelfCheck=true
  postInstallTISBlockReason=missing-enabled-source
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 外部 debug event log 的“不删除、只截断”行为现在有运行时证据。
- 自生成 debug event log 的“旧临时文件先删除”行为也有运行时证据。
- 当前系统状态仍是系统安装版 v40、build/pkg v41，真实 GUI smoke 仍需系统安装版更新并通过 TIS readiness 后再跑。

## v41 Mac mini 续修：TextEdit smoke 按文档引用清理

时间：2026-07-08 01:26 CST。

问题：

- `smoke-textedit.sh` 已经在每个 case 中保存 `docRef`，但 `cleanupDoc(docRef)` 实际关闭的是 `front document`。
- `smoke-clipboard-recall.sh` 在 AppleScript 中创建 TextEdit 文档时没有保存文档引用，cleanup 主要靠“关闭多出来的 front document”。
- 默认 preflight 会阻止 TextEdit 已运行的情况，但 smoke 清理纪律不应依赖“front document 一定是脚本自己的文档”这个假设。

实现：

- `smoke-textedit.sh`
  - `cleanupDoc(docRef)` 改为 `close docRef saving no`，不再关闭 `front document`。
- `smoke-clipboard-recall.sh`
  - 新增 `smokeDocument` 引用。
  - 创建 TextEdit 文档时 `set smokeDocument to make new document`。
  - cleanup 优先 `close smokeDocument saving no`。
  - 原来的按数量清理多余文档逻辑保留为兜底。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态约束：
    - TextEdit smoke 必须捕获 `docRef` 并关闭 `docRef`。
    - Clipboard recall smoke 必须初始化、捕获、传递并关闭 `smokeDocument`。

选择/放弃：

- 选择：按脚本自己创建的文档引用清理，降低误关用户文档的风险。
- 放弃：继续关闭 `front document`；原因是焦点/前台窗口是运行时状态，失败路径下不应作为清理目标的唯一依据。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  rc=0

rg -n 'cleanupDoc|close docRef|smokeDocument|cleanupTextEdit\(' \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  smoke-textedit.sh: close docRef saving no
  smoke-clipboard-recall.sh: set smokeDocument to make new document
  smoke-clipboard-recall.sh: close smokeDocument saving no
  verify-nongui.sh: clipboard/textedit doc-ref cleanup contract present

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEventLogLifecycleSelfCheck=true
  textEditUiDisabled.rc=14
  clipboardUiDisabled.rc=7
  clipboardUiDisabled.clipboardUnchanged=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TextEdit 和 Clipboard recall smoke 的 TextEdit 文档清理现在优先针对脚本创建的文档引用。
- 当前系统安装版仍是 v40，build/pkg v41；真实 GUI smoke 仍被 `missing-enabled-source` 正确阻挡，没有触发实际 TextEdit 输入测试。

## v41 Mac mini 续修：AppleScript 编译验证必须双重 opt-in

时间：2026-07-08 01:32 CST。

问题：

- `verify-nongui.sh` 原本支持 `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1` 时用 `osacompile` 编译 smoke AppleScript 片段。
- 实测发现 `osacompile` 不执行脚本主体，但会因为解析应用术语启动目标 App：本次试跑时 TextEdit 被启动，随后 `verify-nongui.sh` 在 post-install UI TIS gate 的残留断言失败。
- 这说明“AppleScript 编译验证”不是纯非 GUI 行为，不能由单个看似只读的开关触发。

实现：

- `verify-nongui.sh`
  - AppleScript 编译改为双重 opt-in：
    - `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1`
    - `INPUTIA_ALLOW_APPLESCRIPT_COMPILE_APP_LAUNCH=1`
  - 只设置 `INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1` 时，输出：
    - `appleScriptCompileSkipped=true reason=osacompile-may-launch-target-apps`
  - `cleanupPermissionContract` 新增静态约束，要求 `verify-nongui.sh` 保留这个额外 launch gate 和 unsafe skip 输出。

选择/放弃：

- 选择：默认和单开关模式都不运行 `osacompile`，避免非 GUI 验证启动 TextEdit/Safari。
- 放弃：把 AppleScript 编译纳入默认非 GUI；原因是官方工具链的编译阶段也可能启动目标 App，和本项目 smoke 清理纪律冲突。

验证：

```text
INPUTIA_VERIFY_APPLESCRIPT_COMPILE=1 bash macos/InputiaInputMethod/verify-nongui.sh
  appleScriptCompileSkipped=true reason=osacompile-may-launch-target-apps
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogLifecycleSelfCheck=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
```

清理：

- 发现 `osacompile` 启动 TextEdit 后，已执行 `tell application "TextEdit" to quit saving no` 清理。
- 后续轮询确认无 `TextEdit`、`osascript`、`InputiaInputMethod --bridge-*`、`verify-nongui.sh`、`post-install-regression.sh`、`verify-system.sh` 残留。

当前结论：

- AppleScript 片段编译验证的副作用已被记录并用双重 opt-in 隔离。
- 默认非 GUI 验证不再因 AppleScript 编译启动 TextEdit/Safari。
- 当前系统安装版仍是 v40，build/pkg v41；真实 GUI smoke 仍需系统版更新并通过 TIS readiness 后再执行。

## v41 Mac mini 续修：安装后 TIS readiness 增加菜单刷新后采样

时间：2026-07-08 01:35 CST。

问题：

- `install-system.sh` 已在 register/enable/select 后调用 `tis-readiness.sh "$DEST_APP"`，输出 `systemInstallTISReady=true/false`。
- 这次采样发生在 `TextInputMenuAgent` / `SystemUIServer` 重启之前，可以保留刷新前的原始 TIS 证据。
- 但真实 GUI smoke 能否跑，更依赖菜单代理刷新后的 TIS 状态；只有刷新前采样会让安装后诊断少一段关键证据。

实现：

- `install-system.sh`
  - 保留现有刷新前采样：
    - `systemInstallTIS: ...`
    - `systemInstallTISReady=true/false`
  - 在 `killall TextInputMenuAgent` 和 `killall SystemUIServer` 后等待 1 秒。
  - 再次调用 `tis-readiness.sh "$DEST_APP"`，输出：
    - `systemInstallPostRefreshTIS: ...`
    - `systemInstallPostRefreshTISReady=true/false`
- `verify-nongui.sh`
  - 静态合同要求：
    - 刷新前 `tis-readiness.sh "$DEST_APP"` 仍存在。
    - `killall TextInputMenuAgent` 与 `killall SystemUIServer` 存在。
    - `systemInstallPostRefreshTIS:` 必须出现在 SystemUIServer 刷新之后。
    - `systemInstallPostRefreshTISReady=` 必须出现在 post-refresh TIS 输出之后。

选择/放弃：

- 选择：保留刷新前采样，同时增加刷新后采样，形成安装后 TIS 状态的前后对比。
- 放弃：把原采样简单移动到刷新后；原因是刷新前失败信息对定位 LaunchServices/TIS 注册缓存问题仍有价值。

验证：

```text
zsh -n macos/InputiaInputMethod/install-system.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogLifecycleSelfCheck=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 下一次管理员安装 v41 后，安装脚本会同时记录菜单代理刷新前和刷新后的 TIS readiness。
- 当前系统安装版仍是 v40，build/pkg v41；真实 GUI smoke 仍不能安全执行，继续等待系统安装版更新且 TIS enabled/installed matches 恢复。

## v41 Mac mini 续修：pkg postinstall 也记录菜单刷新后 TIS dump

时间：2026-07-08 01:39 CST。

问题：

- 上一轮补强的是 `install-system.sh`，但 latest pkg 的真实安装路径使用 `Packaging/scripts/postinstall`。
- `postinstall` 已在 register/enable/select 后刷新 `TextInputMenuAgent` 和 `SystemUIServer`，但没有输出刷新后的 TIS dump。
- 管理员安装 pkg 后如果 TIS 仍不可选，缺少刷新后证据会降低排查效率。

实现：

- `Packaging/scripts/postinstall`
  - 在 `killall TextInputMenuAgent` / `killall SystemUIServer` 后等待 1 秒。
  - 使用已安装 host 输出：
    - `inputiaPostRefreshEnabledTIS: ...` 来自 `--dump-enabled-input-source`
    - `inputiaPostRefreshCurrentTIS: ...` 来自 `--dump-current-input-source`
- `verify-pkg.sh`
  - 新增 postinstall 行为检查：
    - 必须包含 `inputiaPostRefreshEnabledTIS:`
    - 必须包含 `inputiaPostRefreshCurrentTIS:`
- 重建 latest pkg。

选择/放弃：

- 选择：pkg postinstall 用已安装 host CLI dump TIS 状态，而不是依赖 repo 里的 `tis-readiness.sh`。
- 放弃：让 pkg postinstall 调用 repo 脚本；原因是 pkg 安装时只保证 package scripts 和安装后的 app 存在，不能依赖开发工作区路径。

验证：

```text
zsh -n macos/InputiaInputMethod/Packaging/scripts/postinstall \
  macos/InputiaInputMethod/verify-pkg.sh \
  macos/InputiaInputMethod/build-pkg.sh
  rc=0

bash macos/InputiaInputMethod/build-pkg.sh
  pkgbuild: Wrote package to .../InputiaInputMethod-v41-593ceb737c71.pkg
  appCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  sha256=ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d
  sourcePostinstallSHA256=132f940940d6a1622004d8fa8a172ebd7b7acd6031523fe9ca34d914499a121c
  pkgPostinstallSHA256=132f940940d6a1622004d8fa8a172ebd7b7acd6031523fe9ca34d914499a121c
  postinstallBehaviorChecks=true
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: sha256=ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d
  verifyPkg: postinstallBehaviorChecks=true
  verifyPkg: pkgVerificationPassed=true
  status: sha256=ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `install-system.sh` 和 pkg `postinstall` 都会在菜单代理刷新后输出 TIS 相关证据。
- 最新 package 已重建，latest pkg SHA 为 `ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d`。
- 当前系统安装版仍是 v40；真实 GUI smoke 仍等待管理员安装 v41 后再执行。

## v41 Mac mini 续验：pkg postinstall TIS dump 顺序纳入 verify-pkg

时间：2026-07-08 01:44 CST。

问题：

- 上一轮 `verify-pkg.sh` 已检查 postinstall 包含刷新后 TIS dump 字段，但只是存在性检查。
- 如果未来有人把 `inputiaPostRefreshEnabledTIS:` / `inputiaPostRefreshCurrentTIS:` 移到菜单代理刷新前，存在性检查仍会通过，证据含义会变弱。
- `inputiaPostRefreshTISReady=` 的字符串在 helper 函数定义中也会出现，顺序检查不能简单匹配第一处 echo 文本。

实现：

- `verify-pkg.sh`
  - 新增 `require_order(first_pattern, second_pattern, path, reason)`。
  - 验证 package 内展开后的 `Scripts/postinstall` 顺序：
    - `inputia-select` 早于 `TextInputMenuAgent`
    - `TextInputMenuAgent` 早于 `SystemUIServer`
    - `SystemUIServer` 早于 `inputiaPostRefreshEnabledTIS:`
    - `inputiaPostRefreshEnabledTIS:` 早于执行段的 `post_refresh_tis_ready "$post_refresh_enabled_output"`
    - `post_refresh_tis_ready "$post_refresh_enabled_output"` 早于 `inputiaPostRefreshCurrentTIS:`

选择/放弃：

- 选择：顺序检查匹配执行段中的 `post_refresh_tis_ready "$post_refresh_enabled_output"`，避开 helper 函数定义里的 `inputiaPostRefreshTISReady=` echo。
- 放弃：只 grep `inputiaPostRefreshTISReady=` 的第一处位置；原因是它会匹配函数定义，不代表实际执行顺序。

验证：

```text
zsh -n macos/InputiaInputMethod/verify-pkg.sh
  rc=0

bash macos/InputiaInputMethod/verify-pkg.sh
  sha256=ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d
  sourcePostinstallSHA256=132f940940d6a1622004d8fa8a172ebd7b7acd6031523fe9ca34d914499a121c
  pkgPostinstallSHA256=132f940940d6a1622004d8fa8a172ebd7b7acd6031523fe9ca34d914499a121c
  postinstallBehaviorChecks=true
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: postinstallBehaviorChecks=true
  verifyPkg: pkgVerificationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- pkg postinstall 的刷新后 TIS dump 现在不仅要求存在，还要求位于菜单代理刷新之后。
- 本轮只增强验证脚本，没有改变 package 内容；latest pkg SHA 仍为 `ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d`。

## v41 Mac mini 续修：非 GUI 验证残留等待与诊断

时间：2026-07-08 01:25 CST。

问题：

- 一次 `verify-nongui.sh` 调试运行在 `safari-existing-gate-left-inputia-host` 失败。
- 当时没有 TextEdit/osascript 残留；失败点是在 Safari existing gate 后检查 `InputiaInputMethod` 进程。
- 现有 `assert_process_not_running` 只等待 30 次、每次 0.1 秒，也就是 3 秒。
- `post-install-regression.sh` / `verify-system.sh` 会运行 host self-check；这类短命自检进程可能超过 3 秒才退出，导致验证纪律自身误判。

实现：

- `verify-nongui.sh`
  - 新增 `process_details(process_name)`，失败时输出 PID、父 PID、运行时长、comm 和完整命令行。
  - `assert_process_not_running` 默认等待改为 `INPUTIA_PROCESS_WAIT_TICKS:-100`，也就是最多 10 秒。
  - residue 收尾检查使用同一个 `INPUTIA_PROCESS_WAIT_TICKS` 上限。

选择/放弃：

- 选择：把等待上限从 3 秒提高到 10 秒，并保留硬失败；超过上限仍视为真实残留。
- 放弃：直接忽略 `InputiaInputMethod` host 残留；原因是 GUI smoke 清理纪律必须继续防止真实 host 泄漏。
- 放弃：按命令行白名单跳过 self-check 进程；原因是会掩盖 host 生命周期漂移，等待并输出诊断更稳。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
  bash macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiConsoleUser=lizhelang
  textEditPreflight=not-running docs=0
  safariPreflight=running
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  smokePreflightReady=false reason=tis-not-ready
  rc=8

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogLifecycleSelfCheck=true
  systemPreflight.rc=2
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue sample:
  inputiaHostResidue=false waitIterations=1
  debugEnv=
  id=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true
```

当前结论：

- 非 GUI 验证现在能容忍短暂 self-check 进程退出延迟，同时仍会对超过 10 秒的真实残留硬失败并输出诊断。
- TextEdit 未运行且未被启动；Safari 是预先存在的进程，验证没有关闭或新建 Safari。
- 当前真实 GUI smoke 仍被正确阻断在 TIS readiness：系统安装版 v40、build/pkg v41，`tis.enabledMatches=0`、`tis.installedMatches=0`、`missing-enabled-source`。

## v41 Mac mini 续修：系统安装后 TIS readiness 证据面

时间：2026-07-08 01:29 CST。

依据：

- Apple InputMethodKit 文档：InputMethodKit 负责输入法与客户端应用、候选窗、输入模式之间的通信。
- Apple AppKit `NSTextInputContext.keyboardInputSources` 文档：Text Input Source Services 使用输入源 identifier 字符串识别输入源。
- `ensan-hcl/macOS_IMKitSample_2021`：现代 Swift IMKit 示例要求 bundle identifier 包含 `.inputmethod.`，并在系统设置的 Keyboard > Input Sources 中添加输入法。
- `madeye/ds-input` 的 Info.plist：成熟 Swift 输入法实现把 `InputMethodConnectionName`、`InputMethodServerControllerClass`、`tsInputMethodIconFileKey`、`tsInputMethodCharacterRepertoireKey`、`ComponentInputModeDict`、`tsInputModeListKey` 和可见 mode 列表作为输入源注册的关键 plist 表面。

本地对照：

- build app v41：
  - `CFBundleIdentifier=com.inputia.inputmethod.Inputia`
  - `InputMethodConnectionName=com.inputia.inputmethod.Inputia_Connection`
  - `InputMethodServerControllerClass=InputiaInputMethod.InputiaInputController`
  - `InputMethodServerDelegateClass=InputiaInputMethod.InputiaInputController`
  - `tsInputMethodIconFileKey=inputia.pdf`
  - `ComponentInputModeDict.tsInputModeListKey` 包含 `com.inputia.inputmethod.Inputia.Hans` / `Hant`
  - `tsVisibleInputModeOrderedArrayKey` 包含 Hans / Hant
- 当前阻塞仍不是 build app plist 缺失，而是系统安装与 TIS 枚举状态：
  - build/pkg v41
  - `/Library/Input Methods/InputiaInputMethod.app` v40
  - TIS `enabledMatches=0` / `installedMatches=0`
  - readiness block reason `missing-enabled-source`

实现：

- `install-system.sh`
  - 在复制、LaunchServices 注册、`--register-input-source`、enable/select、enabled/installed dump 后调用：
    - `bash "$ROOT_DIR/tis-readiness.sh" "$DEST_APP"`
  - 输出加前缀 `systemInstallTIS: ...`，避免和其它阶段混淆。
  - 根据 `tisReadiness=true` 输出 `systemInstallTISReady=true/false`。
  - readiness 输出放在 `TextInputMenuAgent` / `SystemUIServer` 重启之前，保留重启前 TIS 原始证据。
- `verify-nongui.sh`
  - 静态合同要求 `install-system.sh` 必须包含安装后 `tis-readiness.sh "$DEST_APP"`、`systemInstallTISReady=` 输出，并且发生在菜单代理重启之前。

选择/放弃：

- 选择：补安装后 readiness 证据面；管理员安装完成后可以立即判断是否进入可跑 GUI smoke 的状态。
- 放弃：在当前无管理员权限的环境里强行运行系统安装或 GUI smoke；原因是 `install-system.sh` 明确返回 `admin-required`，而真实 GUI smoke 仍被 TIS readiness 阻断。
- 放弃：用用户目录 host 暂时绕过系统 v40；原因是当前目标是系统级输入法 MVP，用户 host 会引入 shadow/conflict 变量，削弱后续系统安装证据。

验证：

```text
zsh -n macos/InputiaInputMethod/install-system.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_INSTALL_NO_ADMIN_PROMPT=1 macos/InputiaInputMethod/install-system.sh
  systemInstallNeedsAdmin=true
  systemInstallReady=false reason=admin-required
  rc=12

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogLifecycleSelfCheck=true
  systemPreflight.rc=2
  uiDisabledNoLaunchSkipped=true reason=textedit-already-running
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue sample:
  no InputiaInputMethod / osascript / verify script / tmp inputia residue
  debugEnv=
  id=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true
```

当前结论：

- `install-system.sh` 现在会在系统安装后直接暴露 TIS readiness，下一次管理员安装 v41 后可以直接判断是否允许 TextEdit/Safari/Clipboard GUI smoke。
- 本轮没有强跑 GUI smoke，没有修改当前输入源，没有创建用户 host。
- 本轮完整非 GUI 验证通过；但由于验证开始时 TextEdit 已经存在，TextEdit no-launch gate 按纪律跳过，不关闭用户预先存在的 TextEdit。

## v41 Mac mini 续修：status 阶段 self-check 等待统一

时间：2026-07-08 01:32 CST。

问题：

- 前一轮已把进程残留检查从 3 秒改为默认 10 秒，避免短暂 host self-check 进程导致误判。
- 复查 `verify-nongui.sh` 时发现 `status` 阶段仍只等待 30 次、每次 0.1 秒确认 `running=false`。
- 这和 residue gate 属于同一类风险：`post-install-regression.sh` / `verify-system.sh` 的 host self-check 可能短暂存在，3 秒上限容易让验证本身不稳定。

实现：

- `verify-nongui.sh`
  - `status` 阶段改用 `INPUTIA_PROCESS_WAIT_TICKS:-100`，默认最多等待 10 秒。
  - 超时仍硬失败，输出 `nonGuiVerificationPassed=false reason=inputia-host-running waitedTicks=...`。
  - 失败时调用 `process_details InputiaInputMethod` 输出 PID、父 PID、运行时长和命令行。
  - 移除后续重复的 `require_output running=false`，避免失败时缺少进程细节。

选择/放弃：

- 选择：统一 status 和 residue 的等待上限及诊断方式。
- 放弃：忽略 status 阶段的 running host；原因是真实 host 残留会影响后续 GUI smoke 判定。
- 放弃：继续只等 3 秒；原因是已有多次 self-check 短暂进程证据。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  textEditPreExisting=false
  safariPreExisting=true
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogLifecycleSelfCheck=true
  appleScriptCompileSkipped=true reason=would-launch-target-apps
  status: running=false
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue sample:
  no TextEdit / InputiaInputMethod / osascript / verify script / tmp inputia residue
  Safari pre-existing helpers still present
  debugEnv=
  id=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true
```

当前结论：

- `verify-nongui.sh` 的 status、gate、residue 三处现在都按同一个可调等待上限处理短暂 self-check host。
- 本轮非 GUI 验证覆盖到了 TextEdit 未运行路径，证明 UI-disabled 和 TIS gate 不会启动 TextEdit、不改 debug env、不改当前输入源、不创建 user host。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41，并等待 TIS 从 `missing-enabled-source` 进入 ready。

## v41 Mac mini 续修：await timeout 原因摘要

时间：2026-07-08 01:36 CST。

问题：

- `await-system-install.sh` 是管理员安装完成后衔接 `post-install-regression.sh` 和 GUI smoke 的桥。
- 旧 timeout 只输出 `systemInstallObserved=false reason=timeout`，后续需要从完整 `status.sh` 文本里人工判断到底是系统 app 仍未匹配 build，还是系统 app 已匹配但 TIS 未 ready。
- 当前 Mac mini 正处在系统 app v40、build/pkg v41 的状态；timeout 摘要需要直接给出机器可读原因，避免下一轮误判为 TIS 单独问题。

实现：

- `await-system-install.sh`
  - 循环中记录最后一次 `last_target_matches_build`。
  - 循环中解析最后一次 `tis.readinessBlockReason`。
  - timeout 时输出：
    - `systemInstallTargetMatchesBuild=<true|false>`
    - 如果目标 app 仍不匹配 build：`systemInstallTISReady=false reason=target-cdhash-mismatch`
    - 如果目标 app 已匹配但 TIS 未 ready：`systemInstallTISReady=false reason=$last_tis_block_reason`
- `verify-nongui.sh`
  - 静态合同要求 `await-system-install.sh` 保留 timeout target match summary、target CDHash mismatch 原因、TIS block reason 原因。

选择/放弃：

- 选择：把 timeout 根因做成机器可读摘要，仍保留后续完整 `status.sh` 输出。
- 放弃：只靠完整 `status.sh` 人工解读；原因是自动化接续需要稳定字段判断是否可跑 GUI smoke。
- 放弃：当前强行系统安装或用户 host 绕过；原因是系统安装需要管理员权限，用户 host 已知会引入同 bundle id 解析冲突。

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
  macos/InputiaInputMethod/await-system-install.sh
  rc=2
  currentBefore=com.tencent.inputmethod.wetype.pinyin
  currentAfter=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  uiSmokeRequested=false uiSmokeWouldStart=false uiSmokeBlockReason=ui-smoke-disabled
  systemInstallObserved=false reason=timeout
  systemInstallTargetMatchesBuild=false
  systemInstallTISReady=false reason=target-cdhash-mismatch

bash macos/InputiaInputMethod/verify-nongui.sh
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  awaitShort.rc=2
  awaitShort.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  awaitShort.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue sample:
  no TextEdit / InputiaInputMethod / osascript / verify script / tmp inputia residue
  Safari pre-existing helpers still present
  debugEnv=
  id=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true
```

当前结论：

- `await-system-install.sh` 现在能在 timeout 时明确区分“系统 app 还没更新到 build”与“TIS 未 ready”。
- 当前 Mac mini 的首要阻塞被明确归类为 `target-cdhash-mismatch`：系统安装版仍是 v40，尚未替换为 v41。
- 真实 GUI smoke 仍未运行；TextEdit/Clipboard/Safari 的失败 gate 继续证明不会抢焦点、不会改输入源、不会留下进程或临时文件。

## v41 Mac mini 续修：pkg postinstall 刷新后 TIS ready 结论

时间：2026-07-08 01:40 CST。

依据：

- Apple Text Input Source Services 语义：`TISSelectInputSource` 要求目标 input source 可选且已启用；如果是 input mode，parent input method 也必须启用。
- Apple AppKit `keyboardInputSources` 文档：文本输入源以 input source identifier 字符串识别。

问题：

- `build-pkg.sh` 打包使用的是 `Packaging/scripts/postinstall`，不是 `install-system.sh`。
- 前几轮增强了直接系统安装脚本和 await 脚本，但 pkg 安装路径仍只输出刷新后的 enabled/current TIS dump，没有机器可读的 `ready=true/false` 结论。
- 真实用户更可能通过 pkg 安装；如果 pkg postinstall 没有 ready 结论，安装后是否可以安全跑 TextEdit/Safari/Clipboard GUI smoke 仍需要人工解析。

实现：

- `Packaging/scripts/postinstall`
  - 新增 `dump_value()`，从 postinstall 的 TIS dump 中解析 `id`、`enabled`、`selectable`、`iconURL`。
  - 新增 `post_refresh_tis_ready(enabled_output)`：
    - 从安装后的 Info.plist 读取 `ComponentInputModeDict:tsVisibleInputModeOrderedArrayKey:0` 作为 target mode id。
    - 如果 enabled dump 是 `inputSourceFound=false`，输出 `inputiaPostRefreshTISReady=false reason=missing-enabled-source targetModeID=...`。
    - 如果 id、enabled、selectable、iconURL 均匹配，输出 `inputiaPostRefreshTISReady=true targetModeID=...`。
    - 否则输出 `inputiaPostRefreshTISReady=false reason=<id-mismatch|not-enabled|not-selectable|icon-mismatch> ...`。
  - 在 `TextInputMenuAgent` / `SystemUIServer` 刷新、sleep、enabled dump 后调用该函数。
- `verify-pkg.sh`
  - 新增 postinstall 静态检查：pkg postinstall 必须包含 `inputiaPostRefreshTISReady=`。
- 重新运行 `build-pkg.sh`，生成新的 latest pkg。

选择/放弃：

- 选择：pkg postinstall 不因 TIS ready false 失败安装，只输出机器可读状态；安装成功与 GUI smoke readiness 继续分离。
- 放弃：在 postinstall 里调用源码目录的 `tis-readiness.sh`；原因是 pkg 安装环境只保证 scripts 目录和已安装 app，可移植性更差。
- 放弃：要求 current source 已经 selected 才算 ready；原因是 GUI smoke 脚本本身会在 app/icon 匹配后做 select-only 选择，ready gate 主要证明 source 已启用且可选。

验证：

```text
zsh -n Packaging/scripts/postinstall verify-pkg.sh build-pkg.sh
bash -n verify-nongui.sh
  rc=0

macos/InputiaInputMethod/build-pkg.sh
  pkgbuild: Wrote package to .../dist/InputiaInputMethod-v41-593ceb737c71.pkg
  sha256=ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d
  sourcePostinstallSHA256=132f940940d6a1622004d8fa8a172ebd7b7acd6031523fe9ca34d914499a121c
  pkgPostinstallSHA256=132f940940d6a1622004d8fa8a172ebd7b7acd6031523fe9ca34d914499a121c
  postinstallBehaviorChecks=true
  pkgVerificationPassed=true

INPUTIA_INSTALL_ROOT=/tmp/inputia-postinstall-skip... INPUTIA_POSTINSTALL_SKIP_TIS=1 \
  macos/InputiaInputMethod/build/pkg-scripts/postinstall
  inputiaInstalledVersion=41
  inputiaInstalledCDHash=593ceb737c717c27cc46cd159ff5027f5095df8c
  inputiaSettingsLauncherInstalled=true
  tempAppExecutable=true

pkg expanded postinstall spot-check:
  inputiaPostRefreshTISReady=false reason=missing-enabled-source targetModeID=$target_mode_id
  inputiaPostRefreshTISReady=true targetModeID=$target_mode_id
  inputiaPostRefreshTISReady=false reason=$reason ...
  inputiaPostRefreshEnabledTIS:

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyPkg: sha256=ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d
  verifyPkg: postinstallBehaviorChecks=true
  verifyPkg: pkgVerificationPassed=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

final residue sample:
  no TextEdit / InputiaInputMethod / osascript / verify script / tmp inputia residue
  debugEnv=
  id=com.tencent.inputmethod.wetype.pinyin
  name=微信输入法
  selected=true
```

当前结论：

- 最新 pkg 已包含 postinstall 刷新后的 TIS readiness 布尔结论，安装后可以直接判断是否允许后续 GUI smoke。
- latest pkg SHA 已从旧值更新为 `ab23e406eb7d85884ee25b86b12757f1021b061f82d2e1160e76d5c88cf4194d`。
- 当前系统安装版仍是 v40，latest build/pkg 是 v41；真实 GUI smoke 仍等待管理员安装替换系统 app。

## v41 Mac mini 续验：pkg postinstall TIS 顺序合同与当前 SHA 校正

时间：2026-07-08 01:46 CST。

问题：

- `verify-pkg.sh` 已检查 postinstall 包含 post-refresh TIS dump 和 ready 结论，但最初只是存在性检查。
- `inputiaPostRefreshTISReady=` 在 helper 函数定义里也会出现；如果顺序检查匹配第一处 echo，会误判执行顺序。
- 重新验证当前文件系统后，latest pkg 的实际 SHA 已更新，需要以当前 `verify-pkg.sh` / `status.sh` 输出为准。

实现：

- `verify-pkg.sh`
  - 新增 `require_order()`。
  - 校验 package 内 `Scripts/postinstall` 的执行顺序：
    - `inputia-select` 在菜单代理刷新前。
    - `TextInputMenuAgent` 在 `SystemUIServer` 前。
    - `SystemUIServer` 在 `inputiaPostRefreshEnabledTIS:` 前。
    - `inputiaPostRefreshEnabledTIS:` 在执行段 `post_refresh_tis_ready "$post_refresh_enabled_output"` 前。
    - `post_refresh_tis_ready "$post_refresh_enabled_output"` 在 `inputiaPostRefreshCurrentTIS:` 前。
  - 顺序检查刻意匹配执行段的函数调用，避开 helper 定义里的 `inputiaPostRefreshTISReady=` echo。

验证：

```text
zsh -n macos/InputiaInputMethod/verify-pkg.sh
  rc=0

bash macos/InputiaInputMethod/verify-pkg.sh
  sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  sourcePostinstallSHA256=a21cbca02317bd85e9a60651384be3b57718e77b2468a621d0c907779813a5dd
  pkgPostinstallSHA256=a21cbca02317bd85e9a60651384be3b57718e77b2468a621d0c907779813a5dd
  postinstallBehaviorChecks=true
  inputiaPostinstallSelfCheck=true
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  verifyPkg: postinstallBehaviorChecks=true
  verifyPkg: pkgVerificationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

bash macos/InputiaInputMethod/status.sh
  latest package sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  running=false
  systemMatchesBuild=false
```

当前结论：

- pkg postinstall 的 post-refresh TIS dump 现在有顺序合同，不只是存在性合同。
- 当前 latest pkg SHA 以 `75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301` 为准。
- 系统安装版仍是 v40；真实 GUI smoke 继续等待管理员安装 v41。

## v41 Mac mini 续验：非 GUI 合同锁住 postinstall self-check

时间：2026-07-08 01:52 CST。

问题：

- `verify-pkg.sh` 已运行 package 内 `Scripts/postinstall` 的 `INPUTIA_POSTINSTALL_SELF_CHECK=1` 自检。
- 但如果未来 `verify-pkg.sh` 被改退化，只保留 postinstall 字段/顺序检查，`verify-nongui.sh` 仍可能通过。
- postinstall 的 `inputiaPostRefreshTISReady` 分支是安装后是否允许 GUI smoke 的关键判断，应当被主非 GUI 回归固定住。

实现：

- `verify-nongui.sh`
  - 静态检查 `verify-pkg.sh` 必须包含：
    - `INPUTIA_POSTINSTALL_SELF_CHECK`
    - `postinstallSelfCheck:` 输出前缀
  - 静态检查 `Packaging/scripts/postinstall` 必须包含：
    - `inputiaPostinstallSelfCheck=true`
    - `ready`
    - `missing`
    - `id-mismatch`
    - `not-enabled`
    - `not-selectable`
    - `icon-mismatch`

验证：

```text
zsh -n macos/InputiaInputMethod/verify-pkg.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-pkg.sh
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=ready passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=missing passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=id-mismatch passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=not-enabled passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=not-selectable passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=icon-mismatch passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheck=true
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  awaitShort: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 主非 GUI 回归现在会防止 `verify-pkg.sh` 丢失 postinstall self-check。
- 当前 latest pkg SHA 仍为 `75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301`。

## v41 Mac mini 续验：verify-pkg 对 postinstall self-check 输出做运行时断言

时间：2026-07-08 01:55 CST。

问题：

- `verify-pkg.sh` 已运行 `INPUTIA_POSTINSTALL_SELF_CHECK=1 "$PKG_POSTINSTALL"`，但之前主要是打印输出。
- 如果 postinstall self-check 将来退出 0 但少打印某个 case 或总成功标志，package 验证可能误判通过。

实现：

- `verify-pkg.sh`
  - 捕获 `postinstall_self_check_output`。
  - 打印时继续加 `postinstallSelfCheck:` 前缀，保留可读日志。
  - 运行时断言所有 case 都出现：
    - `ready`
    - `missing`
    - `id-mismatch`
    - `not-enabled`
    - `not-selectable`
    - `icon-mismatch`
  - 运行时断言总成功标志 `inputiaPostinstallSelfCheck=true`。
- `verify-nongui.sh`
  - 静态合同要求 `verify-pkg.sh` 必须捕获 self-check 输出，并包含 case/success 缺失时的 fail reason。

验证：

```text
zsh -n macos/InputiaInputMethod/verify-pkg.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-pkg.sh
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=ready passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=missing passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=id-mismatch passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=not-enabled passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=not-selectable passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheckCase=icon-mismatch passed=true
  postinstallSelfCheck: inputiaPostinstallSelfCheck=true
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  verifyPkg: postinstallSelfCheck: inputiaPostinstallSelfCheck=true
  verifyPkg: pkgVerificationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- postinstall self-check 现在既被执行，也被 `verify-pkg.sh` 按运行时输出断言。
- 当前 latest pkg SHA 仍为 `75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301`。

## v41 Mac mini 续修：pkg postinstall TIS ready 离线自检

问题：

- 上一段只通过 grep/spot-check 证明 pkg postinstall 带有 `inputiaPostRefreshTISReady=` 输出。
- 这仍不能证明 ready 判定分支本身稳定；例如 missing source、id mismatch、enabled=false、selectable=false、icon mismatch 都需要在不触碰系统 TIS/GUI 的情况下被覆盖。

实现：

- `Packaging/scripts/postinstall`
  - 将刷新后的 TIS ready 判定拆为 `post_refresh_tis_ready_for(target_mode_id, target_icon, enabled_output)`。
  - 保留真实安装路径的 `post_refresh_tis_ready(enabled_output)`，继续从安装后的 `Info.plist` 和 `Resources/inputia.pdf` 派生目标值。
  - 新增 `INPUTIA_POSTINSTALL_SELF_CHECK=1` 离线分支，不安装、不 killall、不调用 TIS，只喂入样例 dump。
  - 自检覆盖：
    - `ready`
    - `missing-enabled-source`
    - `id-mismatch`
    - `not-enabled`
    - `not-selectable`
    - `icon-mismatch`
- `verify-pkg.sh`
  - 要求 pkg postinstall 包含 `INPUTIA_POSTINSTALL_SELF_CHECK`。
  - 在展开后的包内 postinstall 上执行 `INPUTIA_POSTINSTALL_SELF_CHECK=1`，把结果纳入 pkg 验证输出。

选择/放弃：

- 选择：离线自检只验证 postinstall 的解析和分类逻辑，不读取真实系统输入源，避免污染 GUI smoke 前置状态。
- 选择：真实安装仍只输出机器可读 ready 结论，不因为 TIS 未 ready 失败安装。
- 放弃：用真实 TIS 状态覆盖这些分支；原因是当前系统仍是 v40/v41 不匹配，且真实 TIS 状态依赖用户会话和系统缓存，不适合验证纯分类逻辑。

验证：

```text
zsh -n macos/InputiaInputMethod/Packaging/scripts/postinstall \
  macos/InputiaInputMethod/verify-pkg.sh \
  macos/InputiaInputMethod/build-pkg.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_POSTINSTALL_SELF_CHECK=1 macos/InputiaInputMethod/Packaging/scripts/postinstall
  inputiaPostinstallSelfCheckCase=ready passed=true
  inputiaPostinstallSelfCheckCase=missing passed=true
  inputiaPostinstallSelfCheckCase=id-mismatch passed=true
  inputiaPostinstallSelfCheckCase=not-enabled passed=true
  inputiaPostinstallSelfCheckCase=not-selectable passed=true
  inputiaPostinstallSelfCheckCase=icon-mismatch passed=true
  inputiaPostinstallSelfCheck=true

macos/InputiaInputMethod/build-pkg.sh
  pkgbuild: Wrote package to .../dist/InputiaInputMethod-v41-593ceb737c71.pkg
  latest pkg sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  sourcePostinstallSHA256=a21cbca02317bd85e9a60651384be3b57718e77b2468a621d0c907779813a5dd
  pkgPostinstallSHA256=a21cbca02317bd85e9a60651384be3b57718e77b2468a621d0c907779813a5dd
  postinstallBehaviorChecks=true
  postinstallSelfCheck: inputiaPostinstallSelfCheck=true
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyPkg: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  verifyPkg: postinstallSelfCheck: inputiaPostinstallSelfCheck=true
  verifyPkg: pkgVerificationPassed=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- pkg postinstall 的 TIS ready 分类已有离线行为覆盖，且源码脚本与包内脚本 SHA 一致。
- GUI smoke 没有被强跑；所有 UI gate 在当前系统 v40/v41 不匹配时正确拒绝启动。
- 当前真实阻塞仍是系统安装版 v40，build/pkg v41；需要管理员安装 latest pkg 后再继续 TextEdit/Clipboard GUI smoke。

## v41 Mac mini 续修：GUI smoke 清理等待契约

问题：

- TextEdit/Safari/Clipboard GUI smoke 的主脚本已经有 preflight、trap、focus-lost 和失败清理，但公共清理 helper 只是发出 quit 后固定 sleep。
- 如果 GUI 应用退出慢，固定 sleep 可能让后续 residue 检查偶发看到残留进程；这个问题会污染 TextEdit/Clipboard smoke 纪律判断。

实现：

- `smoke-common.sh`
  - 新增 `inputia_wait_process_exit(process_name, max_ticks)`，按 100ms 间隔轮询进程退出。
  - `inputia_cleanup_textedit_if_started()` 在 `quit saving no` 后等待 `TextEdit` 退出。
  - `inputia_cleanup_safari_if_started()` 在 quit 后等待 `Safari` 退出。
  - 等待只在 preflight 判定应用原本未运行、且对应 cleanup allowed 时生效，不清理用户原本已打开的应用。
- `verify-nongui.sh`
  - 静态契约新增检查：公共 helper 必须包含进程退出等待，TextEdit/Safari cleanup 必须调用该等待。
  - TextEdit smoke 契约新增 `quit saving no` 检查。
  - Clipboard recall smoke 契约新增 `quit saving no` 与 `focus-lost:` 检查。

选择/放弃：

- 选择：等待进程自然退出，不在 cleanup helper 中 kill 用户应用。
- 选择：通过 `verify-nongui.sh` 将清理等待和 AppleScript 失败路径变成可离线验证契约。
- 放弃：在当前系统 v40/v41 不匹配时强跑真实 GUI smoke；原因是 TIS gate 已证明目标输入源未启用/不可选，硬跑会抢焦点并给出误导结果。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  verifyPkg: postinstallSelfCheck: inputiaPostinstallSelfCheck=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI smoke 的公共清理逻辑现在会等待自己启动的 TextEdit/Safari 退出，降低失败路径和慢退出造成的残留风险。
- 非 GUI 契约已覆盖 TextEdit/Clipboard 的 focus-lost、双 ESC 状态清理、失败关闭文档、quit saving no、clipboard shown/commit 事件检查。
- 真实 GUI smoke 仍需等待系统安装版从 v40 更新到 latest pkg v41 后继续。

## v41 Mac mini 续修：TextEdit/Clipboard smoke AppleScript 超时保护

问题：

- TextEdit 和 Clipboard recall GUI smoke 的 AppleScript 主体会打开 GUI 应用、等待前台和候选事件；如果 System Events、TextEdit 或输入法事件卡住，脚本可能长时间占住焦点或留下未清理状态。
- 之前依赖 trap 做退出清理，但没有给主 AppleScript 调用本身设置硬超时。

实现：

- `smoke-common.sh`
  - 新增 `inputia_run_with_timeout(label, seconds, command...)`。
  - 超时后输出 `inputiaSmokeTimeout=<label> seconds=<n>` 到 stderr，先 TERM 再 KILL 目标命令。
- `smoke-textedit.sh`
  - 将 AppleScript 写入 `/tmp/inputia-textedit-osascript.$$.applescript`。
  - 通过 `inputia_run_with_timeout textedit-osascript ${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}` 执行。
  - trap 清理新增临时 AppleScript 文件。
- `smoke-clipboard-recall.sh`
  - 将 AppleScript 写入 `/tmp/inputia-clipboard-recall-osascript.$$.applescript`。
  - 通过 `inputia_run_with_timeout clipboard-recall-osascript ${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}` 执行，并传入 event log 路径。
  - trap 清理新增临时 AppleScript 文件，同时保留剪贴板、debug env、输入源和 TextEdit 清理。
- `verify-nongui.sh`
  - cleanup contract 新增检查：公共 timeout helper 必须存在。
  - TextEdit/Clipboard smoke 必须使用临时 AppleScript 文件和 timeout 包装。

选择/放弃：

- 选择：只先加固 TextEdit 与 Clipboard recall 两条当前重点 smoke，避免扩大 Safari 脚本改动面。
- 选择：使用 shell 原生后台进程 + sleep timer，避免引入 GNU `timeout` 这类 macOS 默认不存在的依赖。
- 放弃：在超时时强杀 TextEdit/Safari；原因是 helper 只杀 osascript，应用清理由已有 trap 按 preflight 纪律处理。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  verifyPkg: sha256=75d79e3e5797e93109b7f5be17cbf6fc8a338c29ff53ce8a5c897d4ba789c301
  verifyPkg: pkgVerificationPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TextEdit/Clipboard GUI smoke 主 AppleScript 现在有 90 秒默认硬超时，失败路径会回到既有 trap 清理逻辑。
- 非 GUI 验证已把 timeout 包装锁成契约，防止后续重构丢失。
- 系统安装版仍是 v40、latest build/pkg 是 v41；真实 GUI smoke 继续等待系统安装更新后执行。

## v41 Mac mini 续修：Safari smoke/diagnose AppleScript 超时保护

问题：

- 上一段只给 TextEdit 与 Clipboard recall 主 AppleScript 加了硬超时。
- Safari typing、Safari enter 和 Safari input-source diagnose 仍直接运行主 AppleScript；如果 Safari/System Events 卡住，仍可能长时间占用焦点或留下测试窗口。

实现：

- `smoke-safari-typing.sh`
  - 新增 `/tmp/inputia-safari-typing-osascript.$$.applescript` 临时脚本。
  - 主 AppleScript 改为通过 `inputia_run_with_timeout safari-typing-osascript ${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}` 执行。
  - trap 清理临时 AppleScript 文件。
- `smoke-safari-enter.sh`
  - 新增 `/tmp/inputia-safari-enter-osascript.$$.applescript` 临时脚本。
  - 主 AppleScript 改为通过 `inputia_run_with_timeout safari-enter-osascript ${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}` 执行。
  - trap 清理临时 AppleScript 文件，并保留 event log / debug env / input source 清理。
- `diagnose-safari-input-source.sh`
  - 新增 `/tmp/inputia-safari-diagnose-osascript.$$.applescript` 临时脚本。
  - 打开 Safari 测试页并返回 window id 的 AppleScript 改为通过 `inputia_run_with_timeout safari-diagnose-osascript ${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}` 执行。
  - diagnosis cleanup 清理临时 AppleScript 文件。
- `verify-nongui.sh`
  - Safari typing/enter/diagnose 的 cleanup contract 改为要求使用 `inputia_run_with_timeout`。
  - AppleScript compile 提取器从匹配 `/usr/bin/osascript <<APPLESCRIPT` 放宽为匹配 `<<APPLESCRIPT`，兼容“先写临时脚本再执行”的形态。

选择/放弃：

- 选择：所有主 GUI AppleScript 统一使用同一个 shell timeout helper，避免新增依赖。
- 选择：超时只杀 osascript；Safari 窗口/进程仍由脚本内 close handler 和外层 trap 按 preflight 纪律清理。
- 放弃：当前直接跑 Safari GUI smoke；原因是 Safari 已经是用户既有进程，且系统 Inputia 仍是 v40/v41 不匹配，gate 正确阻止了真实 GUI 操作。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/diagnose-safari-input-source.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true
  cleanupPermissionContract=true
  debugEnvRestoreSelfCheck=true
  debugEventLogLifecycleSelfCheck=true
  verifyPkg: pkgVerificationPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TextEdit、Clipboard recall、Safari typing、Safari enter、Safari diagnose 的主 GUI AppleScript 都有统一硬超时。
- 非 GUI 契约已覆盖这些 timeout 包装，避免后续重构把 GUI 卡死保护删掉。
- 系统安装版仍是 v40，latest build/pkg 是 v41；真实 GUI smoke 继续等待系统安装更新后执行。

## v41 Mac mini 续修：timeout helper 运行时自检

问题：

- 前几段已经把 TextEdit/Clipboard/Safari 主 GUI AppleScript 都切到 `inputia_run_with_timeout`。
- 但 `verify-nongui.sh` 之前只静态检查 helper 存在和被调用，没有实际证明 helper 超时时会返回失败并输出机器可读 marker。

实现：

- `verify-nongui.sh`
  - 新增 `timeout helper self-check` 段。
  - 执行 `inputia_run_with_timeout timeout-self-check 1 /bin/sleep 5`。
  - 要求：
    - 返回码非 0。
    - 输出包含 `inputiaSmokeTimeout=timeout-self-check seconds=1`。
  - 通过时输出 `timeoutHelperSelfCheck=true`。

选择/放弃：

- 选择：使用 `/bin/sleep` 做纯本地、非 GUI、无输入源副作用的运行时 spike。
- 选择：只断言“非 0 + marker”，不绑定具体返回码；当前 macOS 上返回 `143`，但不同 kill 路径不应让测试过度脆弱。
- 放弃：用 osascript 模拟超时；原因是这会触碰 AppleScript 运行时，不如 `/bin/sleep` 稳定且无 GUI 风险。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  timeoutSelfCheck: inputiaSmokeTimeout=timeout-self-check seconds=1
  timeoutSelfCheck.rc=143
  timeoutHelperSelfCheck=true
  cleanupPermissionContract=true
  verifyPkg: pkgVerificationPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- timeout helper 现在既有静态契约，也有运行时自检。
- 真实 GUI smoke 仍未强跑；系统安装版仍是 v40，latest build/pkg 是 v41。

## v41 Mac mini 续修：timeout helper 快路径与失败路径自检

问题：

- 上一段只验证了 `inputia_run_with_timeout` 的超时路径。
- GUI smoke 还依赖 helper 保留正常命令输出/返回码，以及保留非超时失败的返回码/错误输出；否则 smoke 失败归因会变弱。

实现：

- `verify-nongui.sh`
  - 在 `timeout helper self-check` 中新增快路径：
    - `inputia_run_with_timeout timeout-fast-check 5 /bin/echo timeout-fast-ok`
    - 要求 rc=0 且输出等于 `timeout-fast-ok`。
  - 新增非超时失败路径：
    - `inputia_run_with_timeout timeout-fail-check 5 /bin/sh -c 'echo timeout-fail-stderr >&2; exit 7'`
    - 要求 rc=7 且输出包含 `timeout-fail-stderr`。
  - 保留原有超时路径：
    - `/bin/sleep 5` 配 1 秒超时，要求非 0 且输出 `inputiaSmokeTimeout=timeout-self-check seconds=1`。

选择/放弃：

- 选择：只用 `/bin/echo`、`/bin/sh`、`/bin/sleep`，避免触碰 GUI、TIS、AppleScript 或输入法状态。
- 选择：继续不固定超时路径的具体 rc，只要求非 0 和 marker；但快路径/失败路径要固定 rc，确保 wrapper 不吞掉子命令语义。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  timeoutFastCheck: timeout-fast-ok
  timeoutFastCheck.rc=0
  timeoutFailCheck: timeout-fail-stderr
  timeoutFailCheck.rc=7
  timeoutSelfCheck: inputiaSmokeTimeout=timeout-self-check seconds=1
  timeoutSelfCheck.rc=143
  timeoutHelperSelfCheck=true
  cleanupPermissionContract=true
  verifyPkg: pkgVerificationPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- timeout helper 的快路径、非超时失败路径、超时路径都已有非 GUI 运行时覆盖。
- 系统安装版仍是 v40，latest build/pkg 是 v41；真实 GUI smoke 继续等待系统安装更新后执行。

## v41 Mac mini 续修：GUI smoke 临时文件按进程隔离

问题：

- TextEdit/Clipboard/Safari/diagnose smoke 之前还有若干固定 `/tmp/inputia-*.log` 或测试 URL 路径。
- 这些文件在 GUI smoke 被中断、并发重跑或上一轮清理不完整时，可能污染下一轮判断，尤其会削弱 clipboard recall mismatch 的归因。

实现：

- `smoke-textedit.sh`
  - `SELECT_LOG`、`RESTORE_LOG` 改为 `/tmp/inputia-textedit-*.$$.log`。
  - 主 AppleScript 临时文件继续使用 `/tmp/inputia-textedit-osascript.$$.applescript`。
- `smoke-clipboard-recall.sh`
  - `SELECT_LOG`、`RESTORE_LOG` 改为 `/tmp/inputia-clipboard-recall-*.$$.log`。
  - event log 和主 AppleScript 临时文件均使用 `$$` 隔离。
- `smoke-safari-typing.sh`、`smoke-safari-enter.sh`
  - 测试 URL、输入源选择日志、恢复日志和主 AppleScript 临时文件均改为 `$$` 隔离。
- `diagnose-safari-input-source.sh`
  - 测试 URL、HIToolbox preference dump、输入源选择日志、恢复日志和主 AppleScript 临时文件均改为 `$$` 隔离。
- `verify-nongui.sh`
  - 新增 `unique_tmp_contracts` 静态契约，防止后续把这些路径退回固定文件名。

选择/放弃：

- 选择：用 shell `$$` 做进程级隔离；它足够覆盖本地 smoke 串扰场景，并保持脚本可读。
- 选择：保留外部传入 `INPUTIA_DEBUG_EVENTS` 的能力，只约束默认路径。
- 放弃：引入 `mktemp` 重写所有路径；当前脚本已经有明确 trap 清理，`$$` 的最小改动更适合这个阶段。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit.sh \
  && bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  && bash -n macos/InputiaInputMethod/smoke-safari-typing.sh \
  && bash -n macos/InputiaInputMethod/smoke-safari-enter.sh \
  && bash -n macos/InputiaInputMethod/diagnose-safari-input-source.sh \
  && bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  timeoutFastCheck.rc=0
  timeoutFailCheck.rc=7
  timeoutSelfCheck: inputiaSmokeTimeout=timeout-self-check seconds=1
  timeoutHelperSelfCheck=true
  cleanupPermissionContract=true
  verifyPkg: pkgVerificationPassed=true
  verifyPkg: sha256=84f4cf2e312931d7ce1b5955ebb78d64ad1cd249a6e4c7f1d228d99025590395
  status: buildVersion=41
  status: buildCDHash=8b68f7005693162ea3eebae9ebff570550c1e93a
  status: systemMatchesBuild=false
  status: matches=0
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- smoke 的默认临时文件已经按进程隔离，非 GUI 契约会阻止固定 `/tmp` 文件名回退。
- 本轮没有强跑真实 GUI smoke；当前系统安装版仍是 v40，build/pkg 是 v41，TIS 仍 `matches=0`，阻塞原因仍是 `target-cdhash-mismatch`/`missing-enabled-source`。

## v41 Mac mini 续修：常用粘贴变体和非 GUI 快捷键回归门禁

背景：

- 用户反馈 Inputia 下 `Command-C` / `Command-V` 不能复制粘贴，并要求不要靠逐个手工发现；常用电脑快捷键应举一反三默认不被输入法接管。
- 依据 Apple 官方 Mac keyboard shortcuts 文档，`Command` 是 macOS 常用系统/App 快捷键主修饰键；复制、粘贴、剪切、撤销、重做、全选、保存、打开、查找、打印、隐藏、窗口切换、Spotlight、锁屏、截图等都属于系统或宿主 App 命令面。
- 因此 Inputia 的原则是：任何包含 `Command` 的 `keyDown` 默认透传；Inputia 自身快捷键继续使用不含 `Command` 的组合，例如 `Control-Shift-V` 剪贴板召回。

实现：

- `InputiaShortcutSelfCheck.swift`
  - 在既有 `Command-C/V/X/Z/A/S/O/W/Q/F/G/...` 覆盖上，新增粘贴相关常用变体：
    - `commandShiftVPassThrough=true`
    - `commandOptionVPassThrough=true`
    - `commandOptionShiftVPassThrough=true`
    - `commandControlVPassThrough=true`
  - 保留 `ctrlShiftVClipboardRecall=true` 和 `ctrlShiftCommandVRejected=true`，确保 Inputia 自身召回不覆盖带 `Command` 的系统/App 组合。
- `InputiaInputMethod.app --host-shortcut-self-check`
  - 同步增加上述 Command 粘贴变体，避免独立工具和 Host 内置诊断漂移。
- `verify-nongui.sh`
  - 新增 `shortcut pass-through self-checks` 阶段。
  - 要求 `build/inputia-shortcut-self-check` 和 `build/inputia-host-text-policy-self-check` 存在且可执行。
  - 运行时断言：
    - `shortcutSelfCheck=true`
    - `commandCPassThrough=true`
    - `commandVPassThrough=true`
    - `commandShiftVPassThrough=true`
    - `commandOptionShiftVPassThrough=true`
    - `commandControlVPassThrough=true`
    - `ctrlShiftVClipboardRecall=true`
    - `ctrlShiftCommandVRejected=true`
    - `hostTextPolicySelfCheck=true`
    - `appCommandcopy/paste/cut/undo/redo/selectAll...PassesThrough=true`

选择/放弃：

- 选择：对所有包含 `Command` 的事件统一透传，而不是只修 `Command-C/V`；这能覆盖 Apple 官方常用快捷键族和 App 自定义 Command 快捷键。
- 选择：保留 `Control-Shift-V` 作为 Inputia 剪贴板召回，因为它不含 `Command`，且项目已经围绕该快捷键建立了功能与 smoke 证据。
- 放弃：不在当前系统 v40 上跑真实 GUI smoke；当前系统安装版 CDHash `8d4f473adcc2f7c093b5629b9b1e742dcba184f8`，而当前 build CDHash `8b68f7005693162ea3eebae9ebff570550c1e93a`，版本不一致会污染结论。

验证：

```text
/usr/bin/swiftc -parse macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift macos/InputiaInputMethod/Tools/InputiaShortcutSelfCheck.swift
  rc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

zsh -n macos/InputiaInputMethod/build.sh
  rc=0

./macos/InputiaInputMethod/build.sh
  rc=0
  codesign valid on disk / satisfies Designated Requirement

./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandAPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandControlQPassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

./macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandgoForwardPassesThrough=true
  appCommandreloadPassesThrough=true

./macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

./macos/InputiaInputMethod/build-pkg.sh
  pkgVerificationPassed=true
  sha256=84f4cf2e312931d7ce1b5955ebb78d64ad1cd249a6e4c7f1d228d99025590395
  appCDHash=8b68f7005693162ea3eebae9ebff570550c1e93a

./macos/InputiaInputMethod/verify-nongui.sh
  shortcutPassThroughSelfChecks=true
  verifyPkg: pkgVerificationPassed=true
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前状态：

- 代码层和非 GUI 回归门禁已覆盖常用 Command 快捷键族，不再只是 `Command-C/V` 单点。
- 最新 build/pkg 是 v41，CDHash `8b68f7005693162ea3eebae9ebff570550c1e93a`，pkg SHA `84f4cf2e312931d7ce1b5955ebb78d64ad1cd249a6e4c7f1d228d99025590395`。
- 系统安装版仍是 v40，CDHash `8d4f473adcc2f7c093b5629b9b1e742dcba184f8`；真实 GUI copy/paste smoke 需等系统安装版更新到 v41 且 TIS readiness 通过后再跑。

## v41 Mac mini 续修：TextEdit Command 快捷键 GUI smoke 安全门禁

背景：

- 代码层已统一透传 `Command` 快捷键，非 GUI 自检也覆盖了 `Command-C/V` 和常见变体。
- 但真实 GUI smoke 里还缺一个面向宿主 App 的验证：当 Inputia 作为当前输入法时，TextEdit 中 `Command-A` / `Command-C` / `Command-V` 应由 TextEdit 处理，而不是被 IME 消耗。
- 当前系统安装版仍是 v40 且 TIS readiness 未通过，所以只能先把 smoke 脚本和 no-launch gate 建好，不能强跑真实 TextEdit。

实现：

- 新增 `smoke-textedit-command-shortcuts.sh`
  - 默认目标仍是 `/Library/Input Methods/InputiaInputMethod.app`，也支持传入 build app。
  - 进入真实 GUI 前依次检查：
    - executable 存在
    - build/system CDHash 一致，除非显式 `INPUTIA_SKIP_CDHASH_CHECK=1`
    - `inputia.pdf` 存在
    - `INPUTIA_RUN_UI_SMOKE=1`
    - GUI session 可用
    - TextEdit 未运行或显式允许
    - TIS 选中 Inputia 成功
  - 只有选源成功后才设置 `INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1` 并创建 AppleScript。
  - AppleScript 流程：
    - 创建独立 TextEdit document 引用 `docRef`
    - ESC 两次清空 IME 状态
    - 设置源文本 `Inputia Command Shortcut Source`
    - 发送 `Command-A`、`Command-C`
    - 清空文档后发送 `Command-V`
    - 断言复制和粘贴结果都等于源文本
    - 关闭 `docRef saving no`
  - 脚本捕获并恢复原剪贴板；失败/退出时恢复输入源、删除临时文件，并只在本脚本启动 TextEdit 时退出 TextEdit。
- `post-install-regression.sh`
  - 在 TIS ready 且 `INPUTIA_RUN_UI_SMOKE=1` 的 UI 回归链里，原 `smoke-textedit.sh` 后新增 `smoke-textedit-command-shortcuts.sh`。
  - TIS 未就绪时仍在打开 TextEdit/Safari 前返回 `postInstallUiSmokeReady=false reason=tis-not-ready`。
- `verify-nongui.sh`
  - `bash -n` 覆盖新脚本。
  - 静态合同覆盖：
    - unique temp 文件
    - TextEdit cleanup allowed marker 在选源之后、osascript 之前
    - ESC 清状态、focus-lost 断言、docRef capture/close、Command-A/C/V、剪贴板恢复、timeout helper
    - post-install UI 链包含新 smoke
  - 运行时 gate 覆盖：
    - UI disabled：期望 rc=16，剪贴板不变，debug env 不变，无 user host
    - TIS gate：期望 rc=17，剪贴板不变，当前输入源不变，debug env 不变，不启动 TextEdit/osascript/Inputia host

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  rc=0

env INPUTIA_SKIP_CDHASH_CHECK=1 ./macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh ./macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReady=false reason=ui-smoke-disabled
  textEditCommandShortcutSmokeReady=false reason=ui-smoke-disabled
  rc=16
  clipboardUnchanged=true
  sourceBefore=com.tencent.inputmethod.wetype.pinyin
  sourceAfter=com.tencent.inputmethod.wetype.pinyin

env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 ./macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh ./macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiConsoleUser=lizhelang
  guiFrontmostApp=System Settings
  textEditPreflight=not-running docs=0
  guiSmokeReady=false reason=input-source-not-selected
  textEditCommandShortcutSmokeReady=false reason=input-source-not-selected
  previousInputSourceID=com.tencent.inputmethod.wetype.pinyin
  selectSourceFoundInEnabledList=false
  inputSourceRestore=skipped reason=already-current
  rc=17
  clipboardUnchanged=true
  sourceBefore=com.tencent.inputmethod.wetype.pinyin
  sourceAfter=com.tencent.inputmethod.wetype.pinyin

./macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  verifyPkg: pkgVerificationPassed=true
  textEditUiDisabled.rc=14
  textEditCommandUiDisabled.rc=16
  textEditCommandUiDisabled.clipboardUnchanged=true
  clipboardUiDisabled.clipboardUnchanged=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGate.rc=17
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前状态：

- TextEdit `Command-A/C/V` 的真实 GUI smoke 脚本已经存在，并已接入 post-install UI 回归链。
- TIS 未就绪和 UI disabled 情况下的 no-launch/no-mutation 门禁已通过。
- 真实 TextEdit `Command-A/C/V` 行为仍需等系统安装版更新到 v41、TIS readiness 通过后再执行。

## v41 Mac mini 续修：PID 临时文件残留扫描补齐

问题：

- 前面已把 TextEdit/Clipboard/Safari/diagnose smoke 的默认临时文件改成 `...$PID...`，避免跨轮串扰。
- 但 `verify-nongui.sh` 末尾的 `/tmp` 残留扫描仍主要匹配旧固定文件名，例如 `inputia-*-select.log`、`inputia-safari-*-test.url`、`inputia-*-applescript.*`。
- 这会漏掉新的 `inputia-*-select.<pid>.log`、`inputia-*-restore.<pid>.log`、`inputia-safari-*-test.<pid>.url`、`inputia-*-osascript.<pid>.applescript` 和 `inputia-hitoolbox-preference.<pid>.txt`，让 `tmpResidue=false` 变成弱证据。

实现：

- `verify-nongui.sh`
  - `tmp_residue` 扫描同时覆盖旧固定路径和新 PID 路径：
    - `inputia-*-select.*.log`
    - `inputia-*-restore.*.log`
    - `inputia-safari-*-test.*.url`
    - `inputia-*-osascript.*.applescript`
    - `inputia-hitoolbox-preference.*.txt`
  - `cleanupPermissionContract` 新增静态契约，要求这些残留扫描 pattern 保留在 `verify-nongui.sh` 里。

选择/放弃：

- 选择：补齐精确 pattern，而不是简单扫描所有 `/tmp/inputia-*`；这样能覆盖本项目 smoke 生成物，同时避免未来外部调用者主动保留的诊断文件被误判。
- 选择：保留旧固定文件名 pattern；这样旧脚本或中断残留仍会被发现。
- 放弃：只依赖最后人工 `find /tmp -name 'inputia-*'`；原因是 CI/本地回归的机器可读 gate 必须自己覆盖这类污染。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  timeoutHelperSelfCheck=true
  verifyPkg: pkgVerificationPassed=true
  verifyPkg: sha256=84f4cf2e312931d7ce1b5955ebb78d64ad1cd249a6e4c7f1d228d99025590395
  status: buildVersion=41
  status: buildCDHash=8b68f7005693162ea3eebae9ebff570550c1e93a
  status: systemMatchesBuild=false
  status: matches=0
  textEditUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `tmpResidue=false` 现在覆盖 PID 化后的 GUI smoke 临时文件命名，不再只证明旧固定文件没有残留。
- 真实 GUI smoke 仍未强跑；系统安装版仍是 v40，build/pkg 是 v41，TIS 仍 `matches=0`。

## v41 Mac mini 续修：README 补齐 Command smoke 与契约缩进修复

问题：

- `smoke-textedit-command-shortcuts.sh` 和 `smoke-safari-command-shortcuts.sh` 已经进入当前工作树，但 README 仍主要描述 TextEdit 输入/Shift、Safari typing/enter 和 Clipboard recall，容易让安装后收口漏跑 Command 快捷键 smoke。
- `verify-nongui.sh` 的 Python 静态契约 heredoc 中有 tab/space 混用；前一次验证在 cleanup contract 阶段直接 `TabError`，导致后续 no-launch、TIS gate、residue 检查都没有真正执行。

实现：

- `README.md`
  - 新增 TextEdit Command 快捷键 smoke 小节，说明 `Command-A/C/V` 验证内容、`INPUTIA_RUN_UI_SMOKE=1` 前置条件、剪贴板/输入源恢复和 TextEdit 文档清理纪律。
  - 更新 `post-install-regression.sh` 说明，明确真实 UI 顺序包含 TextEdit 输入、TextEdit `Command-A/C/V`、Safari 输入源诊断、Safari typing、Safari raw ASCII enter、Clipboard recall。
  - 安装后收口说明补充：单独排查 TextEdit 时可以分别跑 `smoke-textedit.sh` 与 `smoke-textedit-command-shortcuts.sh`，但不能绕过 preflight/no-launch 门禁。
- `verify-nongui.sh`
  - 统一 Python 静态契约 heredoc 中的 tab/space 缩进。
  - 保留 TextEdit command 与 Safari command smoke 的 cleanup marker、PID 临时文件、窗口关闭、剪贴板恢复和 timeout 契约。

选择/放弃：

- 选择：文档写真实 smoke 的安全边界，而不是只列命令；原因是当前重点是防止 TextEdit/Safari/Clipboard smoke 抢焦点、污染状态或留下窗口。
- 选择：修复契约缩进后重跑完整 `verify-nongui.sh`，不只跑 `bash -n`；原因是 `bash -n` 不能解析 Python heredoc 的运行时缩进错误。

验证：

```text
python3 heredoc tab scan for verify-nongui.sh
  rc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash -n macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh
  rc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  verifyPkg: pkgVerificationPassed=true
  textEditCommandUiDisabled.clipboardUnchanged=true
  textEditCommandUiTisGateNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 文档和 post-install 实际 UI smoke 链一致，不再遗漏 Command 快捷键 smoke。
- 非 GUI 验证入口已恢复可运行，并能继续证明 UI disabled/TIS-not-ready 路径不打开 TextEdit/Safari、不改剪贴板、不改输入源、不留下临时文件。

## v41 Mac mini 续修：Safari Command smoke 文档补齐

问题：

- 当前工作树已有 `smoke-safari-command-shortcuts.sh`，并且 `post-install-regression.sh` 的真实 UI 链也会在 Safari typing 后运行它。
- 但 README 上一轮只补了 TextEdit Command smoke，post-install 顺序仍写成 Safari typing、Safari raw ASCII enter、Clipboard recall，遗漏 Safari `Command-A/C/V`。
- 这会让维护者按文档单独收口 Safari 时漏跑宿主 App Command 快捷键验证。

实现：

- `README.md`
  - 新增 Safari Command 快捷键 smoke 小节。
  - 明确脚本验证 Safari 输入框中的 `Command-A/C/V`：打开本地 `data:` 测试页，复制、清空、粘贴，再通过页面标题确认结果。
  - 明确门禁和清理纪律：默认不打开 Safari；必须 `INPUTIA_RUN_UI_SMOKE=1`；图形会话、Safari preflight、系统安装版 CDHash、TIS 选择通过后才改剪贴板或创建窗口；失败/退出路径恢复剪贴板、恢复输入源，并按窗口 id 关闭脚本创建的测试窗口。
  - 更新 post-install UI 顺序，加入 Safari `Command-A/C/V`。
  - 安装后单独排查说明加入 `smoke-safari-command-shortcuts.sh`。

选择/放弃：

- 选择：只更新文档，不改 smoke 逻辑；原因是 `verify-nongui.sh` 已经覆盖 Safari command 的 UI-disabled 和 Safari-existing gate，包括剪贴板不变、输入源不变和 debug env 不变。
- 放弃：当前不强跑真实 Safari Command GUI smoke；系统安装版仍是 v40，TIS 未 ready，强跑会污染用户 Safari/剪贴板结论。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash -n macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
  rc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  safariCommandUiDisabled.clipboardUnchanged=true
  safariCommandExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- README、post-install UI 链和非 GUI 契约现在都包含 Safari `Command-A/C/V` smoke。
- 真实 GUI smoke 仍等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：README 明确 verify-nongui 覆盖 Command smoke

问题：

- `verify-nongui.sh` 已经覆盖 TextEdit 输入、TextEdit Command、Clipboard recall、Safari typing、Safari Command、Safari enter 的 UI-disabled/TIS-not-ready/existing-app 早退门禁。
- README 里的 `verify-nongui.sh` 概述仍写成泛化的 “TextEdit/Clipboard/Safari smoke”，没有明确 Command 快捷键 smoke 也纳入 no-launch/no-mutation 收口，容易低估验证入口覆盖面。

实现：

- `README.md`
  - 更新 `verify-nongui.sh` 说明，明确列出：
    - TextEdit 输入 smoke
    - TextEdit `Command-A/C/V` smoke
    - Clipboard recall smoke
    - Safari typing smoke
    - Safari `Command-A/C/V` smoke
    - Safari enter smoke
  - 明确这些早退路径会验证不改剪贴板、不改当前输入源、不留下 GUI 进程。

选择/放弃：

- 选择：只补文档，不改脚本；原因是本轮检查显示 `verify-nongui.sh` 的实际契约和运行验证已经覆盖这些路径。
- 放弃：当前不运行真实 GUI smoke；系统安装版仍是 v40，TIS 未 ready。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash -n macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh
  rc=0

bash -n macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
  rc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- README 对默认安全收口入口的说明与当前 `verify-nongui.sh` 覆盖范围一致。
- 当前真实 GUI smoke 仍等待系统安装版 v41 和 TIS readiness。

## v41 Mac mini 续修：Command 快捷键通用放行与 Safari Command gate 复核

问题：

- 用户反馈在 Inputia 下 `Command-C` / `Command-V` 不可用，判断为输入法层接管了系统复制粘贴。
- 该问题不能只修复制/粘贴两个键，输入法应举一反三放行常见 macOS `Command` 系统/App 快捷键，避免吞掉剪切、撤销、重做、全选、保存、打开、关闭、查找、隐藏、最小化、打印、标签页、窗口/App 切换、截图等 AppKit/系统快捷键。

实现：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut` 对带 `.command` 的快捷键直接放行，且在 `handleKeyDown` 早于候选、剪贴板召回、Shift/标点等 Inputia 内部快捷键处理。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand` 覆盖常见 AppKit selector，避免 host text command 路径吞掉复制、粘贴、剪切、撤销、重做、全选等编辑命令。
- `InputiaShortcutSelfCheck` 与 host `--host-shortcut-self-check` 覆盖 `Command-C/V`、`Command-Shift-V`、`Command-Option-V`、`Command-Option-Shift-V`、`Command-Control-V` 以及一批常用 `Command` 字母、数字、括号、箭头、Delete、Tab、Space、截图和 Force Quit 组合。
- 保留 Inputia 自己的 `Control-Shift-V` 剪贴板召回；`Control-Shift-Command-V` 明确拒绝，避免和系统/App `Command` 组合冲突。
- 新增/纳入 TextEdit 与 Safari 的 `Command-A/C/V` smoke gate，非 GUI 验证覆盖 UI disabled、TIS-not-ready、Safari already running 三类安全门禁。

选择/放弃：

- 选择：所有带 `Command` 的 key-down 快捷键在输入法层通用放行，而不是维护一张易遗漏的复制/粘贴白名单。
- 选择：仍保留无 `Command` 的 Inputia 自定义快捷键，例如 `Control-Shift-V`。
- 放弃：当前不强跑真实 TextEdit/Safari GUI smoke；系统安装版仍是 v40，build 是 v41，TIS 未 ready，强跑会污染用户 GUI/剪贴板结论。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  shortcutSelfCheck=true
  hostTextPolicySelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true
  safariCommandUiDisabled.clipboardUnchanged=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 常见 `Command` 系统/App 快捷键现在在 Inputia 输入法层通用放行，`Command-C` / `Command-V` 不再走 Inputia 内部处理。
- TextEdit/Safari Command smoke 的安全门禁和 post-install 链路已由 non-GUI 验证覆盖。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：verify-nongui 并发锁 stale 自愈

问题：

- `verify-nongui.sh` 已有并发锁，避免多个非 GUI 验证同时运行时互相抢 `INPUTIA_DEBUG_EVENTS`、临时文件和短暂 host self-check。
- 但旧逻辑只要锁目录存在就直接 `verify-already-running`，如果上一次验证被强制终止并留下 stale lock，后续验证会持续失败。
- 锁路径原先使用 `${TMPDIR:-/tmp}/inputia-verify-nongui.lock`，而残留扫描只扫 `/tmp`，在 macOS 上可能导致锁目录落到 per-user temp 后不被 `tmpResidue` 发现。

实现：

- `verify-nongui.sh`
  - 锁路径固定为 `/tmp/inputia-verify-nongui.lock`，与残留扫描和人工收口命令一致。
  - 新增 `acquire_verify_lock()`：
    - 正常获取锁时输出 `verifyLockAcquired=true`。
    - 如果锁存在且 pid 仍存活，保持拒绝并输出 `verify-already-running` 与 `verifyLockOwnerPid=...`，不清理活锁。
    - 如果锁存在但 pid 不存在，输出 `verifyLockStale=true ...`，删除 stale lock 后重试获取。
    - 重试仍失败时输出 `verify-lock-acquire-failed`。
  - `cleanupPermissionContract` 静态检查锁自愈输出和锁残留 pattern。
  - `tmp_residue` 扫描加入 `inputia-verify-nongui.lock`。

选择/放弃：

- 选择：只自愈 pid 不存在的 stale lock；活锁仍拒绝，避免误杀正在运行的验证。
- 选择：固定使用 `/tmp`，因为当前 smoke 临时文件、人工残留检查和 post-install lock 都以 `/tmp/inputia-*` 为收口面。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

预置 stale lock: /tmp/inputia-verify-nongui.lock/pid = 999999
INPUTIA_PROCESS_WAIT_TICKS=20 bash macos/InputiaInputMethod/verify-nongui.sh
  staleLockVerifyRc=0
  verifyLockStale=true path=/tmp/inputia-verify-nongui.lock pid=999999
  verifyLockAcquired=true
  cleanupPermissionContract=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

预置 live lock: /tmp/inputia-verify-nongui.lock/pid = current shell pid
bash macos/InputiaInputMethod/verify-nongui.sh
  liveLockRc=20
  nonGuiVerificationPassed=false reason=verify-already-running lock=/tmp/inputia-verify-nongui.lock
  verifyLockOwnerPid=<current-shell-pid>

普通完整验证:
bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `verify-nongui.sh` 的并发锁现在能区分活锁和 stale lock；验证被中断后不会永久卡住后续回归。
- 锁目录也纳入 `/tmp` 残留收口，`tmpResidue=false` 语义更完整。

## v41 Mac mini 续修：verify-nongui 锁契约加固

问题：

- 上一段实现了 `verify-nongui.sh` 的 stale lock 自愈，但静态契约还只检查存在 stale/failure 输出，没有防止后续把锁路径改回 `${TMPDIR}`，或把 `release_verify_lock` 移到 `tmp_residue` 扫描之后。
- 如果锁释放顺序漂移，`tmpResidue=false` 可能重新被当前运行自己的 lock 干扰；如果锁路径漂移，`/tmp` 残留检查又会漏掉 per-user temp 下的锁。

实现：

- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态约束：
    - `VERIFY_LOCK_DIR` 必须固定为 `/tmp/inputia-verify-nongui.lock`。
    - 活锁失败路径必须输出 `verifyLockOwnerPid=$existing_pid`。
    - `section "residue"` 后必须先 `release_verify_lock`，再进入 `tmp_residue` 扫描。
    - `tmp_residue` pattern 必须包含 `inputia-verify-nongui.lock`。

选择/放弃：

- 选择：用静态契约锁住顺序，因为这里是验证脚本自身的安全属性，不需要每次完整验证都递归启动另一个验证进程。
- 放弃：不在 `verify-nongui.sh` 内部加入递归 lock self-test；递归会和当前锁设计冲突，适合作为外部专项测试。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

预置 stale lock: /tmp/inputia-verify-nongui.lock/pid = 999999
bash macos/InputiaInputMethod/verify-nongui.sh
  staleLockVerifyRc=0
  verifyLockStale=true path=/tmp/inputia-verify-nongui.lock pid=999999
  verifyLockAcquired=true
  cleanupPermissionContract=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

预置 live lock: /tmp/inputia-verify-nongui.lock/pid = current shell pid
bash macos/InputiaInputMethod/verify-nongui.sh
  liveLockRc=20
  nonGuiVerificationPassed=false reason=verify-already-running lock=/tmp/inputia-verify-nongui.lock
  verifyLockOwnerPid=<current-shell-pid>

普通完整验证:
bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- verify lock 的路径、活锁诊断、stale 自愈和释放顺序现在都有契约覆盖。
- 普通非 GUI 回归仍通过；真实 GUI smoke 继续等待系统安装版 v41 和 TIS readiness。

## v41 Mac mini 续修：non-GUI 验证并发锁

问题：

- `verify-nongui.sh` 会运行 post-install、verify-system、host self-check 和多个 smoke gate。
- 如果两条 `verify-nongui.sh` 并发运行，后一条可能把前一条的 `InputiaInputMethod --bridge-*self-check` 或 shell 链路误判成残留，造成 `left-inputia-host` / `residue` 类假失败。
- 这会削弱 GUI smoke 清理纪律：失败来源不再能区分真实脚本残留和验证器之间的并发污染。

实现：

- `verify-nongui.sh` 增加 `/tmp/inputia-verify-nongui.lock` 原子锁。
- 活锁存在且 owner pid 仍运行时，立即输出 `nonGuiVerificationPassed=false reason=verify-already-running` 并返回 rc=20。
- stale lock owner pid 不存在时，输出 `verifyLockStale=true`，移除旧锁后重试 acquire。
- 正常退出通过 trap 释放锁；进入最终 residue 检查前也显式释放锁，避免验证器自己的 lock 被 tmp residue 检查命中。
- cleanup trap 仍负责恢复 `INPUTIA_DEBUG_EVENTS`，并在异常退出时释放已获得的锁。

选择/放弃：

- 选择：在验证入口加锁，而不是放宽残留断言；残留断言仍应严格捕获真正泄漏的 host、osascript、TextEdit/Safari 或 smoke shell。
- 选择：stale lock 自动恢复，避免异常退出后永久阻塞后续本地验证。
- 放弃：不在真实 GUI smoke 脚本内部共享这把锁；锁只约束聚合 non-GUI 验证入口，避免扩大行为面。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

# live lock
./macos/InputiaInputMethod/verify-nongui.sh
  nonGuiVerificationPassed=false reason=verify-already-running lock=/tmp/inputia-verify-nongui.lock
  verifyLockOwnerPid=<running-shell-pid>
  liveLockRc=20

# stale lock + full verification
./macos/InputiaInputMethod/verify-nongui.sh
  verifyLockStale=true path=/tmp/inputia-verify-nongui.lock pid=999999
  verifyLockAcquired=true
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- non-GUI 聚合验证不再允许并发运行互相污染残留判断。
- stale lock 可自动恢复，便于异常中断后继续验证。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：verify-nongui 锁行为文档化复核

问题：

- `verify-nongui.sh` 已经用 `/tmp/inputia-verify-nongui.lock` 防止聚合验证并发污染残留判断，但 README 只说明它会检查 `/tmp/inputia-*`，没有说明活锁 rc=20、`verifyLockOwnerPid` 和 stale lock 自愈语义。
- 如果后续把活锁误当普通失败处理，或手工删除仍在运行的验证锁，可能重新引入并发污染。

实现：

- `README.md`
  - 在 `verify-nongui.sh` 小节补充并发锁说明。
  - 记录活锁输出 `nonGuiVerificationPassed=false reason=verify-already-running`、`verifyLockOwnerPid=...` 和 rc=20。
  - 记录 pid 不存在的 stale lock 会自动清理，异常中断后重新运行脚本即可自愈。

选择/放弃：

- 选择：只补文档，不改验证脚本逻辑；上一轮已经用静态契约和专项测试锁住路径、活锁诊断、stale 自愈和释放顺序。
- 放弃：不绕过 `cdhash-mismatch` / TIS readiness gate 强跑真实 GUI smoke；当前系统安装版仍是 v40，build/pkg 是 v41。

验证：

```text
git status --short
  保留既有 dirty/untracked；macos/InputiaInputMethod 仍在未跟踪目录内。

bash -n macos/InputiaInputMethod/verify-nongui.sh macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutSelfCheck=true
  hostTextPolicySelfCheck=true
  textEditUiDisabled.rc=14
  textEditCommandUiDisabled.rc=16
  clipboardUiDisabled.rc=7
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- README 现在把 `verify-nongui.sh` 的并发锁行为、活锁诊断和 stale 自愈路径写清楚了。
- 非 GUI 聚合验证仍通过；没有启动 TextEdit，Safari 仅作为既有用户进程触发 existing gate。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：Clipboard recall 触发前状态污染 guard

问题：

- `smoke-clipboard-recall.sh` 已经在触发 `Ctrl+Shift+V` 后等待 `clipboardRecallShown`，并校验 `clipboardRecallCommit index=0 text=$expected`。
- 但脚本没有在触发前证明本轮 event log 里没有 recall 事件；如果双 Escape 清理后仍有残留 recall 状态或旧 host 事件，脚本可能只在后续 result mismatch 才暴露，定位不够直接。

实现：

- `smoke-clipboard-recall.sh`
  - 新增 AppleScript `assertNoClipboardRecallBeforeTrigger(eventLogPath)`。
  - 在双 Escape 清理、空文档断言之后，触发 `Ctrl+Shift+V` 之前读取 event log。
  - 如果触发前已出现 `clipboardRecallShown`，报错 `clipboard-recall-shown-before-trigger`。
  - 如果触发前已出现 `clipboardRecallCommit`，报错 `clipboard-recall-commit-before-trigger`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态契约，要求 clipboard smoke 保留 pre-trigger guard，并确认该 guard 出现在 `key code 9 using {control down, shift down}` 之前。

选择/放弃：

- 选择：在 smoke 脚本中直接区分“触发前已有 recall 污染”和“触发后未成功召回/提交”，让失败原因更靠近真实问题。
- 选择：继续保留后置 `clipboardRecallShown`、`clipboardRecallCommit index=0` 和 `text=$expected` 检查；pre-trigger guard 只负责污染早期发现，不替代成功路径断言。
- 放弃：当前不强跑真实 clipboard GUI smoke；系统安装版仍是 v40，build/pkg 是 v41，TIS readiness 未通过，强跑会绕过既有安全门禁。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  debugEventLogLifecycleSelfCheck=true
  shortcutSelfCheck=true
  ctrlShiftVClipboardRecall=true
  clipboardUiDisabled.rc=7
  clipboardUiDisabled.clipboardUnchanged=true
  clipboardUiTisGate.rc=8
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGateNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard recall smoke 现在会在真正触发 recall 前检查本轮 debug event log 是否已经被 recall 事件污染。
- 非 GUI 回归仍通过；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：Safari enter 触发前状态污染 guard

问题：

- `smoke-safari-enter.sh` 用 Safari 本地 `data:` 页验证 raw ASCII `abc + Return` 进入宿主提交路径，并通过 debug event log 检查 `commit=abc`。
- 但真实路径在打字前没有像 TextEdit/clipboard smoke 那样显式清理 IME 状态，也没有在打字前确认本轮 event log 里没有旧的 `commit=abc`。
- 如果前一轮 composition/candidate 状态污染仍存在，Safari enter smoke 可能在结果阶段才暴露问题，排查粒度不够直接。

实现：

- `smoke-safari-enter.sh`
  - AppleScript 新增 `clearInputiaState()`，在 Safari 测试页前台后连续发送两次 Escape 并等待稳定。
  - 新增 `assertNoRawCommitBeforeTyping(eventLogPath)`，在打 `abc` 前读取本轮 event log。
  - 如果打字前已经出现 `commit=abc`，报错 `safari-enter-raw-commit-before-typing`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 Safari enter 静态契约：
    - 必须有 `clearInputiaState()`。
    - 必须通过 Escape 清状态。
    - 必须有 pre-typing commit guard。
    - guard 必须在第一个 `key code 0` 输入前执行。

选择/放弃：

- 选择：让 Safari enter 和 TextEdit/clipboard smoke 使用同类“先清状态，再证明触发前无污染”的纪律。
- 选择：继续保留后置 `commit=abc` 检查；pre-typing guard 只防止旧日志/旧状态误导结果。
- 放弃：不绕过 `safari-already-running`、`cdhash-mismatch` 或 TIS gate 强跑真实 Safari GUI smoke；当前 Safari 是既有用户进程，系统安装版仍是 v40。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  debugEventLogLifecycleSelfCheck=true
  shortcutSelfCheck=true
  inputTextCarriageReturnIsEnter=true
  safariEnterUiDisabled.rc=5
  safariEnterExistingGate.rc=7
  safariEnterExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari enter smoke 现在和 TextEdit/clipboard smoke 一样，在真实输入前有显式 IME 状态清理和触发前日志污染断言。
- 非 GUI 回归仍通过；真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41、TIS readiness 通过，并且 Safari/TextEdit 不被用户预先打开。

## v41 Mac mini 续修：post-install 回归锁可观测性

问题：

- `post-install-regression.sh` 是安装后回归和 `verify-nongui.sh` 的下游聚合入口，会运行 `verify-system.sh` 和可选 UI smoke 链。
- 脚本已有 `/tmp/inputia-post-install-regression.lock`，但成功获取锁时没有明确输出，释放时也没有把内部 held 状态归零。
- 当排查并发残留或 stale lock 时，缺少 `postInstallLockAcquired=true` 这样的证据点，不利于区分“未进入脚本主体”和“进入后失败”。

实现：

- `post-install-regression.sh`
  - 获取锁成功后输出 `postInstallLockAcquired=true`。
  - `release_regression_lock()` 删除锁后设置 `LOCK_HELD=0`，避免后续 trap 或显式释放重复认为锁仍持有。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 增加 post-install 锁契约检查：固定 `/tmp/inputia-post-install-regression.lock`、活锁拒绝、stale lock 恢复、锁获取输出和释放状态归零。

选择/放弃：

- 选择：只增强锁行为可观测性，不改变 post-install 的 UI/TIS 门禁顺序。
- 选择：继续让 post-install 自己管理锁；`verify-nongui.sh` 不复用这把锁，只检查下游行为和最终残留。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  rc=0

# live lock
INPUTIA_RUN_UI_SMOKE=0 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallRegressionReady=false reason=already-running pid=<current-shell-pid>
  postLiveLockRc=5

# stale lock
INPUTIA_RUN_UI_SMOKE=0 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallLockStale=true path=/tmp/inputia-post-install-regression.lock pid=999999
  postInstallLockAcquired=true
  postInstallRegressionPassed=true
  postStaleLockRc=0

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- post-install 聚合入口的并发/陈旧锁行为现在有明确输出，可被 non-GUI 契约保护。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：verify-nongui 持锁到退出

问题：

- 上一轮为避免 `/tmp/inputia-verify-nongui.lock` 被 `tmp_residue` 扫描命中，在 `section "residue"` 开始时提前释放了 verify 锁。
- 这会重新打开一个竞态窗口：第一条 `verify-nongui.sh` 仍在最终残留检查中，第二条验证已经可以获取锁并启动，导致残留判断再次可能被并发验证污染。

实现：

- `verify-nongui.sh`
  - 移除 residue 段前的 `release_verify_lock`。
  - 锁继续由 `trap cleanup_verify EXIT` 释放，覆盖完整验证生命周期。
  - `tmp_residue` 扫描增加 `! -path "$VERIFY_LOCK_DIR"`，只排除当前运行中的验证锁，其他 `/tmp/inputia-*` 残留仍继续检查。
  - `cleanupPermissionContract` 改为断言 residue 到 tmp scan 之间不能释放锁，并要求 tmp scan 显式排除当前锁。

选择/放弃：

- 选择：持锁到进程退出，避免最终 residue 检查期间出现第二条验证。
- 选择：排除当前进程自己的 verify lock，而不是从残留扫描里移除锁模式；外部最终残留检查仍能发现异常遗留的 `/tmp/inputia-verify-nongui.lock`。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

# live lock still rejected
./macos/InputiaInputMethod/verify-nongui.sh
  nonGuiVerificationPassed=false reason=verify-already-running lock=/tmp/inputia-verify-nongui.lock
  verifyLockOwnerPid=<current-shell-pid>
  verifyLiveLockRc=20

# full run with lock probe
./macos/InputiaInputMethod/verify-nongui.sh
  lockSeenWhileRunning=true
  verify_rc=0
  lockExistsAfterExit=false
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `verify-nongui.sh` 现在覆盖完整运行周期持锁，最终残留检查期间不会再允许第二条验证介入。
- 自身 lock 不再造成 `tmpResidue` 假阳性，退出后 lock 会被清理。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：verify-system 临时日志清理契约

问题：

- `verify-system.sh` 的 `run_inputia()` 会为 self-check/bridge/TIS dump 创建 `${TMPDIR:-/tmp}/inputia-<label>-<attempt>-$$.log`。
- LaunchServices 诊断路径还会创建 `${TMPDIR:-/tmp}/inputia-launchservices-$$.log`。
- 正常路径会删除这些文件，但异常中断时缺少统一 trap 清理；同时 `verify-nongui.sh` 的 `tmp_residue` 模式没有覆盖 `inputia-launchservices-*.log`。

实现：

- `verify-system.sh`
  - 新增 `TEMP_FILES=()` 注册表。
  - `run_inputia()` 创建的每个 output log 都加入 `TEMP_FILES`。
  - LaunchServices dump log 也加入 `TEMP_FILES`。
  - `cleanup` trap 统一删除 `TEMP_FILES` 内所有临时文件。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 增加 `verify-system.sh` 临时文件注册检查。
  - `tmp_residue` 增加 `inputia-launchservices-*.log` 模式，避免 LaunchServices 诊断残留漏报。

选择/放弃：

- 选择：保留 `verify-system.sh` 现有 per-command 临时日志文件名，只补统一生命周期管理。
- 选择：把 LaunchServices 临时日志纳入最终 `/tmp/inputia-*` 收口，而不是只依赖脚本内部正常路径删除。

验证：

```text
zsh -n macos/InputiaInputMethod/verify-system.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_VERIFY_LAUNCHSERVICES=1 macos/InputiaInputMethod/verify-system.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  verifySystemRc=0
  launchservicesResidue=0

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `verify-system.sh` 的自检和 LaunchServices 临时日志现在有统一 trap 清理。
- `verify-nongui.sh` 的临时文件残留扫描覆盖 LaunchServices 日志。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：Safari typing 输入前状态污染 guard

问题：

- `smoke-safari-typing.sh` 打开 Safari 本地 `data:` 页后直接输入 `ni + Space`，不像 TextEdit/clipboard/Safari enter smoke 那样先显式清理 IME 状态。
- Safari typing 不使用 debug event log，因此无法用事件日志判断触发前是否已有旧 commit；但测试页标题会实时反映输入框值，可以作为本轮输入前的污染探针。

实现：

- `smoke-safari-typing.sh`
  - AppleScript 新增 `clearInputiaState()`，在 Safari 测试页前台后连续发送两次 Escape 并等待稳定。
  - 新增 `assertEmptyBeforeTyping(smokeWindowId)`，输入前读取测试页标题。
  - 如果标题不是初始 `VALUE:`，报错 `safari-typing-state-clear-leaked-title:<title>`，说明输入前已有残留状态写入页面。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 Safari typing 静态契约：
    - 必须有 `clearInputiaState()`。
    - 必须通过 Escape 清状态。
    - 必须有输入前空标题 guard。
    - guard 必须在 `$keys` 输入脚本执行前。

选择/放弃：

- 选择：用测试页标题作为输入框内容的可观测状态，避免给 Safari typing 引入新的 debug event log 生命周期。
- 选择：保持原来的后置 CJK/raw/nonempty 结果断言；输入前 guard 只负责更早暴露状态污染。
- 放弃：当前不绕过 `safari-already-running`、`cdhash-mismatch` 或 TIS gate 强跑真实 Safari GUI smoke；当前 Safari 是既有用户进程，系统安装版仍是 v40。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-typing.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  debugEventLogLifecycleSelfCheck=true
  shortcutSelfCheck=true
  safariTypingUiDisabled.rc=7
  safariTypingExistingGate.rc=9
  safariTypingExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari typing smoke 现在在真实输入前也有显式 IME 状态清理和页面空状态断言。
- 非 GUI 回归仍通过；真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41、TIS readiness 通过，并且 Safari/TextEdit 不被用户预先打开。

## v41 Mac mini 续修：Safari Command smoke 输入前状态污染 guard

问题：

- `smoke-safari-command-shortcuts.sh` 验证 Safari 输入框里的 `Command-A/C/V` 由宿主 App 处理，但此前打开本地测试页后直接执行 Command 快捷键。
- 如果上一轮 IME composition/candidate 状态在 Safari 页面激活后仍残留，可能污染测试页内容或选区，再导致 copy/paste 结果难以定位。

实现：

- `smoke-safari-command-shortcuts.sh`
  - AppleScript 新增 `clearInputiaState()`，在 Safari 测试页前台后连续发送两次 Escape 并等待稳定。
  - 新增 `assertCommandSourceBeforeCopy(smokeWindowId)`，在执行 `Command-A` 前读取测试页标题。
  - 如果标题不是初始 `VALUE:Inputia Safari Command Source`，报错 `safari-command-state-clear-leaked-title:<title>`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 Safari Command 静态契约：
    - 必须有 `clearInputiaState()`。
    - 必须通过 Escape 清状态。
    - 必须有输入前 source title guard。
    - guard 必须在 `Command-A` 前执行。

选择/放弃：

- 选择：和 Safari typing 一样使用本地页面标题作为输入前污染探针，不引入新的 debug event log 生命周期。
- 选择：保留原来的 `Command-A/C/V` 结果断言；输入前 guard 只负责更早暴露残留状态。
- 放弃：当前不绕过 `safari-already-running`、`cdhash-mismatch` 或 TIS gate 强跑真实 Safari GUI smoke；当前 Safari 是既有用户进程，系统安装版仍是 v40。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  debugEventLogLifecycleSelfCheck=true
  shortcutSelfCheck=true
  commandAPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  safariCommandUiDisabled.rc=12
  safariCommandUiDisabled.clipboardUnchanged=true
  safariCommandExistingGate.rc=13
  safariCommandExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari Command smoke 现在在执行 `Command-A/C/V` 前也有显式 IME 状态清理和页面 source 状态断言。
- 非 GUI 回归仍通过；真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41、TIS readiness 通过，并且 Safari/TextEdit 不被用户预先打开。

## v41 Mac mini 续修：TextEdit Command smoke 输入前 source 断言

问题：

- `smoke-textedit-command-shortcuts.sh` 已经在 `Command-A/C/V` 前双 Escape 清理 IME 状态，并重新写入 `sourceText`。
- 但脚本没有在 `Command-A` 前明确断言 TextEdit 文档内容仍等于 `sourceText`；如果残留 IME 状态在清理后写入或覆盖文档，失败会在 copy/paste 结果阶段才出现。

实现：

- `smoke-textedit-command-shortcuts.sh`
  - AppleScript 新增 `assertSourceBeforeCopy(docRef, sourceText)`。
  - 在重新写入 `sourceText` 后、执行 `Command-A` 前读取 `docRef` 文本。
  - 如果当前文本不等于 `sourceText`，报错 `textedit-command-state-clear-leaked-text:<text>`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增 TextEdit Command 静态契约：
    - 必须有 `assertSourceBeforeCopy(docRef, sourceText)`。
    - 必须有 `textedit-command-state-clear-leaked-text:` 错误。
    - guard 必须在 `Command-A` 前执行。

选择/放弃：

- 选择：用 TextEdit 文档引用 `docRef` 读取当前文本，和现有“按文档引用清理”纪律保持一致。
- 选择：保留原有 copy/paste 结果断言；输入前 guard 只负责更早定位状态污染。
- 放弃：当前不强跑真实 TextEdit GUI smoke；系统安装版仍是 v40，build/pkg 是 v41，TIS readiness 未通过。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  debugEventLogLifecycleSelfCheck=true
  shortcutSelfCheck=true
  commandAPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  textEditCommandUiDisabled.rc=16
  textEditCommandUiDisabled.clipboardUnchanged=true
  textEditCommandUiTisGate.rc=17
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGateNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TextEdit Command smoke 现在在执行 `Command-A/C/V` 前也有显式 source 文档状态断言。
- 非 GUI 回归仍通过；真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：/private/tmp 残留扫描修正

问题：

- macOS 的 `/tmp` 是指向 `/private/tmp` 的符号链接。
- `verify-nongui.sh` 之前用 `find /tmp -maxdepth 1` 做最终临时文件残留扫描；该命令不会遍历符号链接目标目录，所以 `tmpResidue=false` 对真实 `/private/tmp/inputia-*` 残留证明不足。
- 复核时在 `/private/tmp` 发现 116 个历史 `inputia-*` 临时日志/目录，说明旧的最终残留检查存在盲区。

实现：

- `verify-nongui.sh`
  - 新增 `TMP_RESIDUE_ROOT="/private/tmp"`。
  - `tmp_residue` 扫描改为 `find "$TMP_RESIDUE_ROOT" -maxdepth 1`。
  - 当前 verify lock 同时排除 `/tmp/inputia-verify-nongui.lock` 和真实路径 `/private/tmp/inputia-verify-nongui.lock`，保持持锁到 `EXIT` 的同时避免自身 lock 造成假阳性。
  - `tmp_residue` 增加 `inputia-install-user.*`，覆盖 `install-user.sh` 的 `mktemp -d` 目录。
  - `cleanupPermissionContract` 增加 `/private/tmp` root、真实 lock 排除、install-user 临时目录和 cleanup trap 的静态检查。
- 清理 `/private/tmp` 下历史 `inputia-*` / `InputiaSettingsReloadSelfCheck-*` 临时残留：清理前 116，清理后 0。

选择/放弃：

- 选择：扫描真实目录 `/private/tmp`，不再依赖 `/tmp` 符号链接行为。
- 选择：继续只收口 Inputia 命名空间临时物，不扩大到非本项目临时文件。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/install-user.sh
  rc=0

人工残留: /private/tmp/inputia-install-user.ABC123
./macos/InputiaInputMethod/verify-nongui.sh
  installUserResidueRc=1
  /private/tmp/inputia-install-user.ABC123
  nonGuiVerificationPassed=false reason=tmp-residue

/private/tmp 历史残留清理
  privateTmpInputiaResidueBefore=116
  privateTmpInputiaResidueAfter=0

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `tmpResidue=false` 现在基于真实 `/private/tmp`，能实际发现 Inputia 临时目录残留。
- 历史 Inputia 临时残留已清空；后续 non-GUI 验证的残留结论更可信。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：verify-nongui 自身临时文件 trap 清理

问题：

- `verify-nongui.sh` 的 AppleScript 编译检查会用 `mktemp /tmp/inputia-*-applescript.*` 创建源码和 `.scpt` 临时文件。
- 正常路径会删除这些文件，但 `cleanup_verify` 原先只恢复 `INPUTIA_DEBUG_EVENTS` 和释放锁；如果验证在编译段中断，临时文件需要依赖后续残留扫描发现，而不是当前进程自己清理。
- 初次补临时文件数组后，bash 3.2 + `set -u` 下空数组在 `EXIT` trap 中触发 `VERIFY_TEMP_FILES[@]: unbound variable`，导致主体通过但最终 rc=1。

实现：

- `verify-nongui.sh`
  - 新增 `VERIFY_TEMP_FILES=()`。
  - AppleScript 编译 helper 创建 `script_file` 和 `compiled_file` 后立即注册到 `VERIFY_TEMP_FILES`。
  - `cleanup_verify` 调用 `cleanup_verify_temp_files`，在退出路径清理已注册临时文件。
  - `cleanup_verify_temp_files` 使用 `${VERIFY_TEMP_FILES[*]-}`，兼容空数组和 `set -u`。
  - `cleanupPermissionContract` 增加 verify-nongui 自身临时文件注册和 cleanup 检查。

选择/放弃：

- 选择：对 verify-nongui 自己创建的 mktemp 文件做主动 trap 清理，最终 `/private/tmp` residue 扫描作为兜底。
- 选择：使用无空格的 mktemp 路径列表展开，避免 bash 3.2 空数组 nounset 问题。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- verify-nongui 自身 AppleScript 编译临时文件现在有退出清理路径。
- 空临时文件列表不会再让 `EXIT` trap 把成功验证翻成失败。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：post-install UI 链顺序契约与 verify cleanup 空数组修复

问题：

- post-install UI smoke 必须在 TIS readiness 未通过时先输出 `postInstallUiSmokeReady=false reason=tis-not-ready` 并停止，不能启动 TextEdit/Safari/Clipboard smoke。
- `verify-nongui.sh` 的临时文件清理需要兼容 macOS Bash 3.2 + `set -u` 的空数组，同时不能把带空格的路径元素拆开。
- 真实系统安装版仍为 v40，build/pkg 为 v41；当前不具备真实 GUI smoke 条件。

实现：

- `verify-nongui.sh`
  - 增加 post-install UI smoke 链路顺序静态契约：`UI smoke preflight` 后依次为 TextEdit、TextEdit command shortcuts、Safari diagnose、Safari typing、Safari command shortcuts、Safari Enter、Clipboard recall。
  - 增加 TIS-not-ready gate 顺序契约：`postInstallUiSmokeReady=false reason=tis-not-ready` 必须出现在 `UI smoke preflight` 前。
  - 将 `cleanup_verify_temp_files` 的循环收紧为 `${VERIFY_TEMP_FILES[@]+"${VERIFY_TEMP_FILES[@]}"}`，同时更新自检契约，兼容空数组并保留数组元素边界。
- smoke 脚本前置状态守卫保持为本轮验证对象：
  - Clipboard recall 在触发 `Ctrl+Shift+V` 前确认无 `clipboardRecallShown` / `clipboardRecallCommit`。
  - Safari Enter 在输入 `abc` 前确认无 `commit=abc`。
  - Safari typing / command shortcuts 和 TextEdit command shortcuts 在复制或输入前确认页面/文档仍为预期初始状态。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

并发锁失败路径：
  nonGuiVerificationPassed=false reason=verify-already-running lock=/tmp/inputia-verify-nongui.lock
  verifyLockOwnerPid=47045
  liveLockRc=20
  无 VERIFY_TEMP_FILES[@] trap 二次错误

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- non-GUI 契约验证通过；TIS 未 ready 时不会误启动真实 GUI smoke。
- 临时文件清理在空数组和并发锁失败路径下不会污染最终结果。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过；当前阻塞是系统安装需要管理员权限。

## v41 Mac mini 续修：已有 App preflight 防回退契约

问题：

- TextEdit/Safari/Clipboard GUI smoke 的纪律不仅要依赖脚本当前实现，还需要在 non-GUI 自检里锁住：已有 TextEdit 或 Safari 时，脚本必须先拒跑，不能先切输入源或启动后续 AppleScript。
- 当前 Mac mini 上 Safari 仍是用户既有进程，真实 Safari smoke 不能抢用户窗口。

实现：

- `verify-nongui.sh`
  - 新增 `preflight_contracts` 静态契约。
  - TextEdit 系列脚本必须包含 `inputia_require_textedit_idle`，且调用顺序早于 `inputia_select_input_source_or_exit`。
  - Safari 系列脚本和 Safari diagnose 必须包含 `inputia_require_safari_idle`，且调用顺序早于 `inputia_select_input_source_or_exit`。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 已有 App preflight 现在是 non-GUI gate 的显式契约，后续修改不能绕过先拒跑再切输入源的顺序。
- TIS 未 ready 时 post-install UI smoke 仍不会启动 TextEdit/Safari/Clipboard。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：命令快捷键 smoke 剪贴板恢复顺序契约

问题：

- TextEdit/Safari command shortcut smoke 会清空并读取系统剪贴板，用于验证 `Command-A/C/V` 不被输入法吞掉。
- 之前 non-GUI 自检只要求脚本存在 `restore_clipboard`，但没有锁住顺序：必须先捕获用户原剪贴板，再写入测试剪贴板，再标记 `CLIPBOARD_CHANGED=1`，最后才运行 AppleScript。
- 如果后续改动把顺序写反，失败路径可能覆盖用户剪贴板。

实现：

- `verify-nongui.sh`
  - 新增 `clipboard_mutation_contracts`。
  - 覆盖 `smoke-textedit-command-shortcuts.sh` 和 `smoke-safari-command-shortcuts.sh`。
  - 验证顺序：`restore_clipboard()` 定义 -> cleanup trap 中调用 `restore_clipboard` -> `ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste ...)"` -> `/usr/bin/printf '' | /usr/bin/pbcopy` -> `CLIPBOARD_CHANGED=1` -> `/usr/bin/osascript`。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

首次运行遇到并发锁：
  nonGuiVerificationPassed=false reason=verify-already-running lock=/tmp/inputia-verify-nongui.lock
  verifyLockOwnerPid=60654
  rc=20
  处理：确认 owner 为活的 bash verify-nongui，等待其自然结束，未删除 live lock。

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- command shortcut smoke 的剪贴板修改现在有顺序契约，能防止后续回退为先改剪贴板再捕获原值。
- 并发验证锁按预期保护 non-GUI 验证，遇到 live owner 时等待而不是删除锁。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：公共输入源恢复顺序契约

问题：

- TextEdit/Safari/Clipboard smoke 会临时切到 Inputia 输入源；失败路径必须恢复用户原输入源。
- 之前 non-GUI gate 覆盖了各 smoke 的 cleanup trap，但没有锁住公共 helper 的关键顺序：切换前先捕获原输入源，选择后确认目标命中，恢复时先读当前源再按原 ID 选择并确认恢复。
- 一次完整验证日志曾在 `post-install regression non-gui` 后出现 `unexpected EOF while looking for matching '"'`，但外层 rc 为 0；需要单独复核避免把带错误的绿灯写进证据。

实现：

- `verify-nongui.sh`
  - 在 `cleanupPermissionContract` 中新增公共输入源恢复契约。
  - 验证 `smoke-common.sh` 包含 `inputia_restore_previous_input_source()`、`INPUTIA_PREVIOUS_INPUT_SOURCE_ID=`、`selectCurrentMatchesTarget=true`、`inputSourceRestore=true id=$previous_id`。
  - 验证顺序：`INPUTIA_PREVIOUS_INPUT_SOURCE_ID=` 早于 `--select-inputia-source-id`；选择确认晚于选择；恢复 helper 内先读当前源，再 `--select-source-id "$previous_id"`，最后输出恢复确认。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  zsh_n_rc=0

env INPUTIA_RUN_UI_SMOKE=0 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postinstall_rc=0
  postInstallRegressionPassed=true

env INPUTIA_RUN_UI_SMOKE=1 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postinstall_ui_rc=6
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready

bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

./macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  本次复跑日志无 unexpected EOF
```

当前结论：

- 输入源切换/恢复的公共 helper 现在有顺序契约，后续 smoke 改动不能绕过“先捕获、再切换、退出恢复”的用户状态保护。
- 前一次 EOF 日志未复现；post-install 两条子路径单独验证正常，完整 non-GUI 复跑干净通过。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：常用 Command 快捷键默认放行策略复核

问题：

- 用户反馈在 Inputia 下 `Command-C` / `Command-V` 不能用，怀疑复制粘贴快捷键被输入法接管。
- 这类问题不能按单个组合逐个补丁；macOS 常用快捷键大量使用 Command 作为系统/应用层修饰键，包括复制、粘贴、剪切、撤销、重做、全选、保存、打开、关闭、退出、查找、打印、隐藏、切换 App、输入源切换、截图、锁屏等。

外部依据：

- Apple Support `Mac keyboard shortcuts` 列出 `Command-C`、`Command-V`、`Command-X`、`Command-Z`、`Command-A`、`Command-F`、`Command-G`、`Command-H`、`Command-M`、`Command-O`、`Command-P`、`Command-S`、`Command-T`、`Command-W`、`Command-Q`、`Command-Tab`、`Command-Space`、`Shift-Command-3/4/5`、`Control-Command-Q`、`Option-Command-Esc` 等常用快捷键。
- Apple Developer Human Interface Guidelines 的 macOS keyboard shortcut 指南把标准快捷键视为宿主 App/系统交互约定；输入法不应抢占这些 Command 组合。
- 参考链接：
  - https://support.apple.com/en-us/102650
  - https://developer.apple.com/design/human-interface-guidelines/keyboards

实现/复核：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut` 对任何包含 `.command` 的 keyDown 直接返回 true。
- `InputiaInputController.handleKeyDown` 在剪贴板召回、候选导航、Shift/标点/宽度/中英切换之前先执行 Command pass-through，避免 `Command-V` 被误判为 Inputia 剪贴板召回。
- Inputia 自有快捷键继续限定为无 Command：
  - 剪贴板召回：只接受 `Control+Shift+V`。
  - `Control+Shift+Command+V` 明确拒绝。
  - 标点、字符宽度、中英切换、候选导航、原始组合数字选择都拒绝 `.command`。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand` 覆盖 AppKit selector 路径，放行 `copy:`、`paste:`、`cut:`、`undo:`、`redo:`、`selectAll:`、`saveDocument:`、`openDocument:`、`performClose:`、`terminate:`、`find:`、`print:`、`hide:`、`showPreferences:`、`toggleBold:`、`toggleItalic:`、`toggleUnderline:`、`goBack:`、`reload:` 等常见应用命令。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
  syntaxOK=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandlessShortcutNotForcedThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandHPassThrough=true
  commandMPassThrough=true
  commandPPassThrough=true
  commandTPassThrough=true
  commandNPassThrough=true
  commandDPassThrough=true
  commandEPassThrough=true
  commandIPassThrough=true
  commandRPassThrough=true
  commandJPassThrough=true
  commandKPassThrough=true
  commandYPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandNumberPassThrough=true
  commandBracketPassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandControlQPassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandshowPreferencesPassesThrough=true
  appCommandtoggleBoldPassesThrough=true
  appCommandtoggleItalicPassesThrough=true
  appCommandtoggleUnderlinePassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandreloadPassesThrough=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  shortcutPassThroughSelfChecks=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Inputia 当前策略不是只放行 `Command-C/V`，而是所有带 Command 的 keyDown 默认交还宿主 App/系统。
- 常用 Command 组合和 AppKit selector 路径都有非 GUI 回归覆盖。
- 真实 TextEdit/Safari Command smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过后再跑，当前不绕过门禁抢用户窗口。

## v41 Mac mini 续修：smoke 启动的 App 清理失败必须可见

问题：

- `smoke-common.sh` 的 `inputia_cleanup_textedit_if_started` / `inputia_cleanup_safari_if_started` 会在脚本自己启动 TextEdit/Safari 后尝试退出并等待进程结束。
- 但等待结束后没有检查进程是否仍然存在；如果 TextEdit 或 Safari 拒绝退出、退出过慢或 cleanup AppleScript 失效，真实 smoke 可能在留下进程/窗口的情况下继续报成功。
- 这违背当前 GUI smoke 纪律：测试成功或失败都不能残留 TextEdit；Safari smoke 也不能关闭或污染用户既有 Safari，只能清理自己启动的实例。

实现：

- `smoke-common.sh`
  - `inputia_cleanup_textedit_if_started` 在 wait 后检查 `pgrep -x TextEdit`。
  - 如果脚本自己启动的 TextEdit 仍存在，输出 `textEditCleanupFailed=process-still-running` 并返回 1。
  - `inputia_cleanup_safari_if_started` 同样检查 `pgrep -x Safari`，失败时输出 `safariCleanupFailed=process-still-running` 并返回 1。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态契约：
    - 必须包含 `textEditCleanupFailed=process-still-running`。
    - 必须包含 `safariCleanupFailed=process-still-running`。

选择/放弃：

- 选择：只在 `INPUTIA_TEXTEDIT_PREFLIGHT=not-running` / `INPUTIA_SAFARI_PREFLIGHT=not-running` 且 cleanup allowed 时失败化；这意味着只约束 smoke 自己启动的 App。
- 放弃：不对用户预先运行的 Safari/TextEdit 做退出或 kill；已有 App preflight 仍负责拒跑，避免抢用户窗口。
- 放弃：不在 cleanup helper 里强制 `kill -9`；当前阶段先把残留变成失败证据，避免隐藏真实退出问题。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 cleanup marker check
  textEditCleanupFailed=process-still-running=True
  safariCleanupFailed=process-still-running=True
  return 1=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  textEditUiDisabled.rc=14
  textEditCommandUiDisabled.rc=16
  clipboardUiDisabled.rc=7
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariPreExisting=true
  safariUiDisabledNoLaunchPassed=true
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- smoke 自己启动的 TextEdit/Safari 如果没有退出干净，现在会让 smoke 失败并留下明确 marker。
- non-GUI 回归证明现有 ui-disabled、TIS-not-ready、Safari-existing gate 仍不会启动或关闭用户 App。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过后执行。

## v41 Mac mini 续修：cleanup trap 失败时仍继续执行后续清理

问题：

- `smoke-common.sh` 的输入源恢复和 App 清理现在会在失败时返回 1。
- 但各 smoke 的 `EXIT` trap 原先是顺序直接调用；在 `set -e` 下，如果前一个清理步骤失败，后续临时文件删除、TextEdit/Safari 退出或 debug env 恢复可能被跳过。
- 这会把“一个恢复失败”放大成多种用户环境污染。

实现：

- `smoke-textedit.sh`
- `smoke-textedit-command-shortcuts.sh`
- `smoke-clipboard-recall.sh`
- `smoke-safari-typing.sh`
- `smoke-safari-command-shortcuts.sh`
- `smoke-safari-enter.sh`
- `diagnose-safari-input-source.sh`

这些脚本的 cleanup trap 统一改为：

- `local cleanup_status=0`
- 每个清理步骤用 `|| cleanup_status=1` 记录失败，但继续执行后续清理。
- 最后 `return "$cleanup_status"`，让失败仍能反映到脚本结果。

`verify-nongui.sh` 增加 `cleanup_continuation_contracts`，覆盖上述 7 个脚本，防止回退成短路式 cleanup。

验证：

```text
bash -n \
  smoke-textedit.sh \
  smoke-textedit-command-shortcuts.sh \
  smoke-clipboard-recall.sh \
  smoke-safari-typing.sh \
  smoke-safari-command-shortcuts.sh \
  smoke-safari-enter.sh \
  diagnose-safari-input-source.sh \
  verify-nongui.sh
  rc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- cleanup trap 现在不会因为单个清理步骤失败而跳过后续清理。
- 失败仍通过 cleanup 返回值暴露，不会静默吞掉恢复/退出失败。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过后执行。

## v41 Mac mini 续修：输入源恢复失败必须让 smoke 失败

问题：

- `inputia_restore_previous_input_source` 会记录 smoke 启动前的当前输入源，并在 cleanup 时尝试恢复。
- 之前如果恢复后当前输入源仍不等于原值，只输出 `inputSourceRestore=false expected=... actual=...`，但函数返回 0。
- 真实 GUI smoke 一旦已经切到 Inputia，再遇到恢复失败，脚本可能继续报成功，留下用户输入源被污染的状态。

实现：

- `smoke-common.sh`
  - `inputia_restore_previous_input_source` 在恢复成功时显式 `return 0`。
  - 恢复失败时保留 `inputSourceRestore=false expected=$previous_id actual=${current_id:-unknown}` marker，并改为 `return 1`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态契约：
    - 必须保留恢复失败 marker。
    - `inputia_restore_previous_input_source` 函数体内必须包含 `return 1`，确保失败不是只记录日志。

选择/放弃：

- 选择：把“恢复失败”作为 smoke 失败，因为输入源污染比单次 smoke 结果更关键。
- 选择：保留 `missing-tis-tool` 和 `already-current` 的跳过语义；这些路径没有实际切换成功或无需恢复。
- 放弃：不在恢复失败后继续尝试多轮切换；当前先让失败显式暴露，后续如遇真实恢复失败再根据日志做最小 spike。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 restore contract check
  restoreFailureMarker=True
  restoreFailureFatal=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariExistingGateNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 真实 GUI smoke 如果切换输入源后无法恢复用户原输入源，现在会失败并保留明确 marker。
- 当前 non-GUI gate 仍证明 TIS 未 ready 时不会启动 TextEdit/Safari/Clipboard smoke，也不会改变当前输入源。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：cleanup trap 返回语义自检

问题：

- smoke 的 cleanup trap 现在会累计清理失败并 `return "$cleanup_status"`。
- 需要明确验证 Bash `EXIT` trap 的返回语义：清理失败必须让原本成功的 smoke 失败；同时主流程失败不能被成功 cleanup 洗成成功。
- 否则 cleanup 失败可能只写日志但不影响最终 rc，或者反过来遮蔽原始失败。

实现：

- `verify-nongui.sh`
  - 新增 `cleanup trap status self-check`。
  - 子用例 1：主流程成功、cleanup 返回 1，要求 rc 非 0。
  - 子用例 2：主流程失败、cleanup 返回 0，要求 rc 非 0。
  - 输出 `cleanupTrapStatusSelfCheck=true` 后才继续后续 non-GUI gate。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupTrapSuccessFailure: body-ok
  cleanupTrapSuccessFailure: cleanup-status-return-1
  cleanupTrapSuccessFailure.rc=1
  cleanupTrapBodyFailure: body-fail
  cleanupTrapBodyFailure: cleanup-status-return-0
  cleanupTrapBodyFailure.rc=1
  cleanupTrapStatusSelfCheck=true
  cleanupPermissionContract=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
  direct_rc=0
```

当前结论：

- cleanup trap 失败会让 smoke 失败，不会静默通过。
- 主流程失败不会被 cleanup 成功返回掩盖。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：cleanup trap 必须聚合失败并继续清理

问题：

- 上一轮把 `inputia_restore_previous_input_source` 的恢复失败改成非零返回，能防止 smoke 在污染输入源后仍报成功。
- 但 macOS smoke 脚本都启用 `set -euo pipefail`；如果 cleanup trap 中某一步非零且没有聚合处理，后续临时文件清理、App 退出、debug env 恢复等步骤可能被跳过。
- 真实 GUI smoke 的 cleanup 目标不是“遇到第一个失败就停”，而是尽量执行全部清理步骤，最后再用非零状态暴露失败。

实现：

- `verify-nongui.sh`
  - 新增 `cleanup_status_contracts` 静态契约，覆盖：
    - `smoke-textedit.sh`
    - `smoke-textedit-command-shortcuts.sh`
    - `smoke-clipboard-recall.sh`
    - `smoke-safari-typing.sh`
    - `smoke-safari-command-shortcuts.sh`
    - `smoke-safari-enter.sh`
    - `diagnose-safari-input-source.sh`
  - 每个 cleanup 函数必须包含：
    - `local cleanup_status=0`
    - 多个 `|| cleanup_status=1`
    - `return "$cleanup_status"`
  - 至少聚合两个 cleanup 步骤，防止单步失败截断后续清理。

选择/放弃：

- 选择：先用 non-GUI 静态契约锁住已有 cleanup 聚合模式；不改 smoke 运行语义。
- 选择：保留 cleanup 最终失败信号，让恢复输入源失败、App 退出失败等问题仍能让 smoke 失败。
- 放弃：不在 cleanup 中吞掉失败强行成功；这会隐藏用户输入源或窗口残留风险。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 cleanup aggregate check
  smoke-textedit.sh: status=True aggregate=3 returns=True
  smoke-textedit-command-shortcuts.sh: status=True aggregate=4 returns=True
  smoke-clipboard-recall.sh: status=True aggregate=6 returns=True
  smoke-safari-typing.sh: status=True aggregate=3 returns=True
  smoke-safari-command-shortcuts.sh: status=True aggregate=4 returns=True
  smoke-safari-enter.sh: status=True aggregate=5 returns=True
  diagnose-safari-input-source.sh: status=True aggregate=4 returns=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verify_rc=0
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- cleanup trap 的多步骤聚合现在被 non-GUI 契约保护，后续改动不能把某个失败提前截断成“后续清理没执行”。
- 当前 TIS 可看到旧系统版 v40 的 Inputia source，但 build/pkg 是 v41，真实 GUI smoke 仍被 cdhash/version mismatch 和 TIS readiness gate 拦住。
- 真实 TextEdit/Safari/Clipboard smoke 仍需等系统安装版更新到 v41 后执行。

## v41 Mac mini 续修：切换输入源前必须捕获可恢复的原输入源

问题：

- GUI smoke 在选择 Inputia 前会记录当前输入源，cleanup 再恢复。
- 但 `inputia_select_input_source_or_exit` 之前只把空值打印成 `previousInputSourceID=unknown`，随后仍可能继续尝试切换到 Inputia。
- 如果当前输入源捕获失败但切换成功，cleanup 没有可恢复目标，会污染用户输入源状态。

实现：

- `smoke-common.sh`
  - 在 `INPUTIA_PREVIOUS_INPUT_SOURCE_ID="$(inputia_current_input_source_id ...)"` 之后新增门禁。
  - 如果捕获结果为空，输出：
    - `guiSmokeReady=false reason=input-source-capture-failed`
    - `$ready_var=false reason=input-source-capture-failed`
  - 打印 select log 尾部后用既有 exit code 退出，不再尝试 `--select-inputia-source-id`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态契约：
    - 必须包含 `input-source-capture-failed`。
    - 该门禁必须出现在 previous input source 捕获之后、Inputia select 之前。

选择/放弃：

- 选择：捕获不到原输入源时拒绝运行真实 GUI smoke；用户输入源状态可恢复性优先于本轮 smoke 执行。
- 放弃：不在缺失 previous id 时继续跑并依赖后续 best-effort restore；这会让失败路径不可逆。
- 放弃：不猜测一个默认输入源作为恢复目标；当前输入源是用户状态，只能用实际捕获值恢复。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 capture gate check
  captureFailureGate=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockStale=true path=/tmp/inputia-verify-nongui.lock pid=18007
  verifyLockAcquired=true
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 真实 GUI smoke 现在只有在能捕获用户当前输入源时才会尝试切到 Inputia，降低输入源污染风险。
- non-GUI gate 仍证明 TIS 未 ready 时不会启动 TextEdit/Safari/Clipboard，也不会改变当前输入源。
- 系统安装版仍是 v40；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：去重 cleanup 聚合契约

问题：

- `verify-nongui.sh` 里同时存在两套 cleanup 聚合静态契约：
  - `cleanup_status_contracts`：检查 7 个 GUI smoke/诊断脚本的 cleanup 函数必须用 `cleanup_status` 聚合多步清理失败，并要求至少两个 `|| cleanup_status=1`。
  - `cleanup_continuation_contracts`：覆盖同一批文件，但只重复检查较弱的继续清理语义。
- 两套契约长期并存会让后续维护者误以为必须同步修改两处规则，增加验证逻辑分叉风险。

实现：

- 删除较弱的 `cleanup_continuation_contracts` 块。
- 保留更严格的 `cleanup_status_contracts`，继续覆盖：
  - `smoke-textedit.sh`
  - `smoke-textedit-command-shortcuts.sh`
  - `smoke-clipboard-recall.sh`
  - `smoke-safari-typing.sh`
  - `smoke-safari-command-shortcuts.sh`
  - `smoke-safari-enter.sh`
  - `diagnose-safari-input-source.sh`

选择/放弃：

- 选择：只保留检查更强的契约，减少重复验证逻辑。
- 放弃：不同时保留强/弱两套检查；重复规则不会提高保护力，反而容易产生维护分叉。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

rg cleanup_continuation_contracts macos/InputiaInputMethod/verify-nongui.sh
  no-match

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGateNoLaunchPassed=true
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- cleanup 聚合语义仍被更严格的静态契约和 trap 返回语义自检保护。
- TIS 未 ready 时，TextEdit/Safari/Clipboard smoke 仍只做门禁验证，不启动目标 App、不改变当前输入源。
- 系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：unknown 输入源不能作为未污染证明

问题：

- `verify-nongui.sh` 的 `current_input_source_id` 在无法读取当前输入源时会返回字面量 `unknown`。
- 之前 `assert_current_source_unchanged` 只拒绝空值和前后不等；如果读取失败导致 `unknown -> unknown`，non-GUI 验证会误判为“输入源未变化”。
- 这对当前目标太弱：GUI smoke 纪律需要证明输入源没有被污染，而不是没读到状态。

实现：

- `verify-nongui.sh`
  - `assert_current_source_unchanged` 现在把 `before == unknown` 或 `after == unknown` 视为不可证明。
  - 失败时输出 `nonGuiVerificationPassed=false reason=<label>-current-input-source-unavailable`。
  - `cleanupPermissionContract` 新增静态契约，要求：
    - 存在 `current-input-source-unavailable` marker。
    - `before` 的 `unknown` 被拒绝。
    - `after` 的 `unknown` 被拒绝。

选择/放弃：

- 选择：把无法读取当前输入源归类为验证失败，而不是“未变化”。
- 放弃：不把 `unknown` 当作合法输入源 ID；它只是诊断占位符。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 current-source contract check
  unavailableReason=True
  beforeUnknownRejected=True
  afterUnknownRejected=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- non-GUI 的“输入源未污染”证明现在必须基于具体当前输入源 ID，不能由 `unknown -> unknown` 伪通过。
- 当前环境能读到具体当前输入源 `com.tencent.inputmethod.wetype.pinyin`，验证通过。
- 真实 GUI smoke 仍等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：剪贴板召回选择前禁止提前 commit

问题：

- `smoke-clipboard-recall.sh` 已经验证：
  - 触发 `Ctrl+Shift+V` 前不能出现 `clipboardRecallShown` / `clipboardRecallCommit`。
  - 最终必须出现 `clipboardRecallShown` 与 `clipboardRecallCommit index=0 text=$expected`。
- 但中间仍有缺口：`clipboardRecallShown` 出现后、按 Space 选择前，如果旧 composition 或残留快捷键状态已经触发 `clipboardRecallCommit`，脚本只会在最终结果阶段发现 mismatch 或误归因。
- 这对当前目标里的“修复 Clipboard GUI smoke 状态污染”不够直接。

实现：

- `smoke-clipboard-recall.sh`
  - 新增 `assertNoClipboardRecallCommitBeforeSelection(eventLogPath)`。
  - 在 `waitForClipboardRecallShown(eventLogPath)` 之后、`key code 49` 选择前执行。
  - 如果事件日志中已经出现 `clipboardRecallCommit`，直接报错 `clipboard-recall-commit-before-selection`。
- `verify-nongui.sh`
  - `cleanupPermissionContract` 新增静态契约：
    - 必须存在选择前 commit guard。
    - 必须存在 `clipboard-recall-commit-before-selection` 错误 marker。
    - guard 顺序必须是：等待 `clipboardRecallShown` 之后、Space 选择之前。

选择/放弃：

- 选择：把“候选已显示但尚未选择”作为单独状态检查，直接捕获提前 commit。
- 放弃：不只依赖最终文本 mismatch；最终 mismatch 对定位状态污染太晚，也不能证明 commit 发生在选择前还是选择后。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 preselection guard check
  preselectionCommitGuard=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard recall smoke 现在区分三段状态：触发前无召回事件、显示后选择前无 commit、选择后 commit 文本必须匹配。
- 这能更早暴露旧 composition/状态污染导致的提前提交。
- 系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：Command 快捷键统一透传复核

问题：

- 用户反馈在 Inputia 下 `Command-C` / `Command-V` 复制粘贴不可用。
- 这类问题不能靠逐个用户反馈补白名单；macOS 常用编辑、文件、窗口、查找、系统快捷键大多以 `Command` 为主，输入法层接管会破坏宿主应用和系统行为。

外部依据：

- Apple 官方《Mac keyboard shortcuts》列出 `Command-X/C/V/Z/A/F/G/H/M/O/P/Q/S/T/W`、`Command-Space`、`Command-Tab`、`Shift-Command-3/4/5`、`Option-Command-Esc` 等常用快捷键。
- Apple 官方《Intro to Mac keyboard shortcuts》同样把 `Command-Tab`、`Command-Space`、截图、退出、保存、全选、撤销等归类为 Mac 常用快捷键。

实现策略：

- 保持根策略：`InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 对任何包含 `.command` 的 `keyDown` 直接返回 true。
- `handleKeyDown` 在处理候选、剪贴板召回、Shift 切换、标点切换之前先执行 Command 透传判断。
- Inputia 自有快捷键继续显式排除 Command，例如：
  - `Ctrl+Shift+V` 是剪贴板召回。
  - `Ctrl+Shift+Command+V` 不触发剪贴板召回，交给系统/应用。
  - 候选翻页/选择快捷键带 Command 时不接管。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 继续覆盖 AppKit `didCommand(by:)` selector 路径，放行 `copy:`、`paste:`、`cut:`、`undo:`、`redo:`、`selectAll:`、`saveDocument:`、`openDocument:`、`performClose:`、`terminate:`、`find:`、`print:`、`hide:`、`showPreferences:`、`toggleBold:`、`toggleItalic:`、`toggleUnderline:`、`goBack:`、`reload:` 等常见宿主命令。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  shortcutPassThroughSelfChecks=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  shortcut: commandShiftVPassThrough=true
  shortcut: commandOptionVPassThrough=true
  shortcut: commandOptionShiftVPassThrough=true
  shortcut: commandControlVPassThrough=true
  shortcut: commandXPassThrough=true
  shortcut: commandZPassThrough=true
  shortcut: commandShiftZPassThrough=true
  shortcut: commandAPassThrough=true
  shortcut: commandSPassThrough=true
  shortcut: commandOPassThrough=true
  shortcut: commandWPassThrough=true
  shortcut: commandQPassThrough=true
  shortcut: commandFPassThrough=true
  shortcut: commandGPassThrough=true
  shortcut: commandHPassThrough=true
  shortcut: commandMPassThrough=true
  shortcut: commandPPassThrough=true
  shortcut: commandTPassThrough=true
  shortcut: commandNPassThrough=true
  shortcut: commandDPassThrough=true
  shortcut: commandEPassThrough=true
  shortcut: commandIPassThrough=true
  shortcut: commandRPassThrough=true
  shortcut: commandJPassThrough=true
  shortcut: commandKPassThrough=true
  shortcut: commandYPassThrough=true
  shortcut: commandCommaPassThrough=true
  shortcut: commandTabPassThrough=true
  shortcut: commandSpacePassThrough=true
  shortcut: commandNumberPassThrough=true
  shortcut: commandBracketPassThrough=true
  shortcut: commandArrowPassThrough=true
  shortcut: commandDeletePassThrough=true
  shortcut: commandControlQPassThrough=true
  shortcut: commandShift3PassThrough=true
  shortcut: commandShift4PassThrough=true
  shortcut: commandShift5PassThrough=true
  shortcut: commandOptionEscapePassThrough=true
  hostTextPolicy: appCommandcopyPassesThrough=true
  hostTextPolicy: appCommandpastePassesThrough=true
  hostTextPolicy: appCommandcutPassesThrough=true
  hostTextPolicy: appCommandundoPassesThrough=true
  hostTextPolicy: appCommandredoPassesThrough=true
  hostTextPolicy: appCommandselectAllPassesThrough=true
  hostTextPolicy: appCommandsaveDocumentPassesThrough=true
  hostTextPolicy: appCommandopenDocumentPassesThrough=true
  hostTextPolicy: appCommandperformClosePassesThrough=true
  hostTextPolicy: appCommandterminatePassesThrough=true
  hostTextPolicy: appCommandfindPassesThrough=true
  hostTextPolicy: appCommandprintPassesThrough=true
  hostTextPolicy: appCommandtoggleBoldPassesThrough=true
  hostTextPolicy: appCommandtoggleItalicPassesThrough=true
  hostTextPolicy: appCommandtoggleUnderlinePassesThrough=true
  hostTextPolicy: appCommandgoBackPassesThrough=true
  hostTextPolicy: appCommandreloadPassesThrough=true
  nonGuiVerificationPassed=true
```

当前结论：

- 复制/粘贴不是孤立修复；当前策略会放行所有包含 Command 的 keyDown，覆盖 Apple 官方常用快捷键大类。
- Inputia 仍只接管明确属于输入法自己的非 Command 快捷键，避免和系统/应用快捷键争抢。
- 真实 GUI smoke 仍等待系统安装版从 v40 更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：AppleScript 异常路径必须先清理再抛错

问题：

- GUI smoke 的 shell cleanup trap 已经会聚合失败并继续清理。
- 但真实 TextEdit/Safari/Clipboard smoke 的主要窗口/文档操作在 AppleScript 内部执行；如果 AppleScript 在输入、复制、粘贴、候选提交或断言阶段抛错，必须在 AppleScript 的 `on error` 分支里先关闭测试文档/窗口并恢复前台 App，再把错误抛回 shell。
- 之前 non-GUI 契约只检查了存在清理 helper、窗口 ID/docRef 捕获和 shell trap，没有显式锁住 AppleScript 异常路径里的清理顺序。

实现：

- `verify-nongui.sh`
  - 新增 `require_error_cleanup(...)` 静态契约。
  - 覆盖：
    - `smoke-textedit.sh` 的 `runCase`、`runShiftCase` 和顶层 `try/on error`。
    - `smoke-textedit-command-shortcuts.sh` 顶层 `try/on error`。
    - `smoke-clipboard-recall.sh` 顶层 `try/on error`。
    - `smoke-safari-typing.sh`、`smoke-safari-command-shortcuts.sh`、`smoke-safari-enter.sh` 顶层 `try/on error`。
  - TextEdit 类脚本的 error block 必须调用对应 `cleanupDoc(...)` 或 `cleanupTextEdit(...)`。
  - Safari 类脚本的 error block 必须调用 `closeSmokeWindow(smokeWindowId)` 和 `restoreFrontmost(previousBundleId)`。
  - error block 最后必须继续 rethrow，不能把失败吞掉。

选择/放弃：

- 选择：先用 non-GUI 静态契约锁住异常路径清理纪律；这不需要启动 TextEdit/Safari，也不改变当前 GUI gate。
- 选择：保持失败可见，清理后仍抛出原错误。
- 放弃：不在 AppleScript error 分支里强行忽略错误；真实 smoke 失败必须暴露。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 contract presence check
  errorCleanupContract=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  cleanupTrapStatusSelfCheck=true
  textEditUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI smoke 的成功路径、shell cleanup trap、AppleScript 异常路径三层清理纪律都已有 non-GUI 契约保护。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：TextEdit 主 smoke 正文读写绑定 docRef

问题：

- `smoke-textedit.sh` 每个 case 都会创建并捕获 `docRef`，也会用 `cleanupDoc(docRef)` 关闭测试文档。
- 但正文读写仍有多处使用 `front document`。
- 默认安全路径会拒绝已有 TextEdit；但在诊断时如果显式允许已有 TextEdit，`front document` 会让测试文档边界变弱，增加污染用户文档或读到错误文档的风险。

实现：

- `smoke-textedit.sh`
  - `runCase` 和 `runShiftCase` 的正文清空、清状态断言、结果读取全部改为 `docRef`。
  - 保留顶层 `cleanupTextEdit(...)` 用于关闭额外文档/退出脚本启动的 TextEdit。
- `verify-nongui.sh`
  - 增加静态契约，要求 TextEdit 主 smoke 使用：
    - `set text of docRef to ""`
    - `set clearedText to text of docRef`
    - `set resultText to text of docRef`
    - `set englishText to text of docRef`
    - `set chineseText to text of docRef`
  - 明确禁止回退到 `text of front document`。
  - 修正顶层 error cleanup 契约 marker，匹配实际 AppleScript 的 `my cleanupTextEdit(...)` 调用。

选择/放弃：

- 选择：把测试正文读写绑定到创建时捕获的文档引用，减少对 TextEdit 前台/窗口顺序的依赖。
- 放弃：不依赖 `front document` 作为“通常正确”的隐式目标；GUI smoke 的清理纪律需要更强的文档边界。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 docRef text target check
  textEditUsesDocRefForText=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

运行备注：

- 一次中间运行遇到陈旧 `/tmp/inputia-verify-nongui.lock`，owner pid 已不存在；随后由脚本既有 stale-lock recovery 正常重新获取锁并通过验证。

当前结论：

- TextEdit 主 smoke 现在用 `docRef` 作为正文读写边界，和它的文档关闭逻辑一致。
- non-GUI 回归仍证明 TIS 未 ready 时不会启动 TextEdit/Safari/Clipboard、不改变当前输入源、不留下临时文件。
- 系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：post-install TIS gate 不得改剪贴板

问题：

- `post-install-regression.sh` 在真实 UI smoke 路径会按顺序调用 TextEdit、Safari、Clipboard 相关 smoke，其中 command shortcut smoke 会临时写入系统剪贴板。
- TIS 未 ready 时，post-install UI gate 应该在启动任何真实 UI smoke 前退出。
- 之前 non-GUI gate 已验证该路径不改输入源、不改 debug env、不启动 TextEdit/Safari/Clipboard；但没有显式验证剪贴板不变。

实现：

- `verify-nongui.sh`
  - 在 `postInstallUiTisGate` 前后读取 `/usr/bin/pbpaste`。
  - 如果 TIS-not-ready gate 期间剪贴板变化，输出 `nonGuiVerificationPassed=false reason=post-install-ui-tis-gate-mutated-clipboard` 并失败。
  - 成功时输出 `postInstallUiTisGate.clipboardUnchanged=true`。

选择/放弃：

- 选择：把剪贴板纳入 post-install TIS gate 的“不污染用户状态”证明。
- 放弃：不只依赖下游 command smoke 自己的 `restore_clipboard`；TIS 未 ready 的路径不应该触达下游 smoke，也不应该改剪贴板。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  cleanupTrapStatusSelfCheck=true
  postInstallUiTisGate.rc=6
  postInstallUiTisGate.clipboardUnchanged=true
  postInstallUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUiTisGate.debugEnvBefore=unset
  postInstallUiTisGate.debugEnvAfter=unset
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- post-install UI TIS gate 现在同时证明：不启动目标 App、不改输入源、不改 debug env、不创建 user host、不改剪贴板。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：Clipboard recall 正文读写绑定 smokeDocument

问题：

- `smoke-clipboard-recall.sh` 已经创建并捕获 `smokeDocument`，cleanup 也会优先关闭这个测试文档。
- 但清状态断言和最终结果读取仍使用 `front document`。
- 如果诊断时显式允许已有 TextEdit，或 TextEdit 前台文档顺序被系统/用户动作改变，`front document` 会让 Clipboard recall smoke 读写边界变弱。

实现：

- `smoke-clipboard-recall.sh`
  - 清状态后读取 `clearedText` 改为 `text of smokeDocument`。
  - 选择剪贴板候选后读取 `resultText` 改为 `text of smokeDocument`。
  - 保留现有 `cleanupTextEdit(..., smokeDocument)`，使读写和清理使用同一个文档引用。
- `verify-nongui.sh`
  - 增加静态契约，要求 Clipboard recall smoke 使用：
    - `set text of smokeDocument to ""`
    - `set clearedText to text of smokeDocument`
    - `set resultText to text of smokeDocument`
  - 明确禁止回退到 `text of front document`。

选择/放弃：

- 选择：和 TextEdit 主 smoke 一样，把正文读写绑定到创建时捕获的文档引用。
- 放弃：不依赖 `front document` 作为隐式目标；Clipboard recall 本来就是状态污染敏感 smoke，应使用更强边界。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 smokeDocument text target check
  clipboardUsesSmokeDocumentForText=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard recall smoke 现在用 `smokeDocument` 作为正文读写边界，和它的文档关闭逻辑一致。
- non-GUI 回归仍证明 TIS 未 ready 时不会启动 TextEdit/Safari/Clipboard、不改变当前输入源、不改变剪贴板、不留下临时文件。
- 系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：写剪贴板 smoke 默认拒绝不可文本恢复的剪贴板

问题：

- TextEdit Command、Safari Command 和 Clipboard recall smoke 都会临时写系统剪贴板。
- 之前脚本用 `pbpaste` 保存、`pbcopy` 恢复，只能恢复纯文本。
- 如果用户当前剪贴板是图片、文件、PDF、富文本等非纯文本内容，真实 GUI smoke 即使最终恢复也会把剪贴板降级或丢失原格式。

实现：

- `smoke-common.sh`
  - 新增 `inputia_require_text_clipboard_restorable(ready_var, exit_code)`。
  - 默认读取 `clipboard info`，只有剪贴板可作为文本恢复且未检测到常见非文本类型时才允许继续。
  - 如果检测到 `RTF`、图片、PDF、file URL、alias 等非文本类型，输出：
    - `clipboardRestorable=false reason=non-text-clipboard ...`
    - `$ready_var=false reason=non-text-clipboard`
  - 如果没有文本表示，输出 `reason=missing-text-clipboard`。
  - 提供显式覆盖：`INPUTIA_ALLOW_NON_TEXT_CLIPBOARD_SMOKE=1`。
- `smoke-textedit-command-shortcuts.sh`
  - 在 `pbpaste` 保存和 `pbcopy` 清空剪贴板之前调用该门禁。
- `smoke-safari-command-shortcuts.sh`
  - 在 `pbpaste` 保存和 `pbcopy` 清空剪贴板之前调用该门禁。
- `smoke-clipboard-recall.sh`
  - 在 `pbpaste` 保存和写入 recall 测试文本之前调用该门禁。
- `verify-nongui.sh`
  - 静态契约要求共享 helper、覆盖变量和 `non-text-clipboard` marker 存在。
  - 静态契约要求三个写剪贴板 smoke 的调用顺序都是：
    - restore 函数定义
    - restore 调用注册
    - 文本可恢复门禁
    - `pbpaste` 保存
    - `pbcopy` 写入
    - `CLIPBOARD_CHANGED=1`

选择/放弃：

- 选择：默认保护用户剪贴板格式，遇到不可文本恢复的剪贴板直接拒跑真实写剪贴板 smoke。
- 放弃：不继续用纯文本 `pbcopy` 盲目恢复所有剪贴板类型；这会给用户造成不可见的数据降级。
- 放弃：不在本轮实现完整 NSPasteboard 多类型序列化/恢复；当前目标是 smoke 纪律，最小安全策略是拒跑并允许显式覆盖。

验证：

```text
/usr/bin/osascript -e 'clipboard info'
  «class utf8», ..., «class ut16», ..., string, ..., Unicode text, ...

bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

python3 clipboard gate order check
  textedit-commandClipboardRestorableOrder=True
  safari-commandClipboardRestorableOrder=True
  clipboard-recallClipboardRestorableOrder=True

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  cleanupTrapStatusSelfCheck=true
  shortcutPassThroughSelfChecks=true
  uiDisabledNoLaunchPassed=true
  textEditUiTisGateNoLaunchPassed=true
  textEditCommandUiTisGateNoLaunchPassed=true
  clipboardUiTisGateNoLaunchPassed=true
  safariPreExisting=false
  safariUiDisabledNoLaunchPassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 三个会写剪贴板的真实 GUI smoke 现在默认不会把用户的非文本剪贴板降级成纯文本。
- non-GUI 回归仍证明 TIS 未 ready 时不会启动 TextEdit/Safari/Clipboard、不改变当前输入源、不改变剪贴板、不留下临时文件。
- 系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：smoke-preflight gate 不得改输入源或剪贴板

问题：

- `smoke-preflight.sh` 是真实 TextEdit/Safari/Clipboard GUI smoke 的前置门禁。
- 之前 non-GUI 自检已经验证它不会改 debug env、不会创建 user host，但没有显式证明输入源和剪贴板不变。
- 当前 v40/v41 不一致和 UI disabled 路径都应该在启动真实 GUI smoke 前退出，因此也不应触碰用户当前输入源或剪贴板。

实现：

- `verify-nongui.sh`
  - `systemPreflight` gate 前后捕获当前输入源和剪贴板。
  - `buildPreflightUiDisabled` gate 前后捕获当前输入源和剪贴板。
  - 若剪贴板变化，分别输出：
    - `nonGuiVerificationPassed=false reason=system-preflight-mutated-clipboard`
    - `nonGuiVerificationPassed=false reason=build-preflight-ui-disabled-mutated-clipboard`
  - 成功时输出：
    - `systemPreflight.clipboardUnchanged=true`
    - `buildPreflightUiDisabled.clipboardUnchanged=true`
  - 输入源继续通过 `assert_current_source_unchanged` 验证，且 `unknown` 不能作为通过证据。

选择/放弃：

- 选择：把最外层 preflight gate 也纳入“不污染用户状态”证明，和具体 TextEdit/Safari/Clipboard smoke gate 保持同一标准。
- 放弃：不只依赖具体 smoke 脚本自身的 clipboard/input-source guard；preflight 作为入口也必须可独立证明安全。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  cleanupTrapStatusSelfCheck=true
  systemPreflight.rc=2
  systemPreflight.clipboardUnchanged=true
  systemPreflight.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  systemPreflight.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  systemPreflight.debugEnvBefore=unset
  systemPreflight.debugEnvAfter=unset
  buildPreflightUiDisabled.rc=4
  buildPreflightUiDisabled.clipboardUnchanged=true
  buildPreflightUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  buildPreflightUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  buildPreflightUiDisabled.debugEnvBefore=unset
  buildPreflightUiDisabled.debugEnvAfter=unset
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 最外层 smoke preflight gate 现在也证明不会改输入源、剪贴板、debug env 或 user host。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：写剪贴板 smoke 在输入源切换前拒绝不可安全恢复的剪贴板

问题：

- `smoke-textedit-command-shortcuts.sh`、`smoke-safari-command-shortcuts.sh`、`smoke-clipboard-recall.sh` 会写系统剪贴板。
- 上一轮已经加了“非文本/富文本/图片/文件剪贴板默认拒跑”的 gate，但 gate 仍位于 `inputia_select_input_source_or_exit` 之后。
- 如果用户当前剪贴板不可安全按文本恢复，这类失败路径应在任何 Inputia 输入源切换、debug host 重启、TextEdit/Safari 启动或剪贴板写入之前退出。

实现：

- 三个写剪贴板 smoke 都把 `inputia_require_text_clipboard_restorable` 前移到已有 GUI session 和目标 App idle preflight 之后、`inputia_select_input_source_or_exit` 之前：
  - `smoke-textedit-command-shortcuts.sh`
  - `smoke-safari-command-shortcuts.sh`
  - `smoke-clipboard-recall.sh`
- `verify-nongui.sh` 的静态契约同步收紧：
  - `clipboard-recall` 要求顺序为：`restore_clipboard` 定义 -> `inputia_require_text_clipboard_restorable` -> `inputia_select_input_source_or_exit` -> `pbpaste` 捕获 -> `pbcopy` 写入 -> `CLIPBOARD_CHANGED=1`。
  - TextEdit/Safari command smoke 要求顺序为：`restore_clipboard` 定义 -> `inputia_require_text_clipboard_restorable` -> cleanup 中的 `restore_clipboard` 聚合 -> `trap` -> `inputia_select_input_source_or_exit` -> `pbpaste` 捕获 -> `pbcopy` 写入 -> `CLIPBOARD_CHANGED=1` -> AppleScript 执行。

选择/放弃：

- 选择：在“不可安全恢复剪贴板”的失败路径上完全避免输入源切换，这比依赖 cleanup 再恢复输入源更稳。
- 选择：保留 `INPUTIA_ALLOW_NON_TEXT_CLIPBOARD_SMOKE=1` 作为显式人工覆盖；默认仍保护用户剪贴板。
- 放弃：不把 gate 放到脚本最开头，因为缺失 executable、cdhash mismatch、UI disabled、目标 App 已运行这些更早失败路径本来就不会写剪贴板或切换输入源，应保留原有更具体的失败原因。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  textEditCommandUiTisGate: clipboardRestorable=true
  textEditCommandUiTisGate: guiSmokeReady=false reason=input-source-not-selected
  textEditCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate: clipboardRestorable=true
  clipboardUiTisGate: guiSmokeReady=false reason=input-source-not-selected
  clipboardUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandUiDisabled.clipboardUnchanged=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 三个写剪贴板 smoke 现在会在输入源切换前拒绝不可安全恢复的剪贴板。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：剪贴板可恢复分类逻辑加入 non-GUI 自检

问题：

- `inputia_require_text_clipboard_restorable` 已经保护真实 GUI smoke 不把用户的非文本剪贴板降级成纯文本。
- 但分类逻辑之前直接内联在 runtime gate 中，`verify-nongui.sh` 只检查 marker 和调用顺序，不能证明 RTF、图片、PDF、file URL、空剪贴板信息等具体分支仍按预期拒跑。

实现：

- `smoke-common.sh`
  - 新增纯分类 helper：`inputia_clipboard_info_restorable_reason(info)`。
  - 返回：
    - `text-restorable`，rc=0：纯文本/可按文本安全恢复。
    - `non-text-clipboard`，rc=1：含 RTF、图片、PDF、file URL、alias 等非文本表示，即使同时有文本表示也拒跑。
    - `missing-text-clipboard`，rc=2：没有可恢复的文本表示。
  - `inputia_require_text_clipboard_restorable` 改为调用该 helper，保留原有输出和退出语义。
- `verify-nongui.sh`
  - 静态契约要求 classifier helper 和三个分类 marker 存在。
  - 新增 `clipboard info classifier self-check`，用模拟 `clipboard info` 字符串验证分类结果和返回码。

选择/放弃：

- 选择：用纯字符串 helper 做 self-check，不改真实系统剪贴板，不启动 GUI。
- 选择：把“RTF+文本”“图片+文本”“PDF+文本”“file URL+文本”都判为 `non-text-clipboard`，因为 smoke 的恢复路径只能用 `pbcopy` 恢复纯文本，会丢失原始非文本/富文本 payload。
- 放弃：不在 self-check 中实际写入系统剪贴板制造图片/RTF 样本；这会违背当前 smoke 清理纪律的目标。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardClassifierCase=utf8-text reason=text-restorable rc=0
  clipboardClassifierCase=plain-string reason=text-restorable rc=0
  clipboardClassifierCase=rtf-plus-text reason=non-text-clipboard rc=1
  clipboardClassifierCase=tiff-plus-text reason=non-text-clipboard rc=1
  clipboardClassifierCase=pdf-plus-text reason=non-text-clipboard rc=1
  clipboardClassifierCase=file-url-plus-text reason=non-text-clipboard rc=1
  clipboardClassifierCase=empty-info reason=missing-text-clipboard rc=2
  clipboardClassifierCase=image-only reason=missing-text-clipboard rc=2
  clipboardInfoClassifierSelfCheck=true
  textEditCommandUiTisGate: clipboardRestorable=true
  clipboardUiTisGate: clipboardRestorable=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 剪贴板 gate 的分类逻辑现在有独立 non-GUI 自检覆盖，不依赖真实剪贴板内容或 GUI smoke。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新并通过 TIS readiness。

## v41 Mac mini 续修：真实 smoke 脚本验证非文本剪贴板 gate 早退

问题：

- 上一轮已经验证 `inputia_clipboard_info_restorable_reason` 的纯分类逻辑。
- 但还缺一个更接近真实路径的证明：三个会写剪贴板的 smoke 脚本在遇到非文本/富文本剪贴板时，是否真的会在输入源切换、debug host 重启、目标 App 启动、剪贴板写入之前退出。
- 初次加入真实脚本 gate 验证时暴露了一个实际 bug：`set -e` 下 `reason="$(inputia_clipboard_info_restorable_reason ...)"` 会在 helper 返回 1/2 时让脚本提前以 rc=1 退出，绕过预期的 `$ready_var=false reason=non-text-clipboard` 输出和脚本定义的退出码。

实现：

- `smoke-common.sh`
  - 新增 `inputia_current_clipboard_info()`。
  - 支持测试专用 `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST`，用于 non-GUI verifier 注入模拟 `clipboard info`；正常运行仍调用 `/usr/bin/osascript -e 'clipboard info'`。
  - 修复 `set -e`：调用 `inputia_clipboard_info_restorable_reason` 时显式 `set +e` 捕获返回码，再恢复 `set -e`，确保非文本/缺失文本路径能输出规范 marker 并按传入 exit code 退出。
- `verify-nongui.sh`
  - 静态契约要求 `inputia_current_clipboard_info()` 和 `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST` 存在。
  - 新增真实脚本 gate 验证：
    - `textEditCommandNonTextClipboardGate`：注入 RTF+文本，期望 rc=18。
    - `clipboardNonTextGate`：注入 TIFF+文本，期望 rc=9。
    - `safariCommandNonTextClipboardGate`：注入 PDF+文本，期望 rc=15；仅在 Safari 未预先运行时验证。
  - 每个 gate 后都验证当前输入源、debug env、user host、TextEdit/Safari、osascript、InputiaInputMethod 没有被污染或残留。

选择/放弃：

- 选择：用 `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST` 注入 `clipboard info`，不写真实系统剪贴板。
- 选择：真实调用三个 smoke 脚本，而不是只调用 helper，覆盖 shell `set -e`、ready marker、脚本退出码和早退顺序。
- 放弃：不在 Safari 已运行时强行验证 Safari command 非文本 gate；这种情况下脚本应先按 `safari-already-running` 拒跑，避免干扰用户 Safari。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/verify-nongui.sh \
  macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardInfoClassifierSelfCheck=true
  textEditCommandNonTextClipboardGate: clipboardRestorable=false reason=non-text-clipboard info=«class RTF », 512, string, 11, Unicode text, 22
  textEditCommandNonTextClipboardGate: textEditCommandShortcutSmokeReady=false reason=non-text-clipboard
  textEditCommandNonTextClipboardGate.rc=18
  textEditCommandNonTextClipboardGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandNonTextClipboardGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandNonTextClipboardGate.debugEnvBefore=unset
  textEditCommandNonTextClipboardGate.debugEnvAfter=unset
  textEditCommandNonTextClipboardGatePassed=true
  clipboardNonTextGate: clipboardRestorable=false reason=non-text-clipboard info=TIFF picture, 2048, string, 11, Unicode text, 22
  clipboardNonTextGate: clipboardRecallSmokeReady=false reason=non-text-clipboard
  clipboardNonTextGate.rc=9
  clipboardNonTextGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardNonTextGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardNonTextGate.debugEnvBefore=unset
  clipboardNonTextGate.debugEnvAfter=unset
  clipboardNonTextGatePassed=true
  safariCommandNonTextClipboardGate: clipboardRestorable=false reason=non-text-clipboard info=PDF , 2048, string, 11, Unicode text, 22
  safariCommandNonTextClipboardGate: safariCommandShortcutSmokeReady=false reason=non-text-clipboard
  safariCommandNonTextClipboardGate.rc=15
  safariCommandNonTextClipboardGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandNonTextClipboardGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandNonTextClipboardGate.debugEnvBefore=unset
  safariCommandNonTextClipboardGate.debugEnvAfter=unset
  safariCommandNonTextClipboardGatePassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 三个写剪贴板 smoke 的非文本剪贴板保护现在不仅有 helper 自检，也有真实脚本早退路径验证。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新并通过 TIS readiness。

## v41 Mac mini 续修：剪贴板测试 override 增加显式开关

问题：

- `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST` 是给 `verify-nongui.sh` 注入模拟 `clipboard info` 用的测试入口。
- 如果真实 GUI smoke 环境里意外带上该变量，脚本会用模拟值替代真实系统剪贴板信息，削弱“保护用户剪贴板”的门禁。

实现：

- `smoke-common.sh`
  - `inputia_current_clipboard_info()` 现在只有在同时满足以下两个条件时才使用测试 override：
    - `INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1`
    - `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST` 已设置
  - 只设置 `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST` 时会继续读取真实 `/usr/bin/osascript -e 'clipboard info'`。
- `verify-nongui.sh`
  - 静态契约要求 `INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST` 存在。
  - 新增 `clipboard info override gate self-check`：
    - 只设置 override 不开开关时，确认 override 被忽略。
    - 同时设置开关和 override 时，确认 provider 返回测试值。
  - 三个真实脚本非文本剪贴板 gate 验证都显式设置双开关。

选择/放弃：

- 选择：保留测试注入能力，但必须双开关启用，避免普通运行环境误触发。
- 放弃：不删除测试 override；否则 verifier 只能靠真实系统剪贴板状态覆盖非文本 gate，既不稳定也会污染用户剪贴板。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardInfoClassifierSelfCheck=true
  clipboardInfoOverrideDisabledIgnored=true
  clipboardInfoOverrideEnabledValue=override-with-gate
  clipboardInfoOverrideGateSelfCheck=true
  textEditCommandNonTextClipboardGatePassed=true
  clipboardNonTextGatePassed=true
  safariCommandNonTextClipboardGatePassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 剪贴板测试 override 不会因单个环境变量在真实 smoke 中误触发。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新并通过 TIS readiness。

## v41 Mac mini 续修：真实 smoke 脚本验证缺失文本剪贴板 gate 早退

问题：

- 已验证三类写剪贴板 smoke 对“非文本 + 文本表示”的剪贴板会早退。
- 但“完全没有可恢复文本表示”的剪贴板同样危险，例如图片-only 或空 `clipboard info`。这类路径之前只在 helper classifier 自检中覆盖，没有通过真实 smoke 脚本验证。

实现：

- `verify-nongui.sh`
  - 增加三个真实 smoke 脚本的 `missing-text-clipboard` gate 验证：
    - `textEditCommandMissingTextClipboardGate`：注入 `JPEG picture, 2048`，期望 rc=18。
    - `clipboardMissingTextGate`：注入空 `clipboard info`，期望 rc=9。
    - `safariCommandMissingTextClipboardGate`：注入 `JPEG picture, 2048`，期望 rc=15；仅在 Safari 未预先运行时验证。
  - 每个 gate 后继续验证当前输入源、debug env、user host、TextEdit/Safari、osascript、InputiaInputMethod 均未被污染或残留。

选择/放弃：

- 选择：复用 `INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1` + `INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=...`，避免写真实系统剪贴板。
- 选择：真实调用三个 smoke 脚本，覆盖脚本级 ready marker、退出码、preflight 顺序和 cleanup 行为。
- 放弃：不通过实际设置图片剪贴板来造样本；这会影响用户剪贴板，并且与当前 smoke 清理纪律目标冲突。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardInfoClassifierSelfCheck=true
  clipboardInfoOverrideGateSelfCheck=true
  textEditCommandMissingTextClipboardGate: clipboardRestorable=false reason=missing-text-clipboard info=JPEG picture, 2048
  textEditCommandMissingTextClipboardGate: textEditCommandShortcutSmokeReady=false reason=missing-text-clipboard
  textEditCommandMissingTextClipboardGate.rc=18
  textEditCommandMissingTextClipboardGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandMissingTextClipboardGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandMissingTextClipboardGate.debugEnvBefore=unset
  textEditCommandMissingTextClipboardGate.debugEnvAfter=unset
  textEditCommandMissingTextClipboardGatePassed=true
  clipboardMissingTextGate: clipboardRestorable=false reason=missing-text-clipboard info=unknown
  clipboardMissingTextGate: clipboardRecallSmokeReady=false reason=missing-text-clipboard
  clipboardMissingTextGate.rc=9
  clipboardMissingTextGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardMissingTextGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardMissingTextGate.debugEnvBefore=unset
  clipboardMissingTextGate.debugEnvAfter=unset
  clipboardMissingTextGatePassed=true
  safariCommandMissingTextClipboardGate: clipboardRestorable=false reason=missing-text-clipboard info=JPEG picture, 2048
  safariCommandMissingTextClipboardGate: safariCommandShortcutSmokeReady=false reason=missing-text-clipboard
  safariCommandMissingTextClipboardGate.rc=15
  safariCommandMissingTextClipboardGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandMissingTextClipboardGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandMissingTextClipboardGate.debugEnvBefore=unset
  safariCommandMissingTextClipboardGate.debugEnvAfter=unset
  safariCommandMissingTextClipboardGatePassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 三个写剪贴板 smoke 现在同时覆盖 `non-text-clipboard` 和 `missing-text-clipboard` 的真实脚本早退路径。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新并通过 TIS readiness。

## v41 Mac mini 续修：安装等待与无管理员安装路径不得改剪贴板

问题：

- `await-system-install.sh` 会轮询系统安装状态，并在满足条件时进入 post-install regression；当前 v40/v41 不一致时只应超时退出。
- `install-system.sh` 在 `INPUTIA_INSTALL_NO_ADMIN_PROMPT=1` 且无管理员权限时只应报告 `admin-required`。
- 之前 non-GUI 自检已验证这两条路径不改输入源、debug env 或 user host，但没有显式证明剪贴板不变。

实现：

- `verify-nongui.sh`
  - `awaitShort` 前后捕获 `/usr/bin/pbpaste`，变化时输出 `nonGuiVerificationPassed=false reason=await-short-mutated-clipboard`。
  - `installNoPrompt` 前后捕获 `/usr/bin/pbpaste`，变化时输出 `nonGuiVerificationPassed=false reason=install-no-prompt-mutated-clipboard`。
  - 成功时分别输出：
    - `awaitShort.clipboardUnchanged=true`
    - `installNoPrompt.clipboardUnchanged=true`

选择/放弃：

- 选择：把安装等待和 no-prompt 安装失败路径纳入和 smoke gate 相同的用户状态保护标准。
- 放弃：不把“这些脚本理论上不碰剪贴板”当作证据；non-GUI 自检必须直接证明前后状态不变。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks=true
  cleanupTrapStatusSelfCheck=true
  systemPreflight.clipboardUnchanged=true
  buildPreflightUiDisabled.clipboardUnchanged=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort.rc=2
  awaitShort.clipboardUnchanged=true
  awaitShort.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  awaitShort.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  awaitShort.debugEnvBefore=unset
  awaitShort.debugEnvAfter=unset
  installNoPrompt.rc=12
  installNoPrompt.clipboardUnchanged=true
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.debugEnvBefore=unset
  installNoPrompt.debugEnvAfter=unset
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 安装等待超时路径和无管理员 no-prompt 安装路径现在也证明不会改输入源、剪贴板、debug env 或 user host。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：Command 常用系统快捷键泛化透传

问题：

- 用户发现 `Command-C` / `Command-V` 在 Inputia 下不可用，说明输入法不能只点修单个复制粘贴组合。
- Apple 官方 Mac keyboard shortcuts 清单把 `Command-X/C/V/Z/A/F/G/H/M/O/P/Q/S/T/W`、`Command-Tab`、`Command-Space`、`Command-Shift-3/4/5`、`Command-Option-Escape` 等列为系统或常用 App 快捷键；输入法不能接管这类组合。
- Inputia 自己的剪贴板召回是 `Ctrl-Shift-V`，必须继续拒绝任何带 `Command` 的变体，避免和系统/App 粘贴变体冲突。

实现：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut` 维持按修饰键泛化：只要 modifiers 包含 `.command` 就透传。
- `InputiaInputController.handleKeyDown` 已在处理剪贴板召回、标点切换、候选导航前调用该 classifier，因此带 `Command` 的 keyDown 会先返回 `false` 交还系统/App。
- `Tools/InputiaShortcutSelfCheck.swift`
  - 新增 `commonAppleCommandShortcutSetPassesThrough=true`，覆盖 Apple 常见 Command 快捷键集合。
  - 新增 `anyCommandModifiedKeyPassesThrough=true`，用代表性字母、数字、空格、Tab、Delete、Escape、Return、Page、方向键 keyCode 组合证明不是 C/V 特判。
- `InputiaInputMethod --host-shortcut-self-check`
  - 同步输出上述两个 marker，系统安装版诊断也能证明泛化策略。
- `verify-nongui.sh`
  - 强制检查 `commonAppleCommandShortcutSetPassesThrough=true` 和 `anyCommandModifiedKeyPassesThrough=true`。

选择/放弃：

- 选择：把策略固定为“任何带 Command 的 keyDown 先透传”，再用常用快捷键集合和代表性 keyCode 双重自检防回退。
- 放弃：继续逐个追加 `Command-C`、`Command-V` 特判；这种做法会漏掉保存、关闭、查找、切换 App、截图、强退等同类系统快捷键。

验证：

```text
/usr/bin/swiftc Tools/InputiaShortcutSelfCheck.swift ... -o /tmp/inputia-shortcut-self-check
/tmp/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build.sh
  buildVersion=41
  buildCDHash=029c2d2fd0f1f358400593fcc61c08652d0d33fe
  codesign verify passed for InputiaInputMethod.app and Inputia 设置.app

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build-pkg.sh
  pkgVerificationPassed=true
  archiveAppCDHash=029c2d2fd0f1f358400593fcc61c08652d0d33fe
  buildAppCDHash=029c2d2fd0f1f358400593fcc61c08652d0d33fe
  pkg=/Users/minizl/services/Handy/macos/InputiaInputMethod/dist/InputiaInputMethod-v41-029c2d2fd0f1.pkg

bash macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true
  shortcutPassThroughSelfChecks=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  pkgVerificationPassed=true
  postInstallTISBlockReason=missing-enabled-source
  postInstallUiSmokeReady=false reason=tis-not-ready
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort.rc=2
  awaitShort.clipboardUnchanged=true
  installNoPrompt.rc=12
  installNoPrompt.clipboardUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前 build 已证明常见 Apple Command 快捷键集合透传，且代表性 keyCode 的任意 Command 组合透传；这覆盖 `Command-C` / `Command-V` 以及同类系统/App 快捷键。
- `Ctrl-Shift-V` 仍保留为 Inputia 剪贴板召回；`Ctrl-Shift-Command-V` 被拒绝，避免接管系统/App 粘贴变体。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：non-GUI gate 直接检查 build host 快捷键诊断

问题：

- `verify-nongui.sh` 已检查独立 `inputia-shortcut-self-check`，而 build host 的 `--host-shortcut-self-check` 主要通过 `post-install-regression.sh` 间接出现在输出里。
- Command 透传是输入法主机进程行为，non-GUI gate 应直接证明 build app 自身也输出同一批泛化 marker。

实现：

- `verify-nongui.sh`
  - 在 shortcut pass-through self-checks 中要求 `$BUILD_APP/Contents/MacOS/InputiaInputMethod` 可执行。
  - 直接运行 `InputiaInputMethod --host-shortcut-self-check`。
  - 强制检查：
    - `hostShortcutSelfCheck=true`
    - `commonAppleCommandShortcutSetPassesThrough=true`
    - `anyCommandModifiedKeyPassesThrough=true`
    - `commandCPassThrough=true`
    - `commandVPassThrough=true`
    - `ctrlShiftVClipboardRecall=true`
    - `ctrlShiftCommandVRejected=true`

选择/放弃：

- 选择：让 non-GUI gate 同时覆盖独立 classifier 工具和 build host diagnostic。
- 放弃：只依赖 post-install 输出里的 host diagnostic；那条路径是间接证明，不适合作为快捷键策略的唯一 gate。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

bash macos/InputiaInputMethod/verify-nongui.sh
  shortcut: shortcutSelfCheck=true
  shortcut: commonAppleCommandShortcutSetPassesThrough=true
  shortcut: anyCommandModifiedKeyPassesThrough=true
  hostShortcut: hostShortcutSelfCheck=true
  hostShortcut: commonAppleCommandShortcutSetPassesThrough=true
  hostShortcut: anyCommandModifiedKeyPassesThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  hostShortcut: ctrlShiftVClipboardRecall=true
  hostShortcut: ctrlShiftCommandVRejected=true
  pkgVerificationPassed=true
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort.rc=2
  awaitShort.clipboardUnchanged=true
  installNoPrompt.rc=12
  installNoPrompt.clipboardUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- non-GUI gate 现在会直接失败在 build host 快捷键诊断缺失或退化时。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：快捷键诊断本身不得污染用户状态

问题：

- 前一轮把 Command 快捷键泛化透传固定到独立 self-check 和 build host diagnostic。
- 这些 diagnostic 属于 non-GUI gate，会在用户当前输入源与剪贴板状态下运行；它们不应改剪贴板、当前输入源、debug env 或创建 user host。

实现：

- `verify-nongui.sh`
  - 在 shortcut pass-through self-checks 段运行 `inputia-shortcut-self-check`、`InputiaInputMethod --host-shortcut-self-check`、`inputia-host-text-policy-self-check` 前后捕获：
    - `/usr/bin/pbpaste`
    - 当前输入源 id
    - `INPUTIA_DEBUG_EVENTS`
  - 如果剪贴板变化，输出 `nonGuiVerificationPassed=false reason=shortcut-checks-mutated-clipboard`。
  - 成功时输出：
    - `shortcutPassThroughSelfChecks.clipboardUnchanged=true`
    - `shortcutPassThroughSelfChecks.currentSourceBefore=...`
    - `shortcutPassThroughSelfChecks.currentSourceAfter=...`
    - `shortcutPassThroughSelfChecks.debugEnvBefore=unset`
    - `shortcutPassThroughSelfChecks.debugEnvAfter=unset`
    - `shortcutPassThroughSelfChecks.userHost=false`

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  shortcut: shortcutSelfCheck=true
  shortcut: commonAppleCommandShortcutSetPassesThrough=true
  shortcut: anyCommandModifiedKeyPassesThrough=true
  hostShortcut: hostShortcutSelfCheck=true
  hostShortcut: commonAppleCommandShortcutSetPassesThrough=true
  hostShortcut: anyCommandModifiedKeyPassesThrough=true
  hostTextPolicy: hostTextPolicySelfCheck=true
  shortcutPassThroughSelfChecks.clipboardUnchanged=true
  shortcutPassThroughSelfChecks.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  shortcutPassThroughSelfChecks.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  shortcutPassThroughSelfChecks.debugEnvBefore=unset
  shortcutPassThroughSelfChecks.debugEnvAfter=unset
  shortcutPassThroughSelfChecks.userHost=false
  shortcutPassThroughSelfChecks=true
  pkgVerificationPassed=true
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort.rc=2
  awaitShort.clipboardUnchanged=true
  installNoPrompt.rc=12
  installNoPrompt.clipboardUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- non-GUI shortcut diagnostic 现在不仅证明 Command 透传策略正确，也证明这批诊断不会污染用户剪贴板、输入源、debug env 或 user host。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：GUI smoke 超时清理覆盖子进程组

问题：

- TextEdit/Safari/Clipboard GUI smoke 的 AppleScript 路径依赖 `inputia_run_with_timeout` 防止脚本卡死。
- 旧 helper 超时后只杀直接子进程；如果被测命令再启动子进程，事后 residue gate 可以发现残留，但 timeout helper 本身没有负责清掉整个被测进程组。
- 这会让失败路径更容易留下 `osascript`、Shell 子进程或后续 GUI 相关进程，和“失败也自动清理”的目标不一致。

实现：

- `smoke-common.sh`
  - `inputia_run_with_timeout` 在 `/usr/bin/python3` 可用时通过 `subprocess.Popen(..., preexec_fn=os.setsid)` 让被测命令进入独立进程组。
  - 超时先对进程组 `SIGTERM`，1 秒后仍未退出再对进程组 `SIGKILL`。
  - 保留原 shell 实现作为 Python 不可用时的 fallback。
- `verify-nongui.sh`
  - cleanup contract 强制检查 `preexec_fn=os.setsid`、`os.killpg(... SIGTERM)` 和 `os.killpg(... SIGKILL)`。
  - timeout helper self-check 新增子进程组用例：
    - 被测命令启动后台 `sleep` 后等待。
    - helper 超时后必须输出 `inputiaSmokeTimeout=timeout-child-process-group-check seconds=1`。
    - 验证后台 `sleep` pid 不再存在。

选择/放弃：

- 选择：用 macOS 自带 `/usr/bin/python3` 做最小进程组包装，保持现有 shell API 不变。
- 放弃：依赖 `setsid` 命令；这台 Mac mini 当前没有可用的 `setsid` 可执行文件。
- 放弃：只靠 residue gate 发现残留；那是末端报警，不是 timeout 根因清理。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

source macos/InputiaInputMethod/smoke-common.sh
inputia_run_with_timeout timeout-child-process-group-check 1 /bin/sh -c 'sleep 20 & echo timeoutChildPid=$!; wait'
  timeoutChildPid=80975
  inputiaSmokeTimeout=timeout-child-process-group-check seconds=1
  rc=143
  childStillRunning=false

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  timeoutChildProcessGroupCheck: timeoutChildPid=86610
  timeoutChildProcessGroupCheck: inputiaSmokeTimeout=timeout-child-process-group-check seconds=1
  timeoutChildProcessGroupCheck.rc=143
  timeoutChildProcessGroupCleaned=true
  timeoutHelperSelfCheck=true
  pkgVerificationPassed=true
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI smoke 的超时失败路径现在会清理被测命令的整个进程组，而不是只杀直接子进程。
- 这一步仍未硬跑 TextEdit/Safari GUI smoke；当前系统安装版仍是 v40，build/pkg 是 v41，真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：UI-disabled smoke gate 也必须证明状态不污染

问题：

- `INPUTIA_RUN_UI_SMOKE` 未开启时，TextEdit/Safari/Clipboard smoke 应在第一道门直接退出，不能打开 GUI App。
- 前一轮 gate 已证明不会启动 TextEdit/Safari、不会留下 `osascript`/host、不会污染 debug env；但部分 UI-disabled 分支没有统一证明当前输入源和剪贴板不变。
- 这些分支是防误跑 GUI smoke 的最早保护层，也应纳入 IME 状态污染检查。

实现：

- `verify-nongui.sh`
  - `textEditUiDisabled`、`textEditCommandUiDisabled`、`clipboardUiDisabled` 现在都会捕获并断言：
    - `/usr/bin/pbpaste` 前后不变。
    - 当前输入源 id 前后不变。
    - `INPUTIA_DEBUG_EVENTS` 前后不变。
    - 不创建 user host。
  - `safariTypingUiDisabled`、`safariCommandUiDisabled`、`safariEnterUiDisabled`、`safariDiagnoseUiDisabled` 也加入同样的剪贴板、输入源、debug env、user host 检查。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  textEditUiDisabled.clipboardUnchanged=true
  textEditUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiDisabled.clipboardUnchanged=true
  textEditCommandUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardUiDisabled.clipboardUnchanged=true
  clipboardUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariTypingUiDisabled.clipboardUnchanged=true
  safariTypingUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandUiDisabled.clipboardUnchanged=true
  safariCommandUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterUiDisabled.clipboardUnchanged=true
  safariEnterUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseUiDisabled.clipboardUnchanged=true
  safariDiagnoseUiDisabled.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseUiDisabled.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- UI-disabled/no-launch 分支现在不仅证明不会打开 GUI App，也证明不会污染剪贴板、当前输入源、debug env 或 user host。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：Clipboard recall 写剪贴板顺序纳入静态契约

问题：

- `smoke-clipboard-recall.sh` 会选择输入源、设置 debug event log、写入测试剪贴板，并启动 TextEdit。
- 前序修复已经把不可恢复剪贴板 gate 移到输入源切换前，但 verifier 只对部分写剪贴板脚本做了完整顺序约束；Clipboard recall 自身缺少 trap/select/write/osascript 的同等顺序断言。
- 如果后续维护把 `inputia_require_text_clipboard_restorable` 移到 select 或 `pbcopy` 之后，non-text/missing-text 失败路径可能再次污染输入源或用户剪贴板。

实现：

- `verify-nongui.sh`
  - 在 `smoke-clipboard-recall.sh` 静态契约中新增：
    - `clipboard-missing-clipboard-restore-function`
    - `clipboard-missing-clipboard-restore-call`
    - `clipboard-missing-cleanup-trap-after-restore`
    - `clipboard-changed-after-osascript`
  - 顺序要求扩展为：
    - restore 函数定义
    - 条件恢复逻辑
    - `inputia_require_text_clipboard_restorable`
    - cleanup 内 `restore_clipboard` 调用
    - `trap cleanup_smoke EXIT`
    - `inputia_select_input_source_or_exit`
    - 原始剪贴板捕获
    - 测试剪贴板写入
    - `CLIPBOARD_CHANGED=1`
    - `inputia_run_with_timeout clipboard-recall-osascript`

选择与放弃：

- 选择静态契约先锁住副作用顺序，因为当前系统安装版仍是 v40，build/pkg 是 v41，真实 GUI smoke 会被 TIS readiness 阻止。
- 暂不硬跑 TextEdit GUI recall；在 `target-cdhash-mismatch` / `missing-enabled-source` 状态下硬跑没有新的功能证明，只会增加抢焦点和窗口残留风险。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  shortcutPassThroughSelfChecks.clipboardUnchanged=true
  clipboardInfoClassifierSelfCheck=true
  clipboardInfoOverrideGateSelfCheck=true
  textEditCommandNonTextClipboardGatePassed=true
  textEditCommandMissingTextClipboardGatePassed=true
  clipboardNonTextGatePassed=true
  clipboardMissingTextGatePassed=true
  safariCommandNonTextClipboardGatePassed=true
  safariCommandMissingTextClipboardGatePassed=true
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard recall 的不可恢复剪贴板 gate 现在被 verifier 明确约束在输入源选择、剪贴板写入和 GUI osascript 前。
- 三个写剪贴板 smoke 的 non-text/missing-text 早退路径继续通过，且不改变当前输入源、debug env、user host 或临时文件状态。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：TextEdit/Safari cleanup helper 限定为本次 smoke 启动的 App

问题：

- GUI smoke 的 shell trap 需要在 osascript 超时或失败时兜底清理 TextEdit/Safari。
- 但清理 helper 如果未来被改成只看 `CLEANUP_ALLOWED`，就可能误关用户本来已经打开的 TextEdit/Safari。
- 之前 verifier 只检查 cleanup gate 字符串存在，没有证明 gate 位于具体 cleanup helper 内部。

实现：

- `verify-nongui.sh`
  - 在 `cleanupPermissionContract` 中提取 `inputia_cleanup_textedit_if_started()` 与 `inputia_cleanup_safari_if_started()` 函数块。
  - 对 TextEdit cleanup helper 增加函数块级断言：
    - `INPUTIA_TEXTEDIT_PREFLIGHT` 必须是 `not-running`
    - `INPUTIA_TEXTEDIT_CLEANUP_ALLOWED` 必须为 `1`
    - 必须使用 `tell application "TextEdit" to quit saving no`
    - 必须等待 `inputia_wait_process_exit TextEdit`
    - 必须在仍运行时输出 `textEditCleanupFailed=process-still-running`
  - 对 Safari cleanup helper 增加函数块级断言：
    - `INPUTIA_SAFARI_PREFLIGHT` 必须是 `not-running`
    - `INPUTIA_SAFARI_CLEANUP_ALLOWED` 必须为 `1`
    - 必须使用 `tell application "Safari" to quit`
    - 必须等待 `inputia_wait_process_exit Safari`
    - 必须在仍运行时输出 `safariCleanupFailed=process-still-running`

选择与放弃：

- 选择静态函数块契约，因为当前系统安装版仍是 v40，真实 GUI smoke 被 TIS readiness 阻止。
- 不引入强制 `killall TextEdit/Safari` fallback；当前纪律优先保证不误关用户已有窗口，真实 GUI 阶段再用实际残留结果判断是否需要更强的兜底。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  textEditUiDisabled.clipboardUnchanged=true
  textEditCommandUiDisabled.clipboardUnchanged=true
  clipboardUiDisabled.clipboardUnchanged=true
  safariTypingUiDisabled.clipboardUnchanged=true
  safariCommandUiDisabled.clipboardUnchanged=true
  safariEnterUiDisabled.clipboardUnchanged=true
  safariDiagnoseUiDisabled.clipboardUnchanged=true
  textEditCommandNonTextClipboardGatePassed=true
  textEditCommandMissingTextClipboardGatePassed=true
  clipboardNonTextGatePassed=true
  clipboardMissingTextGatePassed=true
  safariCommandNonTextClipboardGatePassed=true
  safariCommandMissingTextClipboardGatePassed=true
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort: systemInstallTargetMatchesBuild=false
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

备注：

- 本轮第一次验证期间观察到并发 `verify-nongui`/`post-install-regression`/`verify-system` 仍在运行，出现 transient `unexpected EOF` 输出；等待后台验证自然退出后，从干净状态重跑完整 `verify-nongui.sh` 通过。

当前结论：

- TextEdit/Safari shell 兜底清理现在被 verifier 明确限定为“preflight 时 App 未运行，且本次 smoke 已允许清理”。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：TIS readiness 诊断本身必须只读

问题：

- `tis-readiness.sh` 是 GUI smoke 前的关键诊断，用来判断 build app、TIS enabled/installed sources、当前输入源和 icon 是否匹配。
- 这个诊断会调用 `inputia-tis-tool --dump` 和当前输入源读取逻辑；虽然预期只读，但之前 `verify-nongui.sh` 只打印输出，没有断言它不污染剪贴板、当前输入源、debug env 或 user host。
- 如果诊断脚本未来误用 select/register 路径，可能在真正 smoke 前就改变用户输入源。

实现：

- `verify-nongui.sh`
  - 在 `run_and_prefix "tisReadinessBuild: " "$ROOT_DIR/tis-readiness.sh" "$BUILD_APP"` 前后捕获：
    - `/usr/bin/pbpaste`
    - 当前输入源 id
    - `INPUTIA_DEBUG_EVENTS`
  - 诊断后强制输出/断言：
    - `tisReadinessBuild.clipboardUnchanged=true`
    - `tisReadinessBuild.currentSourceBefore=...`
    - `tisReadinessBuild.currentSourceAfter=...`
    - `tisReadinessBuild.debugEnvBefore=unset`
    - `tisReadinessBuild.debugEnvAfter=unset`
    - `tisReadinessBuild.userHost=false`

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  tisReadinessBuild: app=/Users/minizl/services/Handy/macos/InputiaInputMethod/build/InputiaInputMethod.app
  tisReadinessBuild: appMatchesBuild=true
  tisReadinessBuild: tis.readinessBlockReason=missing-enabled-source
  tisReadinessBuild: tisReadiness=false
  tisReadinessBuild.clipboardUnchanged=true
  tisReadinessBuild.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  tisReadinessBuild.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  tisReadinessBuild.debugEnvBefore=unset
  tisReadinessBuild.debugEnvAfter=unset
  tisReadinessBuild.userHost=false
  systemPreflight.clipboardUnchanged=true
  buildPreflightUiDisabled.clipboardUnchanged=true
  postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort.rc=2
  installNoPrompt.rc=12
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TIS readiness 诊断现在被 non-GUI gate 明确约束为只读，不允许它改变剪贴板、当前输入源、debug env 或 user host。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：早退 gate 补齐剪贴板不变性断言

问题：

- 多数 UI-disabled/TIS/not-restorable 早退 gate 已验证当前输入源、debug env、user host 和目标 App 残留。
- 但部分 gate 没有显式检查 `/usr/bin/pbpaste` 前后是否一致，尤其是：
  - `textEditUiTisGate`
  - TextEdit command 的 `non-text-clipboard` / `missing-text-clipboard`
  - Clipboard recall 的 `non-text-clipboard` / `missing-text-clipboard`
  - Safari command 的 `non-text-clipboard` / `missing-text-clipboard`
  - Safari 已运行时的 existing-app gate
- 这些路径本应在真正写测试剪贴板前退出；缺少剪贴板断言会让后续回归更难定位。

实现：

- `verify-nongui.sh`
  - 为上述早退 gate 补充 `/usr/bin/pbpaste` before/after 捕获。
  - 如剪贴板变化，输出对应失败原因，例如：
    - `textedit-ui-tis-gate-mutated-clipboard`
    - `textedit-command-non-text-gate-mutated-clipboard`
    - `textedit-command-missing-text-gate-mutated-clipboard`
    - `clipboard-non-text-gate-mutated-clipboard`
    - `clipboard-missing-text-gate-mutated-clipboard`
    - `safari-command-non-text-gate-mutated-clipboard`
    - `safari-command-missing-text-gate-mutated-clipboard`
    - `safari-typing-existing-gate-mutated-clipboard`
    - `safari-enter-existing-gate-mutated-clipboard`
    - `safari-diagnose-existing-gate-mutated-clipboard`
  - 通过时输出 `*.clipboardUnchanged=true`，让证据直接可读。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  verifySyntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockAcquired=true
  cleanupPermissionContract=true
  tisReadinessBuild.clipboardUnchanged=true
  textEditUiDisabled.clipboardUnchanged=true
  textEditCommandUiDisabled.clipboardUnchanged=true
  clipboardUiDisabled.clipboardUnchanged=true
  textEditUiTisGate.clipboardUnchanged=true
  textEditCommandUiTisGateNoLaunchPassed=true
  textEditCommandNonTextClipboardGate.clipboardUnchanged=true
  textEditCommandNonTextClipboardGatePassed=true
  textEditCommandMissingTextClipboardGate.clipboardUnchanged=true
  textEditCommandMissingTextClipboardGatePassed=true
  clipboardUiTisGateNoLaunchPassed=true
  clipboardNonTextGate.clipboardUnchanged=true
  clipboardNonTextGatePassed=true
  clipboardMissingTextGate.clipboardUnchanged=true
  clipboardMissingTextGatePassed=true
  safariTypingUiDisabled.clipboardUnchanged=true
  safariCommandUiDisabled.clipboardUnchanged=true
  safariEnterUiDisabled.clipboardUnchanged=true
  safariDiagnoseUiDisabled.clipboardUnchanged=true
  safariCommandNonTextClipboardGate.clipboardUnchanged=true
  safariCommandNonTextClipboardGatePassed=true
  safariCommandMissingTextClipboardGate.clipboardUnchanged=true
  safariCommandMissingTextClipboardGatePassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  awaitShort.clipboardUnchanged=true
  installNoPrompt.clipboardUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前可安全验证的 GUI-smoke 早退路径已经把剪贴板、当前输入源、debug env、user host、目标 App 进程和临时文件残留纳入同一套 non-GUI gate。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：常用 Command 快捷键旁路与 TextEdit 已运行 gate 收敛验证

问题：

- 用户发现 `Command-C` / `Command-V` 在 Inputia 下不可用，说明输入法不应只针对复制粘贴做特例，而应按 macOS 惯例把常用 `Command` 修饰快捷键交还给宿主 App。
- GUI smoke 纪律同时要求：如果 TextEdit 已经在运行，测试脚本必须早退，不能抢用户窗口；这个 gate 需要在不真正启动 TextEdit GUI 的前提下可重复验证。

实现：

- 快捷键策略：
  - `Command-C` / `Command-V` / `Command-Shift-V` / `Command-Option-V` / `Command-Option-Shift-V` / `Command-Control-V` 等粘贴变体全部旁路。
  - 常见系统/App 快捷键一并旁路，包括剪切、撤销/重做、全选、保存、打开、关闭、退出、查找、打印、隐藏、新建、标签/窗口、偏好设置、截图、`Command-Tab`、`Command-Space`、`Command-Option-Escape`、`Command`+数字/括号/方向键/Delete 等。
  - Inputia 自有剪贴板召回仍保留在 `Ctrl-Shift-V`；`Ctrl-Shift-Command-V` 明确拒绝，避免和系统/App `Command` 快捷键混淆。
- `verify-nongui.sh`：
  - 用 `/bin/zsh -c "exec -a TextEdit /bin/sleep 60"` 创建假 `TextEdit` 进程，只触发 `pgrep -x TextEdit` gate，不启动真实 TextEdit。
  - 覆盖 `smoke-textedit.sh`、`smoke-textedit-command-shortcuts.sh`、`smoke-clipboard-recall.sh` 的 TextEdit 已运行早退路径。
  - trap 跟踪并清理假进程，防止验证失败时留下 `TextEdit` 进程名残留。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41.log 2>&1
  verifyRc=0
  shortcut: commonAppleCommandShortcutSetPassesThrough=true
  shortcut: anyCommandModifiedKeyPassesThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  hostShortcut: commonAppleCommandShortcutSetPassesThrough=true
  hostShortcut: anyCommandModifiedKeyPassesThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  fakeExistingProcessStarted=true process=TextEdit pid=47469
  textEditExistingGate: textEditPreflight=running docs=0
  textEditExistingGate: guiSmokeReady=false reason=textedit-already-running
  textEditExistingGate.rc=13
  textEditExistingGate.clipboardUnchanged=true
  textEditExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditExistingGate.debugEnvBefore=unset
  textEditExistingGate.debugEnvAfter=unset
  textEditExistingGate.userHost=false
  textEditCommandExistingGate: textEditPreflight=running docs=0
  textEditCommandExistingGate: guiSmokeReady=false reason=textedit-already-running
  textEditCommandExistingGate.rc=13
  textEditCommandExistingGate.clipboardUnchanged=true
  clipboardExistingTextEditGate: textEditPreflight=running docs=0
  clipboardExistingTextEditGate: guiSmokeReady=false reason=textedit-already-running
  clipboardExistingTextEditGate.rc=6
  clipboardExistingTextEditGate.clipboardUnchanged=true
  fakeExistingProcessStopped=true process=TextEdit pid=47469
  textEditExistingGateNoMutationPassed=true
  postInstall: commandCPassThrough=true
  postInstall: commandVPassThrough=true
  postInstallNonGuiNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

外部资料：

- Apple Support: `Copy and paste on Mac` 明确 `Command-C` 为复制、`Command-V` 为粘贴、`Command-Z` 为撤销。
  - https://support.apple.com/guide/mac-help/copy-and-paste-on-mac-mchl5252f3de/mac
- Apple Support: `Keyboard shortcuts on your Mac` 明确 `Command-V`、`Command-Z`、`Command-Shift-Z`、`Command-A`、`Command-F`、`Command-G`、`Command-H` 等通用快捷键。
  - https://support.apple.com/guide/mac-pro/keyboard-shortcuts-apd194062a6d/mac
- Microsoft Support: `Common Office for Mac keyboard shortcuts` 也把 `Command-Z`、`Command-Y`、`Command-X`、`Command-C`、`Command-V`、`Command-Control-V`、`Command-Shift-V`、`Command-A` 等作为常用 Office for Mac 快捷键。
  - https://support.microsoft.com/en-us/accessibility/office-accessibility/common-office-for-mac-keyboard-shortcuts

当前结论：

- `Command` 修饰的常用 macOS/App 快捷键现在按类别旁路，不再只修 `Command-C` / `Command-V` 两个点。
- TextEdit 已运行时，TextEdit 主 smoke、Command shortcut smoke、clipboard recall smoke 都能在不改剪贴板、不改当前输入源、不改 debug env、不创建 user host 的情况下早退。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：post-install non-GUI 回归段也纳入状态不变性 gate

背景：

- `post-install regression non-gui` 段会调用 `post-install-regression.sh` 的非 UI 路径；此前它主要依赖子脚本输出，没有在外层 verifier 里统一证明剪贴板、当前输入源、debug env、user host 和 GUI 进程都未被污染。
- 本轮完整验证还在 fake existing process 清理路径暴露了 `set -u` 下的空数组问题：当最后一个 fake process 被移除时，直接展开空的 `remaining_pids[@]` 会触发 `unbound variable`。

实现：

- `verify-nongui.sh`
  - 在 `post-install regression non-gui` 前后捕获：
    - `/usr/bin/pbpaste`
    - `current_input_source_id`
    - `debug_events_env`
  - 如果剪贴板变化，输出 `nonGuiVerificationPassed=false reason=post-install-non-gui-mutated-clipboard`。
  - 通过 `assert_current_source_unchanged`、`assert_debug_env_unchanged`、`assert_no_user_host` 和 `assert_process_not_running` 证明非 UI post-install 不启动/残留 TextEdit、Safari、osascript 或 Inputia host。
  - 通过时输出 `postInstallNonGuiNoMutationPassed=true`。
  - 修复 `stop_fake_existing_process()`：空数组时直接 `VERIFY_FAKE_PROCESS_PIDS=()`，非空时再展开 `remaining_pids[@]`，避免 `set -u` 空数组展开失败。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  verifySyntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh
  fakeExistingProcessStarted=true process=TextEdit
  textEditExistingGate.clipboardUnchanged=true
  textEditExistingGateNoMutationPassed=true
  textEditCommandExistingGate.clipboardUnchanged=true
  textEditCommandExistingGateNoMutationPassed=true
  clipboardExistingTextEditGate.clipboardUnchanged=true
  clipboardExistingTextEditGateNoMutationPassed=true
  postInstallNonGui.clipboardUnchanged=true
  postInstallNonGui.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallNonGui.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallNonGui.debugEnvBefore=unset
  postInstallNonGui.debugEnvAfter=unset
  postInstallNonGui.userHost=false
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGate.clipboardUnchanged=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort.clipboardUnchanged=true
  installNoPrompt.clipboardUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- post-install non-GUI 回归段现在和其它早退 gate 一样，外层直接证明不改剪贴板、不改当前输入源、不改 debug env、不创建 user host、不启动或残留 GUI 进程。
- fake existing process gate 的清理路径已覆盖 `set -u` 空数组场景，完整 non-GUI verifier 干净通过。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：Safari 已运行早退路径改为 non-GUI 假进程必测

问题：

- Safari typing/command/enter/diagnose 四个 smoke 都有 `safari-already-running` 早退 gate，用来避免抢用户已有 Safari 窗口。
- 但 verifier 之前只有在真实 Safari 已经运行时才验证这组 gate；正常情况下输出 `safariExistingGateSkipped=true reason=safari-not-running`，覆盖不稳定。
- 这和 TextEdit 已运行 gate 的目标一样：需要证明脚本会早退，但不能真的启动或操作 Safari GUI。

实现：

- `verify-nongui.sh`
  - 在 Safari 非文本/缺失文本剪贴板 gate 完成后，如果 Safari 原本未运行，使用 `start_fake_existing_process Safari` 创建假 `Safari` 进程。
  - 让 `smoke-safari-typing.sh`、`smoke-safari-command-shortcuts.sh`、`smoke-safari-enter.sh`、`diagnose-safari-input-source.sh` 进入真实的 `inputia_require_safari_idle` 早退路径。
  - 每条 gate 都断言剪贴板、当前输入源、debug env、user host 不变。
  - 验证结束后通过 `stop_fake_existing_process Safari` 清理假进程；如果假进程未触发 gate，else 分支也会主动清理。
  - 补齐 `safariCommandExistingGate.clipboardUnchanged=true` marker，便于日志检索。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-next.log 2>&1
  fakeExistingProcessStarted=true process=Safari pid=64704
  safariTypingExistingGate: safariPreflight=running
  safariTypingExistingGate: guiSmokeReady=false reason=safari-already-running
  safariTypingExistingGate.rc=9
  safariTypingExistingGate.clipboardUnchanged=true
  safariTypingExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariTypingExistingGate.debugEnvBefore=unset
  safariTypingExistingGate.debugEnvAfter=unset
  safariTypingExistingGate.userHost=false
  safariCommandExistingGate: safariPreflight=running
  safariCommandExistingGate: guiSmokeReady=false reason=safari-already-running
  safariCommandExistingGate.rc=13
  safariCommandExistingGate.clipboardUnchanged=true
  safariCommandExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandExistingGate.debugEnvBefore=unset
  safariCommandExistingGate.debugEnvAfter=unset
  safariCommandExistingGate.userHost=false
  safariEnterExistingGate: safariPreflight=running
  safariEnterExistingGate: guiSmokeReady=false reason=safari-already-running
  safariEnterExistingGate.rc=7
  safariEnterExistingGate.clipboardUnchanged=true
  safariEnterExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterExistingGate.debugEnvBefore=unset
  safariEnterExistingGate.debugEnvAfter=unset
  safariEnterExistingGate.userHost=false
  safariDiagnoseExistingGate: safariPreflight=running
  safariDiagnoseExistingGate: guiSmokeReady=false reason=safari-already-running
  safariDiagnoseExistingGate.rc=11
  safariDiagnoseExistingGate.clipboardUnchanged=true
  safariDiagnoseExistingGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseExistingGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseExistingGate.debugEnvBefore=unset
  safariDiagnoseExistingGate.debugEnvAfter=unset
  safariDiagnoseExistingGate.userHost=false
  fakeExistingProcessStopped=true process=Safari pid=64704
  safariExistingGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari 已运行早退路径现在和 TextEdit 已运行早退路径一样，不依赖用户机器上刚好有真实 App 正在运行。
- 这轮仍没有启动或操作真实 Safari；验证只使用进程名 gate，符合 GUI smoke 不抢焦点的纪律。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：输入源恢复 helper 增加动态失败自测

问题：

- `inputia_restore_previous_input_source()` 是所有 GUI smoke 清理当前输入源污染的公共 helper。
- 之前 verifier 已有静态契约，确认 helper 会读取当前源、尝试 `--select-source-id`、再次读取并在 mismatch 时返回 1。
- 但缺少动态自测来证明“恢复失败不能被静默吞掉”，一旦未来 helper 改坏，可能让 TextEdit/Safari/Clipboard smoke 在失败后留下错误输入源。

实现：

- `verify-nongui.sh`
  - 新增 `input source restore helper self-check`。
  - 创建临时 fake TIS tool，仅响应：
    - `--dump-current-input-source` -> 固定输出 `id=com.inputia.current`
    - `--select-source-id <id>` -> 记录 `selectedSourceID=<id>`
  - already-current 路径：`INPUTIA_PREVIOUS_INPUT_SOURCE_ID=com.inputia.current`，要求输出 `inputSourceRestore=skipped reason=already-current`。
  - failure 路径：`INPUTIA_PREVIOUS_INPUT_SOURCE_ID=com.inputia.previous`，fake tool 仍报告 current，要求：
    - helper 返回码为 1
    - 输出 `inputSourceRestore=false expected=com.inputia.previous actual=com.inputia.current`
    - restore log 证明确实尝试选择过 `com.inputia.previous`
  - 临时 fake tool 和 log 注册进 `VERIFY_TEMP_FILES` 并在自测后删除。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-restore-selfcheck.log 2>&1
  verifyRc=0
  inputSourceRestoreCurrentSelfCheck: inputSourceRestore=skipped reason=already-current
  inputSourceRestoreFailureSelfCheck: inputSourceRestore=false expected=com.inputia.previous actual=com.inputia.current
  inputSourceRestoreFailureSelfCheck.rc=1
  inputSourceRestoreHelperSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前输入源恢复 helper 现在有静态契约和动态失败自测两层保护。
- 如果未来恢复失败被改成返回 0、缺少 expected/actual marker，或没有真正尝试 `--select-source-id`，non-GUI verifier 会失败。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：await/post-install GUI gate 输出 marker 纳入 verifier 契约

问题：

- `await-system-install.sh` 和 `post-install-regression.sh` 是真实 GUI smoke 的入口前置门。
- 之前 verifier 已验证这些路径不会改剪贴板、当前输入源、debug env 或启动 GUI App。
- 但返回码和 no-mutation 还不够：如果 await 在 target/TIS 未 ready 时误报 `uiSmokeWouldStart=true`，后续人读日志可能被误导，真实 GUI smoke 风险会变高。

实现：

- `verify-nongui.sh`
  - `run_expect_rc()` 现在把子命令输出保存到 `RUN_EXPECT_RC_OUTPUT`，供当前 gate 立即做 marker 断言。
  - `postInstallUiTisGate` 要求输出：
    - `guiSmokeReady=false reason=tis-not-ready`
    - `postInstallUiSmokeReady=false reason=tis-not-ready`
  - `awaitShort` 要求 UI smoke 未请求时输出：
    - `uiSmokeRequested=false`
    - `uiSmokeWouldStart=false`
    - `uiSmokeBlockReason=ui-smoke-disabled`
  - `awaitUiNotReady` 要求 UI smoke 已请求但 target/TIS 未 ready 时输出：
    - `uiSmokeRequested=true`
    - `uiSmokeWouldStart=false`
    - `uiSmokeBlockReason=target-cdhash-mismatch`

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-markers.log 2>&1
  verifyRc=0
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  postInstallUiTisGate: postInstallUiSmokeReady=false reason=tis-not-ready
  awaitShort: uiSmokeRequested=false uiSmokeWouldStart=false uiSmokeBlockReason=ui-smoke-disabled
  awaitShort: systemInstallTISReady=false reason=target-cdhash-mismatch
  awaitUiNotReady: uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch
  awaitUiNotReady: systemInstallTISReady=false reason=target-cdhash-mismatch
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 真实 GUI smoke 入口现在不仅证明“不启动、不污染”，还证明在未 ready 状态下日志语义明确表达“不会启动 GUI smoke”及具体 block reason。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待 v41 安装和 TIS readiness 通过。

## v41 Mac mini 续修：Safari 选择输入源失败路径不得打开或污染 Safari

问题：

- Safari typing、Safari command shortcut、Safari Enter 和 Safari input-source diagnosis 都会先选择 Inputia，再打开 Safari 测试页。
- 当前系统安装版仍是 v40，build/pkg 是 v41；这些脚本在 `INPUTIA_RUN_UI_SMOKE=1` 且跳过 cdhash gate 后，会因为 TIS 未 ready 停在 `input-source-not-selected`。
- 这条路径此前没有逐脚本证明：选择输入源失败时必须在打开 Safari 前停止，并且不得改变剪贴板、当前输入源、debug env，不得创建 user host，不得残留 Safari、osascript 或 Inputia host。

实现：

- `verify-nongui.sh`
  - 新增四条 Safari TIS gate：
    - `safariTypingUiTisGate`
    - `safariCommandUiTisGate`
    - `safariEnterUiTisGate`
    - `safariDiagnoseUiTisGate`
  - 每条 gate 都在运行前后捕获：
    - `/usr/bin/pbpaste`
    - `current_input_source_id`
    - `debug_events_env`
  - 每条 gate 都断言：
    - 剪贴板未变化。
    - 当前输入源未变化。
    - debug env 未变化。
    - 未创建 `~/Library/Input Methods/InputiaInputMethod.app` 或 `IputiaInputMethod.app`。
    - 未启动或残留 Safari、osascript、InputiaInputMethod。
  - Safari command gate 使用剪贴板 info override 的文本形态，确保覆盖的是 TIS 选择失败路径，而不是不可恢复剪贴板早退路径。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  verifySyntaxOK=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-next.log 2>&1
  verifyRc=0
  safariTypingUiTisGate: guiSmokeReady=false reason=input-source-not-selected
  safariTypingUiTisGate: safariTypingSmokeReady=false reason=input-source-not-selected
  safariTypingUiTisGate.rc=8
  safariTypingUiTisGate.clipboardUnchanged=true
  safariTypingUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariTypingUiTisGate.debugEnvBefore=unset
  safariTypingUiTisGate.debugEnvAfter=unset
  safariTypingUiTisGate.userHost=false
  safariTypingUiTisGateNoLaunchPassed=true
  safariCommandUiTisGate: clipboardRestorable=true
  safariCommandUiTisGate: guiSmokeReady=false reason=input-source-not-selected
  safariCommandUiTisGate.rc=14
  safariCommandUiTisGate.clipboardUnchanged=true
  safariCommandUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandUiTisGate.debugEnvBefore=unset
  safariCommandUiTisGate.debugEnvAfter=unset
  safariCommandUiTisGate.userHost=false
  safariCommandUiTisGateNoLaunchPassed=true
  safariEnterUiTisGate: guiSmokeReady=false reason=input-source-not-selected
  safariEnterUiTisGate.rc=6
  safariEnterUiTisGate.clipboardUnchanged=true
  safariEnterUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterUiTisGate.debugEnvBefore=unset
  safariEnterUiTisGate.debugEnvAfter=unset
  safariEnterUiTisGate.userHost=false
  safariEnterUiTisGateNoLaunchPassed=true
  safariDiagnoseUiTisGate: guiSmokeReady=false reason=input-source-not-selected
  safariDiagnoseUiTisGate.rc=12
  safariDiagnoseUiTisGate.clipboardUnchanged=true
  safariDiagnoseUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseUiTisGate.debugEnvBefore=unset
  safariDiagnoseUiTisGate.debugEnvAfter=unset
  safariDiagnoseUiTisGate.userHost=false
  safariDiagnoseUiTisGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari 四条真实 GUI smoke/diagnosis 脚本现在都有 TIS 选择失败路径的 non-GUI 证据；当前系统未 ready 时，它们不会打开 Safari，不会污染用户状态，也不会留下临时进程或文件。
- 真实 Safari 上屏 smoke 仍等待系统安装版从 v40 更新到 v41，并且 TIS readiness 通过。

## v41 Mac mini 续修：await UI smoke 预判必须受 target/TIS gate 约束

问题：

- `await-system-install.sh` 会在等待系统安装期间输出 UI smoke 预判行。
- 此前 `INPUTIA_RUN_UI_SMOKE=1` 时，`uiSmokeWouldStart` 只看 TextEdit/Safari 是否运行，没有先检查目标安装包是否已匹配 build、TIS 是否 ready。
- 当前系统安装版仍是 v40、build/pkg 是 v41；如果预判只看 App 进程，可能在 target/TIS 未 ready 时给出过于乐观的 `uiSmokeWouldStart=true`，误导后续真实 GUI smoke 操作。

实现：

- `await-system-install.sh`
  - `ui_smoke_status_line` 现在接收 `target_matches_build` 和 `tis_status_line`。
  - `INPUTIA_RUN_UI_SMOKE=1` 时先检查：
    - target 不匹配 build：输出 `uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch`。
    - target 已匹配但 TIS 未 ready：输出 `uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=tis-not-ready`。
  - 只有 target 和 TIS 都 ready 后，才继续检查 TextEdit/Safari 是否已运行。
- `verify-nongui.sh`
  - 增加静态契约，要求 await 的 UI 预判先有 target mismatch gate，再有 TIS-not-ready gate，最后才检查 TextEdit/Safari。
  - 增加 `awaitUiNotReady` 运行 gate，证明 UI smoke 请求开启但 target/TIS 未 ready 时不会启动 GUI App，也不会污染剪贴板、当前输入源、debug env 或 user host。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxOK=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 macos/InputiaInputMethod/await-system-install.sh
  rc=2
  uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch
  systemInstallObserved=false reason=timeout
  systemInstallTargetMatchesBuild=false
  systemInstallTISReady=false reason=target-cdhash-mismatch

bash macos/InputiaInputMethod/verify-nongui.sh
  awaitUiNotReady: uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch
  awaitUiNotReady.clipboardUnchanged=true
  awaitUiNotReady.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  awaitUiNotReady.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  awaitUiNotReady.debugEnvBefore=unset
  awaitUiNotReady.debugEnvAfter=unset
  awaitUiNotReady.userHost=false
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 安装等待阶段现在不会在 target/TIS 未 ready 时声称 UI smoke 可以启动。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：await UI 预判对齐 GUI session gate

问题：

- `post-install-regression.sh` 真正执行 UI smoke 前会检查 GUI session：有 console user、登录完成、屏幕未锁、frontmost App 可读且不是 loginwindow。
- `await-system-install.sh` 的 UI smoke 预判在上一轮已经受 target/TIS gate 约束，但 target/TIS ready 后仍只检查 TextEdit/Safari 进程。
- 如果未来系统安装和 TIS 都 ready，但桌面未登录、锁屏或 frontmost 不可用，await 可能仍输出 `uiSmokeWouldStart=true`，而真实 post-install 入口随后会拒跑。这会让等待阶段状态和真实入口纪律不一致。

实现：

- `await-system-install.sh`
  - 新增 `gui_session_block_reason()`，以非退出方式复用 post-install 的 GUI session 判定原因：
    - `no-console-user`
    - `login-not-complete`
    - `screen-locked`
    - `frontmost-unavailable`
    - `loginwindow-frontmost`
  - `ui_smoke_status_line` 的顺序现在是：
    1. `INPUTIA_RUN_UI_SMOKE` 是否启用。
    2. target cdhash 是否匹配 build。
    3. TIS readiness 是否通过。
    4. GUI session 是否 ready。
    5. TextEdit/Safari 是否已有进程。
- `verify-nongui.sh`
  - 静态契约要求 await 的 GUI session gate 位于 target/TIS gate 之后、TextEdit/Safari process preflight 之前。
  - 静态契约要求 await 覆盖和 post-install 对齐的五个 GUI session block reason。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxOK=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 macos/InputiaInputMethod/await-system-install.sh
  rc=2
  uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch
  systemInstallTargetMatchesBuild=false
  systemInstallTISReady=false reason=target-cdhash-mismatch

bash macos/InputiaInputMethod/verify-nongui.sh
  awaitShort: uiSmokeRequested=false uiSmokeWouldStart=false uiSmokeBlockReason=ui-smoke-disabled
  awaitUiNotReady: uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- await 阶段的 UI smoke 预判现在和 post-install 的真实执行入口使用同一层级的 gate：target、TIS、GUI session、目标 App 进程。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：await GUI session gate 增加可运行自测

问题：

- `await-system-install.sh` 已经把 UI smoke 预判顺序对齐为 target、TIS、GUI session、目标 App 进程。
- 但 GUI session 的五个 block reason 很难在正常桌面上动态验证：不能为了测试去登出、锁屏或切到 loginwindow。
- 只有静态契约时，未来如果某个 reason 的输出语义被改坏，non-GUI verifier 不一定能在当前桌面状态下发现。

实现：

- `await-system-install.sh`
  - `gui_session_block_reason()` 增加 verifier 专用 override：`INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST`。
  - 新增 `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 自测入口。
  - 自测入口构造 ready TIS status，并显式设置 `INPUTIA_RUN_UI_SMOKE=1`，逐个模拟：
    - `no-console-user`
    - `login-not-complete`
    - `screen-locked`
    - `frontmost-unavailable`
    - `loginwindow-frontmost`
  - 每个 reason 都通过真实 `ui_smoke_status_line true "$ready_tis_status"` 输出预判行，不触碰真实 GUI。
- `verify-nongui.sh`
  - 静态契约要求 await 保留 self-check 入口和 test override。
  - 运行 `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 await-system-install.sh`，要求每个 reason 都输出：
    - `uiSmokeRequested=true`
    - `uiSmokeWouldStart=false`
    - `uiSmokeBlockReason=<reason>`

验证：

```text
INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=no-console-user uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=no-console-user
  awaitUiStatusSelfCheck reason=login-not-complete uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=login-not-complete
  awaitUiStatusSelfCheck reason=screen-locked uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=screen-locked
  awaitUiStatusSelfCheck reason=frontmost-unavailable uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=frontmost-unavailable
  awaitUiStatusSelfCheck reason=loginwindow-frontmost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=loginwindow-frontmost
  awaitUiStatusSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-preexisting-guard.log 2>&1
  verifyRc=0
  awaitUiStatusSelfCheck reason=no-console-user uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=no-console-user
  awaitUiStatusSelfCheck reason=login-not-complete uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=login-not-complete
  awaitUiStatusSelfCheck reason=screen-locked uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=screen-locked
  awaitUiStatusSelfCheck reason=frontmost-unavailable uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=frontmost-unavailable
  awaitUiStatusSelfCheck reason=loginwindow-frontmost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=loginwindow-frontmost
  awaitUiStatusSelfCheck=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- await 的 GUI session gate 现在不仅有静态顺序契约，也有可运行的 reason-by-reason 自测。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：verifier 不把用户已有 App 误判为测试污染

问题：

- `verify-nongui.sh` 会在多个入口级 gate 后断言 TextEdit/Safari 未运行，以证明脚本没有启动真实 GUI App。
- 这些断言对当前干净桌面有效，但如果用户本来已经打开 TextEdit 或 Safari，无条件 `assert_process_not_running` 会把用户已有进程误判成 smoke 污染。
- 受影响的入口级 gate 包括：
  - `postInstallNonGui`
  - `postInstallUiTisGate`
  - `awaitUiNotReady`

实现：

- `verify-nongui.sh`
  - 上述入口级 gate 的 TextEdit/Safari no-launch 断言现在只在本轮开始时对应 App 不存在时执行。
  - 如果用户已有 App 进程，verifier 仍继续检查剪贴板、当前输入源、debug env、user host、osascript 和 Inputia host；不会关闭或误判用户已有 App。
  - 静态契约新增检查，要求这些入口级 no-launch 断言都受 `TEXTEDIT_PREEXISTING` / `SAFARI_PREEXISTING` guard 保护。
- `await-system-install.sh`
  - `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 现在通过 `INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST` 覆盖 GUI session block reason，能在 non-GUI verifier 中验证所有 GUI session 预判原因。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxOK=true

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=no-console-user uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=no-console-user
  awaitUiStatusSelfCheck reason=login-not-complete uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=login-not-complete
  awaitUiStatusSelfCheck reason=screen-locked uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=screen-locked
  awaitUiStatusSelfCheck reason=frontmost-unavailable uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=frontmost-unavailable
  awaitUiStatusSelfCheck reason=loginwindow-frontmost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=loginwindow-frontmost
  awaitUiStatusSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyRc=0
  cleanupPermissionContract=true
  awaitUiStatusSelfCheck=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- verifier 现在区分“用户已有 TextEdit/Safari”和“本轮测试启动的 TextEdit/Safari”，入口级 no-launch 证明不会误报用户已有 App。
- await 的 GUI session block reason 有动态 self-check 覆盖。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：post-install UI process preflight 增加可运行自测

问题：

- `post-install-regression.sh` 的真实 UI smoke 入口会在 TIS ready 后检查 TextEdit/Safari 是否已运行。
- 当前系统仍停在 v40/v41 不一致和 TIS 未 ready，所以完整 post-install 运行无法自然进入 TextEdit/Safari process preflight。
- 只有静态契约时，未来如果 `textedit-already-running` / `safari-already-running` 的拒跑输出或 allow-existing 分支漂移，non-GUI verifier 不一定能发现。

实现：

- `post-install-regression.sh`
  - `require_ui_process_idle()` 增加 verifier 专用 override：`INPUTIA_UI_PROCESS_RUNNING_FOR_TEST`。
  - 新增 `INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1` 自测入口。
  - 自测入口不启动 TextEdit/Safari，只模拟 process running，并覆盖：
    - TextEdit 已运行且未 allow -> rc 4，输出 `guiSmokeReady=false reason=textedit-already-running` 和 `postInstallUiSmokeReady=false reason=textedit-already-running`
    - Safari 已运行且未 allow -> rc 4，输出 `guiSmokeReady=false reason=safari-already-running` 和 `postInstallUiSmokeReady=false reason=safari-already-running`
    - TextEdit allow-existing -> rc 0，输出 `TextEditPreflightAllowed=true`
    - Safari allow-existing -> rc 0，输出 `SafariPreflightAllowed=true`
- `verify-nongui.sh`
  - 静态契约要求 post-install 保留 UI preflight self-check 和 process-running override。
  - 运行 `INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 post-install-regression.sh`，逐项断言上述 marker。

验证：

```text
INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiPreflightSelfCheck case=textedit-block TextEditPreflight=running
  postInstallUiPreflightSelfCheck case=textedit-block guiSmokeReady=false reason=textedit-already-running
  postInstallUiPreflightSelfCheck case=textedit-block postInstallUiSmokeReady=false reason=textedit-already-running
  postInstallUiPreflightSelfCheck case=textedit-block rc=4
  postInstallUiPreflightSelfCheck case=safari-block SafariPreflight=running
  postInstallUiPreflightSelfCheck case=safari-block guiSmokeReady=false reason=safari-already-running
  postInstallUiPreflightSelfCheck case=safari-block postInstallUiSmokeReady=false reason=safari-already-running
  postInstallUiPreflightSelfCheck case=safari-block rc=4
  postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflight=running
  postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflightAllowed=true
  postInstallUiPreflightSelfCheck case=textedit-allow rc=0
  postInstallUiPreflightSelfCheck case=safari-allow SafariPreflight=running
  postInstallUiPreflightSelfCheck case=safari-allow SafariPreflightAllowed=true
  postInstallUiPreflightSelfCheck case=safari-allow rc=0
  postInstallUiPreflightSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-postinstall-preflight.log 2>&1
  verifyRc=0
  postInstallUiPreflightSelfCheck case=textedit-block TextEditPreflight=running
  postInstallUiPreflightSelfCheck case=textedit-block guiSmokeReady=false reason=textedit-already-running
  postInstallUiPreflightSelfCheck case=safari-block SafariPreflight=running
  postInstallUiPreflightSelfCheck case=safari-block guiSmokeReady=false reason=safari-already-running
  postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflightAllowed=true
  postInstallUiPreflightSelfCheck case=safari-allow SafariPreflightAllowed=true
  postInstallUiPreflightSelfCheck=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- post-install UI preflight 的 TextEdit/Safari 已运行拒跑和 allow-existing 行为现在都有可运行证据，不依赖真实 GUI readiness。
- 当前系统安装版仍是 v40，build/pkg 是 v41；真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：Safari 成功路径输出测试窗口关闭证据

问题：

- Safari typing / command shortcut / enter 三个真实 GUI smoke 的错误路径已有 `closeSmokeWindow(smokeWindowId)` 和 `restoreFrontmost(previousBundleId)` 契约。
- 成功路径此前只输出输入结果或提交结果；如果后续真实 smoke 成功但 Safari 测试窗口没有关闭，日志里缺少直接可审计的成功路径清理证据。
- 当前系统安装版仍是 v40，build/pkg 是 v41；不能绕过 target/TIS gate 强跑真实 Safari GUI smoke。

实现：

- `smoke-safari-typing.sh`
  - `closeSmokeWindow(smokeWindowId)` 改为返回 `"true"` / `"false"`。
  - 成功路径返回结构化输出：`safariTypingTitle=...` 和 `safariSmokeWindowClosed=true`。
  - shell 层要求 `safariSmokeWindowClosed=true`，否则失败：`safariTypingSmokePassed=false reason=smoke-window-not-closed`。
- `smoke-safari-command-shortcuts.sh`
  - 成功路径追加 `safariSmokeWindowClosed=true`。
  - shell 层要求该 marker，否则失败：`safariCommandShortcutSmokePassed=false step=smoke-window-close`。
- `smoke-safari-enter.sh`
  - 成功路径返回结构化输出：`safariEnterTitle=...` 和 `safariSmokeWindowClosed=true`。
  - shell 层要求该 marker，否则失败：`safariEnterSmokePassed=false reason=smoke-window-not-closed`。
- `verify-nongui.sh`
  - 静态契约要求三个 Safari smoke 都保留：
    - `return "true"` / `return "false"` 的关闭窗口结果；
    - `safariSmokeWindowClosed=` 输出；
    - 成功路径的关闭窗口断言。
  - 同步 cleanup permission contract 的 `inputia_run_with_timeout` 目标匹配到新的 `results=` 输出结构。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-typing.sh macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/verify-nongui.sh
  bashSyntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-safari-window-close.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  awaitUiStatusSelfCheck=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

residual checks
  TextEdit=not-running
  Safari=not-running
  InputiaInputMethod=not-running
  osascript=not-running
  sleep=not-running
  smoke script residue: none
  /tmp/inputia-* residue: none
```

当前结论：

- Safari 真实 GUI smoke 一旦可跑，成功日志会同时包含业务结果和 `safariSmokeWindowClosed=true` 清理证据。
- 当前仍未运行真实 Safari/TextEdit GUI smoke；系统安装版仍需先更新到 v41，并通过 TIS readiness。

## v41 Mac mini 续修：Clipboard recall 成功路径输出状态清空和 TextEdit 清理证据

问题：

- `smoke-clipboard-recall.sh` 已经有 `clearInputiaState()`、pre-trigger / pre-selection event guard，以及 `state-clear-leaked-text:` 断言。
- 但成功路径此前只输出召回结果和 TextEdit 文档数量；日志里缺少：
  - TextEdit 是否本来已运行；
  - 清空 IME 状态后 smoke 文档内容是否确认为空；
  - `cleanupTextEdit()` 是否明确成功。
- 上一轮真实 recall 曾出现 mismatch，后续需要能从日志直接判断是状态污染、清理失败，还是候选召回本身失败。

实现：

- `smoke-clipboard-recall.sh`
  - `cleanupTextEdit(...)` 增加 `cleanupSucceeded` 布尔状态，关闭 smoke 文档、退出测试启动的 TextEdit、关闭多余文档任一失败都会置为 false。
  - 成功路径返回结构化 marker：
    - `textEditWasRunningBefore=...`
    - `clipboardRecallClearedText=...`
    - `textEditCleanupSucceeded=...`
  - shell 层输出上述 marker，并新增断言：
    - `clipboardRecallClearedText` 必须为空，否则 `clipboardRecallSmokePassed=false reason=state-clear-leaked-text`；
    - `textEditCleanupSucceeded` 必须为 `true`，否则 `clipboardRecallSmokePassed=false reason=textedit-cleanup-failed`。
- `verify-nongui.sh`
  - 静态契约要求 Clipboard recall 保留上述输出 marker、shell 断言、cleanup 成功/失败 flag 和返回状态。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/verify-nongui.sh
  bashSyntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-clipboard-cleanup-markers-rerun.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  awaitUiStatusSelfCheck=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

residual checks
  TextEdit=not-running
  Safari=not-running
  InputiaInputMethod=not-running
  osascript=not-running
  sleep=not-running
  smoke script residue: none
  /tmp/inputia-* residue: none
```

当前结论：

- Clipboard recall 真实 GUI smoke 一旦可跑，成功日志会同时说明输入法状态清空结果、TextEdit 原始运行状态、TextEdit 清理结果和召回提交事件。
- 当前仍未运行真实 TextEdit/Safari/Clipboard GUI smoke；系统安装版仍需先更新到 v41，并通过 TIS readiness。

## v41 Mac mini 续修：Safari 输入源诊断也输出窗口关闭证据

背景：

- `diagnose-safari-input-source.sh` 也会打开 Safari 本地 `data:` 页，虽然不打字，但仍然属于真实 GUI 路径。
- Safari typing / command / enter 已经在成功路径输出 `safariSmokeWindowClosed=true`；诊断脚本此前主要依赖 shell trap，成功路径缺少可审计的“已关闭自己窗口并恢复前台”结果。
- 当前系统安装版仍是 v40，build/pkg 是 v41；不能绕过 target/TIS gate 强跑真实 Safari GUI smoke。

实现：

- `diagnose-safari-input-source.sh`
  - 新增 `PREVIOUS_BUNDLE_ID`，打开 Safari 前捕获当前前台 App bundle id。
  - `close_safari_diagnose_window` 改为按 `SAFARI_DIAGNOSE_WINDOW_ID` 关闭诊断窗口，并在关闭后恢复之前的前台 App。
  - trap cleanup 继续静默调用同一关闭逻辑作为错误路径兜底。
  - 成功路径显式调用关闭逻辑，输出 `safariDiagnoseWindowClosed=true`；关闭失败则输出 `safariInputSourceDiagnosisPassed=false reason=diagnose-window-not-closed` 并失败。
- `verify-nongui.sh`
  - 静态契约要求诊断脚本保留：
    - `SAFARI_DIAGNOSE_WINDOW_ID`；
    - `PREVIOUS_BUNDLE_ID`；
    - `close_safari_diagnose_window`；
    - 按 `window id smokeWindowId` 关闭 Safari 窗口；
    - 恢复 `previousBundleId`；
    - `safariDiagnoseWindowClosed=` 输出和失败断言。

验证：

```text
bash -n macos/InputiaInputMethod/diagnose-safari-input-source.sh
  bashSyntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  bashSyntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-diagnose-cleanup.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  safariDiagnoseUiDisabled.rc=10
  safariDiagnoseUiDisabled.clipboardUnchanged=true
  safariDiagnoseUiTisGate.rc=12
  safariDiagnoseUiTisGate.clipboardUnchanged=true
  safariDiagnoseUiTisGateNoLaunchPassed=true
  safariDiagnoseExistingGate.rc=11
  safariDiagnoseExistingGate.clipboardUnchanged=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari 输入源诊断现在和其它 Safari smoke 一样，在成功路径留下窗口关闭 marker，并在失败路径通过 trap 兜底清理。
- 本轮仍未运行真实 TextEdit/Safari GUI smoke；系统安装版仍是 v40，build/pkg 是 v41，真实 GUI smoke 继续等待系统安装版更新到 v41 且 TIS readiness 通过。

## v41 Mac mini 续修：Command 修饰键变体全量旁路自检

背景：

- 用户指出 `Command-C` / `Command-V` 在输入法下不可用，并要求“举一反三”处理常用电脑快捷键，而不是逐个等用户测试。
- 前面已经按 Apple / Office for Mac 常见快捷键资料把策略定为“任何带 `Command` 的 keyDown 先交还系统/App”，并覆盖常见 `Command` 快捷键集合。
- 继续加固点：不只证明 C/V 和少数粘贴变体，也要证明所有包含 `Command` 的修饰键组合都旁路，覆盖 Terminal/编辑器常见的 `Command` + `Shift` / `Option` / `Control` 叠加组合。

实现：

- `Tools/InputiaShortcutSelfCheck.swift`
  - 新增 `commandModifierVariants`：
    - `.command`
    - `.command + .shift`
    - `.command + .option`
    - `.command + .control`
    - `.command + .option + .shift`
    - `.command + .control + .shift`
    - `.command + .control + .option`
    - `.command + .control + .option + .shift`
  - 新增 `allCommandModifierVariantsPassThrough=true` marker。
  - `anyCommandModifiedKeyPassesThrough` 改为用上述完整变体集合乘以代表性 keyCode 验证。
- `Sources/InputiaInputMethod/main.swift`
  - build host 的 `--host-shortcut-self-check` 同步新增同一变体集合和 `allCommandModifierVariantsPassThrough=true` marker。
- `verify-nongui.sh`
  - 静态契约要求工具和 host diagnostic 都包含完整变体集合，特别是 `.command + .control + .option` 和 `.command + .control + .option + .shift`。
  - 动态契约要求独立 self-check 和 build host self-check 都输出 `allCommandModifierVariantsPassThrough=true`。

验证：

```text
swiftc -typecheck macos/InputiaInputMethod/Tools/InputiaShortcutSelfCheck.swift \
  macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaInputTextRouter.swift \
  macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift \
  -framework AppKit
  typecheckRc=0

macos/InputiaInputMethod/build.sh
  buildRc=0
  buildVersion=41
  buildCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  latestPkg=InputiaInputMethod-latest.pkg
  latestPkgSha256=e6af057c5199c590a6eba4529439738cd4919e0d1be42530b7776ddc3b16858c
  versionedPkg=InputiaInputMethod-v41-6d7e1033ef95.pkg

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-command-variants.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  awaitUiNotReady: expectedCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  awaitUiNotReady: target.version=40
  awaitUiNotReady: target.matchesBuild=false
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Inputia 的快捷键策略现在有两层证据：常见 Command 快捷键集合旁路，以及所有包含 Command 的修饰键变体旁路。
- `Ctrl-Shift-V` 仍是 Inputia 剪贴板召回；带 `Command` 的召回变体继续拒绝，避免抢系统/App 粘贴快捷键。
- 当前系统安装版仍是 v40，build/pkg 已更新为 v41 cdhash `6d7e1033ef95597258f7c9c30f7d361f1b3dee2f`；真实 GUI smoke 仍等待系统安装版更新和 TIS readiness。

## v41 Mac mini 续修：TextEdit smoke 成功路径输出清理成功证据并同步本地 pkg

问题：

- `smoke-textedit.sh` 和 `smoke-textedit-command-shortcuts.sh` 已输出 `textEditWasRunningBefore`、`textEditDocsBefore`、`textEditDocsAfter`，并检查文档数量不增加。
- 但清理函数此前没有在所有路径返回布尔状态，成功日志也缺少 `textEditCleanupSucceeded=true`。
- 这会留下一个证据缺口：真实 GUI smoke 业务输入成功、文档数量刚好未增加，但关闭 smoke 文档、退出测试启动的 TextEdit 或恢复前台失败时，日志不能直接说明清理是否成功。

实现：

- `smoke-textedit.sh`
  - `cleanupDoc(docRef)` 返回 `true` / `false`。
  - `cleanupTextEdit(...)` 增加 `cleanupSucceeded` 布尔状态；退出测试启动的 TextEdit、关闭多余文档失败都会置为 false。
  - 成功路径输出 `textEditCleanupSucceeded=...`。
  - shell 层要求该 marker 为 `true`，否则 `textEditSmokePassed=false step=textedit-cleanup-failed`。
- `smoke-textedit-command-shortcuts.sh`
  - 同步 `cleanupDoc` / `cleanupTextEdit` 返回状态。
  - 成功路径输出并断言 `textEditCleanupSucceeded=true`。
- `verify-nongui.sh`
  - 静态契约要求 TextEdit 主 smoke 和 command smoke 保留 cleanup 成功/失败 flag、返回状态、输出 marker 和 shell 断言。
- 本地 build/pkg
  - 发现 verifier 要求 `allCommandModifierVariantsPassThrough=true`，源码已有实现但 build 产物较旧。
  - 运行 `./build.sh` 刷新 build app。
  - 运行 `./build-pkg.sh` 同步 `dist/InputiaInputMethod-latest.pkg`，避免 pkg archive CDHash 与 build app CDHash 不一致。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit.sh macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/verify-nongui.sh
  bashSyntaxRc=0

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandOptionShiftVPassThrough=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandOptionShiftVPassThrough=true

./build.sh
  build/InputiaInputMethod.app: valid on disk
  build/Inputia 设置.app: valid on disk
  note: ld emitted macOS 26.0 object linked to macOS 13.0 warnings from libinputia_capi.a

./build-pkg.sh
  appCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  sha256=e6af057c5199c590a6eba4529439738cd4919e0d1be42530b7776ddc3b16858c
  archiveAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  buildAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  pkgVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-textedit-cleanup-markers.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  allCommandModifierVariantsPassThrough=true
  pkgVerificationPassed=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

residual checks
  TextEdit=not-running
  Safari=not-running
  InputiaInputMethod=not-running
  osascript=not-running
  sleep=not-running
  smoke script residue: none
  /tmp/inputia-* residue: none
```

当前结论：

- TextEdit 主 smoke、TextEdit command smoke、Clipboard recall、Safari smoke/diagnose 的成功路径现在都有清理结果 marker 或窗口关闭 marker。
- 本轮仅刷新本地 build/pkg，未安装到 `/Library/Input Methods`；系统安装版仍需后续更新到新 v41 CDHash 后再进入真实 GUI smoke。

## v41 Mac mini 续修：确认系统安装 gate 仍停在管理员权限与 target CDHash mismatch

目的：

- 本地 build/pkg 已更新到 v41 新 CDHash，但真实 GUI smoke 的前置条件要求系统 `/Library/Input Methods/InputiaInputMethod.app` 与 build app 一致，并且 TIS readiness 通过。
- 本轮不绕过 `target-cdhash-mismatch` / TIS readiness gate 强跑 TextEdit/Safari/Clipboard GUI smoke。
- 先确认是否能无交互安装，若不能，记录可审计阻塞原因和用户状态不污染证据。

当前 build/pkg：

```text
./status.sh
  buildVersion=41
  buildCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  system version=40
  system cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  system settings version=40
  systemSettingsMatchesBuildVersion=false
  latest pkg sha256=e6af057c5199c590a6eba4529439738cd4919e0d1be42530b7776ddc3b16858c

./verify-pkg.sh
  packageVersion=41
  buildVersion=41
  archiveAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  buildAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  pkgVerificationPassed=true
```

安装 gate：

```text
INPUTIA_INSTALL_NO_ADMIN_PROMPT=1 ./install-system.sh
  installNoPromptRc=12
  systemInstallNeedsAdmin=true
  systemInstallReady=false reason=admin-required
```

await / UI smoke 预判：

```text
INPUTIA_RUN_UI_SMOKE=1 ./await-system-install.sh
  awaitRc=2
  uiSmokeRequested=true
  uiSmokeWouldStart=false
  uiSmokeBlockReason=target-cdhash-mismatch
  target.version=40
  target.cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  target.matchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  tis.readinessBlockReason=missing-enabled-source
  systemInstallObserved=false reason=timeout
  systemInstallTargetMatchesBuild=false
  systemInstallTISReady=false reason=target-cdhash-mismatch
```

回归验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-admin-gate.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  pkgVerificationPassed=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  installNoPrompt: systemInstallNeedsAdmin=true
  installNoPrompt: systemInstallReady=false reason=admin-required
  installNoPrompt.rc=12
  installNoPrompt.clipboardUnchanged=true
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.debugEnvBefore=unset
  installNoPrompt.debugEnvAfter=unset
  installNoPrompt.userHost=false
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

residual checks
  TextEdit=not-running
  Safari=not-running
  InputiaInputMethod=not-running
  osascript=not-running
  sleep=not-running
  smoke script residue: none
  /tmp/inputia-* residue: none
```

当前结论：

- 新 v41 build/pkg 已准备好且包内归档与 build app CDHash 一致。
- 当前无法无交互更新系统安装版：`admin-required`。
- 真实 GUI smoke 仍未运行；正确下一步是拿到管理员安装能力后更新 `/Library/Input Methods` 和 `/Applications/Inputia 设置.app` 到 CDHash `6d7e1033ef95597258f7c9c30f7d361f1b3dee2f`，再等待 TIS readiness，通过后再跑 TextEdit / Clipboard recall / Safari 真 GUI smoke。

## v41 Mac mini 续修：新增真实 GUI smoke readiness 汇总脚本

问题：

- 真实 GUI smoke 的前置条件分散在 `status.sh`、`verify-pkg.sh`、`tis-readiness.sh`、`await-system-install.sh`、`install-system.sh` 和各 smoke 脚本里。
- 当前阻塞已经反复出现为：build/pkg ready，但系统安装版仍是 v40，且无交互安装被 `admin-required` 阻止。
- 后续拿到管理员安装能力后，需要一个快速、只读、无副作用的汇总入口来判断是否能进入 TextEdit / Clipboard / Safari 真 GUI smoke，而不是人工拼多段日志。

实现：

- 新增 `gui-smoke-readiness.sh`
  - 只读输出：
    - build app version/CDHash；
    - system target version/CDHash/matchesBuild；
    - settings launcher version/matchesBuild；
    - latest pkg SHA 和 `pkg.ready`；
    - TIS readiness 详情；
    - admin install readiness；
    - GUI session block reason；
    - TextEdit/Safari 进程 preflight；
    - 最终 `guiSmokeReadinessReady=<true|false> reason=<reason>`。
  - 不安装、不启动 TextEdit/Safari、不选择输入源。
  - 新增 `INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1`，覆盖原因优先级：
    - `pkg-not-ready`
    - `admin-required`
    - `target-cdhash-mismatch`
    - `settings-version-mismatch`
    - `tis-not-ready`
    - GUI session reason
    - `textedit-already-running`
    - `safari-already-running`
    - ready / `none`
- `verify-nongui.sh`
  - 增加脚本语法检查。
  - 增加静态契约，要求 readiness reason 和输出 marker 存在。
  - 运行 readiness self-check 并断言关键 case。

当前 readiness：

```text
macos/InputiaInputMethod/gui-smoke-readiness.sh
  target.matchesBuild=false
  settings.matchesBuild=false
  pkg.ready=true
  tis.ready=false
  adminInstallReady=false reason=admin-required
  guiSessionBlockReason=none
  textEditPreflight=not-running
  safariPreflight=not-running
  guiSmokeReadinessReady=false reason=admin-required
```

验证：

```text
INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh
  guiSmokeReadinessSelfCheck case=pkg expected=pkg-not-ready actual=pkg-not-ready
  guiSmokeReadinessSelfCheck case=admin expected=admin-required actual=admin-required
  guiSmokeReadinessSelfCheck case=target expected=target-cdhash-mismatch actual=target-cdhash-mismatch
  guiSmokeReadinessSelfCheck case=settings expected=settings-version-mismatch actual=settings-version-mismatch
  guiSmokeReadinessSelfCheck case=tis expected=tis-not-ready actual=tis-not-ready
  guiSmokeReadinessSelfCheck case=gui expected=screen-locked actual=screen-locked
  guiSmokeReadinessSelfCheck case=textedit expected=textedit-already-running actual=textedit-already-running
  guiSmokeReadinessSelfCheck case=safari expected=safari-already-running actual=safari-already-running
  guiSmokeReadinessSelfCheck case=ready expected=none actual=none
  guiSmokeReadinessSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-readiness.log 2>&1
  verifyRc=0
  guiSmokeReadinessSelfCheck=true
  pkgVerificationPassed=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  installNoPrompt: systemInstallReady=false reason=admin-required
  installNoPrompt.clipboardUnchanged=true
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

residual checks
  TextEdit=not-running
  Safari=not-running
  InputiaInputMethod=not-running
  osascript=not-running
  sleep=not-running
  /tmp/inputia-* residue: none
```

当前结论：

- 现在有一个单命令 readiness 汇总入口，可以在管理员安装后快速判断是否进入真实 GUI smoke。
- 当前首要阻塞仍是无交互管理员安装不可用；未强跑真实 GUI smoke。

## v41 Mac mini 续修：build 与 verifier/smoke 互斥，避免构建期删除 build app 造成假失败

问题：

- `build.sh` 在构建开始会 `rm -rf build/InputiaInputMethod.app` 后重建。
- 本轮继续验证时观察到后台仍有 `verify-nongui.sh` / `smoke-textedit-command-shortcuts.sh` 正在使用 build app；如果此时运行 `build.sh`，verifier 可能撞上短暂缺失的 `build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod`，产生 `missing-build-host-executable` 假失败，也可能让 GUI smoke 中途读到漂移的 build 产物。
- 这属于 smoke 清理/验证纪律问题：构建不应在 verifier 或 GUI smoke 活动时替换它们正在验证的 app。

实现：

- `build.sh`
  - 新增 `detect_verification_processes()`，扫描当前 workspace 下正在运行的 verifier / post-install / smoke / status / TIS readiness 脚本。
  - 在 `rm -rf "$APP_DIR" "$SETTINGS_APP_DIR"` 前调用 `require_no_verification_processes`。
  - 检测到 verifier/smoke 正在运行时输出：
    - `buildReady=false reason=verification-running`
    - `buildBlockingProcess: ...`
    - 并以 rc `20` 退出，不删除 build app。
  - 新增 `INPUTIA_BUILD_PREFLIGHT_SELF_CHECK=1`，不触发真实构建，仅验证 clear / blocked 两个分支。
- `verify-nongui.sh`
  - 静态契约要求 build preflight 存在，且发生在 `rm -rf "$APP_DIR" "$SETTINGS_APP_DIR"` 之前。
  - 动态运行 `INPUTIA_BUILD_PREFLIGHT_SELF_CHECK=1 build.sh`，要求：
    - `buildPreflightSelfCheck clear=true`
    - `buildPreflightSelfCheck blocked=true`
    - `buildPreflightSelfCheck=true`

验证：

```text
zsh -n macos/InputiaInputMethod/build.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_BUILD_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/build.sh
  buildPreflightSelfCheck clear=true
  buildPreflightSelfCheck blocked=true
  buildPreflightSelfCheck=true

INPUTIA_BUILD_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/verify-nongui.sh" macos/InputiaInputMethod/build.sh
  buildReady=false reason=verification-running
  buildBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/verify-nongui.sh
  buildRc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-build-preflight.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  buildPreflightSelfCheck clear=true
  buildPreflightSelfCheck blocked=true
  buildPreflightSelfCheck=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  awaitUiNotReady: target.version=40
  awaitUiNotReady: target.matchesBuild=false
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 以后本地构建不会在 verifier/smoke 活动时删除 build app，减少假失败和中途产物漂移。
- 当前系统安装版仍是 v40，build/pkg 是 v41 cdhash `6d7e1033ef95597258f7c9c30f7d361f1b3dee2f`；真实 GUI smoke 仍等待系统安装版更新和 TIS readiness。

## v41 Mac mini 续修：system install 与 verifier/smoke 互斥

问题：

- `install-system.sh` 会构建、替换 `/Library/Input Methods/InputiaInputMethod.app` 和 `/Applications/Inputia 设置.app`，kill 旧 host，注册/启用/选择输入源，并刷新 TextInputMenuAgent / SystemUIServer。
- 即使 `build.sh` 已经避免构建期删除 build app，系统安装本身仍不能和 verifier 或 GUI smoke 并发；否则 smoke 可能在运行中遇到系统输入法 bundle、TIS 状态或当前输入源被替换。

实现：

- `install-system.sh`
  - 新增 `detect_verification_processes()`，扫描当前 workspace 下正在运行的 verifier / post-install / smoke / status / TIS readiness 脚本。
  - 在 admin gate、build、kill host、复制系统 app、刷新 TIS 之前调用 `require_no_verification_processes`。
  - 检测到 verifier/smoke 正在运行时输出：
    - `systemInstallReady=false reason=verification-running`
    - `systemInstallBlockingProcess: ...`
    - 并以 rc `20` 退出，不触发安装副作用。
  - 新增 `INPUTIA_INSTALL_PREFLIGHT_SELF_CHECK=1`，不触发真实安装，仅验证 clear / blocked 两个分支。
- `verify-nongui.sh`
  - 静态契约要求 install preflight 存在，且发生在 build 和 `killall InputiaInputMethod` 之前。
  - 动态运行 `INPUTIA_INSTALL_PREFLIGHT_SELF_CHECK=1 install-system.sh`，要求：
    - `systemInstallPreflightSelfCheck clear=true`
    - `systemInstallPreflightSelfCheck blocked=true`
    - `systemInstallPreflightSelfCheck=true`

验证：

```text
zsh -n macos/InputiaInputMethod/install-system.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_INSTALL_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/install-system.sh
  systemInstallPreflightSelfCheck clear=true
  systemInstallPreflightSelfCheck blocked=true
  systemInstallPreflightSelfCheck=true

INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-textedit.sh" macos/InputiaInputMethod/install-system.sh
  systemInstallReady=false reason=verification-running
  systemInstallBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-textedit.sh
  installRc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-install-preflight.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  systemInstallPreflightSelfCheck clear=true
  systemInstallPreflightSelfCheck blocked=true
  systemInstallPreflightSelfCheck=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 系统安装入口现在和 build 一样，会在 verifier/smoke 活动时拒绝执行，避免系统输入法 bundle 或 TIS 状态在 smoke 过程中被替换。
- 当前系统安装版仍是 v40，build/pkg 是 v41 cdhash `6d7e1033ef95597258f7c9c30f7d361f1b3dee2f`；真实 GUI smoke 仍等待系统安装版更新和 TIS readiness。

## v41 Mac mini 续修：新增真实 GUI smoke suite 门禁入口

背景：

- `post-install-regression.sh` 已经能在 `INPUTIA_RUN_UI_SMOKE=1` 时串起 TextEdit / Safari / Clipboard 真实 smoke，但它不是一个“先汇总 readiness，再决定是否启动前台 App”的单一入口。
- 当前系统安装版仍是 v40，而 build/pkg 是 v41；如果直接跑真实 GUI smoke，容易把系统旧版、TIS 未 ready、admin install 未完成和输入法行为问题混在一起。

实现：

- 新增 `gui-smoke-suite.sh`：
  - 先调用 `gui-smoke-readiness.sh`，把原始 readiness 输出统一加前缀 `guiSmokeSuiteReadiness:`。
  - 只有看到 `guiSmokeReadinessReady=true reason=none` 才输出 `guiSmokeSuiteWouldRun=true`，并以 `INPUTIA_RUN_UI_SMOKE=1` 委托 `post-install-regression.sh` 执行真实 GUI smoke。
  - 未 ready 时输出 `guiSmokeSuiteReady=false reason=<reason>`、`guiSmokeSuiteWouldRun=false`，以 rc `12` 退出，不启动 TextEdit/Safari。
  - 增加 `INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1`，覆盖 blocked / ready 两条分支；ready 分支通过 `INPUTIA_GUI_SMOKE_SUITE_SKIP_RUN_FOR_TEST=1` 证明 would-run，不实际打开 GUI。
- `build.sh` / `install-system.sh` 的 verifier/smoke 并发保护新增识别 `gui-smoke-readiness.sh` 和 `gui-smoke-suite.sh`，避免 suite/readiness 活动时替换 build app 或系统 app。
- `verify-nongui.sh`：
  - 语法检查纳入 `gui-smoke-suite.sh`。
  - 静态契约要求 suite 先读取 readiness，再通过 ready gate，最后才允许委托 `post-install-regression.sh`。
  - 动态运行 suite self-check，要求 blocked 分支 `guiSmokeSuiteWouldRun=false`，ready 分支 `guiSmokeSuiteWouldRun=true` 且只在 self-check 中跳过真实 GUI。
  - 残留扫描纳入 `gui-smoke-readiness.sh` 和 `gui-smoke-suite.sh`。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/install-system.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-suite.sh
  guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteWouldRun=false
  guiSmokeSuiteSelfCheck case=blocked rc=12
  guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteWouldRun=true
  guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteRunSkipped=true reason=self-check
  guiSmokeSuiteSelfCheck=true

macos/InputiaInputMethod/gui-smoke-suite.sh > /tmp/inputia-gui-smoke-suite-v41.log 2>&1
  guiSmokeSuiteRc=12
  target.version=40
  target.matchesBuild=false
  settings.systemVersion=40
  settings.matchesBuild=false
  pkg.ready=true
  tis.ready=false
  adminInstallReady=false reason=admin-required
  guiSessionBlockReason=none
  textEditPreflight=not-running
  safariPreflight=not-running
  guiSmokeReadinessReady=false reason=admin-required
  guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteWouldRun=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-suite-rerun.log 2>&1
  verifyRc=0
  guiSmokeSuiteSelfCheck=true
  postInstallUiTisGateNoLaunchPassed=true
  installNoPrompt: systemInstallReady=false reason=admin-required
  installNoPrompt.clipboardUnchanged=true
  installNoPrompt.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  installNoPrompt.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- 真实 GUI smoke 现在有单一安全入口：未 ready 时只报告原因，不启动前台 App；ready 后才进入既有 `post-install-regression.sh` 的真实 UI smoke 顺序。
- 当前 Mac mini 图形会话可读且未锁屏，但系统安装仍停在 v40，build/pkg 是 v41，且无无提示 admin 安装权限；因此本轮没有强跑 TextEdit/Safari/Clipboard 真实 smoke。

## v41 Mac mini 续修：Command 系统/应用快捷键透传边界

问题：

- 用户反馈 `Command-C` 和 `Command-V` 在 Inputia 下不可用，说明输入法不能只保护复制/粘贴两个点，而要把 macOS 上常用的 `Command` 系统/应用快捷键整体作为不接管边界。

官方依据：

- Apple Support `Mac keyboard shortcuts` 明确列出 `Command-X/C/V/Z/A/F/G/H/M/O/P/Q/S/T/W`、`Option-Command-Esc`、`Command-Space`、`Control-Command-Space`、`Command-Tab`、`Command-Comma`、`Control-Command-Q`、`Shift-Command-3/4/5`、Finder `Command-D/E/I/R/J/K/N/Y`、`Option-Command-V`、`Command-1/2/3/4`、`Command-[` / `Command-]`、`Command-Arrow`、`Command-Delete` 等常用快捷键。
- Apple HIG `Keyboards` 要求快捷键遵循平台预期；输入法 host 不应覆盖用户已经熟悉的标准 App / 系统快捷键。

实现边界：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut` 的规则保持为：任何包含 `.command` 的组合都返回 `true`。
- `InputiaInputController.handleKeyDown` 在处理剪贴板召回、标点切换、全半角切换、候选导航等输入法快捷键前，先遇到 `.command` 就 `return false`，交回 macOS / 前台 App。
- Inputia 自己的快捷键保留为不含 `Command` 的组合，例如 `Ctrl+Shift+V` 剪贴板召回；`Ctrl+Shift+Command+V` 明确拒绝接管。

验证：

```text
./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandHPassThrough=true
  commandMPassThrough=true
  commandPPassThrough=true
  commandTPassThrough=true
  commandNPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandNumberPassThrough=true
  commandBracketPassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandControlQPassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

./macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-command-shortcuts-20260708.log 2>&1
  verifyRc=0
  nonGuiVerificationPassed=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  awaitUiNotReady: version=40
  awaitUiNotReady: target.matchesBuild=false
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
```

当前结论：

- 这次不再逐个补 `Command-C` / `Command-V`，而是把所有含 `Command` 的常用系统/应用快捷键统一透传，并用代表性 keyCode + 全部 `Command` 修饰组合变体锁住。
- 当前 build 侧验证通过；由于系统安装仍需管理员权限，系统版仍是 v40，真实 TextEdit/Safari GUI smoke 继续等待系统安装版更新和 TIS readiness。

## v41 Mac mini 续修：debug event log 清洁断言

问题：

- `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 依赖 `INPUTIA_DEBUG_EVENTS` 判断 recall shown / commit / raw commit 是否发生。
- 之前公共 helper 已经会在外部提供日志时清空、内部临时日志时删除重建；但 smoke 脚本本身没有显式证明 prepare 后日志为空。若未来有人改坏 helper 或传入旧日志，可能让前一轮事件污染本轮断言。

实现：

- `smoke-common.sh`
  - 新增 `inputia_assert_debug_event_log_clean(event_log, ready_var, exit_code)`。
  - 若日志非空，输出：
    - `debugEventLogClean=false path=...`
    - `<ready_var>=false reason=debug-event-log-not-clean`
    - 最近 120 行日志
    - 并按调用方指定 rc 退出。
  - 空日志或不存在日志输出 `debugEventLogClean=true`。
- `smoke-clipboard-recall.sh`
  - 在 `inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"` 后、`launchctl setenv INPUTIA_DEBUG_EVENTS` 前调用清洁断言。
- `smoke-safari-enter.sh`
  - 同样在 prepare 后、setenv 前调用清洁断言。
- `verify-nongui.sh`
  - 静态合约要求公共 helper 存在。
  - 锁定顺序：`capture -> trap -> select input source -> prepare event log -> assert clean -> setenv -> restart host`。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=0 bash macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  clipboardRecallSmokeReady=false reason=ui-smoke-disabled
  rc=7

INPUTIA_RUN_UI_SMOKE=0 bash macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  safariEnterSmokeReady=false reason=ui-smoke-disabled
  rc=5

inputia_assert_debug_event_log_clean clean self-check
  cleanRc=0
  debugEventLogClean=true

inputia_assert_debug_event_log_clean dirty self-check
  dirtyRc=32
  debugEventLogClean=false path=/tmp/inputia-debug-log-dirty-selfcheck.54698
  dirtyReady=false reason=debug-event-log-not-clean

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-debug-log-clean-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  clipboardUiDisabled: clipboardRecallSmokeReady=false reason=ui-smoke-disabled
  safariEnterUiDisabled: safariEnterSmokeReady=false reason=ui-smoke-disabled
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- clipboard recall / Safari enter 的事件判断现在先证明本轮日志是干净起点，降低前一轮 composition / recall / commit 事件污染本轮 smoke 的风险。
- 真实 GUI smoke 仍未运行；当前阻塞仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：GUI smoke suite blocked 分支无污染证明

问题：

- `gui-smoke-suite.sh` 是真实 GUI smoke 的单一安全入口；上一轮已经证明 blocked 分支输出 `guiSmokeSuiteWouldRun=false`。
- 但 verifier 还没有像其它早退 gate 一样，在外层证明该 blocked 分支不会污染剪贴板、当前输入源、debug env、user host，也不会启动或残留 TextEdit/Safari/Inputia host。

实现：

- `verify-nongui.sh`
  - 新增 `GUI smoke suite blocked gate` 段。
  - 使用 `INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST="guiSmokeReadinessReady=false reason=admin-required"` 合成未 ready 状态，避免依赖当前机器权限状态，也避免真实 readiness 或 GUI App 启动。
  - 断言：
    - suite rc 为 `12`；
    - 输出 `guiSmokeSuiteReady=false reason=admin-required`；
    - 输出 `guiSmokeSuiteWouldRun=false`；
    - 剪贴板、当前输入源、debug env 不变；
    - 不创建 user host；
    - 不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='guiSmokeReadinessReady=false reason=admin-required' \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReadiness: guiSmokeReadinessReady=false reason=admin-required
  guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteWouldRun=false
  suiteRc=12

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-suite-blocked-gate.log 2>&1
  verifyRc=0
  guiSmokeSuiteBlockedGate: guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteBlockedGate: guiSmokeSuiteWouldRun=false
  guiSmokeSuiteBlockedGate.rc=12
  guiSmokeSuiteBlockedGate.clipboardUnchanged=true
  guiSmokeSuiteBlockedGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteBlockedGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteBlockedGate.debugEnvBefore=unset
  guiSmokeSuiteBlockedGate.debugEnvAfter=unset
  guiSmokeSuiteBlockedGate.userHost=false
  guiSmokeSuiteBlockedGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- 真实 GUI smoke 的统一入口现在不仅会在未 ready 时报告原因和拒绝启动，也由 non-GUI verifier 证明其 blocked 分支不污染用户状态。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍等待系统安装版更新到 v41、TIS readiness 通过、管理员安装权限可用。

## v41 Mac mini 续修：debug event log 清洁断言纳入 verifier 动态覆盖

问题：

- 上一轮已经给事件日志增加清洁断言，并手工验证 clean / dirty 分支。
- 但一次性手工命令不是长期回归保护；`verify-nongui.sh` 应该每次都动态验证 helper 的 clean / dirty 行为，避免未来误改导致 clipboard recall / Safari enter 的事件日志污染防线失效。

实现：

- 扩展 `verify-nongui.sh` 的 `debug event log lifecycle self-check`：
  - provided log 被 `inputia_prepare_debug_event_log` 清空后，立即调用 `inputia_assert_debug_event_log_clean`，要求输出 `debugEventLogClean=true`。
  - generated log 删除重建行为验证后，再写入 `stale-event`，用子 shell 调用 `inputia_assert_debug_event_log_clean`，要求：
    - rc 为 `32`
    - 输出 `debugEventLogClean=false`
    - 输出 `debugEventLogLifecycleReady=false reason=debug-event-log-not-clean`
  - 使用子 shell 是为了验证 helper 的退出行为，同时不直接终止主 verifier。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-debug-log-lifecycle-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  == debug event log lifecycle self-check ==
  debugEventLogClean=true
  debugEventLogDirty: debugEventLogClean=false path=/tmp/inputia-debug-event-generated.64900
  debugEventLogDirty: debugEventLogLifecycleReady=false reason=debug-event-log-not-clean
  debugEventLogDirty: stale-event
  debugEventLogLifecycleSelfCheck=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 事件日志清洁断言现在不只是脚本逻辑和手工证据，而是 `verify-nongui.sh` 的常规动态回归项。
- 真实 GUI smoke 仍未运行；阻塞条件仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：GUI smoke suite readiness 输出缺失分支无污染证明

问题：

- `gui-smoke-suite.sh` 已有 `readiness-output-missing` 分支，用于防止 `gui-smoke-readiness.sh` 异常、无输出或输出不可解析时继续进入真实 GUI smoke。
- 该分支已有 self-check，但还没有像 `admin-required` blocked 分支一样，由外层 verifier 证明不会污染用户状态或启动 GUI App。

实现：

- `verify-nongui.sh`
  - 新增 `GUI smoke suite missing-readiness gate` 段。
  - 使用空的 `INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=""` 合成 readiness 无有效输出状态。
  - 断言：
    - suite rc 为 `12`；
    - 输出 `guiSmokeSuiteReady=false reason=readiness-output-missing`；
    - 输出 `guiSmokeSuiteWouldRun=false`；
    - 剪贴板、当前输入源、debug env 不变；
    - 不创建 user host；
    - 不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='' \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReadiness:
  guiSmokeSuiteReady=false reason=readiness-output-missing
  guiSmokeSuiteWouldRun=false
  suiteRc=12

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-suite-missing-readiness-gate.log 2>&1
  verifyRc=0
  guiSmokeSuiteMissingReadinessGate: guiSmokeSuiteReady=false reason=readiness-output-missing
  guiSmokeSuiteMissingReadinessGate: guiSmokeSuiteWouldRun=false
  guiSmokeSuiteMissingReadinessGate.rc=12
  guiSmokeSuiteMissingReadinessGate.clipboardUnchanged=true
  guiSmokeSuiteMissingReadinessGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteMissingReadinessGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteMissingReadinessGate.debugEnvBefore=unset
  guiSmokeSuiteMissingReadinessGate.debugEnvAfter=unset
  guiSmokeSuiteMissingReadinessGate.userHost=false
  guiSmokeSuiteMissingReadinessGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- 真实 GUI smoke 的统一入口现在覆盖两类不应启动的异常：readiness 明确 blocked，以及 readiness 输出缺失/不可解析；两者都由 non-GUI verifier 证明不污染用户状态。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍等待系统安装版更新到 v41、TIS readiness 通过、管理员安装权限可用。

## v41 Mac mini 续修：GUI smoke suite 空 readiness 输出不运行保护

问题：

- `gui-smoke-suite.sh` 负责统一读取 `gui-smoke-readiness.sh`，只有 readiness 明确 `guiSmokeReadinessReady=true` 时才允许跑 `post-install-regression.sh` 的真实 GUI smoke。
- 代码已有 `readiness-output-missing` 分支，但原 self-check 只覆盖 blocked / ready 两类；如果 readiness 脚本异常返回空输出，缺少动态回归证明 suite 会拒绝运行 GUI smoke。

实现：

- `gui-smoke-suite.sh`
  - 测试注入变量 `INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST` 改为按“变量是否存在”判断，而不是按“非空”判断，因此可以模拟空 readiness 输出。
  - `INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1` 新增 `missing` case：
    - readiness 输出为空
    - 期望 rc `12`
    - 期望 `guiSmokeSuiteReady=false reason=readiness-output-missing`
    - 期望 `guiSmokeSuiteWouldRun=false`
- `verify-nongui.sh`
  - 要求 suite self-check 输出 missing case 的 ready / would-run / rc marker。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-suite.sh
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteReadiness:
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteReady=false reason=readiness-output-missing
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteWouldRun=false
  guiSmokeSuiteSelfCheck case=missing rc=12
  guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteRunSkipped=true reason=self-check
  guiSmokeSuiteSelfCheck=true

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='' macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReadiness:
  guiSmokeSuiteReady=false reason=readiness-output-missing
  guiSmokeSuiteWouldRun=false
  rc=12

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-gui-suite-missing-readiness-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteReady=false reason=readiness-output-missing
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteWouldRun=false
  guiSmokeSuiteSelfCheck case=missing rc=12
  guiSmokeSuiteSelfCheck=true
  guiSmokeSuiteBlockedGateNoMutationPassed=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI smoke suite 现在对 readiness 脚本异常空输出有动态回归保护，空输出不会误触发 TextEdit/Safari/Clipboard GUI smoke。
- 真实 GUI smoke 仍未运行；阻塞条件仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：GUI smoke suite post-install 失败显式传播

问题：

- `gui-smoke-suite.sh` 的 ready 路径会委托 `post-install-regression.sh` 执行真实 TextEdit / Safari / Clipboard GUI smoke。
- 原实现依赖 `set -e` 隐式中断；如果 post-install regression 失败，suite 没有明确输出 `guiSmokeSuitePassed=false`，也缺少动态 self-check 证明失败 rc 会向上传播且不会误报 passed。

实现：

- `gui-smoke-suite.sh`
  - 新增 `run_post_install_regression()`，生产路径仍执行：
    - `env INPUTIA_RUN_UI_SMOKE=1 "$POST_INSTALL_REGRESSION" "$APP"`
  - ready 路径显式捕获 post-install rc：
    - rc 非 0 时输出 `guiSmokeSuitePassed=false reason=post-install-regression-failed rc=<rc>` 并返回原 rc。
    - rc 为 0 时才输出 `guiSmokeSuitePassed=true`。
  - `INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1` 新增 `post-install-failure` case：
    - 合成 readiness ready。
    - 通过 `INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23` 模拟 post-install 失败。
    - 要求 suite 返回 rc `23`，并输出明确 failure marker。
- `verify-nongui.sh`
  - 静态合约更新为：真实 post-install 命令保留在 helper 中，`run_gui_smoke_suite` 的 ready gate 后调用 helper。
  - 动态要求 self-check 覆盖 `post-install-failure` case。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-suite.sh
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuiteWouldRun=true
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  guiSmokeSuiteSelfCheck case=post-install-failure rc=23
  guiSmokeSuiteSelfCheck=true

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='guiSmokeReadinessReady=true reason=none' \
  INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23 \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteWouldRun=true
  guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  rc=23

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-gui-suite-post-install-failure-20260708-r4.log 2>&1
  verifyRc=0
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  guiSmokeSuiteSelfCheck case=post-install-failure rc=23
  guiSmokeSuiteSelfCheck=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI smoke suite 的 ready 路径现在对 post-install regression 失败有明确 marker 和 rc 传播验证，不会在下游失败时误报 `guiSmokeSuitePassed=true`。
- 真实 GUI smoke 仍未运行；阻塞条件仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：GUI smoke suite ready-skip 分支无污染证明

问题：

- `gui-smoke-suite.sh` 的 ready 分支负责在 readiness 明确通过后进入真实 `post-install-regression.sh` GUI smoke。
- 当前系统仍无法真实进入该路径，但可以用合成 readiness + skip-run 模式证明：当 suite 判断 would-run 时，测试模式不会额外污染用户状态，也不会绕过既有 post-install 入口。

实现：

- `verify-nongui.sh`
  - 静态契约按当前 `gui-smoke-suite.sh` 结构调整：
    - `run_gui_smoke_suite()` 主函数必须先读取 readiness，再输出 ready gate，最后才调用 `run_post_install_regression`。
    - `run_post_install_regression()` wrapper 内必须以 `INPUTIA_RUN_UI_SMOKE=1` 委托 `"$POST_INSTALL_REGRESSION" "$APP"`。
  - 新增 `GUI smoke suite ready-skip gate` 段：
    - 使用 `INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST="guiSmokeReadinessReady=true reason=none"` 合成 ready 状态。
    - 设置 `INPUTIA_GUI_SMOKE_SUITE_SKIP_RUN_FOR_TEST=1`，避免真实打开 GUI。
    - 断言 suite 输出 `guiSmokeSuiteWouldRun=true`、`guiSmokeSuiteRunSkipped=true reason=self-check`、`guiSmokeSuitePassed=true`、rc `0`。
    - 断言剪贴板、当前输入源、debug env 不变，不创建 user host，不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod。

验证：

```text
INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='guiSmokeReadinessReady=true reason=none' \
  INPUTIA_GUI_SMOKE_SUITE_SKIP_RUN_FOR_TEST=1 \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReadiness: guiSmokeReadinessReady=true reason=none
  guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteWouldRun=true
  guiSmokeSuiteRunSkipped=true reason=self-check
  guiSmokeSuitePassed=true
  suiteRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-suite-ready-skip-gate.log 2>&1
  verifyRc=0
  guiSmokeSuiteReadySkipGate: guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteReadySkipGate: guiSmokeSuiteWouldRun=true
  guiSmokeSuiteReadySkipGate: guiSmokeSuiteRunSkipped=true reason=self-check
  guiSmokeSuiteReadySkipGate: guiSmokeSuitePassed=true
  guiSmokeSuiteReadySkipGate.rc=0
  guiSmokeSuiteReadySkipGate.clipboardUnchanged=true
  guiSmokeSuiteReadySkipGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteReadySkipGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteReadySkipGate.debugEnvBefore=unset
  guiSmokeSuiteReadySkipGate.debugEnvAfter=unset
  guiSmokeSuiteReadySkipGate.userHost=false
  guiSmokeSuiteReadySkipGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- suite 的三类可离线分支现在都被 verifier 覆盖：readiness 缺失、readiness blocked、readiness ready 但测试跳过真实 GUI。
- 真正打开 TextEdit/Safari/Clipboard 的路径仍只允许通过 `post-install-regression.sh`，且仍等待系统安装版更新到 v41、TIS readiness 通过、管理员安装权限可用。

## v41 Mac mini 续修：GUI smoke suite post-install 失败分支无污染证明

问题：

- `gui-smoke-suite.sh` 已能在 ready 路径中显式传播 `post-install-regression.sh` 失败。
- 还需要证明这个 failure-injection 路径不会污染用户状态：即便 readiness 合成为 ready、suite 进入 would-run 分支，只要下游失败被模拟，测试路径也不能改剪贴板、当前输入源、debug env，不能启动或残留 TextEdit / Safari / host。

实现：

- `verify-nongui.sh`
  - 新增 `GUI smoke suite post-install failure gate` 段。
  - 使用：
    - `INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST="guiSmokeReadinessReady=true reason=none"`
    - `INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23`
  - 要求 suite 输出：
    - `guiSmokeSuiteReady=true reason=none`
    - `guiSmokeSuiteWouldRun=true`
    - `guiSmokeSuitePostInstallForTest=true rc=23`
    - `guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23`
    - rc `23`
  - 然后断言：
    - 剪贴板不变
    - 当前输入源不变
    - `INPUTIA_DEBUG_EVENTS` launchctl 环境不变
    - 不创建 user host
    - 不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='guiSmokeReadinessReady=true reason=none' \
  INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23 \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteWouldRun=true
  guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  rc=23

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-gui-suite-post-install-failure-gate-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuiteReady=true reason=none
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuiteWouldRun=true
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  guiSmokeSuitePostInstallFailureGate.rc=23
  guiSmokeSuitePostInstallFailureGate.clipboardUnchanged=true
  guiSmokeSuitePostInstallFailureGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuitePostInstallFailureGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuitePostInstallFailureGate.debugEnvBefore=unset
  guiSmokeSuitePostInstallFailureGate.debugEnvAfter=unset
  guiSmokeSuitePostInstallFailureGate.userHost=false
  guiSmokeSuitePostInstallFailureGateNoMutationPassed=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- suite 的四类可离线分支现在都被 verifier 覆盖：readiness 缺失、readiness blocked、readiness ready 但测试跳过真实 GUI、readiness ready 但下游 post-install 失败。
- 真实 GUI smoke 仍未运行；阻塞条件仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：post-install 活锁拒绝分支无污染证明

问题：

- `post-install-regression.sh` 使用 `/tmp/inputia-post-install-regression.lock` 防止并发回归互相抢 TextEdit/Safari/Clipboard 或 TIS 状态。
- 之前已经有锁路径、stale lock 和 already-running marker 的静态/历史证据；但当前 verifier 还缺少动态 no-mutation gate，证明活锁存在时即使请求 `INPUTIA_RUN_UI_SMOKE=1`，脚本也会在进入 verify-system/TIS/UI smoke 之前拒绝。

实现：

- `verify-nongui.sh`
  - 新增 `post-install active lock gate` 段。
  - 创建临时 lock 目录 `/tmp/inputia-post-install-active-lock.$$`，写入当前 verifier pid，模拟另一个活跃回归持锁。
  - 运行：
    - `INPUTIA_POST_INSTALL_LOCK_DIR=<temp-lock>`
    - `INPUTIA_RUN_UI_SMOKE=1`
    - `post-install-regression.sh <build app>`
  - 要求 rc `5`，输出 `postInstallRegressionReady=false reason=already-running pid=<current pid>`。
  - 断言剪贴板、当前输入源、debug env 不变，不创建 user host，不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod，并清理临时 lock。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

# 手工 active-lock gate
INPUTIA_POST_INSTALL_LOCK_DIR=/tmp/inputia-post-install-active-lock-manual.<pid> \
  INPUTIA_RUN_UI_SMOKE=1 \
  macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallRegressionReady=false reason=already-running pid=<shell-pid>
  rc=5

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-post-install-active-lock-gate-20260708.log 2>&1
  verifyRc=0
  postInstallActiveLockGate: postInstallRegressionReady=false reason=already-running pid=40083
  postInstallActiveLockGate.rc=5
  postInstallActiveLockGate.clipboardUnchanged=true
  postInstallActiveLockGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallActiveLockGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallActiveLockGate.debugEnvBefore=unset
  postInstallActiveLockGate.debugEnvAfter=unset
  postInstallActiveLockGate.userHost=false
  postInstallActiveLockGateNoMutationPassed=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- post-install 回归入口的活锁拒绝现在也有动态无污染证明；并发回归不会继续进入真实 GUI smoke 或系统/TIS 状态读取链。
- 真实 GUI smoke 仍未运行；阻塞条件仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：verify-nongui 临时目录统一清理

问题：

- `verify-nongui.sh` 已有 `VERIFY_TEMP_FILES` 和退出 trap，用来清理临时文件。
- 新增的 `post-install active lock gate` 会创建 `/tmp/inputia-post-install-active-lock.$$` 目录。正常路径会显式删除，但如果中途断言失败，旧逻辑没有统一临时目录清理 registry，可能留下 `/tmp/inputia-*` 目录并污染后续 residue gate。

实现：

- `verify-nongui.sh`
  - 新增 `VERIFY_TEMP_DIRS=()`。
  - 新增 `cleanup_verify_temp_dirs()`，只清理注册且路径匹配 `/tmp/inputia-*` 的目录，避免误删非 Inputia 路径。
  - `cleanup_verify()` 在清理临时文件后调用临时目录清理，再释放 verifier lock。
  - `post-install active lock gate` 创建 `post_install_active_lock_dir` 后立即注册到 `VERIFY_TEMP_DIRS`。
  - `cleanupPermissionContract` 静态要求：
    - `VERIFY_TEMP_DIRS=()`
    - `cleanup_verify_temp_dirs`
    - `${VERIFY_TEMP_DIRS[@]+"${VERIFY_TEMP_DIRS[@]}"}`
    - `VERIFY_TEMP_DIRS+=("$post_install_active_lock_dir")`

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-temp-dir-cleanup-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  postInstallActiveLockGate: postInstallRegressionReady=false reason=already-running pid=60698
  postInstallActiveLockGate.rc=5
  postInstallActiveLockGate.clipboardUnchanged=true
  postInstallActiveLockGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallActiveLockGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallActiveLockGate.debugEnvBefore=unset
  postInstallActiveLockGate.debugEnvAfter=unset
  postInstallActiveLockGate.userHost=false
  postInstallActiveLockGateNoMutationPassed=true
  postInstallUiTisGate: guiSmokeReady=false reason=tis-not-ready
  installNoPrompt: systemInstallReady=false reason=admin-required
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `verify-nongui.sh` 现在同时有临时文件和临时目录的统一清理路径，active-lock gate 即使未来中途失败也不会长期留下 `/tmp/inputia-post-install-active-lock.*`。
- 真实 GUI smoke 仍未运行；阻塞条件仍是系统安装版 v40 与 build v41 不一致、TIS 未就绪、系统安装需要管理员权限。

## v41 Mac mini 续修：GUI smoke suite post-install 失败分支无污染证明

问题：

- `gui-smoke-suite.sh` 的 ready 路径已经显式传播 `post-install-regression.sh` 的失败 rc，并输出 `guiSmokeSuitePassed=false reason=post-install-regression-failed`。
- 但此前只有 suite self-check 证明 rc/marker；还缺少外层 verifier 证明“合成 post-install 失败”本身不会污染剪贴板、当前输入源、debug env、user host，也不会残留 GUI 进程。

实现：

- `verify-nongui.sh`
  - 新增 `GUI smoke suite post-install failure gate` 段。
  - 使用 `INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST="guiSmokeReadinessReady=true reason=none"` 合成 ready 状态。
  - 使用 `INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23` 模拟 `post-install-regression.sh` 失败。
  - 断言：
    - suite 输出 `guiSmokeSuiteReady=true reason=none`；
    - suite 输出 `guiSmokeSuiteWouldRun=true`；
    - test hook 输出 `guiSmokeSuitePostInstallForTest=true rc=23`；
    - suite 输出 `guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23`；
    - suite rc 为 `23`；
    - 剪贴板、当前输入源、debug env 不变；
    - 不创建 user host；
    - 不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST='guiSmokeReadinessReady=true reason=none' \
  INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23 \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteReadiness: guiSmokeReadinessReady=true reason=none
  guiSmokeSuiteReady=true reason=none
  guiSmokeSuiteWouldRun=true
  guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  suiteRc=23

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-suite-post-install-failure-gate.log 2>&1
  verifyRc=0
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuiteReady=true reason=none
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuiteWouldRun=true
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuitePostInstallForTest=true rc=23
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23
  guiSmokeSuitePostInstallFailureGate.rc=23
  guiSmokeSuitePostInstallFailureGate.clipboardUnchanged=true
  guiSmokeSuitePostInstallFailureGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuitePostInstallFailureGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuitePostInstallFailureGate.debugEnvBefore=unset
  guiSmokeSuitePostInstallFailureGate.debugEnvAfter=unset
  guiSmokeSuitePostInstallFailureGate.userHost=false
  guiSmokeSuitePostInstallFailureGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- suite ready 分支的成功跳过、post-install 失败传播、blocked、missing-readiness 分支都已有 verifier 动态覆盖和无污染证明。
- 真实 GUI smoke 仍未运行；仍需系统安装版更新到 v41、TIS readiness 通过、管理员安装权限可用。

## v41 Mac mini 续修：GUI smoke readiness 真实运行只读证明

问题：

- `gui-smoke-suite.sh` 依赖 `gui-smoke-readiness.sh` 的输出决定是否进入真实 GUI smoke。
- 此前 readiness 有 reason self-check，但缺少外层 verifier 证明“真实 readiness 汇总运行”本身不污染用户状态；如果 readiness 在读取 TIS、pkg、GUI session 或进程状态时改变剪贴板/输入源/debug env，就会污染后续 smoke 判断。

实现：

- `verify-nongui.sh`
  - 新增 `GUI smoke readiness current gate` 段。
  - 真实运行 `gui-smoke-readiness.sh "$BUILD_APP"`，要求输出 `guiSmokeReadinessReady=...`。
  - 运行前后断言：
    - 剪贴板不变；
    - 当前输入源不变；
    - `INPUTIA_DEBUG_EVENTS` launchctl 环境不变；
    - 不创建 user host；
    - 不启动/残留 TextEdit、Safari、osascript、InputiaInputMethod。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  tis.readinessBlockReason=missing-enabled-source
  tisReadiness=false
  adminInstallReady=false reason=admin-required
  guiSessionBlockReason=none
  textEditPreflight=not-running
  safariPreflight=not-running
  guiSmokeReadinessReady=false reason=admin-required

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-v41-gui-readiness-current-gate-rerun.log 2>&1
  verifyRc=0
  guiSmokeReadinessCurrent: target.matchesBuild=true
  guiSmokeReadinessCurrent: settings.matchesBuild=false
  guiSmokeReadinessCurrent: pkg.ready=true
  guiSmokeReadinessCurrent: tis.ready=false
  guiSmokeReadinessCurrent: adminInstallReady=false reason=admin-required
  guiSmokeReadinessCurrent: guiSmokeReadinessReady=false reason=admin-required
  guiSmokeReadinessCurrent.clipboardUnchanged=true
  guiSmokeReadinessCurrent.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeReadinessCurrent.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeReadinessCurrent.debugEnvBefore=unset
  guiSmokeReadinessCurrent.debugEnvAfter=unset
  guiSmokeReadinessCurrent.userHost=false
  guiSmokeReadinessCurrentNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- suite 上游 readiness 汇总现在也被证明为只读 gate；真实 GUI smoke 的进入条件从 readiness 到 suite 再到 post-install 都有 non-GUI 无污染证明。
- 真实 GUI smoke 仍未运行；仍需系统安装版更新到 v41、TIS readiness 通过、管理员安装权限可用。

## v41 Mac mini 续验：post-install 活锁 gate 当前完整验证

问题：

- 继续复核 `post-install-regression.sh` 已有活锁时的二次触发纪律：即使请求 `INPUTIA_RUN_UI_SMOKE=1`，也必须在进入 `verify-system`、TIS 切换或 GUI smoke 前早退。
- 早退路径必须不污染剪贴板、当前输入源、`INPUTIA_DEBUG_EVENTS`、user host，也不能启动或残留 TextEdit/Safari/InputiaInputMethod/osascript。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-post-install-active-lock-current.log 2>&1
  verifyRc=0
  guiSmokeReadinessCurrentNoMutationPassed=true
  guiSmokeSuiteMissingReadinessGateNoMutationPassed=true
  guiSmokeSuiteBlockedGateNoMutationPassed=true
  guiSmokeSuiteReadySkipGateNoMutationPassed=true
  guiSmokeSuitePostInstallFailureGateNoMutationPassed=true
  postInstallActiveLockGate: postInstallRegressionReady=false reason=already-running pid=65268
  postInstallActiveLockGate.rc=5
  postInstallActiveLockGate.clipboardUnchanged=true
  postInstallActiveLockGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallActiveLockGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallActiveLockGate.debugEnvBefore=unset
  postInstallActiveLockGate.debugEnvAfter=unset
  postInstallActiveLockGate.userHost=false
  postInstallActiveLockGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- post-install 活锁二次触发现在有当前完整 verifier 证据：活锁路径以 rc=5 早退，未进入 GUI smoke，也未污染用户状态。
- 真实 GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40、构建版为 v41，且无非交互管理员安装权限。

## v41 Mac mini 续验：Command 常用快捷键不接管边界复核

问题：

- 用户反馈 `Command-C` / `Command-V` 在 Inputia 下不可用，并明确要求按常用电脑快捷键举一反三，不要逐个等用户测试。
- 复核策略不能只特殊处理复制/粘贴，而要把 macOS 中以 `Command` 为主修饰键的系统/App 快捷键统一交还给宿主 App。

外部依据：

- Apple Support `Copy and paste on Mac` 明确 `Command-C` 复制、`Command-V` 粘贴、`Command-Z` 撤销。
  - https://support.apple.com/guide/mac-help/copy-and-paste-on-mac-mchl5252f3de/mac
- Apple Support `Keyboard shortcuts on your Mac` 列出常见 `Command-X/C/V/Z/A/F/G/H/M/N/O/P/S/W/Q`、`Command-Tab`、`Command-Shift-3/4/5`、`Command-Option-Esc` 等系统/App 快捷键。
  - https://support.apple.com/guide/imac/keyboard-shortcuts-apd194062a6d/mac
- Apple Support `Keyboard shortcuts in Terminal on Mac` 显示 Terminal 也大量使用 `Command` 及 `Command` + `Shift` / `Option` / `Control` 变体，例如复制、粘贴、查找、跳转、书签和设置。
  - https://support.apple.com/guide/terminal/keyboard-shortcuts-trmlshtcts/mac
- Apple InputMethodKit 文档说明输入法通过 Input Method Kit 管理与客户端应用的通信；在当前 IMK `handle(_ event:client:)` 路径中，返回 `false` 是把事件交还给客户端的必要边界。
  - https://developer.apple.com/documentation/inputmethodkit

实现复核：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 仍保持为单一规则：任何包含 `.command` 的 keyDown 都返回 `true`。
- `InputiaController.handleKeyDown` 在剪贴板召回、标点切换、全半角切换、中英文切换、候选导航之前先调用该规则；命中后清掉 Shift toggle armed 状态并 `return false`。
- Inputia 自有快捷键继续限定在不含 `Command` 的组合：
  - 剪贴板召回：`Ctrl-Shift-V`；
  - `Ctrl-Shift-Command-V` 明确拒绝，避免抢宿主粘贴变体；
  - 候选上下翻页、裸数字 raw composition fallback 也拒绝含 `Command` 的组合。

验证：

```text
macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandControlVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandNumberPassThrough=true
  commandBracketPassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/build.sh
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-clipboard-cleanup-self-check-20260708.log 2>&1
  nonGuiVerificationPassed=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  shortcut: allCommandModifierVariantsPassThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  hostShortcut: allCommandModifierVariantsPassThrough=true
  postInstall: commandCPassThrough=true
  postInstall: commandVPassThrough=true
  postInstall: allCommandModifierVariantsPassThrough=true
  residue=false
  tmpResidue=false
```

当前结论：

- 当前 v41 build 已经不是逐个抢救 `Command-C` / `Command-V`，而是统一透传所有含 `Command` 的系统/App 快捷键组合。
- 如果用户当前机器上仍复现，最可能原因是系统安装版仍为 v40；当前 build 为 v41，系统 `/Library/Input Methods/InputiaInputMethod.app` 和 `/Applications/Inputia 设置.app` 仍显示 v40，且本环境没有非交互管理员权限完成系统安装替换。

## v41 Mac mini 续修：Clipboard recall 写入后失败路径 cleanup 自检

问题：

- Clipboard recall smoke 已经有触发前/选择前 event guard、双 Escape 清状态和成功路径清理 marker。
- 但还缺一个非 GUI 动态证明：在脚本已经写入测试剪贴板、设置 `INPUTIA_DEBUG_EVENTS`、创建临时日志后，如果主 AppleScript 失败，`EXIT` trap 是否仍能恢复剪贴板/debug env 并清理临时文件。

实现：

- `smoke-clipboard-recall.sh`
  - 新增 `INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK=1` 自检分支。
  - 分支位于 `trap cleanup_smoke EXIT` 之后、输入源选择和 TextEdit GUI 操作之前。
  - 自检会模拟：
    - 保存原剪贴板；
    - 写入测试剪贴板并设置 `CLIPBOARD_CHANGED=1`；
    - 创建 clipboard recall 的 event/select/restore/osascript 临时文件；
    - 临时设置 `INPUTIA_DEBUG_EVENTS`；
    - 输出 `clipboardRecallCleanupSelfCheck=true phase=after-clipboard-write`；
    - 以默认 rc `23` 退出，由 `EXIT` trap 负责恢复和清理。
- `verify-nongui.sh`
  - 静态契约区分 self-check 路径和主路径，主路径仍要求 `select -> pbpaste -> pbcopy -> CLIPBOARD_CHANGED -> osascript` 顺序。
  - 新增 `clipboard recall cleanup self-check` 动态段，使用：
    - `INPUTIA_RUN_UI_SMOKE=1`
    - `INPUTIA_SKIP_GUI_SESSION_CHECK=1`
    - `INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1`
    - `INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK=1`
  - 断言 rc 为 `23`，并验证剪贴板、当前输入源、debug env、user host、临时文件和 GUI/host 进程均未残留。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST='Unicode text, 42' \
INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  clipboardRestorable=true
  clipboardRecallCleanupSelfCheck=true phase=after-clipboard-write
  selfCheckRc=23
  clipboardRestored=true
  debugEnvRestored=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-clipboard-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  clipboardRecallCleanupSelfCheck: guiSessionCheck=skipped
  clipboardRecallCleanupSelfCheck: textEditPreflight=not-running docs=0
  clipboardRecallCleanupSelfCheck: clipboardRestorable=true
  clipboardRecallCleanupSelfCheck: clipboardRecallCleanupSelfCheck=true phase=after-clipboard-write
  clipboardRecallCleanupSelfCheck.rc=23
  clipboardRecallCleanupSelfCheck.clipboardUnchanged=true
  clipboardRecallCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardRecallCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardRecallCleanupSelfCheck.debugEnvBefore=unset
  clipboardRecallCleanupSelfCheck.debugEnvAfter=unset
  clipboardRecallCleanupSelfCheck.userHost=false
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- Clipboard recall smoke 现在不仅有成功路径状态污染 guard，也有“写入后失败路径”的动态 cleanup 证明。
- 真实 Clipboard/TextEdit/Safari GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40、构建版为 v41，且无非交互管理员安装权限。

## v41 Mac mini 当前回合：GUI smoke 清理纪律与状态污染完整非 GUI 复验

问题：

- 继续推进当前目标：在真实 GUI smoke 仍受系统安装/TIS 状态阻塞时，先用非 GUI verifier 锁住 TextEdit/Safari/Clipboard smoke 的清理纪律、状态污染防线和证据记录。
- 本轮还复核用户反馈的 `Command-C` / `Command-V` 不应被接管问题，确认当前 build 仍保持“所有含 `Command` 的系统/App 快捷键透传”边界。

当前系统状态：

```text
build/InputiaInputMethod.app: version=41 cdhash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
/Library/Input Methods/InputiaInputMethod.app: version=40 cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
/Applications/Inputia 设置.app: version=40 cdhash=9a9c915e7fddecac15d59112b96dae0405f5943d
current input source: com.tencent.inputmethod.wetype.pinyin
target TIS: com.inputia.inputmethod.Inputia.Hans enabled=true selected=false
readiness block: target-cdhash-mismatch / missing-enabled-source
admin install gate: systemInstallReady=false reason=admin-required
```

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
zsh -n macos/InputiaInputMethod/post-install-regression.sh
zsh -n macos/InputiaInputMethod/smoke-textedit.sh
zsh -n macos/InputiaInputMethod/smoke-clipboard-recall.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-current-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  shortcut: allCommandModifierVariantsPassThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  hostShortcut: allCommandModifierVariantsPassThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  tempDirCleanupSelfCheck=true
  guiSmokeReadinessCurrentNoMutationPassed=true
  guiSmokeSuiteMissingReadinessGateNoMutationPassed=true
  guiSmokeSuiteBlockedGateNoMutationPassed=true
  guiSmokeSuiteReadySkipGateNoMutationPassed=true
  guiSmokeSuitePostInstallFailureGateNoMutationPassed=true
  postInstallActiveLockGateNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  postInstall: allCommandModifierVariantsPassThrough=true
  postInstall: commandCPassThrough=true
  postInstall: commandVPassThrough=true
  postInstallUiTisGate: allCommandModifierVariantsPassThrough=true
  postInstallUiTisGate: commandCPassThrough=true
  postInstallUiTisGate: commandVPassThrough=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari main app=not-running
InputiaInputMethod=not-running
osascript=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- 非 GUI 证据继续证明：TextEdit/Safari/Clipboard smoke 的已运行早退、cleanup trap、debug event log 清理、剪贴板恢复、临时文件/目录清理、post-install 活锁和 GUI suite gate 均未污染剪贴板、当前输入源、debug env 或 user host。
- `Command-C` / `Command-V` 及其它含 `Command` 的常用系统/App 快捷键在当前 build 和 post-install dry path 均保持透传。
- 真实 GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40、build 为 v41，且本环境没有非交互管理员权限完成系统安装替换与 TIS readiness。

## v41 Mac mini 续修：Safari Command 写入后失败路径 cleanup 自检

问题：

- `smoke-safari-command-shortcuts.sh` 和 Clipboard recall 一样会临时写系统剪贴板。
- 既有 verifier 已覆盖 UI disabled、TIS not ready、已有 Safari 和非文本剪贴板等早退路径；但还缺一个非 GUI 动态证明：脚本已经写入测试剪贴板并创建临时文件后，如果后续 Safari AppleScript 失败，`EXIT` trap 是否仍能恢复剪贴板并清理临时文件。

实现：

- `smoke-safari-command-shortcuts.sh`
  - 新增 `INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK=1` 自检分支。
  - 分支位于 `trap cleanup_smoke EXIT` 之后、输入源选择和 Safari GUI 操作之前。
  - 自检会模拟：
    - 保存原剪贴板；
    - 写入测试剪贴板并设置 `CLIPBOARD_CHANGED=1`；
    - 创建 Safari command 的 url/select/restore/osascript 临时文件；
    - 输出 `safariCommandCleanupSelfCheck=true phase=after-clipboard-write`；
    - 以默认 rc `24` 退出，由 `EXIT` trap 负责恢复和清理。
- `verify-nongui.sh`
  - 静态契约要求 Safari command cleanup self-check marker 和 rc override 存在。
  - 剪贴板顺序契约改为从主路径 `inputia_select_input_source_or_exit` 之后查找主路径 `pbpaste/pbcopy/CLIPBOARD_CHANGED`，避免 self-check 故障注入路径误伤主路径判断。
  - 新增 `safari command cleanup self-check` 动态段，允许已有 Safari 但不启动 Safari，断言：
    - rc 为 `24`；
    - 剪贴板、当前输入源、debug env 不变；
    - 不创建 user host；
    - 不留下 `inputia-safari-command-*` 临时文件；
    - 不残留 osascript/InputiaInputMethod；若本轮开始 Safari 未运行，也断言未启动 Safari。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST='Unicode text, 42' \
INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  clipboardRestorable=true
  safariCommandCleanupSelfCheck=true phase=after-clipboard-write
  selfCheckRc=24
  clipboardRestored=true
  debugEnvUnchanged=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-command-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  safariCommandCleanupSelfCheck: guiSessionCheck=skipped
  safariCommandCleanupSelfCheck: safariPreflight=not-running
  safariCommandCleanupSelfCheck: clipboardRestorable=true
  safariCommandCleanupSelfCheck: safariCommandCleanupSelfCheck=true phase=after-clipboard-write
  safariCommandCleanupSelfCheck.rc=24
  safariCommandCleanupSelfCheck.clipboardUnchanged=true
  safariCommandCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariCommandCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariCommandCleanupSelfCheck.debugEnvBefore=unset
  safariCommandCleanupSelfCheck.debugEnvAfter=unset
  safariCommandCleanupSelfCheck.userHost=false
  safariCommandCleanupSelfCheckNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- Safari command smoke 现在也有“写入剪贴板后失败路径”的动态 cleanup 证明，和 Clipboard recall 的防污染强度对齐。
- 真实 Safari/TextEdit/Clipboard GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40、构建版为 v41，且无非交互管理员安装权限。

## v41 Mac mini 续修：TextEdit Command 写入后失败路径 cleanup 自检

问题：

- `smoke-textedit-command-shortcuts.sh` 会临时清空并写入系统剪贴板，用来验证 `Command-A/C/V` 透传。
- Clipboard recall 和 Safari command 已经有“写入后失败路径”的动态 cleanup 证明；TextEdit command 也需要同等保护，避免 AppleScript 失败后污染剪贴板、当前输入源或留下临时文件。

实现：

- `smoke-textedit-command-shortcuts.sh`
  - 新增 `INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK=1` 自检分支。
  - 分支位于 `trap cleanup_textedit_command_smoke EXIT` 之后、输入源选择和 TextEdit GUI 操作之前。
  - 自检会模拟：
    - 保存原剪贴板；
    - 写入测试剪贴板并设置 `CLIPBOARD_CHANGED=1`；
    - 创建 TextEdit command 的 select/restore/osascript 临时文件；
    - 输出 `textEditCommandCleanupSelfCheck=true phase=after-clipboard-write`；
    - 以默认 rc `25` 退出，由 `EXIT` trap 负责恢复和清理。
- `verify-nongui.sh`
  - 静态契约要求 TextEdit command cleanup self-check marker 和 rc override 存在。
  - 新增 `textedit command cleanup self-check` 动态段，允许已有 TextEdit 但不启动 TextEdit，断言：
    - rc 为 `25`；
    - 剪贴板、当前输入源、debug env 不变；
    - 不创建 user host；
    - 不留下 `inputia-textedit-command-*` 临时文件；
    - 不残留 osascript/InputiaInputMethod；若本轮开始 TextEdit 未运行，也断言未启动 TextEdit。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST='Unicode text, 42' \
INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  clipboardRestorable=true
  textEditCommandCleanupSelfCheck=true phase=after-clipboard-write
  selfCheckRc=25
  clipboardRestored=true
  debugEnvUnchanged=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-textedit-command-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  textEditCommandCleanupSelfCheck: guiSessionCheck=skipped
  textEditCommandCleanupSelfCheck: textEditPreflight=not-running docs=0
  textEditCommandCleanupSelfCheck: clipboardRestorable=true
  textEditCommandCleanupSelfCheck: textEditCommandCleanupSelfCheck=true phase=after-clipboard-write
  textEditCommandCleanupSelfCheck.rc=25
  textEditCommandCleanupSelfCheck.clipboardUnchanged=true
  textEditCommandCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCommandCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCommandCleanupSelfCheck.debugEnvBefore=unset
  textEditCommandCleanupSelfCheck.debugEnvAfter=unset
  textEditCommandCleanupSelfCheck.userHost=false
  textEditCommandCleanupSelfCheckNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- 三条写系统剪贴板的重点 GUI smoke：TextEdit command、Clipboard recall、Safari command，现在都有写入后失败路径的动态 cleanup 证明。
- 真实 Safari/TextEdit/Clipboard GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40、构建版为 v41，且无非交互管理员安装权限。

## v41 Mac mini 续修：Safari Enter debug env 写入后失败路径 cleanup 自检

问题：

- `smoke-safari-enter.sh` 不写系统剪贴板，但会设置 `INPUTIA_DEBUG_EVENTS`，并创建 event/url/select/restore/osascript 临时文件。
- Clipboard recall 已经有事件日志与 debug env 写入后的 cleanup 证明；Safari enter 也依赖事件日志判断 `commit=abc`，需要同等防污染证明。
- 本轮首次完整验证还暴露 `residue collector self-check` 在 bash 3.2 + `set -u` 下空数组计数会触发 `remaining_fake_pids[@]: unbound variable`，导致 verifier 尾部失败。

实现：

- `smoke-safari-enter.sh`
  - 新增 `INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK=1` 自检分支。
  - 分支位于 `trap cleanup_smoke EXIT` 之后、真实输入源选择和 Safari GUI 操作之前。
  - 自检会模拟：
    - 创建 event/url/select/restore/osascript 临时文件；
    - 临时设置 `INPUTIA_DEBUG_EVENTS`；
    - 输出 `safariEnterCleanupSelfCheck=true phase=after-debug-env-write`；
    - 以默认 rc `26` 退出，由 `EXIT` trap 负责恢复 debug env 并清理临时文件。
- `verify-nongui.sh`
  - 静态契约要求 Safari enter cleanup self-check marker 和 rc override 存在。
  - 新增 `safari enter cleanup self-check` 动态段，断言：
    - rc 为 `26`；
    - 剪贴板、当前输入源、debug env 不变；
    - 不创建 user host；
    - 不留下 `inputia-safari-enter*` 临时文件；
    - 不残留 osascript/InputiaInputMethod；若本轮开始 Safari 未运行，也断言未启动 Safari。
  - residue collector 的 fake pid 数组回写改为显式 `remaining_fake_count`，避免空数组在 `set -u` 下报错。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-enter.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  safariEnterCleanupSelfCheck=true phase=after-debug-env-write
  selfCheckRc=26
  clipboardUnchanged=true
  debugEnvRestored=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-enter-cleanup-self-check-20260708-rerun.log 2>&1
  verifyRc=0
  safariEnterCleanupSelfCheck: guiSessionCheck=skipped
  safariEnterCleanupSelfCheck: safariPreflight=not-running
  safariEnterCleanupSelfCheck: safariEnterCleanupSelfCheck=true phase=after-debug-env-write
  safariEnterCleanupSelfCheck.rc=26
  safariEnterCleanupSelfCheck.clipboardUnchanged=true
  safariEnterCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterCleanupSelfCheck.debugEnvBefore=unset
  safariEnterCleanupSelfCheck.debugEnvAfter=unset
  safariEnterCleanupSelfCheck.userHost=false
  safariEnterCleanupSelfCheckNoMutationPassed=true
  textEditCommandCleanupSelfCheckNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- Safari enter 现在也有 debug env 写入后失败路径的动态 cleanup 证明；事件日志类 smoke 的失败路径防污染覆盖更完整。
- residue collector self-check 在 bash 3.2 + `set -u` 下稳定通过。
- 真实 Safari/TextEdit/Clipboard GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40、构建版为 v41，且无非交互管理员安装权限。

## v41 Mac mini 当前回合最终复验：非 GUI gate 通过且无残留

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-current-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  shortcut: allCommandModifierVariantsPassThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  hostShortcut: allCommandModifierVariantsPassThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  tempDirCleanupSelfCheck=true
  guiSmokeReadinessCurrentNoMutationPassed=true
  guiSmokeSuiteMissingReadinessGateNoMutationPassed=true
  guiSmokeSuiteBlockedGateNoMutationPassed=true
  guiSmokeSuiteReadySkipGateNoMutationPassed=true
  guiSmokeSuitePostInstallFailureGateNoMutationPassed=true
  postInstallActiveLockGateNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  postInstall: allCommandModifierVariantsPassThrough=true
  postInstall: commandCPassThrough=true
  postInstall: commandVPassThrough=true
  postInstallUiTisGate: allCommandModifierVariantsPassThrough=true
  postInstallUiTisGate: commandCPassThrough=true
  postInstallUiTisGate: commandVPassThrough=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari main app=not-running
InputiaInputMethod=not-running
osascript=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue count=0
```

当前结论：

- 当前工作区的 TextEdit/Safari/Clipboard smoke 清理纪律与状态污染防线，在非 GUI verifier 中重新通过。
- 真实 GUI smoke 仍未运行；原因仍是系统安装版/settings 为 v40、build 为 v41，且本环境没有非交互管理员权限完成系统安装替换与 TIS readiness。

## v41 Mac mini 续修：verifier residue collector 不漏 shell wrapper

问题：

- 手工残留检查曾被 Codex 通知进程参数里的旧消息文本误匹配，说明按整行命令文本搜索存在误报风险。
- 复核 `verify-nongui.sh` 后发现另一个更实际的问题：`collect_residue()` 为避免 shell `-c` 噪声，直接跳过 `/bin/bash -c` / `/bin/zsh -c`，这可能漏掉真正卡住的 smoke shell wrapper。

实现：

- `verify-nongui.sh`
  - `collect_residue()` 不再跳过所有 shell `-c` wrapper。
  - 现在只有当命令行包含 Inputia smoke/verifier 脚本路径，且 `comm` 是 `bash` / `zsh` / `sh` / `env` 时，才视为脚本残留。
  - `osascript` 和 `InputiaInputMethod` 仍按进程名直接视为残留。
  - 增加 `residue collector self-check`：
    - 启动 `/bin/zsh /tmp/smoke-textedit.sh`，要求 `collect_residue` 能抓到该 shell wrapper；
    - 启动 `/usr/bin/python3 ... /tmp/smoke-textedit.sh`，要求 `collect_residue` 忽略这种“参数文本含脚本名但进程不是 shell wrapper”的情况。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-residue-collector-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  residueCollectorShellWrapperDetected=true
  residueCollectorArgumentTextIgnored=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-residue-collector-final-20260708.log 2>&1
  verifyRc=0
  guiSmokeReadinessCurrentNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  residueCollectorShellWrapperDetected=true
  residueCollectorArgumentTextIgnored=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- verifier 的残留检测现在不靠粗暴全行搜索，也不再漏掉 shell wrapper 形式的卡住 smoke。
- 真实 GUI smoke 仍未运行；当前系统安装/TIS 阻塞不变。

## v41 Mac mini 续修：GUI readiness 输出并列阻塞原因

问题：

- `gui-smoke-readiness.sh` 原先只输出一个主阻塞原因，例如当前环境会给出 `guiSmokeReadinessReady=false reason=admin-required`。
- 这会掩盖同一时间存在的其他阻塞：系统 settings 仍是 v40、构建版是 v41、TIS 当前找不到 Inputia source。

实现：

- `gui-smoke-readiness.sh`
  - 新增 `readiness_block_reasons()`，保留原来的单一 `reason=` 输出作为兼容字段。
  - 新增 `guiSmokeReadinessBlockReasons=`，用于输出并列阻塞原因。
  - self-check 增加多阻塞场景，当前期望为 `settings-version-mismatch,admin-required,tis-not-ready`。
- `verify-nongui.sh`
  - 静态契约新增 `readiness_block_reasons()`、`guiSmokeReadinessBlockReasons=`、`guiSmokeReadinessSelfCheck blockReasons=` 检查，避免 readiness 输出退化。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReadinessSelfCheck blockReasons=settings-version-mismatch,admin-required,tis-not-ready actual=settings-version-mismatch,admin-required,tis-not-ready

macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  target.matchesBuild=true
  settings.matchesBuild=false
  tis.ready=false
  adminInstallReady=false reason=admin-required
  guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessReady=false reason=admin-required

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-block-reasons-20260708.log 2>&1
  verifyRc=0
  guiSmokeReadinessSelfCheck blockReasons=settings-version-mismatch,admin-required,tis-not-ready actual=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessCurrent: guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  safariEnterCleanupSelfCheckNoMutationPassed=true
  textEditCommandCleanupSelfCheckNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- GUI readiness 现在能同时说明所有已知阻塞，便于判断是否能安全进入真实 TextEdit/Safari/Clipboard GUI smoke。
- 兼容字段 `guiSmokeReadinessReady=false reason=admin-required` 保留不变。
- 当前仍不能运行真实 GUI smoke：系统安装版/settings 为 v40，构建版为 v41；TIS readiness 不满足；本环境没有非交互管理员安装权限。

## v41 Mac mini 续修：主 TextEdit smoke trap 后失败路径 cleanup 自检

问题：

- TextEdit command、Clipboard recall、Safari command 和 Safari enter 已有写入/调试环境变更后的失败路径 cleanup 自检。
- 主 `smoke-textedit.sh` 虽然不改剪贴板和 debug env，但会创建 select/restore/osascript 临时文件，并在真实路径中可能启动 TextEdit；需要一个非 GUI 动态证明：trap 建立后、输入源选择和 TextEdit GUI 操作前失败时不会留下临时文件或启动 TextEdit。

实现：

- `smoke-textedit.sh`
  - 新增 `INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK=1` 分支。
  - 分支位于 `trap cleanup_textedit_smoke EXIT` 之后、`inputia_select_input_source_or_exit` 之前。
  - 自检创建 `inputia-textedit-select/restore/osascript` 临时文件，输出 `textEditCleanupSelfCheck=true phase=after-temp-write`，并以默认 rc `27` 退出，由 `EXIT` trap 清理。
- `verify-nongui.sh`
  - 静态契约要求 self-check marker、rc override 和顺序：trap -> self-check -> input source select。
  - 新增 `textedit cleanup self-check` 动态段，断言 rc、剪贴板、当前输入源、debug env、user host、TextEdit/osascript/InputiaInputMethod 进程和 `/private/tmp/inputia-textedit-*` 临时文件均无污染。

验证：

```text
INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-textedit.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  textEditCleanupSelfCheck=true phase=after-temp-write
  selfCheckRc=27
  clipboardUnchanged=true
  tisBefore=com.tencent.inputmethod.wetype.pinyin
  tisAfter=com.tencent.inputmethod.wetype.pinyin

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-textedit-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  guiSmokeReadinessCurrentNoMutationPassed=true
  textEditCleanupSelfCheck: textEditCleanupSelfCheck=true phase=after-temp-write
  textEditCleanupSelfCheck.rc=27
  textEditCleanupSelfCheck.clipboardUnchanged=true
  textEditCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  textEditCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  textEditCleanupSelfCheck.debugEnvBefore=unset
  textEditCleanupSelfCheck.debugEnvAfter=unset
  textEditCleanupSelfCheck.userHost=false
  textEditCleanupSelfCheckNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 主 TextEdit smoke 现在也有 trap 后、GUI 前失败路径的动态 cleanup 证明；它不会留下临时文件、启动 TextEdit、污染剪贴板/当前输入源/debug env/user host。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍未运行；当前系统安装/TIS 阻塞不变。

## v41 Mac mini 续修：Safari typing smoke trap 后失败路径 cleanup 自检

问题：

- `smoke-safari-typing.sh` 不写剪贴板/debug env，但会创建 Safari 测试 URL、select/restore log 和 AppleScript 临时文件，并在真实路径中可能启动 Safari。
- 需要和主 TextEdit smoke 对齐：证明 trap 建立后、输入源选择和 Safari GUI 操作前失败时，不会留下临时文件或启动 Safari。

实现：

- `smoke-safari-typing.sh`
  - 新增 `INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK=1` 分支。
  - 分支位于 `inputia_require_safari_idle` 之后、`inputia_select_input_source_or_exit` 之前。
  - 自检创建 `inputia-safari-typing-test/select/restore/osascript` 临时文件，输出 `safariTypingCleanupSelfCheck=true phase=after-temp-write`，并以默认 rc `28` 退出，由 `EXIT` trap 清理。
- `verify-nongui.sh`
  - 静态契约要求 self-check marker、rc override 和顺序：trap -> self-check -> input source select。
  - 新增 `safari typing cleanup self-check` 动态段，断言 rc、剪贴板、当前输入源、debug env、user host、Safari/osascript/InputiaInputMethod 进程和 `/private/tmp/inputia-safari-typing-*` 临时文件均无污染。

验证：

```text
INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-safari-typing.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  safariTypingCleanupSelfCheck=true phase=after-temp-write
  selfCheckRc=28
  clipboardUnchanged=true
  tisBefore=com.tencent.inputmethod.wetype.pinyin
  tisAfter=com.tencent.inputmethod.wetype.pinyin

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-typing-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  textEditCleanupSelfCheckNoMutationPassed=true
  safariTypingCleanupSelfCheck: safariTypingCleanupSelfCheck=true phase=after-temp-write
  safariTypingCleanupSelfCheck.rc=28
  safariTypingCleanupSelfCheck.clipboardUnchanged=true
  safariTypingCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariTypingCleanupSelfCheck.debugEnvBefore=unset
  safariTypingCleanupSelfCheck.debugEnvAfter=unset
  safariTypingCleanupSelfCheck.userHost=false
  safariTypingCleanupSelfCheckNoMutationPassed=true
  safariEnterCleanupSelfCheckNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari typing smoke 现在也有 trap 后、GUI 前失败路径的动态 cleanup 证明；它不会留下临时文件、启动 Safari、污染剪贴板/当前输入源/debug env/user host。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍未运行；当前系统安装/TIS 阻塞不变。

## v41 Mac mini 续修：Safari typing smoke trap 后失败路径 cleanup 自检

问题：

- TextEdit、Clipboard recall、Safari command 和 Safari enter 已有 trap 后失败路径 cleanup 自检。
- `smoke-safari-typing.sh` 仍缺同等级动态证明；该脚本会创建 Safari URL、select、restore、osascript 临时文件，真实路径还会打开 Safari。若未来失败路径漂移，可能留下临时文件或误启动/残留 Safari。

实现：

- `smoke-safari-typing.sh`
  - 新增 `INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK=1` 分支。
  - 分支位于 `trap cleanup_smoke EXIT` 之后、`inputia_select_input_source_or_exit` 和 Safari GUI 操作之前。
  - 自检写入 `inputia-safari-typing-*` 临时文件，输出 `safariTypingCleanupSelfCheck=true phase=after-temp-write`，并以默认 rc `28` 退出，由 `EXIT` trap 清理。
- `verify-nongui.sh`
  - 静态契约要求 self-check marker、rc override 和顺序：trap -> self-check -> input source select。
  - 新增 `safari typing cleanup self-check` 动态段，断言 rc、剪贴板、当前输入源、debug env、user host、Safari/osascript/InputiaInputMethod 进程和 `/private/tmp/inputia-safari-typing-*` 临时文件均无污染。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-typing.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-safari-typing.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  safariTypingCleanupSelfCheck=true phase=after-temp-write
  selfCheckRc=28

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-typing-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  safariTypingCleanupSelfCheck: safariTypingCleanupSelfCheck=true phase=after-temp-write
  safariTypingCleanupSelfCheck.rc=28
  safariTypingCleanupSelfCheck.clipboardUnchanged=true
  safariTypingCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariTypingCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariTypingCleanupSelfCheck.debugEnvBefore=unset
  safariTypingCleanupSelfCheck.debugEnvAfter=unset
  safariTypingCleanupSelfCheck.userHost=false
  safariTypingCleanupSelfCheckNoMutationPassed=true
  safariEnterCleanupSelfCheckNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari typing smoke 现在也有 trap 后、GUI 前失败路径的动态 cleanup 证明。
- 真实 Safari typing GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini 续修：Safari input-source diagnosis trap 后失败路径 cleanup 自检

问题：

- `diagnose-safari-input-source.sh` 是 Safari 相关 GUI 诊断入口，会读取 HIToolbox 偏好、选择 Inputia、打开 Safari data: 页面并检查 focused input 的 Text Input Source。
- 主 smoke 脚本已有 trap 后失败路径 cleanup 自检，但该诊断入口此前只有静态窗口关闭契约和已有 Safari gate，缺少动态证明：trap 建立后、真正选择输入源和打开 Safari 前失败时不会留下临时文件或污染状态。

实现：

- `diagnose-safari-input-source.sh`
  - 新增 `INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK=1` 分支。
  - 分支位于 `trap cleanup_diagnosis EXIT` 之后、`inputia_select_input_source_or_exit` 之前。
  - 自检写入 URL、HIToolbox、source/focused select、restore、osascript 临时文件，输出 `safariDiagnoseCleanupSelfCheck=true phase=after-temp-write`，并以默认 rc `29` 退出，由 `EXIT` trap 清理。
- `verify-nongui.sh`
  - 静态契约要求 self-check marker、rc override 和顺序：trap -> self-check -> input source select。
  - 新增 `safari diagnose cleanup self-check` 动态段，断言 rc、剪贴板、当前输入源、debug env、user host、Safari/osascript/InputiaInputMethod 进程，以及全部 diagnosis 临时文件均无污染。

验证：

```text
bash -n macos/InputiaInputMethod/diagnose-safari-input-source.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/diagnose-safari-input-source.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  safariDiagnoseCleanupSelfCheck=true phase=after-temp-write
  selfCheckRc=29

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-diagnose-cleanup-self-check-20260708.log 2>&1
  verifyRc=0
  safariDiagnoseCleanupSelfCheck: safariDiagnoseCleanupSelfCheck=true phase=after-temp-write
  safariDiagnoseCleanupSelfCheck.rc=29
  safariDiagnoseCleanupSelfCheck.clipboardUnchanged=true
  safariDiagnoseCleanupSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseCleanupSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariDiagnoseCleanupSelfCheck.debugEnvBefore=unset
  safariDiagnoseCleanupSelfCheck.debugEnvAfter=unset
  safariDiagnoseCleanupSelfCheck.userHost=false
  safariDiagnoseCleanupSelfCheckNoMutationPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari input-source diagnosis 入口现在也有 trap 后、GUI 前失败路径的动态 cleanup 证明。
- 真实 Safari diagnosis GUI 路径仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini：宿主 App Command 快捷键统一放行

问题：

- 用户反馈在 Inputia 下 `Command-C` / `Command-V` 不能用，说明输入法可能接管了宿主 App 的复制/粘贴快捷键。
- 这类问题不能逐个等用户手测；需要按 macOS 常用快捷键整体处理，避免输入法吃掉系统/App 级 `Command` 组合。

依据：

- Apple 官方《Mac keyboard shortcuts》列出常见系统和 App 快捷键，包括 `Command-X/C/V/Z/A/F/G/H/M/O/P/S/T/W/Q`、`Command-Tab`、`Command-Space`、`Command-Shift-3/4/5`、`Command-Option-Esc` 等。
- 结论：Inputia 的 IME 自有快捷键不得使用 `Command` 作为接管前缀；带 `Command` 的键盘事件应先返回给宿主 App/系统。

实现状态：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 对任何包含 `.command` 的组合返回 `true`。
- `handleKeyDown` 在剪贴板召回、标点切换、全半角切换、输入模式切换、候选导航之前先检查 `Command` 并返回 `false`。
- Inputia 自有快捷键继续保留但显式排除 `Command`：
  - `Ctrl+Shift+V`：剪贴板召回。
  - `Ctrl+.`：中英文标点切换。
  - `Shift+Space`：全半角切换。
  - `Ctrl+Space`：可配置中英文切换。
  - 候选窗口上下箭头：仅无 `Command/Control/Option/Shift` 时处理。

验证：

```text
bash macos/InputiaInputMethod/build.sh
  buildRc=0
  buildVersion=41
  buildCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f

macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandHPassThrough=true
  commandMPassThrough=true
  commandPPassThrough=true
  commandTPassThrough=true
  commandNPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandNumberPassThrough=true
  commandBracketPassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandControlQPassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftCommandVRejected=true
  candidateDownArrowRejectedWithCommand=true

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftCommandVRejected=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-preflight-tis-gate-20260708.log 2>&1
  verifyRc=0
  shortcutSelfCheck=true
  hostShortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 本地构建版已按“任何 `Command` 组合全部放行”的策略覆盖常见 macOS 快捷键，`Command-C/V` 不再由 Inputia 接管。
- 真实 TextEdit/Safari GUI smoke 仍未重新运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini：GUI smoke 清理纪律与 readiness 当前复核

问题：

- 真实 TextEdit/Safari/Clipboard GUI smoke 仍不能硬跑，必须先证明当前系统状态与门禁结论。
- 前面已分别补齐 TextEdit、Clipboard recall、Safari typing、Safari command、Safari enter 和 Safari diagnose 的失败路径 cleanup 自检；需要用当前工作区再跑一次完整非 GUI verifier，确认这些契约没有漂移。

当前状态：

```text
bash macos/InputiaInputMethod/status.sh
  buildVersion=41
  buildCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  system.version=40
  system.cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  settings.systemVersion=40
  systemSettingsMatchesBuildVersion=false
  tis.includeAllInstalled=false matches=0
  tis.includeAllInstalled=true matches=0
  running=false
  package.sha256=e6af057c5199c590a6eba4529439738cd4919e0d1be42530b7776ddc3b16858c
```

验证：

```text
bash -n \
  macos/InputiaInputMethod/verify-nongui.sh \
  macos/InputiaInputMethod/smoke-textedit.sh \
  macos/InputiaInputMethod/smoke-textedit-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh \
  macos/InputiaInputMethod/smoke-safari-typing.sh \
  macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh \
  macos/InputiaInputMethod/smoke-safari-enter.sh \
  macos/InputiaInputMethod/diagnose-safari-input-source.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-all-block-reasons-20260708.log 2>&1
  verifyRc=0
  guiSmokeReadinessSelfCheck=true
  guiSmokeReadinessSelfCheck blockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessSelfCheck allBlockReasons=settings-version-mismatch,admin-required,tis-not-ready,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessCurrentNoMutationPassed=true
  textEditCleanupSelfCheckNoMutationPassed=true
  safariTypingCleanupSelfCheckNoMutationPassed=true
  safariEnterCleanupSelfCheckNoMutationPassed=true
  textEditCommandCleanupSelfCheckNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  safariCommandCleanupSelfCheckNoMutationPassed=true
  safariDiagnoseCleanupSelfCheckNoMutationPassed=true
  buildPreflightUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  installNoPrompt.clipboardUnchanged=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前工作区的 GUI smoke 清理纪律仍通过：失败路径不改剪贴板、不改当前输入源、不污染 debug env、不留下 TextEdit/Safari/osascript/InputiaInputMethod 进程、不留下 `/tmp/inputia-*` 脚本临时文件。
- 真实 GUI smoke 仍被正确阻断，主因是系统安装版/settings 仍为 v40、构建版为 v41，且 TIS enabled/installed matches 为 0；没有非交互管理员权限时不得硬跑 TextEdit/Safari/Clipboard GUI smoke。

## v41 Mac mini 续修：smoke preflight TIS gate no-launch 证明

问题：

- `smoke-preflight.sh` 是进入真实 TextEdit/Safari/Clipboard GUI smoke 前的预检入口。
- 之前 verifier 已覆盖系统 cdhash mismatch 和 UI disabled 的 no-mutation 路径，但缺少更接近真实入口的一层动态证明：当 UI smoke 被允许、GUI session gate 放行、TextEdit/Safari 均空闲，但 TIS readiness 未满足时，preflight 必须停在 `tis-not-ready`，且不能启动 GUI app、污染当前输入源或留下 host。

实现：

- `verify-nongui.sh`
  - 新增 `buildPreflightUiTisGate` 动态段。
  - 使用 `INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1` 让 preflight 走到 TIS readiness gate。
  - 断言输出包含：
    - `guiSessionCheck=skipped`
    - `textEditPreflight=not-running`
    - `safariPreflight=not-running`
    - `smokePreflightReady=false reason=tis-not-ready`
  - 断言剪贴板、当前输入源、debug env、user host、TextEdit/Safari/osascript/InputiaInputMethod 均无污染。

验证：

```text
INPUTIA_RUN_UI_SMOKE=1 \
INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SKIP_CDHASH_CHECK=1 \
  macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  safariPreflight=not-running
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.readinessBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  smokePreflightReady=false reason=tis-not-ready
  preflightRc=8

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-preflight-tis-gate-20260708.log 2>&1
  verifyRc=0
  buildPreflightUiTisGate: guiSessionCheck=skipped
  buildPreflightUiTisGate: textEditPreflight=not-running docs=0
  buildPreflightUiTisGate: safariPreflight=not-running
  buildPreflightUiTisGate: smokePreflightReady=false reason=tis-not-ready
  buildPreflightUiTisGate.rc=8
  buildPreflightUiTisGate.clipboardUnchanged=true
  buildPreflightUiTisGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  buildPreflightUiTisGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  buildPreflightUiTisGate.debugEnvBefore=unset
  buildPreflightUiTisGate.debugEnvAfter=unset
  buildPreflightUiTisGate.userHost=false
  buildPreflightUiTisGateNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `smoke-preflight.sh` 现在在最接近真实 GUI smoke 的 TIS readiness 阻塞路径上也有 no-launch/no-mutation 证明。
- 真实 GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini 续修：GUI readiness 并列阻塞原因去重与组合自检

问题：

- `gui-smoke-readiness.sh` 已输出 `guiSmokeReadinessBlockReasons=`，但原 self-check 只覆盖 settings/admin/TIS 的组合。
- 真实进入 GUI smoke 前还需要同时看见 GUI session、TextEdit 已运行、Safari 已运行等阻塞；同时 `admin-required` 可能由 target/settings 多处触发，重复输出会降低诊断清晰度。

实现：

- `gui-smoke-readiness.sh`
  - `readiness_block_reasons()` 的 `append_reason` 改为去重追加。
  - 新增 `allBlockReasons` self-check，组合覆盖：
    - `settings-version-mismatch`
    - `admin-required`
    - `tis-not-ready`
    - `screen-locked`
    - `textedit-already-running`
    - `safari-already-running`
  - 自检同时断言 `admin-required` 不重复。
- `verify-nongui.sh`
  - 静态契约要求 `guiSmokeReadinessSelfCheck allBlockReasons=` 和去重检查存在。
  - 动态 verifier 要求 readiness self-check 输出上述全部阻塞原因。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 \
  macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReadinessSelfCheck blockReasons=settings-version-mismatch,admin-required,tis-not-ready actual=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessSelfCheck allBlockReasons=settings-version-mismatch,admin-required,tis-not-ready,screen-locked,textedit-already-running,safari-already-running actual=settings-version-mismatch,admin-required,tis-not-ready,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessSelfCheck=true

macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessReady=false reason=admin-required

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-all-block-reasons-20260708.log 2>&1
  verifyRc=0
  guiSmokeReadinessSelfCheck allBlockReasons=settings-version-mismatch,admin-required,tis-not-ready,screen-locked,textedit-already-running,safari-already-running actual=settings-version-mismatch,admin-required,tis-not-ready,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessCurrentNoMutationPassed=true
  buildPreflightUiTisGateNoLaunchPassed=true
  safariDiagnoseCleanupSelfCheckNoMutationPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI readiness 现在能在组合阻塞场景下给出完整、去重的阻塞原因列表，降低后续真实 GUI smoke 前误判风险。
- 真实 GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini 续修：GUI readiness 去重样例覆盖 target/settings 双 admin 来源

问题：

- 上一轮 `allBlockReasons` self-check 已加入 `admin-required` 去重断言，但样例只让 settings mismatch 触发 admin 阻塞，没有同时触发 target mismatch 和 settings mismatch 两个 admin 来源。
- 这意味着去重逻辑存在但测试样例不够强，未来如果 target/settings 同时不匹配时重复输出 `admin-required`，self-check 可能发现不了。

实现：

- `gui-smoke-readiness.sh`
  - 将 `allBlockReasons` 样例改为 `target.matchesBuild=false` 且 `settings.matchesBuild=false`，同时 `adminInstallReady=false`。
  - 期望列表加入 `target-cdhash-mismatch`，并继续要求 `admin-required` 只出现一次。
- `verify-nongui.sh`
  - 动态 readiness self-check 的必选原因列表同步加入 `target-cdhash-mismatch`。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 \
  macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReadinessSelfCheck allBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,screen-locked,textedit-already-running,safari-already-running actual=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-dedupe-true-20260708.log 2>&1
  verifyRc=0
  guiSmokeReadinessSelfCheck allBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,screen-locked,textedit-already-running,safari-already-running actual=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessCurrentNoMutationPassed=true
  buildPreflightUiTisGateNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- readiness self-check 现在真正覆盖 target/settings 同时不匹配导致的双 admin 来源，并证明输出去重。
- 真实 GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini 续修：await-system-install 输出并列 UI smoke 阻塞原因

问题：

- `await-system-install.sh` 是安装后等待系统版生效并决定是否进入 `post-install-regression.sh` UI smoke 的入口。
- 原输出只有 `uiSmokeBlockReason=` 单一主因；当前真实状态里 target cdhash mismatch 和 TIS not ready 同时存在，只报 `target-cdhash-mismatch` 会隐藏后续仍需处理的 TIS 阻塞。

实现：

- `await-system-install.sh`
  - 新增 `uiSmokeBlockReasons=` 兼容字段；保留原 `uiSmokeBlockReason=` 主因字段不变。
  - UI disabled、TIS not ready、GUI session blocker、TextEdit/Safari already running、ready path 均输出对应 block reasons。
  - target mismatch 路径会同时根据 TIS 状态附加 `tis-not-ready`。
  - 新增 `append_block_reason()`，避免并列原因重复。
  - `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 新增 `target-and-tis` 组合自检。
- `verify-nongui.sh`
  - 静态契约要求 `uiSmokeBlockReasons=`、去重 helper 和 `target-and-tis` self-check。
  - 动态 verifier 要求：
    - UI disabled 输出 `uiSmokeBlockReasons=ui-smoke-disabled`
    - GUI session blockers 输出各自 `uiSmokeBlockReasons=<reason>`
    - 当前安装未 ready 路径输出 `uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready`

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=target-and-tis uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  awaitUiStatusSelfCheck reason=no-console-user uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=no-console-user uiSmokeBlockReasons=no-console-user
  awaitUiStatusSelfCheck reason=login-not-complete uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=login-not-complete uiSmokeBlockReasons=login-not-complete
  awaitUiStatusSelfCheck reason=screen-locked uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=screen-locked uiSmokeBlockReasons=screen-locked
  awaitUiStatusSelfCheck reason=frontmost-unavailable uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=frontmost-unavailable uiSmokeBlockReasons=frontmost-unavailable
  awaitUiStatusSelfCheck reason=loginwindow-frontmost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=loginwindow-frontmost uiSmokeBlockReasons=loginwindow-frontmost
  awaitUiStatusSelfCheck=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
  macos/InputiaInputMethod/await-system-install.sh
  uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  systemInstallObserved=false reason=timeout
  systemInstallTargetMatchesBuild=false
  systemInstallTISReady=false reason=target-cdhash-mismatch

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-block-reasons-20260708.log 2>&1
  verifyRc=0
  awaitShort: uiSmokeRequested=false uiSmokeWouldStart=false uiSmokeBlockReason=ui-smoke-disabled uiSmokeBlockReasons=ui-smoke-disabled
  awaitUiNotReady: uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  awaitUiNotReadyNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 安装等待入口现在能同时暴露 target mismatch 和 TIS not ready，避免后续误以为只需更新系统 app 即可直接进入真实 GUI smoke。
- 真实 GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini：latest pkg 安装前契约复核

问题：

- 真实 GUI smoke 的主阻塞仍是系统安装版/settings 未更新到 v41、TIS readiness 为 0，以及当前没有非交互管理员权限。
- 在等待可交互安装前，需要确认当前 `InputiaInputMethod-latest.pkg` 本身不是阻塞源：包内 app/settings 版本、CDHash、postinstall 行为和自检应与构建产物一致。

验证：

```text
bash macos/InputiaInputMethod/verify-pkg.sh macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  packageVersion=41
  buildVersion=41
  pkgSignature=none
  sourcePostinstallSHA256=a21cbca02317bd85e9a60651384be3b57718e77b2468a621d0c907779813a5dd
  pkgPostinstallSHA256=a21cbca02317bd85e9a60651384be3b57718e77b2468a621d0c907779813a5dd
  postinstallBehaviorChecks=true
  inputiaPostinstallSelfCheckCase=ready passed=true
  inputiaPostinstallSelfCheckCase=missing passed=true
  inputiaPostinstallSelfCheckCase=id-mismatch passed=true
  inputiaPostinstallSelfCheckCase=not-enabled passed=true
  inputiaPostinstallSelfCheckCase=not-selectable passed=true
  inputiaPostinstallSelfCheckCase=icon-mismatch passed=true
  inputiaPostinstallSelfCheck=true
  appArchivePresent=true
  settingsArchivePresent=true
  archiveAppVersion=41
  archiveAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  buildAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
  archiveSettingsVersion=41
  buildSettingsVersion=41
  archiveExecutables=true
  pkgVerificationPassed=true

pkgutil --check-signature macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  Status: no signature
```

当前结论：

- latest pkg 与 v41 构建产物一致，postinstall 自检和行为契约通过；当前阻塞不是包内容不一致。
- 该 pkg 仍未签名，当前适合作为本机验证包；真实分发前仍需要签名/公证链路。
- 真实 GUI smoke 仍未运行；需要系统安装版/settings 更新到 v41 且 TIS readiness 通过后再跑。

## v41 Mac mini：post-install GUI smoke 入口 TIS gate 独立复核

问题：

- `post-install-regression.sh` 是系统安装后进入真实 TextEdit/Safari/Clipboard GUI smoke 的最终入口。
- 在系统安装版仍未 ready 时，需要独立证明：即使显式请求 `INPUTIA_RUN_UI_SMOKE=1`，入口也会先停在 TIS readiness gate，不进入任何真实 GUI smoke section，不污染用户状态。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 \
  macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiPreflightSelfCheck case=textedit-block TextEditPreflight=running
  postInstallUiPreflightSelfCheck case=textedit-block guiSmokeReady=false reason=textedit-already-running
  postInstallUiPreflightSelfCheck case=textedit-block postInstallUiSmokeReady=false reason=textedit-already-running
  postInstallUiPreflightSelfCheck case=textedit-block rc=4
  postInstallUiPreflightSelfCheck case=safari-block SafariPreflight=running
  postInstallUiPreflightSelfCheck case=safari-block guiSmokeReady=false reason=safari-already-running
  postInstallUiPreflightSelfCheck case=safari-block postInstallUiSmokeReady=false reason=safari-already-running
  postInstallUiPreflightSelfCheck case=safari-block rc=4
  postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflightAllowed=true
  postInstallUiPreflightSelfCheck case=safari-allow SafariPreflightAllowed=true
  postInstallUiPreflightSelfCheck=true

INPUTIA_RUN_UI_SMOKE=1 \
  macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  uiTisGateRc=6
  postInstallTISReady=false
  postInstallTISBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready
  clipboardUnchanged=true
  currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  currentSourceUnchanged=true
  debugEnvBefore=unset
  debugEnvAfter=unset
  debugEnvUnchanged=true
  TextEditRunning=false
  SafariRunning=false
  osascriptRunning=false
  InputiaRunning=false
```

当前结论：

- post-install GUI smoke 入口仍按正确顺序执行：system verification -> TIS readiness -> GUI session / TextEdit / Safari preflight -> 真实 GUI smoke。
- 当前 TIS 未 ready 时，即使 `INPUTIA_RUN_UI_SMOKE=1`，入口也以 rc=6 停在 `tis-not-ready`，没有启动 TextEdit/Safari/osascript/InputiaInputMethod，也没有污染剪贴板、当前输入源或 debug env。
- 真实 GUI smoke 仍需系统安装版/settings 更新到 v41 且 TIS readiness 通过后再跑。

## v41 Mac mini：Host blocker runbook 当前状态修正

问题：

- `HOST_BLOCKER_RUNBOOK.md` 仍保留历史 v4 结论“当前机器上阻塞已解除”。
- 当前实际状态已经变成：build/pkg 为 v41，但系统 host/settings 仍是 v40，TIS enabled/installed matches 为 0。继续保留旧表述会误导后续接手者绕过 readiness gate 或硬跑真实 GUI smoke。

实现：

- `HOST_BLOCKER_RUNBOOK.md`
  - 增加 2026-07-08 更新说明。
  - 把“阻塞已解除”改为“历史上 v4 曾经解除”，并明确当前 v41 必须以 `status.sh` / `gui-smoke-readiness.sh` 为准。
  - 增加当前 v41 事实：build v41 CDHash、system/settings v40、TIS matches 0、latest pkg SHA256。
  - 后续动作改为：先管理员安装 v41 -> `status.sh` 对齐 -> `gui-smoke-readiness.sh` ready -> `INPUTIA_RUN_UI_SMOKE=1 gui-smoke-suite.sh`。

验证：

```text
rg "当前机器上阻塞已解除|macOS enabled/selectable 入口阻塞已经解除" HOST_BLOCKER_RUNBOOK.md
  no matches

rg "当前 v41|当前正确下一步|guiSmokeReadinessReady=true|INPUTIA_RUN_UI_SMOKE=1" HOST_BLOCKER_RUNBOOK.md
  当前 v41 状态必须以 status.sh / gui-smoke-readiness.sh 为准
  当前 v41 事实
  当前正确下一步
  guiSmokeReadinessReady=true reason=none
  INPUTIA_RUN_UI_SMOKE=1 gui-smoke-suite.sh

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n \
  macos/InputiaInputMethod/gui-smoke-readiness.sh \
  macos/InputiaInputMethod/gui-smoke-suite.sh \
  macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

bash macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  build.version=41
  target.matchesBuild=true
  settings.systemVersion=40
  settings.matchesBuild=false
  pkg.ready=true
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.readinessBlockReason=missing-enabled-source
  adminInstallReady=false reason=admin-required
  guiSessionBlockReason=none
  textEditPreflight=not-running
  safariPreflight=not-running
  guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessReady=false reason=admin-required
```

当前结论：

- Host blocker runbook 现在不再把历史 v4 ready 状态写成当前事实。
- 后续接手者可以按 runbook 明确知道：当前不能硬跑真实 GUI smoke，必须先更新系统 host/settings 到 v41，并等待 TIS readiness 通过。

## v41 Mac mini 当前尾部复核：await UI block reasons 与无残留

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=target-and-tis uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  awaitUiStatusSelfCheck reason=no-console-user uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=no-console-user uiSmokeBlockReasons=no-console-user
  awaitUiStatusSelfCheck reason=login-not-complete uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=login-not-complete uiSmokeBlockReasons=login-not-complete
  awaitUiStatusSelfCheck reason=screen-locked uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=screen-locked uiSmokeBlockReasons=screen-locked
  awaitUiStatusSelfCheck reason=frontmost-unavailable uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=frontmost-unavailable uiSmokeBlockReasons=frontmost-unavailable
  awaitUiStatusSelfCheck reason=loginwindow-frontmost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=loginwindow-frontmost uiSmokeBlockReasons=loginwindow-frontmost
  awaitUiStatusSelfCheck=true

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
  macos/InputiaInputMethod/await-system-install.sh
  uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  systemInstallObserved=false reason=timeout
  systemInstallTargetMatchesBuild=false
  systemInstallTISReady=false reason=target-cdhash-mismatch

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-block-reasons-20260708.log 2>&1
  verifyRc=0
  awaitShort: uiSmokeRequested=false uiSmokeWouldStart=false uiSmokeBlockReason=ui-smoke-disabled uiSmokeBlockReasons=ui-smoke-disabled
  awaitUiNotReady: uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  awaitUiNotReadyNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- `await-system-install.sh` 现在兼容输出主阻塞原因 `uiSmokeBlockReason=` 与并列阻塞原因 `uiSmokeBlockReasons=`。
- 当前安装等待路径同时暴露 `target-cdhash-mismatch,tis-not-ready`，不会误导后续只处理系统 app 版本而漏掉 TIS readiness。
- 真实 GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员权限。

## v41 Mac mini：gui-smoke-suite 汇总并列 readiness 阻塞原因

问题：

- `gui-smoke-readiness.sh` 已输出 `guiSmokeReadinessBlockReasons=`，但 `gui-smoke-suite.sh` 自身汇总此前只保留单主因 `guiSmokeSuiteReady=false reason=...`。
- 如果后续自动化只读取 suite 汇总，而不解析带前缀的 readiness 原始输出，就可能再次漏掉并列阻塞原因，例如当前同时存在 `settings-version-mismatch,admin-required,tis-not-ready`。

实现：

- `gui-smoke-suite.sh`
  - 从 readiness 输出读取 `guiSmokeReadinessBlockReasons=`。
  - 在 suite 汇总中新增 `guiSmokeSuiteBlockReasons=`。
  - readiness 输出缺失时输出 `guiSmokeSuiteBlockReasons=readiness-output-missing`。
  - readiness 未提供 block reasons 时回退到单主因，兼容旧格式测试输入。
- `verify-nongui.sh`
  - 增加静态契约，要求 suite 解析并输出 block reasons。
  - 增加 missing、blocked、ready、post-install-failure 四条 gate 的运行时断言。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-suite.sh
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteBlockReasons=readiness-output-missing
  guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteBlockReasons=none
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuiteBlockReasons=none
  guiSmokeSuiteSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-gui-suite-block-reasons-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuiteMissingReadinessGateNoMutationPassed=true
  guiSmokeSuiteBlockedGateNoMutationPassed=true
  guiSmokeSuiteReadySkipGateNoMutationPassed=true
  guiSmokeSuitePostInstallFailureGateNoMutationPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteCurrentRc=12
  guiSmokeSuiteReadiness: settings.systemVersion=40
  guiSmokeSuiteReadiness: settings.matchesBuild=false
  guiSmokeSuiteReadiness: tis.enabledMatches=0
  guiSmokeSuiteReadiness: tis.installedMatches=0
  guiSmokeSuiteReadiness: adminInstallReady=false reason=admin-required
  guiSmokeSuiteReadiness: textEditPreflight=not-running
  guiSmokeSuiteReadiness: safariPreflight=not-running
  guiSmokeSuiteReadiness: guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteWouldRun=false

残留检查：

```text
TextEdit=not-running
Safari=not-running
InputiaInputMethod=not-running
osascript=not-running
sleep=not-running
smoke/verifier script residue: none
/tmp/inputia-* residue: none
```

当前结论：

- 现在 suite 层和 readiness 层都会暴露并列阻塞原因；只读取 suite 汇总也能看到当前不能跑真实 GUI smoke 的全部原因。
- 当前 suite gate 停在 readiness，未进入真实 TextEdit/Safari/Clipboard smoke。
- 真实 GUI smoke 仍未运行；当前系统安装版/settings 为 v40、构建版为 v41，TIS readiness 不满足，且没有非交互管理员权限。

## v41 Mac mini 复核：常用 Command 快捷键统一透传

背景：用户再次反馈在 Inputia 下 `Command-C` / `Command-V` 不能用，并要求不要逐个等用户测试，而是按常用电脑快捷键举一反三处理。

官方依据：

- Apple Support `Mac keyboard shortcuts` 将 `Command-X/C/V/Z/A/F/G/H/M/N/O/P/S/T/W/Q`、`Command-Tab`、`Command-Space`、`Control-Command-Space`、`Shift-Command-3/4/5`、`Option-Command-Esc` 等列为 macOS 常用系统/App 快捷键。
- Apple Support `Copy and paste on Mac` 明确 `Command-C` 为复制、`Command-V` 为粘贴、`Command-X` 为剪切、`Command-Z` 为撤销。

代码复核：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 规则是任何包含 `.command` 的 keyDown 都返回 `true`。
- `InputiaInputController.handleKeyDown` 在剪贴板召回、标点切换、全半角切换、输入模式切换、候选导航之前先执行 Command 透传并 `return false`。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 覆盖 `copy:`、`paste:`、`cut:`、`undo:`、`redo:`、`selectAll:`、`saveDocument:`、`openDocument:`、`performClose:`、`terminate:`、`find:`、`print:`、`hide:`、`showHelp:`、`pasteAsPlainText:`、`goBack:`、`goForward:`、`reload:`、`stopLoading:` 等 AppKit selector。
- Inputia 自有快捷键保持不含 Command，例如 `Control-Shift-V` 剪贴板召回；`Control-Shift-Command-V` 明确拒绝，避免抢宿主粘贴变体。

验证：

```text
./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftVClipboardRecall=true
  ctrlShiftCommandVRejected=true

./macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandpasteAsPlainTextPassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandgoForwardPassesThrough=true
  appCommandreloadPassesThrough=true
  appCommandstopLoadingPassesThrough=true

bash macos/InputiaInputMethod/verify-nongui.sh
  shortcut: allCommandModifierVariantsPassThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  hostShortcut: allCommandModifierVariantsPassThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  hostTextPolicy: appCommandcopyPassesThrough=true
  hostTextPolicy: appCommandpastePassesThrough=true
  postInstall: allCommandModifierVariantsPassThrough=true
  postInstall: commandCPassThrough=true
  postInstall: commandVPassThrough=true
  awaitUiNotReady: uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前 v41 build 已经不是单独修 `Command-C` / `Command-V`，而是统一透传所有含 `Command` 的系统/App 快捷键，并额外透传常见 AppKit command selector。
- 用户实际仍可能复现，是因为系统安装版仍是 v40：`/Library/Input Methods/InputiaInputMethod.app` cdhash `8d4f473adcc2f7c093b5629b9b1e742dcba184f8`，当前 v41 build cdhash `6d7e1033ef95597258f7c9c30f7d361f1b3dee2f`，且 TIS matches 仍为 0。
- 真实 TextEdit/Safari `Command-A/C/V` GUI smoke 仍需等 v41 安装并 TIS readiness 通过后运行；本轮没有打开 TextEdit/Safari。

## v41 Mac mini 续修：post-install 成功 marker 只能在 UI smoke 调度之后输出

问题：

- `post-install-regression.sh` 是真实 TextEdit/Safari/Clipboard GUI smoke 的最终调度入口。
- 既有 verifier 已确认 UI smoke 顺序，但还缺少一个明确契约：`postInstallRegressionPassed=true` 不能被放到 UI smoke 调度之前，否则未来可能在真实 GUI smoke 未完整调度时误报 post-install 通过。

实现：

- `verify-nongui.sh`
  - 在 post-install 静态契约中记录 `smoke-clipboard-recall.sh`、`section "result"`、`postInstallRegressionPassed=true` 的位置。
  - 要求 `postInstallRegressionPassed=true` 必须位于 Clipboard recall smoke 调度之后。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

grep line order:
  postInstallClipboardLine=320
  postInstallResultLine=327
  postInstallPassedLine=328
  postInstallSuccessMarkerAfterUiSmoke=true

INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 \
  macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiPreflightSelfCheck case=textedit-block TextEditPreflight=running
  postInstallUiPreflightSelfCheck case=textedit-block postInstallUiSmokeReady=false reason=textedit-already-running
  postInstallUiPreflightSelfCheck case=safari-block SafariPreflight=running
  postInstallUiPreflightSelfCheck case=safari-block postInstallUiSmokeReady=false reason=safari-already-running
  postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflightAllowed=true
  postInstallUiPreflightSelfCheck case=safari-allow SafariPreflightAllowed=true
  postInstallUiPreflightSelfCheck=true

INPUTIA_RUN_UI_SMOKE=0 \
  macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallNonGuiRc=0
  uiSmokeSkipped=true reason=disabled
  == result ==
  postInstallRegressionPassed=true

INPUTIA_RUN_UI_SMOKE=1 \
  macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiTisGateRc=6
  postInstallTISReady=false
  postInstallTISBlockReason=missing-enabled-source
  guiSmokeReady=false reason=tis-not-ready
  postInstallUiSmokeReady=false reason=tis-not-ready
  postInstallUiTisGateNoLaunchExpected=true
```

补充：

- 本轮尝试跑完整 `verify-nongui.sh` 时，第一次与上一轮 verifier 活锁并发冲突返回 rc=20；后续长跑两次被外部中断，未形成完整 verifier 证据，因此不把它们作为通过证据。
- 定向验证已覆盖本次新增契约、post-install 非 UI 成功路径、UI preflight 自检和当前 UI TIS gate 早退路径。

当前结论：

- post-install 成功 marker 现在有静态契约保护，必须在最后一个 UI smoke 调度点之后。
- 当前 `INPUTIA_RUN_UI_SMOKE=1` 仍停在 TIS gate，未进入真实 TextEdit/Safari/Clipboard smoke。
- 真实 GUI smoke 仍未运行；系统安装版/settings 仍需更新到 v41，并等待 TIS readiness 通过。

## v41 Mac mini 续修：Clipboard recall debug log 清洁门禁前移

背景：当前目标仍在修 GUI smoke 测试纪律和 IME 状态污染。`smoke-clipboard-recall.sh` 已经会在 TextEdit 前台后连续 Escape 清状态、确认文档为空，并在触发 `Control-Shift-V` 前检查事件日志中不能提前出现 `clipboardRecallShown` / `clipboardRecallCommit`。本轮继续收紧一个副作用边界：debug event log 的准备和清洁断言原本在输入源选择之后，若日志路径异常或污染，发现点偏晚。

实现：

- `smoke-clipboard-recall.sh`
  - 将 `inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"` 和 `inputia_assert_debug_event_log_clean "$EVENT_LOG" "clipboardRecallSmokeReady" 13` 提前到 `inputia_select_input_source_or_exit` 之前。
  - 这样日志路径不可用或非空时，会在 TIS 选择和后续 GUI 链路之前失败，减少对当前输入源和前台状态的副作用窗口。
  - 仍保持 `launchctl setenv INPUTIA_DEBUG_EVENTS` 在 select 之后、host restart 之前，避免在准备阶段改变全局 debug env。
- `verify-nongui.sh`
  - 更新静态 cleanup permission contract：Clipboard recall 必须满足 `capture -> trap -> prepare log -> clean assert -> select input source -> set debug env -> restart host` 的顺序。
  - Safari enter 保持原顺序，避免把 Clipboard recall 的特殊早门禁误套到其它脚本。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
  INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
  INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST='Unicode text, 42' \
  INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK=1 \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  clipboardRestorable=true
  clipboardRecallCleanupSelfCheck=true phase=after-clipboard-write
  rc=23

bash macos/InputiaInputMethod/verify-nongui.sh
  cleanupPermissionContract=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard recall smoke 的事件日志污染会在输入源选择前被挡住；真实 GUI smoke 前的状态清洁边界更早、更可审计。
- 本轮仍没有运行真实 TextEdit/Safari/Clipboard GUI smoke；当前系统安装版/settings 仍是 v40，构建版为 v41，TIS readiness 不满足，且没有非交互管理员权限。

## v41 Mac mini 续验：Clipboard recall debug log 准备失败负例

背景：上一段把 Clipboard recall 的 debug event log 准备和清洁断言提前到输入源选择之前。本轮补一个运行时负例，确保日志路径不可写/被目录占用时，脚本在 TIS 选择前失败，且不污染剪贴板、输入源或 debug env。

实现：

- `verify-nongui.sh`
  - 新增 `clipboard recall debug log prepare failure gate`。
  - 将 `INPUTIA_DEBUG_EVENTS` 指向临时目录，触发 `inputia_prepare_debug_event_log` 写入失败。
  - 断言输出中不能出现 `previousInputSourceID=`，用于证明没有进入 `inputia_select_input_source_or_exit`。
  - 断言剪贴板、当前输入源、`INPUTIA_DEBUG_EVENTS`、TextEdit、osascript、Inputia host 都未被污染。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh
  verifyLockStale=true path=/tmp/inputia-verify-nongui.lock pid=10986
  cleanupPermissionContract=true
  clipboardDebugLogPrepareGateNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  awaitUiNotReadyNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

manual negative check:
  manualClipboardDebugLogGate: guiSessionCheck=skipped
  manualClipboardDebugLogGate: textEditPreflight=not-running docs=0
  manualClipboardDebugLogGate: clipboardRestorable=true
  manualClipboardDebugLogGate: smoke-common.sh: line 341: /tmp/inputia-clipboard-dirty-log-manual.QJ0ulq: Is a directory
  manualClipboardDebugLogGate.rc=1
  manualClipboardDebugLogGate.selectedInputSourceOutput=0
  manualClipboardDebugLogGate.clipboardUnchanged=true
  manualClipboardDebugLogGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  manualClipboardDebugLogGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  manualClipboardDebugLogGate.debugEnvBefore=unset
  manualClipboardDebugLogGate.debugEnvAfter=unset
```

当前结论：

- Clipboard recall 的 debug log 路径异常会在输入源选择之前失败；`selectedInputSourceOutput=0` 证明没有进入 TIS 选择输出路径。
- 该负例已纳入 `verify-nongui.sh`，后续如果有人把日志 prepare/clean assert 移回选择输入源之后，非 GUI 验证会失败。
- 真实 GUI smoke 仍需等系统安装版升级到 v41 且 TIS readiness 通过后执行。

## v41 Mac mini 续修：Safari enter debug log 清洁门禁前移

背景：审计剩余 GUI smoke 脚本时发现 `smoke-safari-enter.sh` 与 Clipboard recall 存在同类风险：debug event log 的准备和清洁断言在输入源选择之后执行。若 `INPUTIA_DEBUG_EVENTS` 指向目录或不可写路径，失败会发生在 TIS 选择之后，副作用边界偏晚。

实现：

- `smoke-safari-enter.sh`
  - 将 `inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"` 和 `inputia_assert_debug_event_log_clean "$EVENT_LOG" "safariEnterSmokeReady" 10` 提前到 `inputia_select_input_source_or_exit` 之前。
  - `launchctl setenv INPUTIA_DEBUG_EVENTS` 仍保留在 select 之后、host restart 之前。
- `verify-nongui.sh`
  - 统一 debug-log GUI smoke 顺序契约：`capture -> trap -> prepare log -> clean assert -> select input source -> set debug env -> restart host`。
  - 新增 `safari enter debug log prepare failure gate`，将 `INPUTIA_DEBUG_EVENTS` 指向目录，断言脚本在输入源选择前失败且无污染。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

manual negative check:
  manualSafariEnterDebugLogGate: guiSessionCheck=skipped
  manualSafariEnterDebugLogGate: safariPreflight=not-running
  manualSafariEnterDebugLogGate: smoke-common.sh: line 341: /tmp/inputia-safari-enter-dirty-log-manual.tJjgkU: Is a directory
  manualSafariEnterDebugLogGate.rc=1
  manualSafariEnterDebugLogGate.selectedInputSourceOutput=0
  manualSafariEnterDebugLogGate.clipboardUnchanged=true
  manualSafariEnterDebugLogGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  manualSafariEnterDebugLogGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  manualSafariEnterDebugLogGate.debugEnvBefore=unset
  manualSafariEnterDebugLogGate.debugEnvAfter=unset

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-full-current-20260708.log 2>&1
  cleanupPermissionContract=true
  safariEnterDebugLogPrepareGateNoMutationPassed=true
  clipboardDebugLogPrepareGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari enter 和 Clipboard recall 两条带 debug event log 的真实 GUI smoke 入口，现在都会在输入源选择前挡住异常日志路径。
- 本轮没有打开 Safari/TextEdit，也没有留下 osascript/Inputia host 或 `/tmp/inputia-*` 测试残留。
- 真实 GUI smoke 的主阻塞仍是系统安装版/settings v40、build v41、TIS matches 0。

## v41 Mac mini 续验：完整非 GUI verifier 当前通过

背景：

- 上一轮新增 `post-install 成功 marker 只能在 UI smoke 调度之后输出` 的静态契约后，曾尝试跑完整 `verify-nongui.sh`，但当时一次撞到活锁、两次被外部中断，没有形成完整通过证据。
- 本轮先确认无 verifier lock、无 TextEdit/Safari/Inputia/osascript/sleep 残留，再以前台方式重跑完整 verifier，避免后台 job 被 shell 退出中断。

验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-full-current-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteBlockReasons=readiness-output-missing
  guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteBlockReasons=none
  guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuiteBlockReasons=none
  guiSmokeSuiteMissingReadinessGate: guiSmokeSuiteBlockReasons=readiness-output-missing
  guiSmokeSuiteBlockedGate: guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteReadySkipGate: guiSmokeSuiteBlockReasons=none
  guiSmokeSuitePostInstallFailureGate: guiSmokeSuiteBlockReasons=none
  safariEnterDebugLogPrepareGateNoMutationPassed=true
  clipboardDebugLogPrepareGateNoMutationPassed=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前工作区完整非 GUI verifier 通过；上一轮 post-install 成功 marker 静态契约已被完整 verifier 覆盖。
- GUI smoke suite、Safari enter / Clipboard recall debug log prepare gate、post-install 非 GUI、post-install UI TIS gate 和 residue collector 均保持通过。
- 真实 GUI smoke 仍未运行；当前仍需系统安装版/settings 更新到 v41，且 TIS readiness 通过后再执行真实 TextEdit/Safari/Clipboard smoke。

## v41 Mac mini 续修：status 输出真实 GUI smoke 总阻塞原因

问题：

- `status.sh` 是接手者最常用的系统状态入口，但此前只输出 system host、settings、TIS 和 pkg 的原始字段。
- 当前真实 GUI smoke 同时被 system host v40、settings v40、TIS missing 和 admin 权限阻塞；如果只读 `status.sh`，需要人工拼接这些原因，容易漏掉其中一项或误以为可以硬跑 GUI smoke。

实现：

- `status.sh`
  - 新增 `gui smoke summary` section。
  - 汇总输出 `statusAdminInstallReady=`、`statusTISEnabledMatches=`、`statusTISInstalledMatches=`、`statusGuiSmokeBlockReasons=`、`statusGuiSmokeReady=`。
  - 当前阻塞汇总为 `target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready`。
- `verify-nongui.sh`
  - 增加静态契约，要求 `status.sh` 保留上述 summary 字段和四类阻塞 reason。
  - 在 status 阶段要求当前输出包含完整四类阻塞 summary。

验证：

```text
zsh -n macos/InputiaInputMethod/status.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/status.sh
  statusAdminInstallReady=false
  statusTISEnabledMatches=0
  statusTISInstalledMatches=0
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-status-summary-clean-20260708.log 2>&1
  verifyRc=0
  status: statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  status: statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  awaitUiNotReady: statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  awaitUiNotReadyNoLaunchPassed=true
  residueCollectorSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

验证备注：

- 首次 verifier 失败是因为并发诊断 shell 的命令行里包含 `status.sh`，被 residue collector 正确识别为验证脚本残留；这不是代码失败。
- 清理并发观察命令后重跑完整 verifier 通过。

当前结论：

- `status.sh` 现在能直接给出真实 GUI smoke 的并列阻塞原因；接手者不需要人工拼 system host/settings/TIS/admin 字段。
- `await-system-install.sh` timeout 后调用的完整 `status.sh` 输出也会携带同一组 summary 字段。
- 真实 GUI smoke 仍未运行；当前仍需系统安装版/settings 更新到 v41，且 TIS readiness 通过后再执行真实 TextEdit/Safari/Clipboard smoke。

## v41 Mac mini 续修：status GUI smoke summary 覆盖会话和前台 App 预检

问题：

- 上一段 `status.sh` 已汇总 system host、settings、TIS 和 admin 阻塞。
- 但真实 GUI smoke 还要求图形会话可用，且默认 TextEdit/Safari 不能已经运行；如果未来系统安装版升级到 v41 后只看旧 summary，仍可能在 TextEdit/Safari 或锁屏状态下误以为可以跑真实 GUI smoke。

实现：

- `status.sh`
  - 增加 `gui_session_block_reason()`，与 `gui-smoke-readiness.sh` 的图形会话判定保持一致。
  - 增加 TextEdit/Safari 进程 preflight。
  - `gui smoke summary` 新增：
    - `statusGuiSessionBlockReason=`
    - `statusTextEditPreflight=`
    - `statusSafariPreflight=`
  - `statusGuiSmokeBlockReasons=` 会把 GUI session、TextEdit already running、Safari already running 纳入总阻塞原因。
- `verify-nongui.sh`
  - 增加静态契约，要求 status summary 覆盖 GUI session 与 TextEdit/Safari preflight。
  - 当前机器上要求输出 `statusGuiSessionBlockReason=none`、`statusTextEditPreflight=not-running`、`statusSafariPreflight=not-running`。

验证：

```text
zsh -n macos/InputiaInputMethod/status.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/status.sh
  statusGuiSessionBlockReason=none
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-status-gui-preflight-20260708.log 2>&1
  verifyRc=0
  status: statusGuiSessionBlockReason=none
  status: statusTextEditPreflight=not-running
  status: statusSafariPreflight=not-running
  status: statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  status: statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  postInstallUiTisGateNoLaunchPassed=true
  awaitShort: statusGuiSessionBlockReason=none
  awaitShort: statusTextEditPreflight=not-running
  awaitShort: statusSafariPreflight=not-running
  awaitShort: statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  awaitUiNotReady: statusGuiSessionBlockReason=none
  awaitUiNotReady: statusTextEditPreflight=not-running
  awaitUiNotReady: statusSafariPreflight=not-running
  awaitUiNotReady: statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `status.sh` 的 GUI smoke summary 现在覆盖真实 GUI smoke 的主要前置条件：系统 host、settings、TIS、admin、图形会话、TextEdit/Safari preflight 和 Inputia host 是否运行。
- 当前系统仍因 system host v40、settings v40、TIS matches 0 和 admin 权限不可用而 blocked；TextEdit/Safari 当前未运行，图形会话可用。
- 真实 GUI smoke 仍未运行；系统安装版/settings 更新到 v41 且 TIS readiness 通过后，再通过 `gui-smoke-suite.sh` 进入真实 TextEdit/Safari/Clipboard smoke。

## v41 Mac mini 续验：status GUI blocker 动态自检

背景：

- `status.sh` 已把 GUI session、TextEdit preflight 和 Safari preflight 纳入 `statusGuiSmokeBlockReasons=`。
- 还需要运行时证明这些 blocker 真会进入 status summary，而不是只有静态字符串存在；同时不能启动真实 TextEdit/Safari。

实现：

- `verify-nongui.sh`
  - 新增 `status GUI blocker self-check` 段。
  - 用 `INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=screen-locked` 注入 GUI session blocker。
  - 用已有 `start_fake_existing_process TextEdit/Safari` 创建假进程名，验证 `statusTextEditPreflight=running` / `statusSafariPreflight=running` 和对应 block reason。
  - 验证后清理假进程，并断言剪贴板、当前输入源、debug env、osascript、Inputia host 没有残留或污染。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/status.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-status-blockers-20260708.log 2>&1
  verifyRc=0
  statusGuiSessionBlockerSelfCheck=true
  statusTextEditBlockerSelfCheck=true
  statusSafariBlockerSelfCheck=true
  statusBlockerSelfCheck.clipboardUnchanged=true
  statusBlockerSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  statusBlockerSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  statusBlockerSelfCheck.debugEnvBefore=unset
  statusBlockerSelfCheck.debugEnvAfter=unset
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `status.sh` 的 GUI session/TextEdit/Safari blocker 不只是静态输出字段，已被完整非 GUI verifier 动态覆盖。
- 假 TextEdit/Safari 进程没有启动真实 App；验证后无 TextEdit/Safari/Inputia/osascript/sleep 残留。
- 真实 GUI smoke 仍未运行；当前仍需系统安装版/settings 更新到 v41，且 TIS readiness 通过后再执行真实 TextEdit/Safari/Clipboard smoke。

## v41 Mac mini 续验：status TextEdit/Safari allow override 动态自检

背景：

- `status.sh` 默认会把已有 TextEdit/Safari 进程作为真实 GUI smoke blocker。
- 真实 smoke 脚本仍保留 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1` / `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1` 覆盖开关；status summary 也需要证明这些开关生效，避免显式 allow 场景下仍被 status 错误阻塞。

实现：

- `verify-nongui.sh`
  - 在 `status GUI blocker self-check` 内复用假 TextEdit/Safari 进程。
  - 默认状态下断言 `textedit-already-running` / `safari-already-running` 会进入 `statusGuiSmokeBlockReasons=`。
  - 设置对应 allow env 后，断言 `statusTextEditPreflight=running` / `statusSafariPreflight=running` 仍可见，但 `statusGuiSmokeBlockReasons=` 不再包含对应 already-running blocker。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/status.sh
  syntaxRc=0

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-status-allow-20260708.log 2>&1
  verifyRc=0
  statusTextEditBlockerSelfCheck=true
  statusTextEditAllowSelfCheck=true
  statusSafariBlockerSelfCheck=true
  statusSafariAllowSelfCheck=true
  statusBlockerSelfCheck.clipboardUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `status.sh` 的 TextEdit/Safari blocker 与 allow override 行为已被动态覆盖。
- allow override 不会隐藏 preflight 状态本身，只是不再把已有 App 计入 `statusGuiSmokeBlockReasons=`。
- 真实 GUI smoke 仍未运行；当前仍需系统安装版/settings 更新到 v41，且 TIS readiness 通过后再执行真实 TextEdit/Safari/Clipboard smoke。

## v41 Mac mini 续验：gui-smoke-readiness allow override 自检

背景：

- `status.sh` 已动态覆盖 TextEdit/Safari allow override。
- `gui-smoke-readiness.sh` 是真实 GUI smoke suite 的主 gate，也有同样的 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING` / `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING` 判断；需要在 readiness 自检中明确覆盖 allow 分支。

实现：

- `gui-smoke-readiness.sh`
  - `INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1` 新增：
    - `case=allow-textedit expected=none actual=none`
    - `case=allow-safari expected=none actual=none`
- `verify-nongui.sh`
  - 增加上述两个 marker 的断言。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh
  guiSmokeReadinessSelfCheck case=allow-textedit expected=none actual=none
  guiSmokeReadinessSelfCheck case=allow-safari expected=none actual=none
  guiSmokeReadinessSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-allow-20260708-rerun.log 2>&1
  verifyRc=0
  guiSmokeReadinessSelfCheck case=allow-textedit expected=none actual=none
  guiSmokeReadinessSelfCheck case=allow-safari expected=none actual=none
  guiSmokeReadinessSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

验证备注：

- 首次完整 verifier 运行遇到 stale lock 返回 rc=20；移除已无存活 owner 的 `/tmp/inputia-verify-nongui.lock` 后重跑通过。

当前结论：

- readiness 主 gate 和 status summary 现在都覆盖 TextEdit/Safari allow override。
- allow override 不会绕过其它 blocker；当前系统仍被 system host/settings v40、TIS matches 0 和 admin 权限阻塞。
- 真实 GUI smoke 仍未运行；系统安装版/settings 更新到 v41 且 TIS readiness 通过后，再通过 `gui-smoke-suite.sh` 执行真实 TextEdit/Safari/Clipboard smoke。
## v41 Mac mini 复验：常用电脑快捷键不由输入法接管

背景：用户反馈 `Command-C` / `Command-V` 在 Inputia 下不能用，并要求按常用电脑快捷键举一反三处理，不能逐个等用户手测。

官方依据：

- Apple Support `Mac keyboard shortcuts` 将复制、粘贴、剪切、撤销、全选、查找、隐藏、最小化、打开、打印、退出、保存、关闭窗口、强制退出、Spotlight、字符检视器、App 切换、截图、偏好设置、Finder 导航、文本选择/移动等都定义为 `Command` 或含 `Command` 的系统/App 快捷键。
- Apple Developer `Handling Key Events` 说明 Cocoa 文本输入管理会把 key event 解释成 `doCommandBySelector:` 或 `insertText:`；默认 `doCommandBySelector:` 会沿 responder chain 交给能处理该 selector 的对象。因此输入法 host 只应处理自己的输入法命令，宿主 App 命令 selector 应返回给宿主。

实现复核：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 对任何包含 `.command` 的 keyDown 返回 `true`。
- `InputiaInputController.handleKeyDown` 在剪贴板召回、标点切换、全半角切换、输入模式切换、候选导航之前先执行 Command 透传并 `return false`。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 覆盖 `copy:`、`paste:`、`cut:`、`undo:`、`redo:`、`selectAll:`、`saveDocument:`、`openDocument:`、`performClose:`、`terminate:`、`find:`、`print:`、`hide:`、`showHelp:`、`pasteAsPlainText:`、`goBack:`、`goForward:`、`reload:`、`stopLoading:` 以及常见窗口/格式/查找 selector。
- Inputia 自有快捷键继续使用非 Command 组合，例如 `Control-Shift-V` 剪贴板召回；`Control-Shift-Command-V` 明确拒绝，避免抢宿主粘贴变体。

验证：

```text
bash macos/InputiaInputMethod/build.sh
  buildRc=0
  注：仍有现存 ld warning：Rust 静态库对象标记为 macOS 26.0，高于当前链接目标 13.0；本轮不是快捷键逻辑失败。

./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandShiftZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandGPassThrough=true
  commandHPassThrough=true
  commandMPassThrough=true
  commandPPassThrough=true
  commandTPassThrough=true
  commandNPassThrough=true
  commandDPassThrough=true
  commandEPassThrough=true
  commandIPassThrough=true
  commandRPassThrough=true
  commandJPassThrough=true
  commandKPassThrough=true
  commandYPassThrough=true
  commandCommaPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandNumberPassThrough=true
  commandBracketPassThrough=true
  commandArrowPassThrough=true
  commandDeletePassThrough=true
  commandControlQPassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftCommandVRejected=true
  candidateDownArrowRejectedWithCommand=true

./macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandpasteAsPlainTextPassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandgoForwardPassesThrough=true
  appCommandreloadPassesThrough=true
  appCommandstopLoadingPassesThrough=true

./macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  ctrlShiftCommandVRejected=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-command-shortcuts-20260708.log 2>&1
  verifyRc=0
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前 v41 build 已统一透传所有含 `Command` 的 keyDown，并透传常见 AppKit command selector；`Command-C/V`、`Command-Shift/Option/Control-V`、Finder/文本/系统常用 `Command` 快捷键不再由 Inputia 接管。
- 本轮没有打开 TextEdit/Safari/osascript，也没有改当前输入源或系统剪贴板。
- 用户当前机器上仍可能在使用系统安装版 v40；真实 GUI smoke 和真实手感修复需要等系统安装版/settings 升级到 v41 且 TIS readiness 通过后验证。

## v41 Mac mini 续修：README 澄清 Command 快捷键覆盖边界

背景：

- 用户要求常用电脑快捷键都不要被输入法接管，前面已经按“所有含 `Command` 的 keyDown 透传”和“常见 AppKit command selector 透传”实现并验证。
- README 仍把 TextEdit/Safari command smoke 描述成 `Command-A/C/V`，容易让接手者误以为只保护了复制/粘贴/全选三个快捷键。

实现：

- README 的 `verify-nongui.sh` 说明新增“快捷键自检”边界：非 GUI 自检要求所有含 `Command` 的 keyDown 组合透传，并覆盖常见 Apple `Command` 快捷键集合和常见 AppKit command selector。
- TextEdit/Safari GUI command smoke 改写为“代表性 `Command-A/C/V` 真实输入链”，明确更广泛快捷键由非 GUI self-check 覆盖。
- `post-install-regression.sh` 说明同步澄清：默认非 GUI 回归包含所有含 `Command` 的 keyDown 透传和常见 AppKit selector 透传；真实 GUI 只跑代表路径。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh macos/InputiaInputMethod/status.sh macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/verify-pkg.sh
  syntaxRc=0

rg -n '快捷键自检|所有含 `Command`|代表性 `Command-A/C/V`|常见 AppKit command selector|更广泛' macos/InputiaInputMethod/README.md
  README.md:89  verify-nongui.sh 说明快捷键自检不是只测复制/粘贴
  README.md:115 TextEdit 代表性 Command-A/C/V，广泛 Command 由非 GUI 自检覆盖
  README.md:123 Safari 代表性 Command-A/C/V，广泛浏览器/系统 Command 由非 GUI 自检覆盖
  README.md:147 post-install 默认非 GUI 回归覆盖所有含 Command 的 keyDown 和常见 AppKit selector

./macos/InputiaInputMethod/status.sh
  systemMatchesBuild=false
  systemSettingsMatchesBuildVersion=false
  matches=0
  matches=0
  running=false
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

bash macos/InputiaInputMethod/verify-pkg.sh macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg
  pkgVerificationPassed=true
  packageVersion=41
  buildVersion=41
  archiveAppCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readme-command-docs-20260708.log 2>&1
  staleLockPid=6680
  staleLockPidAlive=false
  verifyRc=0
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- README 现在和实现/验证边界一致：非 GUI 自检负责常用 `Command` 快捷键泛化覆盖，GUI smoke 负责 TextEdit/Safari 的代表性真实输入链。
- 当前系统安装版/settings 仍为 v40，TIS matches 0，真实 GUI smoke 仍未运行。

## v41 Mac mini 续修：GUI smoke suite 当前真实阻塞动态门禁

背景：

- `verify-nongui.sh` 已覆盖 `gui-smoke-suite.sh` 的模拟 missing/blocked/ready/post-install-failure 分支。
- 但接手者最可能直接运行的是当前真实状态下的 `gui-smoke-suite.sh build/InputiaInputMethod.app`；这个入口需要被完整 verifier 动态证明会安全早退，不会打开 TextEdit/Safari 或进入 post-install GUI smoke。

实现：

- `verify-nongui.sh`
  - 新增 `GUI smoke suite current blocked gate`。
  - 直接运行 `gui-smoke-suite.sh "$BUILD_APP"`，不注入 fake readiness。
  - 要求当前真实输出为：
    - `guiSmokeSuiteReady=false reason=admin-required`
    - `guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready`
    - `guiSmokeSuiteWouldRun=false`
    - rc `12`
  - 断言剪贴板、当前输入源、`INPUTIA_DEBUG_EVENTS`、user host、TextEdit、Safari、osascript、Inputia host 均无污染。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  rc=12
  guiSmokeSuiteReadiness: target.version=41
  guiSmokeSuiteReadiness: target.matchesBuild=true
  guiSmokeSuiteReadiness: settings.systemVersion=40
  guiSmokeSuiteReadiness: settings.matchesBuild=false
  guiSmokeSuiteReadiness: tis.ready=false
  guiSmokeSuiteReadiness: adminInstallReady=false reason=admin-required
  guiSmokeSuiteReadiness: guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteWouldRun=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-suite-current-blocked-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuiteCurrentBlockedGate: target.matchesBuild=true
  guiSmokeSuiteCurrentBlockedGate: settings.systemVersion=40
  guiSmokeSuiteCurrentBlockedGate: settings.matchesBuild=false
  guiSmokeSuiteCurrentBlockedGate: tis.ready=false
  guiSmokeSuiteCurrentBlockedGate: guiSmokeSuiteReady=false reason=admin-required
  guiSmokeSuiteCurrentBlockedGate: guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeSuiteCurrentBlockedGate: guiSmokeSuiteWouldRun=false
  guiSmokeSuiteCurrentBlockedGate.rc=12
  guiSmokeSuiteCurrentBlockedGate.clipboardUnchanged=true
  guiSmokeSuiteCurrentBlockedGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteCurrentBlockedGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteCurrentBlockedGate.debugEnvBefore=unset
  guiSmokeSuiteCurrentBlockedGate.debugEnvAfter=unset
  guiSmokeSuiteCurrentBlockedGate.userHost=false
  guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 当前真实 Mac mini 状态下，直接运行 `gui-smoke-suite.sh build/InputiaInputMethod.app` 会在 readiness 阶段安全早退，不会进入 post-install GUI smoke，也不会启动 TextEdit/Safari。
- build app 已是 v41 且匹配 build；真实阻塞集中在系统 settings v40、admin 权限和 TIS missing。

## v41 Mac mini 续修：GUI smoke suite current gate 防回退契约

背景：

- 上一节新增了 `GUI smoke suite current blocked gate` 动态验证，但如果未来有人误删该段，`verify-nongui.sh` 本身可能不再运行这条真实入口 gate。
- 需要让 verifier 的静态契约也要求该 gate 存在，避免“删掉测试而不是修复问题”的回退。

实现：

- `verify-nongui.sh` 的静态契约新增三条要求：
  - 必须包含 `section "GUI smoke suite current blocked gate"`。
  - 必须包含 `guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true` marker。
  - 必须包含 `gui-suite-current-blocked-gate-missing-would-not-run` 断言，确保检查 `guiSmokeSuiteWouldRun=false`。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

rg -n "verify-nongui-missing-gui-suite-current|guiSmokeSuiteCurrentBlockedGateNoMutationPassed|nonGuiVerificationPassed=true" \
  /tmp/inputia-verify-nongui-suite-current-contract-20260708.log \
  macos/InputiaInputMethod/verify-nongui.sh
  verify-nongui.sh:1119 require current blocked gate section
  verify-nongui.sh:1120 require no-mutation marker
  verify-nongui.sh:1121 require would-not-run assertion
  verify-nongui.sh:1525 guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true
  log:431 guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true
  log:1939 nonGuiVerificationPassed=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-suite-current-contract-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `gui-smoke-suite.sh` 当前真实阻塞 no-launch/no-mutation gate 不只是临时验证，已被 `verify-nongui.sh` 静态契约保护。
- 后续删除该 gate 或漏掉 `guiSmokeSuiteWouldRun=false` 断言会让完整非 GUI verifier 失败。

## v41 Mac mini 续修：GUI smoke suite 拒绝矛盾 readiness 输出

背景：

- `gui-smoke-suite.sh` 是真实 GUI smoke 的聚合入口；它读取 `gui-smoke-readiness.sh` 输出后决定是否运行 `post-install-regression.sh`。
- 之前 suite 只要看到 `guiSmokeReadinessReady=true` 就会继续，即使同一份 readiness 输出同时带着非 `none` 的 `reason` 或 `guiSmokeReadinessBlockReasons`。这在真实脚本正常输出下不会发生，但属于防线单点信任：如果 readiness 输出或解析未来漂移，suite 可能误进真实 GUI smoke。

实现：

- `gui-smoke-suite.sh`
  - 当 `ready=true` 但 `reason != none` 或 `block_reasons != none` 时，输出：
    - `guiSmokeSuiteReady=false reason=readiness-inconsistent`
    - `guiSmokeSuiteBlockReasons=<原 block reasons>`
    - `guiSmokeSuiteWouldRun=false`
    - rc `12`
  - 自检新增 `inconsistent` case。
- `verify-nongui.sh`
  - 静态契约要求 suite 包含 `readiness-inconsistent` 门禁，以及 `reason` / `block_reasons` 必须同时为 `none` 的条件。
  - 自检输出必须包含 inconsistent case。
  - 新增 `GUI smoke suite inconsistent readiness gate`，动态断言该分支不改剪贴板、不改当前输入源、不改 debug env、不创建 user host、不启动 TextEdit/Safari/osascript/Inputia host。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-suite.sh
  guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteReadiness: guiSmokeReadinessBlockReasons=tis-not-ready
  guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteReadiness: guiSmokeReadinessReady=true reason=none
  guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteReady=false reason=readiness-inconsistent
  guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteBlockReasons=tis-not-ready
  guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteWouldRun=false
  guiSmokeSuiteSelfCheck case=inconsistent rc=12
  guiSmokeSuiteSelfCheck=true

INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=$'guiSmokeReadinessBlockReasons=tis-not-ready\nguiSmokeReadinessReady=true reason=none' \
  macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  rc=12
  guiSmokeSuiteReady=false reason=readiness-inconsistent
  guiSmokeSuiteBlockReasons=tis-not-ready
  guiSmokeSuiteWouldRun=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-suite-inconsistent-20260708.log 2>&1
  verifyRc=0
  guiSmokeSuiteInconsistentReadinessGate: guiSmokeSuiteReady=false reason=readiness-inconsistent
  guiSmokeSuiteInconsistentReadinessGate: guiSmokeSuiteBlockReasons=tis-not-ready
  guiSmokeSuiteInconsistentReadinessGate: guiSmokeSuiteWouldRun=false
  guiSmokeSuiteInconsistentReadinessGate.rc=12
  guiSmokeSuiteInconsistentReadinessGate.clipboardUnchanged=true
  guiSmokeSuiteInconsistentReadinessGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteInconsistentReadinessGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  guiSmokeSuiteInconsistentReadinessGate.debugEnvBefore=unset
  guiSmokeSuiteInconsistentReadinessGate.debugEnvAfter=unset
  guiSmokeSuiteInconsistentReadinessGate.userHost=false
  guiSmokeSuiteInconsistentReadinessGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- GUI smoke suite 现在只有在 `ready=true`、`reason=none`、`blockReasons=none` 三者同时满足时才会进入 post-install GUI smoke。
- readiness 输出出现矛盾时会安全早退，且该安全早退路径已有完整非 GUI no-mutation/no-launch 证明。

## v41 Mac mini 续修：readiness 汇总 Inputia host 运行中阻塞

背景：

- `status.sh` 的 GUI smoke summary 已经把 `InputiaInputMethod` 运行中纳入 `statusGuiSmokeBlockReasons=inputia-host-running`。
- `gui-smoke-readiness.sh` 是 `gui-smoke-suite.sh` 的统一入口，但此前只汇总 system/settings/TIS/GUI session/TextEdit/Safari；如果旧 Host 仍在运行，readiness 不会直接暴露该阻塞。
- 为了避免 suite 入口和 status 入口对“是否可以跑真实 GUI smoke”的判断不一致，需要把 Host 运行状态也纳入 readiness。

实现：

- `gui-smoke-readiness.sh`
  - `readiness_reason(...)` 和 `readiness_block_reasons(...)` 新增 `inputia_state` 参数。
  - 当 `InputiaInputMethod` 进程为 `running` 时输出 `inputia-host-running`。
  - 常规输出新增 `inputiaHostPreflight=running|not-running`。
  - 自检新增 `case=inputia expected=inputia-host-running actual=inputia-host-running`，并把 `inputia-host-running` 纳入 all block reasons。
- `verify-nongui.sh`
  - readiness self-check 新增 `inputia-host-running` 断言。
  - 用 fake `InputiaInputMethod` 进程动态验证 `gui-smoke-readiness.sh` 会输出 `inputiaHostPreflight=running`，并把 `inputia-host-running` 放入 block reasons。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReadinessSelfCheck case=inputia expected=inputia-host-running actual=inputia-host-running
  guiSmokeReadinessSelfCheck allBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,inputia-host-running,screen-locked,textedit-already-running,safari-already-running actual=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,inputia-host-running,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-inputia-host-20260708-rerun.log 2>&1
  verifyRc=0
  guiSmokeReadinessInputiaHostGate: inputiaHostPreflight=running
  guiSmokeReadinessInputiaHostGate: guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready,inputia-host-running
  guiSmokeReadinessInputiaHostGateSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  target.matchesBuild=true
  settings.systemVersion=40
  settings.matchesBuild=false
  tis.ready=false
  inputiaHostPreflight=not-running
  guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessReady=false reason=admin-required

bash macos/InputiaInputMethod/status.sh
  version=41
  version=40
  systemMatchesBuild=false
  systemSettingsMatchesBuildVersion=false
  matches=0
  matches=0
  running=false
  statusGuiSessionBlockReason=none
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
```

当前结论：

- `status.sh` 和 `gui-smoke-readiness.sh` 现在都能表达 `InputiaInputMethod` 运行中这一真实 GUI smoke blocker。
- fake Host 进程验证未启动真实 Inputia Host；完整 verifier 收尾后无 TextEdit/Safari/osascript/InputiaInputMethod 残留，无 `/tmp/inputia-*` 残留。
- 真实 GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 为 v40，构建版为 v41，TIS readiness 不满足，且没有非交互管理员安装权限。

## v41 Mac mini 续修：readiness Inputia host blocker 静态防回退契约

背景：

- 上一节已让 `gui-smoke-readiness.sh` 汇总 `InputiaInputMethod` 运行中阻塞，并在 `verify-nongui.sh` 中动态验证 fake host 进程会触发 `inputia-host-running`。
- 仍需一层静态契约保护，避免未来误删 `inputiaHostPreflight=` 输出、`inputia-host-running` blocker，或误删 verifier 中的 fake host gate。

实现：

- `verify-nongui.sh` 静态契约新增：
  - `gui-readiness-missing-inputia-host-block`：要求 `gui-smoke-readiness.sh` 保留 `inputia-host-running`。
  - `gui-readiness-missing-inputia-host-preflight-output`：要求 readiness 保留 `inputiaHostPreflight=` 输出。
  - `verify-nongui-missing-readiness-inputia-host-gate`：要求 verifier 保留 `guiSmokeReadinessInputiaHostGateSelfCheck=true` marker。
  - `verify-nongui-missing-readiness-inputia-host-preflight-assert` 和 `verify-nongui-missing-readiness-inputia-host-blocker-assert`：要求 verifier 保留 fake host preflight 与 blocker 断言。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeReadinessSelfCheck case=inputia expected=inputia-host-running actual=inputia-host-running
  guiSmokeReadinessSelfCheck allBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,inputia-host-running,screen-locked,textedit-already-running,safari-already-running actual=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,inputia-host-running,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-readiness-inputia-static-contract-20260708.log 2>&1
  verifyRc=0
  verify-nongui.sh:956 require inputia-host-running static contract
  verify-nongui.sh:957 require inputiaHostPreflight static contract
  verify-nongui.sh:1123 require fake host gate marker
  verify-nongui.sh:1124 require fake host preflight assertion
  verify-nongui.sh:1125 require fake host blocker assertion
  guiSmokeReadinessInputiaHostGate: inputiaHostPreflight=running
  guiSmokeReadinessInputiaHostGate: guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready,inputia-host-running
  guiSmokeReadinessInputiaHostGateSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  settings.systemVersion=40
  tis.ready=false
  inputiaHostPreflight=not-running
  guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready
  guiSmokeReadinessReady=false reason=admin-required
```

当前结论：

- `inputia-host-running` 不再只是当前实现行为，已被 `verify-nongui.sh` 的静态契约和动态 fake-process gate 双重保护。
- 当前真实 readiness 仍停在系统/settings v40 与 TIS missing；未打开 TextEdit/Safari，也未运行真实 Clipboard recall smoke。

## v41 Mac mini 复核：常用 Command 快捷键统一不接管

背景：用户再次反馈在 Inputia 下 `Command-C` / `Command-V` 不能用，并要求网上查常用电脑快捷键后举一反三处理，不能逐个等用户测试。

外部依据：

- Apple Support `Mac keyboard shortcuts` 把复制、粘贴、剪切、撤销、全选、查找、保存、打开、打印、关闭窗口、退出、隐藏、App 切换、Spotlight、字符检视器、截图、强制退出等列为 `Command` 或含 `Command` 的 macOS 常用系统/App 快捷键。
- Apple Developer `Handling Key Events` 说明 Cocoa 文本输入会把按键事件转换成 `doCommandBySelector:` 或 `insertText:` 路径；输入法 host 只应处理自己的输入法命令，宿主 App 命令 selector 应返回给宿主响应链。

当前实现复核：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 的根规则仍是任何包含 `.command` 的 keyDown 直接返回 `true`。
- `InputiaInputController.handleKeyDown` 在剪贴板召回、标点切换、全半角切换、输入模式切换、候选导航之前先执行 Command 透传并 `return false`。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 覆盖常见 AppKit command selector，包括 `copy:`、`paste:`、`cut:`、`undo:`、`redo:`、`selectAll:`、`saveDocument:`、`openDocument:`、`performClose:`、`terminate:`、`find:`、`print:`、`showPreferences:`、`toggleBold:`、`toggleItalic:`、`toggleUnderline:`、`goBack:`、`goForward:`、`reload:`、`stopLoading:` 等。
- Inputia 自有剪贴板召回保留为不含 Command 的 `Control-Shift-V`；`Control-Shift-Command-V` 明确拒绝，避免抢宿主粘贴变体。

验证：

```text
macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionShiftVPassThrough=true
  commandControlVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandAPassThrough=true
  commandSPassThrough=true
  commandOPassThrough=true
  commandWPassThrough=true
  commandQPassThrough=true
  commandFPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandShift3PassThrough=true
  commandShift4PassThrough=true
  commandShift5PassThrough=true
  commandOptionEscapePassThrough=true
  ctrlShiftCommandVRejected=true

macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true
  appCommandshowPreferencesPassesThrough=true
  appCommandtoggleBoldPassesThrough=true
  appCommandtoggleItalicPassesThrough=true
  appCommandtoggleUnderlinePassesThrough=true
  appCommandgoBackPassesThrough=true
  appCommandgoForwardPassesThrough=true
  appCommandreloadPassesThrough=true
  appCommandstopLoadingPassesThrough=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-postinstall-inputia-host-preflight-20260708.log 2>&1
  shortcutPassThroughSelfChecks.clipboardUnchanged=true
  shortcutPassThroughSelfChecks.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  shortcutPassThroughSelfChecks.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  shortcutPassThroughSelfChecks.debugEnvBefore=unset
  shortcutPassThroughSelfChecks.debugEnvAfter=unset
  shortcutPassThroughSelfChecks.userHost=false
  shortcutPassThroughSelfChecks=true
  postInstall: postInstallRegressionPassed=true
  postInstallUiTisGate.rc=6
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/status.sh
  build version=41
  system version=40
  systemMatchesBuild=false
  systemSettingsMatchesBuildVersion=false
  running=false
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
```

当前结论：

- v41 build 已按“所有含 `Command` 的 keyDown 先交还宿主”处理，不是只修 `Command-C/V` 两个特例；常用 Apple/macOS `Command` 快捷键和常见 AppKit command selector 均有非 GUI 自检覆盖。
- 用户当前仍可能看到 `Command-C/V` 失效，是因为系统安装版仍是 v40，尚未安装/启用 v41；真实 TextEdit/Safari GUI smoke 仍被 admin/TIS/版本不匹配门禁挡住，本轮没有打开 TextEdit/Safari。

## v41 Mac mini 续修：await UI smoke 覆盖 TextEdit/Safari 已运行 preflight

时间：2026-07-08 07:13:14 CST

背景：

- `await-system-install.sh` 是安装后等待系统版生效，并预判是否可以进入 `post-install-regression.sh` GUI smoke 的入口。
- 真实路径已经会在 TextEdit 或 Safari 预先运行时输出 `uiSmokeWouldStart=false`，避免抢用户窗口；但 `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 只覆盖 target/TIS 和 GUI session blockers，缺少 TextEdit/Safari process preflight 的动态自检。
- 为了继续收紧 GUI smoke 清理纪律，需要让这个聚合入口在非 GUI verifier 中证明：TextEdit/Safari 已运行会阻止 UI smoke；显式 allow 时仍保留 preflight 可见性，但不计入 blocker。

实现：

- `await-system-install.sh`
  - `process_preflight()` 新增 `INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST` 测试注入；仅用于自检，真实路径仍走 `pgrep -x`。
  - `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 新增四个 case：
    - `textedit-already-running`：fake TextEdit running，输出 `uiSmokeWouldStart=false` 和 `uiSmokeBlockReasons=textedit-already-running`。
    - `safari-already-running`：fake Safari running，输出 `uiSmokeWouldStart=false` 和 `uiSmokeBlockReasons=safari-already-running`。
    - `textedit-allow`：fake TextEdit running 且 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1`，输出 preflight running 但 `uiSmokeWouldStart=true`、`uiSmokeBlockReasons=none`。
    - `safari-allow`：fake Safari running 且 `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1`，输出 preflight running 但 `uiSmokeWouldStart=true`、`uiSmokeBlockReasons=none`。
- `verify-nongui.sh`
  - 静态契约要求 `INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST` 和四个 self-check marker。
  - 动态 verifier 要求四个新增 case 的完整输出，防止以后删掉 process preflight 自检。

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=textedit-already-running uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=textedit-already-running uiSmokeBlockReasons=textedit-already-running
  awaitUiStatusSelfCheck reason=safari-already-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiSmokeWouldStart=false uiSmokeBlockReason=safari-already-running uiSmokeBlockReasons=safari-already-running
  awaitUiStatusSelfCheck reason=textedit-allow uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiStatusSelfCheck reason=safari-allow uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiStatusSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-process-preflight-20260708.log 2>&1
  verifyRc=0
  shortcutPassThroughSelfChecks=true
  awaitUiStatusSelfCheck reason=textedit-already-running uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=textedit-already-running uiSmokeBlockReasons=textedit-already-running
  awaitUiStatusSelfCheck reason=safari-already-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiSmokeWouldStart=false uiSmokeBlockReason=safari-already-running uiSmokeBlockReasons=safari-already-running
  awaitUiStatusSelfCheck reason=textedit-allow uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiStatusSelfCheck reason=safari-allow uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiNotReadyNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `await-system-install.sh` 现在对 TextEdit/Safari 已运行 preflight 有动态自检覆盖；安装后自动入口不会在用户已有 TextEdit/Safari 时误判为可启动真实 GUI smoke。
- 本轮没有启动 TextEdit/Safari，没有运行真实 Clipboard recall smoke；当前系统版仍是 v40，真实 GUI smoke 仍需等 v41 安装、TIS ready 后再跑。

## v41 Mac mini 续修：公共 process preflight helper 动态自检

时间：2026-07-08 07:20:18 CST

背景：

- 多个 GUI smoke 入口都依赖公共“已有进程拒跑”纪律，避免 TextEdit/Safari 等用户已有窗口被测试抢走。
- `post-install-regression.sh`、`status.sh`、`await-system-install.sh` 等上层入口已有各自的 process preflight 自检；但 `smoke-common.sh` 的通用 helper `inputia_require_process_not_running()` 本身缺少可执行自检。
- 如果这个公共 helper 以后漂移，上层间接测试不一定能直接暴露“未 allow 时必须拒跑、allow 时必须显式输出 allowed”的基础契约。

实现：

- `smoke-common.sh`
  - `inputia_require_process_not_running()` 新增 `INPUTIA_PROCESS_RUNNING_FOR_TEST` 测试注入。
  - 正常路径仍使用 `/usr/bin/pgrep -x "$process_name"`；测试注入只在 env 显式设置时生效。
- `verify-nongui.sh`
  - 静态契约要求 `INPUTIA_PROCESS_RUNNING_FOR_TEST` 和 `inputia_require_process_not_running()` 存在。
  - 新增 `process preflight helper self-check`：
    - clear case：假进程未运行，输出 `InputiaFakeProcessForTestPreflight=not-running`。
    - block case：注入假进程运行，未 allow 时输出 `guiSmokeReady=false reason=fake-running`、`fakeReady=false reason=fake-running`，并以 rc=44 退出。
    - allow case：注入假进程运行且 allow env 为 1，输出 `InputiaFakeProcessForTestPreflightAllowed=true` 并正常返回。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

bash -c 'source macos/InputiaInputMethod/smoke-common.sh; INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaFakeProcessForTest inputia_require_process_not_running InputiaFakeProcessForTest fakeReady 44 fake-running INPUTIA_FAKE_PROCESS_ALLOW'
  InputiaFakeProcessForTestPreflight=running
  guiSmokeReady=false reason=fake-running
  fakeReady=false reason=fake-running
  rc=44

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-process-preflight-helper-20260708.log 2>&1
  verifyRc=0
  processPreflightClearSelfCheck: InputiaFakeProcessForTestPreflight=not-running
  processPreflightBlockSelfCheck: InputiaFakeProcessForTestPreflight=running
  processPreflightBlockSelfCheck: guiSmokeReady=false reason=fake-running
  processPreflightBlockSelfCheck: fakeReady=false reason=fake-running
  processPreflightBlockSelfCheck.rc=44
  processPreflightAllowSelfCheck: InputiaFakeProcessForTestPreflight=running
  processPreflightAllowSelfCheck: InputiaFakeProcessForTestPreflightAllowed=true
  processPreflightHelperSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 公共 process preflight helper 现在有直接动态自检；以后 TextEdit/Safari 等已有进程拒跑纪律不只依赖上层脚本间接覆盖。
- 本轮没有启动 TextEdit/Safari/Safari diagnose/Clipboard recall 真实 GUI smoke；系统安装版仍是 v40，真实 GUI smoke 仍被版本/admin/TIS 门禁挡住。

## v41 Mac mini 续修：公共 smoke 临时文件清理 helper 动态自检

时间：2026-07-08 07:27:47 CST

背景：

- TextEdit、Safari、Clipboard recall 等 GUI smoke 都依赖 `smoke-common.sh` 的 `inputia_cleanup_smoke_files()` 清理 select/restore/debug/osascript 临时文件。
- 各 smoke 的 cleanup self-check 已经间接证明临时文件不会残留，但公共 helper 自身缺少“默认删除”和“保留日志模式不删除”的直接动态自检。
- 这个 helper 如果漂移，会影响多个 smoke 的失败路径清理纪律，因此需要单独纳入 `verify-nongui.sh`。

实现：

- `verify-nongui.sh`
  - 静态契约新增：
    - `inputia_cleanup_smoke_files()` 必须存在。
    - `INPUTIA_KEEP_SMOKE_LOGS` 保留日志开关必须存在。
    - `smokeTempCleanup=skipped` 保留模式标记必须存在。
  - 新增 `smoke file cleanup helper self-check`：
    - 创建 `/tmp/inputia-smoke-file-cleanup-removed.$$`，调用 helper 后要求文件被删除。
    - 创建 `/tmp/inputia-smoke-file-cleanup-kept.$$`，设置 `INPUTIA_KEEP_SMOKE_LOGS=1` 后要求输出 `smokeTempCleanup=skipped` 且文件仍存在。
    - 自检结束后主动删除 kept 文件，避免 `/tmp` 残留。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

source macos/InputiaInputMethod/smoke-common.sh
inputia_cleanup_smoke_files /tmp/inputia-smoke-file-cleanup-manual.$$
  removed=true

INPUTIA_KEEP_SMOKE_LOGS=1 inputia_cleanup_smoke_files /tmp/inputia-smoke-file-cleanup-manual-keep.$$
  smokeTempCleanup=skipped
  kept=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-smoke-file-cleanup-helper-rerun-20260708.log 2>&1
  verifyRc=0
  smokeFileCleanupRemoved=true
  smokeFileCleanupKeepSelfCheck: smokeTempCleanup=skipped
  smokeFileCleanupKeepPreserved=true
  smokeFileCleanupHelperSelfCheck=true
  processPreflightHelperSelfCheck=true
  safariExistingGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 公共临时文件清理 helper 现在有直接动态自检，覆盖默认删除和 `INPUTIA_KEEP_SMOKE_LOGS=1` 保留日志两条路径。
- 本轮未运行真实 GUI smoke；系统安装版仍是 v40，真实 TextEdit/Safari/Clipboard smoke 仍需等 v41 安装并 TIS ready 后执行。

## v41 Mac mini 续修：post-install 直接入口阻断已运行 Inputia Host

背景：

- `status.sh` 与 `gui-smoke-readiness.sh` 已把 `InputiaInputMethod` 运行中视为真实 GUI smoke blocker。
- 但 `post-install-regression.sh INPUTIA_RUN_UI_SMOKE=1` 也可以被直接调用；此前它的 UI preflight 只检查 TextEdit/Safari，可能绕过 suite readiness，在旧 Host 仍运行时继续调度真实 GUI smoke。

实现：

- `post-install-regression.sh`
  - `INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1` 新增 `inputia-block` case。
  - 真实 UI preflight 在 TextEdit/Safari 后新增 `require_ui_process_idle "InputiaInputMethod" "0" "inputia-host-running"`。
  - 该 blocker 不提供 allow override；真实 GUI smoke 前必须没有运行中的 Inputia Host。
- `verify-nongui.sh`
  - 静态契约新增 `post-install-missing-inputia-host-preflight` 和 `post-install-missing-inputia-host-running-reason`。
  - 动态 self-check 新增 `inputia-block` 输出断言，要求同时出现 `guiSmokeReady=false reason=inputia-host-running` 与 `postInstallUiSmokeReady=false reason=inputia-host-running`，rc 为 4。

验证：

```text
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiPreflightSelfCheck case=inputia-block InputiaInputMethodPreflight=running
  postInstallUiPreflightSelfCheck case=inputia-block guiSmokeReady=false reason=inputia-host-running
  postInstallUiPreflightSelfCheck case=inputia-block postInstallUiSmokeReady=false reason=inputia-host-running
  postInstallUiPreflightSelfCheck case=inputia-block rc=4
  postInstallUiPreflightSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-postinstall-inputia-host-preflight-20260708.log 2>&1
  verifyRc=0
  verify-nongui.sh:1066 require post-install Inputia host preflight
  verify-nongui.sh:1067 require post-install inputia-host-running reason
  verify-nongui.sh:1701 require inputia-block running output
  verify-nongui.sh:1702 require inputia-block guiSmokeReady blocker
  verify-nongui.sh:1703 require inputia-block postInstallUiSmokeReady blocker
  verify-nongui.sh:1704 require inputia-block rc
  postInstallUiPreflightSelfCheck case=inputia-block InputiaInputMethodPreflight=running
  postInstallUiPreflightSelfCheck case=inputia-block guiSmokeReady=false reason=inputia-host-running
  postInstallUiPreflightSelfCheck case=inputia-block postInstallUiSmokeReady=false reason=inputia-host-running
  postInstallUiPreflightSelfCheck case=inputia-block rc=4
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `gui-smoke-suite.sh` 和直接 `post-install-regression.sh INPUTIA_RUN_UI_SMOKE=1` 两条入口现在都不会在 `InputiaInputMethod` 已运行时继续调度真实 TextEdit/Safari/Clipboard GUI smoke。
- 本轮仍没有打开 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；当前真实阻塞仍是系统/settings v40、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：smoke-preflight 阻断已运行 Inputia Host

背景：

- `smoke-preflight.sh` 是真实 TextEdit/Safari/Clipboard smoke 前的只读预检入口。
- `status.sh`、`gui-smoke-readiness.sh` 和 `post-install-regression.sh` 已经把 `InputiaInputMethod` 运行中视为 blocker；`smoke-preflight.sh` 也需要同样阻断，避免直接跑 preflight 时在旧 Host 仍运行的状态下继续进入 TIS/GUI 判断。

实现：

- `smoke-preflight.sh`
  - TextEdit/Safari idle 检查后新增 `InputiaInputMethod` 进程检查。
  - 运行中时输出：
    - `InputiaInputMethodPreflight=running`
    - `guiSmokeReady=false reason=inputia-host-running`
    - `smokePreflightReady=false reason=inputia-host-running`
    - rc `9`
  - 未运行时输出 `InputiaInputMethodPreflight=not-running`，再继续 TIS readiness。
- `verify-nongui.sh`
  - 静态契约新增 `smoke-preflight-missing-inputia-host-preflight-output`、`smoke-preflight-missing-inputia-host-running-reason`、`smoke-preflight-missing-inputia-host-ready-block`。
  - 动态 gate 新增 `buildPreflightInputiaHostGate`，用 fake `InputiaInputMethod` 进程验证 preflight 会在 TIS gate 前安全早退，且不污染剪贴板、当前输入源、debug env 或 user host，也不启动 TextEdit/Safari。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-preflight.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
  macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  fake InputiaInputMethod running
  textEditPreflight=not-running docs=0
  safariPreflight=not-running
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  smokePreflightReady=false reason=inputia-host-running
  preflightRc=9

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-smoke-preflight-inputia-host-clean-20260708.log 2>&1
  verifyRc=0
  verify-nongui.sh:1050 require smoke-preflight Inputia host preflight output
  verify-nongui.sh:1051 require smoke-preflight inputia-host-running reason
  verify-nongui.sh:1052 require smoke-preflight ready block
  buildPreflightInputiaHostGate: textEditPreflight=not-running docs=0
  buildPreflightInputiaHostGate: safariPreflight=not-running
  buildPreflightInputiaHostGate: InputiaInputMethodPreflight=running
  buildPreflightInputiaHostGate: guiSmokeReady=false reason=inputia-host-running
  buildPreflightInputiaHostGate: smokePreflightReady=false reason=inputia-host-running
  buildPreflightInputiaHostGate.rc=9
  buildPreflightInputiaHostGate.clipboardUnchanged=true
  buildPreflightInputiaHostGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  buildPreflightInputiaHostGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  buildPreflightInputiaHostGate.debugEnvBefore=unset
  buildPreflightInputiaHostGate.debugEnvAfter=unset
  buildPreflightInputiaHostGate.userHost=false
  buildPreflightInputiaHostGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `smoke-preflight.sh`、`gui-smoke-readiness.sh`、`gui-smoke-suite.sh` 和 `post-install-regression.sh` 现在都把已运行的 `InputiaInputMethod` 视为真实 GUI smoke blocker。
- fake Host gate 已证明不会启动 TextEdit/Safari，也不会污染剪贴板、当前输入源、debug env 或 user host。
- 当前真实阻塞仍未变：系统/settings v40、build/pkg v41、TIS matches 0；真实 GUI smoke 仍未运行。

## v41 Mac mini 续修：await-system-install UI smoke 状态阻断已运行 Inputia Host

背景：

- `await-system-install.sh` 是等待系统安装/TIS 就绪后判断是否可以继续 UI smoke 的入口。
- `smoke-preflight.sh`、`gui-smoke-readiness.sh`、`gui-smoke-suite.sh` 和 `post-install-regression.sh` 已经把运行中的 `InputiaInputMethod` 视为 blocker；`await-system-install.sh` 也需要保持同一规则，否则等待安装路径可能在旧 Host 仍运行时报告 `uiSmokeWouldStart=true`。

实现：

- `await-system-install.sh`
  - `ui_smoke_status_line` 在 TextEdit/Safari preflight 后新增 `InputiaInputMethod` preflight。
  - 输出新增 `uiInputiaHostPreflight=running|not-running`。
  - `InputiaInputMethod` 运行中时输出 `uiSmokeBlockReason=inputia-host-running`、`uiSmokeBlockReasons=inputia-host-running`、`uiSmokeWouldStart=false`。
  - 该 blocker 不提供 allow override；即使 TextEdit/Safari allow case 为 true，Host 运行中仍不能启动真实 UI smoke。
- `verify-nongui.sh`
  - 静态契约新增 await-system 的 `uiInputiaHostPreflight=` 和 `inputia-host-running` 检查。
  - `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 新增 `inputia-host-running` case。
  - 动态断言要求该 case 同时出现 `uiInputiaHostPreflight=running`、`uiSmokeWouldStart=false` 和 `uiSmokeBlockReason=inputia-host-running`。

验证：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh | rg "inputia-host-running|awaitUiStatusSelfCheck=true|uiInputiaHostPreflight"
  awaitUiStatusSelfCheck reason=textedit-already-running uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiInputiaHostPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=textedit-already-running uiSmokeBlockReasons=textedit-already-running
  awaitUiStatusSelfCheck reason=safari-already-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiInputiaHostPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=safari-already-running uiSmokeBlockReasons=safari-already-running
  awaitUiStatusSelfCheck reason=inputia-host-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=not-running uiInputiaHostPreflight=running uiSmokeWouldStart=false uiSmokeBlockReason=inputia-host-running uiSmokeBlockReasons=inputia-host-running
  awaitUiStatusSelfCheck reason=textedit-allow uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiInputiaHostPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiStatusSelfCheck reason=safari-allow uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiInputiaHostPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiStatusSelfCheck=true

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-inputia-host-preflight-20260708.log 2>&1
  verifyRc=0
  awaitUiStatusSelfCheck reason=inputia-host-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=not-running uiInputiaHostPreflight=running uiSmokeWouldStart=false uiSmokeBlockReason=inputia-host-running uiSmokeBlockReasons=inputia-host-running
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 所有当前真实 GUI smoke 调度前入口都已把运行中的 `InputiaInputMethod` 视为 blocker。
- `await-system-install.sh` 不再可能在旧 Host 仍运行时报告 `uiSmokeWouldStart=true`。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：status 摘要暴露 Inputia Host preflight

背景：

- `status.sh` 已经把运行中的 `InputiaInputMethod` 加入 `statusGuiSmokeBlockReasons`。
- 但 GUI smoke 摘要只打印 TextEdit/Safari preflight，缺少 Host preflight 可见字段；排查时无法直接从 status 摘要看出 Host 是否是阻断来源。

实现：

- `status.sh`
  - GUI smoke summary 新增 `statusInputiaHostPreflight=running|not-running`。
  - 该字段直接来自 `running host` 检测结果，并与 `inputia-host-running` blocker 保持一致。
- `verify-nongui.sh`
  - 静态契约新增 `statusInputiaHostPreflight=` 和 `inputia-host-running` 检查。
  - status 摘要过滤新增 `statusInputiaHostPreflight`。
  - 当前状态断言要求 Host preflight 为 `not-running`。
  - 新增 fake `InputiaInputMethod` self-check，要求 status 输出 `statusInputiaHostPreflight=running`，并把 `inputia-host-running` 加入 block reasons / ready reason。

验证：

```text
zsh -n macos/InputiaInputMethod/status.sh
  syntaxRc=0

bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/status.sh | rg "status(InputiaHostPreflight|GuiSmokeBlockReasons|GuiSmokeReady)|running="
  running=false
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-status-inputia-host-preflight-20260708.log 2>&1
  verifyRc=0
  status: statusInputiaHostPreflight=not-running
  statusInputiaHostBlockerSelfCheck=true
  statusBlockerSelfCheck.clipboardUnchanged=true
  statusBlockerSelfCheck.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  statusBlockerSelfCheck.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  statusBlockerSelfCheck.debugEnvBefore=unset
  statusBlockerSelfCheck.debugEnvAfter=unset
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- `status.sh` 现在能在同一摘要中同时暴露 TextEdit、Safari 和 Inputia Host preflight。
- fake Host 自检证明 status 层的 `inputia-host-running` blocker 可见且不会污染剪贴板、当前输入源或 debug env。
- 真实 GUI smoke 仍未运行；当前阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：会重启 Host 的 smoke 先阻断旧 Inputia Host

背景：

- `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 会在设置 `INPUTIA_DEBUG_EVENTS` 后 `killall InputiaInputMethod`，让新 Host 带 debug env 启动。
- 这个重启只应发生在脚本自己选择输入源之后；如果脚本启动前已有旧 Host 运行，继续执行会绕过“旧 Host 运行中不硬跑 GUI smoke”的纪律，并可能杀掉用户当前输入法进程。

实现：

- `smoke-clipboard-recall.sh`
  - TextEdit idle gate 后新增 `inputia_require_process_not_running "InputiaInputMethod"`。
  - 旧 Host 运行中时输出 `InputiaInputMethodPreflight=running`、`guiSmokeReady=false reason=inputia-host-running`、`clipboardRecallSmokeReady=false reason=inputia-host-running`，rc `10`。
- `smoke-safari-enter.sh`
  - Safari idle gate 后新增同样 Host preflight。
  - 旧 Host 运行中时输出 `safariEnterSmokeReady=false reason=inputia-host-running`，rc `11`。
- `verify-nongui.sh`
  - 静态契约要求两个脚本都有 Host preflight，且顺序必须是 Host preflight < select input source < debug setenv < killall。
  - 动态 gate 新增 `clipboardInputiaHostGate` 和 `safariEnterInputiaHostGate`，用 `INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod` 验证早退，不进入剪贴板 mutation / TIS selection / Safari 或 TextEdit 启动。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh
bash -n macos/InputiaInputMethod/smoke-safari-enter.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
  INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  clipboardRecallSmokeReady=false reason=inputia-host-running
  rc=10

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
  INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
  macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  safariEnterSmokeReady=false reason=inputia-host-running
  rc=11

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-restart-host-preflight-20260708.log 2>&1
  verifyRc=0
  clipboardInputiaHostGate: InputiaInputMethodPreflight=running
  clipboardInputiaHostGate: guiSmokeReady=false reason=inputia-host-running
  clipboardInputiaHostGate: clipboardRecallSmokeReady=false reason=inputia-host-running
  clipboardInputiaHostGate.rc=10
  clipboardInputiaHostGate.clipboardUnchanged=true
  clipboardInputiaHostGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardInputiaHostGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardInputiaHostGate.debugEnvBefore=unset
  clipboardInputiaHostGate.debugEnvAfter=unset
  clipboardInputiaHostGate.userHost=false
  clipboardInputiaHostGateNoLaunchPassed=true
  safariEnterInputiaHostGate: InputiaInputMethodPreflight=running
  safariEnterInputiaHostGate: guiSmokeReady=false reason=inputia-host-running
  safariEnterInputiaHostGate: safariEnterSmokeReady=false reason=inputia-host-running
  safariEnterInputiaHostGate.rc=11
  safariEnterInputiaHostGate.clipboardUnchanged=true
  safariEnterInputiaHostGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterInputiaHostGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterInputiaHostGate.debugEnvBefore=unset
  safariEnterInputiaHostGate.debugEnvAfter=unset
  safariEnterInputiaHostGate.userHost=false
  safariEnterInputiaHostGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 直接调用 `smoke-clipboard-recall.sh` 或 `smoke-safari-enter.sh` 时，若旧 `InputiaInputMethod` 已运行，会在 killall 之前安全早退。
- 允许的 Host 重启路径只保留在脚本完成输入源选择并设置 debug env 之后，用于该 smoke 自己的可观测 Host。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：剪贴板召回菜单不再注册 `Command-V`

背景：

- 用户反馈在 Inputia 下 `Command-C` / `Command-V` 不能用，并要求按常用电脑快捷键举一反三，不要逐个等用户手测。
- Apple Support `Mac keyboard shortcuts` 将 `Command-X/C/V/Z/A/F/G/H/M/N/O/P/S/T/W/Q`、`Command-Tab`、`Command-Space`、`Shift-Command-3/4/5`、`Option-Command-Esc` 等列为 macOS 常用系统/App 快捷键。
- Apple Developer `NSMenuItem.init(title:action:keyEquivalent:)` 说明没有 key equivalent 时应传空字符串。当前 Inputia keyDown 已统一透传所有含 `Command` 的组合，但输入法菜单里的“召回剪贴板”曾用 `keyEquivalent: "v"`；菜单快捷键路径可能绕过 keyDown 分类器并抢宿主 `Command-V`。

实现：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 的根规则仍是任何包含 `.command` 的 keyDown 直接返回 `true`。
- `InputiaInputController.handleKeyDown` 仍在剪贴板召回、标点切换、全半角切换、输入模式切换、候选导航之前先执行 Command 透传并 `return false`。
- `InputiaHostTextPolicy.recallClipboardMenuKeyEquivalent = ""`，输入法菜单里的“召回剪贴板”改为使用该空 key equivalent，不再注册菜单级 `v` 快捷键。
- `InputiaHostTextPolicySelfCheck` 新增 `recallClipboardMenuHasNoCommandKeyEquivalent=true`。
- `verify-nongui.sh` 新增静态契约：
  - Host policy 必须保持 `recallClipboardMenuKeyEquivalent = ""`。
  - Host menu 不能再出现 `NSMenuItem(... keyEquivalent: "v")`。
  - Host menu 必须从 policy 常量读取 key equivalent。
  - 非 GUI verifier 必须看到 `recallClipboardMenuHasNoCommandKeyEquivalent=true`。

验证：

```text
/usr/bin/swiftc -parse macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaHostTextPolicy.swift macos/InputiaInputMethod/Tools/InputiaHostTextPolicySelfCheck.swift
/usr/bin/swiftc -parse macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift macos/InputiaInputMethod/Tools/InputiaShortcutSelfCheck.swift
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/build-pkg.sh
  pkgVerificationPassed=true
  archiveAppCDHash=913e882766c8d13d8e39ecdc60d377b556945da0
  buildAppCDHash=913e882766c8d13d8e39ecdc60d377b556945da0

macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-command-menu-keyequiv-final-20260708.log 2>&1
  verifyRc=0
  shortcut: shortcutSelfCheck=true
  shortcut: commonAppleCommandShortcutSetPassesThrough=true
  shortcut: anyCommandModifiedKeyPassesThrough=true
  shortcut: allCommandModifierVariantsPassThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  shortcut: ctrlShiftCommandVRejected=true
  hostShortcut: hostShortcutSelfCheck=true
  hostShortcut: commonAppleCommandShortcutSetPassesThrough=true
  hostShortcut: anyCommandModifiedKeyPassesThrough=true
  hostShortcut: allCommandModifierVariantsPassThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  hostShortcut: ctrlShiftCommandVRejected=true
  hostTextPolicy: hostTextPolicySelfCheck=true
  hostTextPolicy: recallClipboardMenuHasNoCommandKeyEquivalent=true
  postInstallUiTisGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/status.sh | rg "^(version|systemMatchesBuild|systemSettingsMatchesBuildVersion|running|statusInputiaHostPreflight|statusGuiSmokeBlockReasons|statusGuiSmokeReady)="
  version=41
  version=40
  systemMatchesBuild=false
  version=40
  systemSettingsMatchesBuildVersion=false
  running=false
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

residue checks:
  verifier/smoke shell residue: none
  TextEdit/Safari/osascript/InputiaInputMethod: none
  /tmp inputia smoke residue: none
```

当前结论：

- `Command-C/V` 失效的一个真实高风险入口已经补掉：输入法菜单不再把“召回剪贴板”注册成菜单级 `v` key equivalent。
- 这不是只修 `Command-V` 特例；keyDown 层仍统一透传所有含 `Command` 的组合，host selector 层仍透传常见 AppKit command selector，自有剪贴板召回继续只使用非 Command 的 `Control-Shift-V`。
- 真实 TextEdit/Safari GUI smoke 仍未运行；当前系统安装版仍是 v40，build/pkg 为 v41，且有 admin/TIS/version 门禁。

## v41 Mac mini 续修：Host blocker 不允许环境变量绕过

背景：

- 上一轮把 `smoke-clipboard-recall.sh` 和 `smoke-safari-enter.sh` 接到公共 `inputia_require_process_not_running` helper。
- 公共 helper 原本支持 allow env；如果 Host blocker 也传入 allow env，调用者可通过环境变量绕过旧 Host 阻断并继续执行后续 `killall InputiaInputMethod`。
- 这和真实 GUI smoke 纪律冲突：旧 Host 运行中必须早退，不能提供 allow override。

实现：

- `smoke-common.sh`
  - `inputia_require_process_not_running` 支持 allow var 为 `-` 的 sentinel。
  - `-` 表示不可配置 allow，`allow_value` 固定为 `0`。
- `smoke-clipboard-recall.sh`
  - Host preflight 的 allow var 改为 `-`。
- `smoke-safari-enter.sh`
  - Host preflight 的 allow var 改为 `-`。
- `verify-nongui.sh`
  - 静态契约要求两个 Host preflight 使用 `"inputia-host-running" "-"`。
  - 静态契约禁止脚本出现 `INPUTIA_HOST_SMOKE_ALLOW_EXISTING`。
  - 新增 `processPreflightNoAllowSelfCheck`：即使设置 allow env，`-` sentinel 仍必须以 rc `45` 阻断。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh
bash -n macos/InputiaInputMethod/smoke-safari-enter.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
  INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod INPUTIA_HOST_SMOKE_ALLOW_EXISTING=1 \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  clipboardRecallSmokeReady=false reason=inputia-host-running
  rc=10

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
  INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod INPUTIA_HOST_SMOKE_ALLOW_EXISTING=1 \
  macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  safariEnterSmokeReady=false reason=inputia-host-running
  rc=11

bash -c 'source macos/InputiaInputMethod/smoke-common.sh; INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaFakeProcessForTest INPUTIA_FAKE_PROCESS_ALLOW=1 inputia_require_process_not_running InputiaFakeProcessForTest fakeReady 45 fake-running -'
  InputiaFakeProcessForTestPreflight=running
  guiSmokeReady=false reason=fake-running
  fakeReady=false reason=fake-running
  rc=45

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-host-no-allow-sentinel-20260708.log 2>&1
  verifyRc=0
  processPreflightNoAllowSelfCheck.rc=45
  clipboardInputiaHostGate.rc=10
  safariEnterInputiaHostGate.rc=11
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Host blocker 现在不可被环境变量绕过。
- TextEdit/Safari allow-existing 分支仍保留原 helper 语义；仅 Host preflight 使用 `-` sentinel。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：聚合 GUI smoke 入口强制 TextEdit/Safari 未运行

背景：

- 底层 TextEdit/Safari smoke helper 保留 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1` / `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1`，用于 no-launch 自检和少数显式诊断路径。
- 真实 GUI smoke 聚合入口不应继承这些外部环境变量；否则用户或上游脚本设置 allow env 后，`gui-smoke-suite.sh` / `post-install-regression.sh` 可能绕过“测试前 TextEdit/Safari 必须未运行”的纪律。

实现：

- `post-install-regression.sh`
  - UI smoke preflight 对 TextEdit/Safari 传固定 `"0"`，不再读取外部 allow-existing env。
  - `INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1` 仍保留 helper 的 allow 分支自检，证明底层能力存在但真实聚合路径不用它。
- `gui-smoke-suite.sh`
  - 调用 `gui-smoke-readiness.sh` 时强制 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0`、`INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0`。
  - 调用 `post-install-regression.sh` 时同样强制两个 allow env 为 `0`。
- `verify-nongui.sh`
  - 静态契约要求 suite 强制清零两个 allow env。
  - 静态契约要求 post-install 的真实 UI preflight 对 TextEdit/Safari 传固定 `"0"`。

验证：

```text
zsh -n macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/post-install-regression.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-suite.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSmokeSuiteSelfCheck=true

INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiPreflightSelfCheck case=textedit-block rc=4
  postInstallUiPreflightSelfCheck case=safari-block rc=4
  postInstallUiPreflightSelfCheck case=inputia-block rc=4
  postInstallUiPreflightSelfCheck case=textedit-allow rc=0
  postInstallUiPreflightSelfCheck case=safari-allow rc=0
  postInstallUiPreflightSelfCheck=true

macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-suite-force-no-existing-final-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  processPreflightNoAllowSelfCheck.rc=45
  guiSmokeSuiteSelfCheck=true
  postInstallUiPreflightSelfCheck=true
  postInstallUiTisGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/status.sh | rg "^(version|systemMatchesBuild|systemSettingsMatchesBuildVersion|running|statusInputiaHostPreflight|statusGuiSmokeBlockReasons|statusGuiSmokeReady)="
  version=41
  version=40
  systemMatchesBuild=false
  version=40
  systemSettingsMatchesBuildVersion=false
  running=false
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

residue checks:
  verifier/smoke shell residue: none
  TextEdit/Safari/osascript/InputiaInputMethod: none
  /tmp inputia smoke residue: none
```

当前结论：

- `gui-smoke-suite.sh` 和 `post-install-regression.sh` 现在不会被外部 allow-existing 环境变量绕过；真实聚合 GUI smoke 入口始终要求 TextEdit/Safari 预先未运行。
- 底层 allow-existing 自检仍存在，避免丢掉 no-launch 验证能力。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 GUI smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：smoke-preflight 不接受 TextEdit/Safari allow-existing

背景：

- `smoke-preflight.sh` 是真实 TextEdit/Safari/Clipboard GUI smoke 前的只读预检入口。
- 底层 smoke 脚本仍保留 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1` / `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1` 用于显式诊断和 no-launch 自检；但 preflight 的语义应是“现在是否可以安全开始真实 GUI smoke”，不能被外部 allow env 改成 ready。

实现：

- `smoke-preflight.sh`
  - 调用 TextEdit preflight 时强制 `INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0`。
  - 调用 Safari preflight 时强制 `INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0`。
- `verify-nongui.sh`
  - 静态契约要求 `smoke-preflight.sh` 对 TextEdit/Safari 使用强制 no-existing 调用。
  - 新增 fake TextEdit/Safari 动态 gate：即使外部 allow env 为 `1`，`smoke-preflight.sh` 仍必须早退。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-smoke-preflight-no-allow-20260708.log 2>&1
  verifyRc=0
  cleanupPermissionContract=true
  buildPreflightTextEditNoAllowGate: textEditPreflight=running docs=0
  buildPreflightTextEditNoAllowGate: guiSmokeReady=false reason=textedit-already-running
  buildPreflightTextEditNoAllowGate: smokePreflightReady=false reason=textedit-already-running
  buildPreflightTextEditNoAllowGate.rc=6
  buildPreflightTextEditNoAllowGate.clipboardUnchanged=true
  buildPreflightTextEditNoAllowGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  buildPreflightTextEditNoAllowGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  buildPreflightTextEditNoAllowGate.debugEnvBefore=unset
  buildPreflightTextEditNoAllowGate.debugEnvAfter=unset
  buildPreflightTextEditNoAllowGate.userHost=false
  buildPreflightTextEditNoAllowGateNoLaunchPassed=true
  buildPreflightSafariNoAllowGate: safariPreflight=running
  buildPreflightSafariNoAllowGate: guiSmokeReady=false reason=safari-already-running
  buildPreflightSafariNoAllowGate: smokePreflightReady=false reason=safari-already-running
  buildPreflightSafariNoAllowGate.rc=7
  buildPreflightSafariNoAllowGate.clipboardUnchanged=true
  buildPreflightSafariNoAllowGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  buildPreflightSafariNoAllowGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  buildPreflightSafariNoAllowGate.debugEnvBefore=unset
  buildPreflightSafariNoAllowGate.debugEnvAfter=unset
  buildPreflightSafariNoAllowGate.userHost=false
  buildPreflightSafariNoAllowGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true

macos/InputiaInputMethod/status.sh | rg "^(version|systemMatchesBuild|systemSettingsMatchesBuildVersion|running|statusInputiaHostPreflight|statusGuiSmokeBlockReasons|statusGuiSmokeReady)="
  version=41
  version=40
  systemMatchesBuild=false
  version=40
  systemSettingsMatchesBuildVersion=false
  running=false
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

residue checks:
  verifier/smoke shell residue: none
  TextEdit/Safari/osascript/InputiaInputMethod: none
  /tmp inputia smoke residue: none
```

当前结论：

- `smoke-preflight.sh`、`gui-smoke-suite.sh` 和 `post-install-regression.sh` 现在都不会被外部 TextEdit/Safari allow-existing env 绕过。
- 直接运行底层 TextEdit/Safari smoke 脚本时，allow-existing 仍可用于明确诊断；但“是否可以安全开始真实 GUI smoke”的入口统一保持严格。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 GUI smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：用户级安装避开正在运行的 smoke/verifier

背景：

- `install-system.sh` 已经有 verification process preflight，避免在 smoke/verifier 正在运行时继续安装并 `killall InputiaInputMethod`。
- `install-user.sh` 同样会执行 `killall InputiaInputMethod`，但此前缺少同类 guard；如果在 GUI smoke 或非 GUI verifier 运行中误调用，可能干扰当前验证状态。

实现：

- `install-user.sh`
  - 新增 `detect_verification_processes` / `require_no_verification_processes`。
  - 在 build、copy、`killall InputiaInputMethod` 之前先检查当前工作区的 verifier/smoke/status/tis 脚本。
  - 检测到阻塞进程时输出 `userInstallReady=false reason=verification-running` 和 `userInstallBlockingProcess:`，rc `20`。
  - 新增 `INPUTIA_INSTALL_USER_PREFLIGHT_SELF_CHECK=1`，只跑 preflight 自检，不触发 build/install/killall。
- `verify-nongui.sh`
  - 静态契约要求 user install preflight 在 build/killall 之前。
  - 新增 `user install preflight self-check`。
  - status session blocker self-check 改为检查必需 reason 集合，允许 `status.sh` 查询期间短暂出现额外 `inputia-host-running`，避免 transient Host 造成假失败。

验证：

```text
zsh -n macos/InputiaInputMethod/install-user.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_INSTALL_USER_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/install-user.sh
  userInstallPreflightSelfCheck clear=true
  userInstallPreflightSelfCheck blocked=true
  userInstallPreflightSelfCheck=true

INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-clipboard-recall.sh" \
  macos/InputiaInputMethod/install-user.sh
  userInstallReady=false reason=verification-running
  userInstallBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-clipboard-recall.sh
  rc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-install-user-preflight-20260708-rerun.log 2>&1
  verifyRc=0
  userInstallPreflightSelfCheck clear=true
  userInstallPreflightSelfCheck blocked=true
  userInstallPreflightSelfCheck=true
  statusGuiSessionBlockerSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 系统级安装和用户级安装现在都不会在当前工作区 smoke/verifier 运行中继续进入会杀 Host 的安装路径。
- 这不改变显式安装语义，只给验证/调试期间加了防误伤 guard。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：打包入口避开正在运行的 smoke/verifier

背景：

- `build.sh`、`install-system.sh` 和 `install-user.sh` 已有 verifier/smoke preflight。
- `build-pkg.sh` 会调用 `build.sh`，随后清理并重写 `build/pkg-scripts` 和 `dist`；如果在 smoke/verifier 运行中误调用，仍可能改写验证所依赖的 pkg 产物。

实现：

- `build-pkg.sh`
  - 新增 `detect_verification_processes` / `require_no_verification_processes`。
  - 在调用 `build.sh` 和 `rm -rf "$PKG_SCRIPTS_DIR" "$DIST_DIR"` 前先检查当前工作区的 verifier/smoke/status/tis 脚本。
  - 检测到阻塞进程时输出 `buildPkgReady=false reason=verification-running` 和 `buildPkgBlockingProcess:`，rc `20`。
  - 新增 `INPUTIA_BUILD_PKG_PREFLIGHT_SELF_CHECK=1`，只跑 preflight 自检，不触发 build/pkg/dist 清理。
- `verify-nongui.sh`
  - 静态契约要求 build-pkg preflight 在 build 和 dist 清理之前。
  - 新增 `build package preflight self-check`。

验证：

```text
zsh -n macos/InputiaInputMethod/build-pkg.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_BUILD_PKG_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/build-pkg.sh
  buildPkgPreflightSelfCheck clear=true
  buildPkgPreflightSelfCheck blocked=true
  buildPkgPreflightSelfCheck=true

INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/gui-smoke-suite.sh" \
  macos/InputiaInputMethod/build-pkg.sh
  buildPkgReady=false reason=verification-running
  buildPkgBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/gui-smoke-suite.sh
  rc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-build-pkg-preflight-20260708-rerun.log 2>&1
  verifyRc=0
  buildPkgPreflightSelfCheck clear=true
  buildPkgPreflightSelfCheck blocked=true
  buildPkgPreflightSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

备注：

- 首次完整验证在静态检查阶段失败后留下了 stale `/tmp/inputia-verify-nongui.lock`；确认 owner pid 已不存在且无 GUI/Host 残留后清理该 stale lock，并重跑得到上面的通过结果。
- 当前真实 GUI smoke 仍未运行；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：设置页启动避开正在运行的 smoke/verifier

背景：

- `open-settings.sh` 会优先 `/usr/bin/open -n` 打开已安装的 `Inputia 设置.app`，否则用 Host `--open-settings` 打开设置页。
- 这个入口如果在 TextEdit/Safari/Clipboard GUI smoke 运行中被误调用，会抢前台焦点或拉起 Host，破坏当前正在修的 GUI smoke 清理纪律。

实现：

- `open-settings.sh`
  - 新增 `detect_verification_processes` / `require_no_verification_processes`。
  - 在读取 bundle 版本和任何 `/usr/bin/open -n` 之前检查当前工作区的 verifier/smoke/status/tis 脚本。
  - 检测到阻塞进程时输出 `openSettingsReady=false reason=verification-running` 和 `openSettingsBlockingProcess:`，rc `20`。
  - 新增 `INPUTIA_OPEN_SETTINGS_PREFLIGHT_SELF_CHECK=1`，只跑 preflight 自检，不打开设置 launcher 或 Host。
- `verify-nongui.sh`
  - 将 `open-settings.sh` 纳入语法检查。
  - 静态契约要求 open-settings preflight 出现在首个 `/usr/bin/open -n` 之前。
  - 新增 `open settings preflight self-check`。

验证：

```text
zsh -n macos/InputiaInputMethod/open-settings.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_OPEN_SETTINGS_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/open-settings.sh
  openSettingsPreflightSelfCheck clear=true
  openSettingsPreflightSelfCheck blocked=true
  openSettingsPreflightSelfCheck=true

INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-textedit.sh" \
  macos/InputiaInputMethod/open-settings.sh
  openSettingsReady=false reason=verification-running
  openSettingsBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-textedit.sh
  rc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-open-settings-preflight-20260708.log 2>&1
  verifyRc=0
  openSettingsPreflightSelfCheck clear=true
  openSettingsPreflightSelfCheck blocked=true
  openSettingsPreflightSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 设置页启动入口现在不会在当前工作区 smoke/verifier 运行中打开设置 launcher 或 Host。
- 这不改变手动打开设置的正常路径，只在验证脚本并发运行时拒绝抢焦点/拉 Host。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 复核：常用 `Command` 快捷键不由输入法接管

背景：

- 用户反馈在 Inputia 下 `Command-C` / `Command-V` 不能用，并明确要求按常用电脑快捷键举一反三处理，不能靠用户逐个测试。
- Apple Support `Mac keyboard shortcuts` 将复制、粘贴、剪切、撤销、全选、查找、保存、打开、打印、关闭窗口、退出、隐藏、App 切换、Spotlight、截图、强制退出等列为 macOS 常用系统/App 快捷键，核心都是 `Command` 或含 `Command` 的组合。
- Apple Developer `Handling Key Events` 说明 Cocoa 文本系统会把按键事件转换成 `doCommandBySelector:` 或 `insertText:` 路径；输入法 host 只应处理自己的输入命令，宿主 App 命令 selector 应交回响应链。
- Apple Developer `NSMenuItem.init(title:action:keyEquivalent:)` 说明菜单项的 key equivalent 是独立快捷键入口；因此不能只修 `keyDown`，还必须避免 Inputia 菜单注册会抢宿主的 `Command-V`。

实现边界：

- `InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:)` 根规则：任何包含 `.command` 的 `keyDown` 都返回 `true`。
- `InputiaInputController.handleKeyDown` 在剪贴板召回、标点切换、全半角切换、候选导航之前先执行 Command 透传并 `return false`。
- `InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName:)` 覆盖常见 AppKit selector：`copy:`、`paste:`、`cut:`、`undo:`、`redo:`、`selectAll:`、`saveDocument:`、`openDocument:`、`performClose:`、`terminate:`、`find:`、`print:`、`showPreferences:`、`toggleBold:`、`toggleItalic:`、`toggleUnderline:`、`goBack:`、`goForward:`、`reload:`、`stopLoading:` 等。
- “召回剪贴板”菜单使用 `InputiaHostTextPolicy.recallClipboardMenuKeyEquivalent = ""`，不再注册菜单级 `v` key equivalent；Inputia 自有召回仍只接受不含 Command 的 `Control-Shift-V`，`Control-Shift-Command-V` 明确拒绝。

验证：

```text
macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-command-shortcuts-final-20260708.log 2>&1
  verifyRc=0
  shortcut: shortcutSelfCheck=true
  shortcut: commonAppleCommandShortcutSetPassesThrough=true
  shortcut: anyCommandModifiedKeyPassesThrough=true
  shortcut: allCommandModifierVariantsPassThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  shortcut: commandShiftVPassThrough=true
  shortcut: commandOptionShiftVPassThrough=true
  shortcut: commandControlVPassThrough=true
  shortcut: ctrlShiftCommandVRejected=true
  hostShortcut: commonAppleCommandShortcutSetPassesThrough=true
  hostShortcut: anyCommandModifiedKeyPassesThrough=true
  hostShortcut: allCommandModifierVariantsPassThrough=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  hostShortcut: commandShiftVPassThrough=true
  hostShortcut: commandOptionShiftVPassThrough=true
  hostShortcut: commandControlVPassThrough=true
  hostShortcut: ctrlShiftCommandVRejected=true
  hostTextPolicy: hostTextPolicySelfCheck=true
  hostTextPolicy: recallClipboardMenuHasNoCommandKeyEquivalent=true
  hostTextPolicy: appCommandcopyPassesThrough=true
  hostTextPolicy: appCommandpastePassesThrough=true
  hostTextPolicy: appCommandcutPassesThrough=true
  hostTextPolicy: appCommandundoPassesThrough=true
  hostTextPolicy: appCommandredoPassesThrough=true
  hostTextPolicy: appCommandselectAllPassesThrough=true
  hostTextPolicy: appCommandsaveDocumentPassesThrough=true
  hostTextPolicy: appCommandopenDocumentPassesThrough=true
  hostTextPolicy: appCommandperformClosePassesThrough=true
  hostTextPolicy: appCommandterminatePassesThrough=true
  hostTextPolicy: appCommandfindPassesThrough=true
  hostTextPolicy: appCommandprintPassesThrough=true
  hostTextPolicy: appCommandshowPreferencesPassesThrough=true
  hostTextPolicy: appCommandtoggleBoldPassesThrough=true
  hostTextPolicy: appCommandgoBackPassesThrough=true
  hostTextPolicy: appCommandreloadPassesThrough=true
  hostTextPolicy: appCommandstopLoadingPassesThrough=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

状态：

- 本轮没有启动 TextEdit/Safari，没有运行真实 GUI smoke；真实 GUI smoke 仍被系统/settings v40、build/pkg v41、TIS missing、无非交互管理员安装能力挡住。
- 因为当前系统安装版仍是 v40，用户机器上仍可能继续看到旧版本抢快捷键；需要安装/启用 v41 后，真实 TextEdit/Safari command smoke 才有意义。

## v41 Mac mini 续修：Installer GUI 入口避开正在运行的 smoke/verifier

背景：

- `open-installer.sh` 会先调用 `build-pkg.sh`，再 `/usr/bin/open "$pkg_path"` 拉起 Installer。
- `build-pkg.sh` 已能在 smoke/verifier 运行中阻断，但 `open-installer.sh` 自己没有明确 preflight 输出；为避免未来绕过或误判，需要在 GUI 打开入口本身建立契约。

实现：

- `open-installer.sh`
  - 新增 `detect_verification_processes` / `require_no_verification_processes`。
  - 在调用 `build-pkg.sh` 和任何 `/usr/bin/open` 前检查当前工作区的 verifier/smoke/status/tis 脚本。
  - 检测到阻塞进程时输出 `openInstallerReady=false reason=verification-running` 和 `openInstallerBlockingProcess:`，rc `20`。
  - 新增 `INPUTIA_OPEN_INSTALLER_PREFLIGHT_SELF_CHECK=1`，只跑 preflight 自检，不打包、不打开 Installer。
- `verify-nongui.sh`
  - 将 `open-installer.sh` 纳入语法检查。
  - 静态契约要求 open-installer preflight 出现在 `build-pkg.sh` 和 `/usr/bin/open "$pkg_path"` 之前。
  - 新增 `open installer preflight self-check`。

验证：

```text
zsh -n macos/InputiaInputMethod/open-installer.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_OPEN_INSTALLER_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/open-installer.sh
  openInstallerPreflightSelfCheck clear=true
  openInstallerPreflightSelfCheck blocked=true
  openInstallerPreflightSelfCheck=true

INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-clipboard-recall.sh" \
  macos/InputiaInputMethod/open-installer.sh
  openInstallerReady=false reason=verification-running
  openInstallerBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-clipboard-recall.sh
  rc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-open-installer-preflight-20260708.log 2>&1
  verifyRc=0
  openInstallerPreflightSelfCheck clear=true
  openInstallerPreflightSelfCheck blocked=true
  openInstallerPreflightSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Installer GUI 入口现在不会在当前工作区 smoke/verifier 运行中进入打包或打开 Installer。
- 这不改变手动打开 Installer 的正常路径，只在验证脚本并发运行时拒绝抢焦点/拉 Installer。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：卸载入口避开正在运行的 smoke/verifier

背景：

- `uninstall-system.sh` 和 `uninstall-user.sh` 会 disable Inputia input source、注销 LaunchServices、删除 app/settings launcher，并 `killall InputiaInputMethod`。
- 如果这些入口在 TextEdit/Safari/Clipboard GUI smoke 或 verifier 运行中被误触发，会破坏当前安装状态、TIS 状态和 Host 生命周期，导致 smoke 结果不可解释。

实现：

- `uninstall-system.sh`
  - 新增 `detect_verification_processes` / `require_no_verification_processes`。
  - 在 `--disable-input-source`、`rm -rf`、管理员 `osascript` 和 `killall` 前检查当前工作区的 verifier/smoke/status/tis 脚本。
  - 检测到阻塞进程时输出 `systemUninstallReady=false reason=verification-running` 和 `systemUninstallBlockingProcess:`，rc `20`。
  - 新增 `INPUTIA_UNINSTALL_PREFLIGHT_SELF_CHECK=1`，只跑 preflight 自检，不删除、不弹管理员权限。
- `uninstall-user.sh`
  - 同样在 disable、`rm -rf` 和 `killall` 前检查并发验证脚本。
  - 检测到阻塞进程时输出 `userUninstallReady=false reason=verification-running` 和 `userUninstallBlockingProcess:`，rc `20`。
  - 新增 `INPUTIA_UNINSTALL_USER_PREFLIGHT_SELF_CHECK=1`。
- `verify-nongui.sh`
  - 将两个卸载脚本纳入语法检查。
  - 静态契约要求 preflight 出现在 disable、删除、管理员提示和 killall 之前。
  - 新增 `system uninstall preflight self-check` 和 `user uninstall preflight self-check`。

验证：

```text
zsh -n macos/InputiaInputMethod/uninstall-system.sh macos/InputiaInputMethod/uninstall-user.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

INPUTIA_UNINSTALL_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/uninstall-system.sh
  systemUninstallPreflightSelfCheck clear=true
  systemUninstallPreflightSelfCheck blocked=true
  systemUninstallPreflightSelfCheck=true

INPUTIA_UNINSTALL_USER_PREFLIGHT_SELF_CHECK=1 macos/InputiaInputMethod/uninstall-user.sh
  userUninstallPreflightSelfCheck clear=true
  userUninstallPreflightSelfCheck blocked=true
  userUninstallPreflightSelfCheck=true

INPUTIA_UNINSTALL_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-textedit.sh" \
  macos/InputiaInputMethod/uninstall-system.sh
  systemUninstallReady=false reason=verification-running
  systemUninstallBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/smoke-textedit.sh
  rc=20

INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST="456 /Users/minizl/services/Handy/macos/InputiaInputMethod/gui-smoke-suite.sh" \
  macos/InputiaInputMethod/uninstall-user.sh
  userUninstallReady=false reason=verification-running
  userUninstallBlockingProcess: 456 /Users/minizl/services/Handy/macos/InputiaInputMethod/gui-smoke-suite.sh
  rc=20

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-uninstall-preflight-20260708.log 2>&1
  verifyRc=0
  systemUninstallPreflightSelfCheck clear=true
  systemUninstallPreflightSelfCheck blocked=true
  systemUninstallPreflightSelfCheck=true
  userUninstallPreflightSelfCheck clear=true
  userUninstallPreflightSelfCheck blocked=true
  userUninstallPreflightSelfCheck=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 系统级和用户级卸载入口现在不会在当前工作区 smoke/verifier 运行中 disable input source、删除 app、弹管理员权限或 kill Host。
- 这不改变显式卸载的正常路径，只在验证脚本并发运行时拒绝破坏 GUI smoke 的安装/TIS/Host 状态。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：TIS readiness 统一按目标 app/icon 过滤

背景：

- 当前 Mac mini 一度出现用户级 `~/Library/Input Methods/InputiaInputMethod.app` v41 与系统级 `/Library/Input Methods/InputiaInputMethod.app` v40 并存。
- 两者使用相同 bundle/source id，`TISCreateInputSourceList(..., includeAllInstalled=true)` 可能返回多条 `com.inputia.inputmethod.Inputia.Hans`，其中 iconURL 可能指向系统旧 app，也可能指向用户 app。
- 旧的 readiness 逻辑只读第一条 Hans，容易把“同 id 但不是目标 app”的 source 当成可测目标，或者把用户级/系统级状态混在一起。

实现：

- `tis-readiness.sh`
  - 新增按 `expectedTISIcon` 精确过滤的 `tis_value_for_icon()` / `tis_count_for_icon()`。
  - 新增输出 `tis.targetEnabledMatches=` 和 `tis.targetInstalledMatches=`。
  - 当目标 icon 对应的 source 不存在时输出 `tis.readinessBlockReason=target-source-not-installed`。
- `smoke-preflight.sh`
  - 不再复制 TIS 判断逻辑，直接调用 `tis-readiness.sh "$APP"` 并沿用其输出，再决定 `smokePreflightReady`。
- `post-install-regression.sh`
  - `print_tis_gui_readiness()` 同样委托 `tis-readiness.sh "$APP"`，再映射为 `postInstallTISReady` / `postInstallTISBlockReason`。
- `verify-nongui.sh`
  - 静态契约要求 `smoke-preflight.sh` 与 `post-install-regression.sh` 委托 `tis-readiness.sh`。
  - 由于当前机器可能已经存在用户级 v41，原 `assert_no_user_host` 改为以 verifier 启动时的 `user_host_state()` 为基线，断言后续 no-launch gate 不改变用户 Host/设置 app，而不是假设它们必须不存在。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/post-install-regression.sh
  syntaxRc=0

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
  macos/InputiaInputMethod/smoke-preflight.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  expectedCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  actualCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.readinessBlockReason=target-source-not-installed
  guiSmokeReady=false reason=tis-not-ready
  smokePreflightReady=false reason=tis-not-ready
  rc=8

macos/InputiaInputMethod/build-pkg.sh > /tmp/inputia-build-pkg-refresh-20260708.log 2>&1
  buildPkgRc=0
  pkgbuild: Wrote package to .../dist/InputiaInputMethod-v41-117dcb379750.pkg
  appCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  archiveAppCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  buildAppCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  pkgVerificationPassed=true

macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-tis-readiness-delegation-20260708.log 2>&1
  verifyRc=0
  guiSmokeReadinessCurrent: tis: tis.targetEnabledMatches=0
  guiSmokeReadinessCurrent: tis: tis.targetInstalledMatches=0
  guiSmokeReadinessCurrent: tis: tis.readinessBlockReason=target-source-not-installed
  guiSmokeReadinessCurrent.userHostUnchanged=true
  tisReadinessBuild: tis.targetEnabledMatches=0
  tisReadinessBuild: tis.targetInstalledMatches=0
  tisReadinessBuild: tis.readinessBlockReason=target-source-not-installed
  postInstall: tis.targetEnabledMatches=0
  postInstall: tis.targetInstalledMatches=0
  postInstall: tis.readinessBlockReason=target-source-not-installed
  verifyPkg: pkgVerificationPassed=true
  status: statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

状态：

- 当前 build/pkg 已刷新到 v41 CDHash `117dcb3797504ee99805c1f14c57a49c87f92a4c`。
- 系统安装版仍是 v40，系统设置 launcher 仍是 v40；TIS enabled list 仍没有目标 build/system v41 source。
- 本轮没有启动 TextEdit/Safari，没有运行真实 Clipboard recall smoke；真实 GUI smoke 仍必须等系统级 v41 安装并 TIS ready 后再跑。

## v41 Mac mini 续修：用户级安装 TIS spike 与 target-filter readiness

背景：

- 真实 GUI smoke 当前被系统安装版 v40 / build v41 / TIS not ready 阻塞。
- 系统级安装需要管理员权限；为了验证是否存在无管理员的推进路径，尝试用户级安装 v41 到 `~/Library/Input Methods`。
- Apple Text Input Source Services Reference 说明：`TISEnableInputSource` 用于让 input source 可在 UI 中选择；若启用的是 input mode，其 parent input method 必须已经 enabled。这个约束解释了为什么 parent/mode 的 enabled list 状态必须分别看。

用户级安装 spike：

```text
macos/InputiaInputMethod/install-user.sh > /tmp/inputia-install-user-v41-20260708.log 2>&1
  installUserRc=0
  userInstallBuild=true
  userInstallVerified=true
  settingsLauncherInstalled=/Users/minizl/Applications/Inputia 设置.app
  userInstallTISReady=false

macos/InputiaInputMethod/build/inputia-tis-tool --select-inputia-source-id com.inputia.inputmethod.Inputia.Hans
  selectSourceFoundInEnabledList=false
  selectRc=0
```

观察：

- 用户目录 v41 app 复制成功，cdhash 与 build 一致。
- TIS registry 仍没有可选择的 enabled list source；`TISSelectInputSource` 无法切到 Inputia。
- spike 留下用户级 host 后，完整验证正确报出 `userHostConflict=true`，说明不能把用户安装残留留给系统版 GUI smoke 基线。

清理 spike：

```text
macos/InputiaInputMethod/uninstall-user.sh > /tmp/inputia-uninstall-user-after-tis-spike-20260708.log 2>&1
  uninstallUserRc=0
  removed /Users/minizl/Library/Input Methods/InputiaInputMethod.app

macos/InputiaInputMethod/status.sh
  user host exists=false
  user settings launcher exists=false
```

实现：

- `tis-readiness.sh`
  - 新增按目标 app iconURL 过滤的 `tis_value_for_icon` / `tis_count_for_icon`。
  - 输出 `tis.targetEnabledMatches=` 和 `tis.targetInstalledMatches=`。
  - 当同 bundle/source id 存在但没有目标 app 路径的 source 时，输出 `tis.readinessBlockReason=target-source-not-installed`，避免把系统 v40 和用户/build v41 混在一起误诊断。
- `verify-nongui.sh`
  - 静态契约要求 `tis-readiness.sh` 保留 target-filter helper、target count 输出和 `target-source-not-installed` reason。

验证：

```text
bash -n macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

macos/InputiaInputMethod/tis-readiness.sh "$HOME/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=true
  tis.enabledMatches=0
  tis.installedMatches=6
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.hansIconMatchesApp=false
  tis.readinessBlockReason=target-source-not-installed
  tisReadiness=false

macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=6
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=2
  tis.hansIconMatchesApp=true
  tis.readinessBlockReason=missing-enabled-source
  tisReadiness=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-tis-target-filter-rerun-20260708.log 2>&1
  verifyRc=0
  tisReadinessBuild: tis.targetEnabledMatches=0
  tisReadinessBuild: tis.targetInstalledMatches=0
  tisReadinessBuild: tis.readinessBlockReason=target-source-not-installed
  postInstall: userHostConflict=false
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 用户级 v41 安装不是当前 GUI smoke 的可用捷径：同 bundle/source id 与系统 v40 共存时，TIS 仍不会给当前目标 app 产生可选择 enabled source。
- 真实 GUI smoke 仍应等待系统安装版/settings 从 v40 更新到 v41，并等待 TIS target source 出现在 enabled list。
- 本轮没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke；用户级 spike 已清理，验证基线恢复为 no user host conflict。
- 最终收尾再次执行 `uninstall-user.sh > /tmp/inputia-uninstall-user-final-clean-20260708.log`，确认 `userHostExists=false`、`userSettingsExists=false`，避免用户级 spike 残留影响后续系统版 GUI smoke。

## v41 Mac mini 续修：Host TIS source 选择按当前 bundle 路径过滤

背景：

- 上一轮用户级安装 spike 证明：同一个 `com.inputia.inputmethod.Inputia` bundle/source id 可以同时在系统路径和用户路径留下 TIS 记录。
- Apple Text Input Source Services Reference 说明，input mode 的启用/选择依赖 parent input method；`TISCreateInputSourceList(..., includeAllInstalled: true)` 返回的是快照，且安装但未启用的 source 也会出现在列表里。
- 因此只按 input source id 取第一个 match 会误选系统 v40 或 TIS 缓存残影，导致 `--enable-input-source` / `--select-input-source` 在多安装源共存时操作错对象。

实现：

- `Sources/InputiaInputMethod/main.swift`
  - 新增 `expectedTISIconPath`，指向当前 Host bundle 的 `Contents/Resources/inputia.pdf`。
  - `inputSource(inputSourceID:includeAllInstalled:)` 先收集同 id source，再优先返回 `kTISPropertyIconImageURL == expectedTISIconPath` 的 source。
  - 如果找不到当前 bundle icon match，保留原 fallback：返回第一个同 id source。
- `verify-nongui.sh`
  - 静态契约要求 Host 保留 `expectedTISIconPath`、iconURL 过滤和 fallback。

验证：

```text
macos/InputiaInputMethod/build.sh > /tmp/inputia-build-host-icon-filter-20260708.log 2>&1
  buildRc=0

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --dump-input-source
  inputSourceFound=false

macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --select-input-source
  inputSourceFound=false
  inputSourceFoundInEnabledList=false

macos/InputiaInputMethod/tis-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  appMatchesBuild=true
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.readinessBlockReason=target-source-not-installed
  tisReadiness=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-host-tis-icon-filter-rerun-20260708.log 2>&1
  verifyRc=0
  hostShortcutSelfCheck=true
  tisReadinessBuild: tis.targetEnabledMatches=0
  tisReadinessBuild: tis.targetInstalledMatches=0
  tisReadinessBuild: tis.readinessBlockReason=target-source-not-installed
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

备注：

- 首次完整验证在 `safariCommandCleanupSelfCheck` 处看到 build executable 瞬时缺失并失败；复核时 executable 已存在，环境无残留，重跑完整验证通过。
- 完整验证过程中用户级 app/settings 又出现；收尾执行 `uninstall-user.sh > /tmp/inputia-uninstall-user-after-host-icon-filter-20260708.log` 清掉，`status.sh` 确认 user host/settings 不存在。
- 真实 GUI smoke 仍不应硬跑：系统 host/settings 仍是 v40，build/pkg 是 v41，系统路径 TIS 仍未 ready，且无非交互管理员安装能力。

## v41 Mac mini 续修：post-install regression 隔离用户级 Host 状态

背景：

- 继续验证 TextEdit/Safari/Clipboard GUI smoke 清理纪律时，`verify-nongui.sh` 首次运行被 `/tmp/inputia-verify-nongui.lock` 挡住；owner 进程已消失后重跑。
- 重跑失败在 `GUI smoke suite post-install failure gate`：`assert_no_user_host` 发现真实用户级安装从存在变成 missing。
- 失败时基线显示：
  - `/Users/minizl/Library/Input Methods/InputiaInputMethod.app|41|913e882766c8d13d8e39ecdc60d377b556945da0`
  - `/Users/minizl/Applications/Inputia 设置.app|41|78053ac6e1732362661e4ea921ea2580f76b48e3`
- 这说明 verifier 自身仍可能污染真实用户级 Host/设置启动器状态，和本轮 GUI smoke 清理纪律目标冲突。

实现：

- `verify-nongui.sh` 新增隔离路径：
  - `VERIFY_POST_INSTALL_USER_CONFLICT_ROOT=/tmp/inputia-post-install-user-conflict.*`
  - `VERIFY_POST_INSTALL_USER_APP`
  - `VERIFY_POST_INSTALL_USER_LEGACY_APP`
- 所有真实 `post-install-regression.sh` 调用都传入：
  - `INPUTIA_USER_APP="$VERIFY_POST_INSTALL_USER_APP"`
  - `INPUTIA_USER_LEGACY_APP="$VERIFY_POST_INSTALL_USER_LEGACY_APP"`
- 静态 cleanup contract 新增断言，防止 post-install regression 回退到读取真实 `~/Library/Input Methods` 用户 Host。
- 修复后执行 `install-user.sh` 恢复本轮验证误删的用户级 Host/设置启动器。

验证：

```text
macos/InputiaInputMethod/install-user.sh
  userInstallVerified=true
  settingsLauncherInstalled=/Users/minizl/Applications/Inputia 设置.app

Python path check immediately after restore:
  /Users/minizl/Library/Input Methods/InputiaInputMethod.app|exists=True|isdir=True
  /Users/minizl/Applications/Inputia 设置.app|exists=True|isdir=True

INPUTIA_INSTALL_NO_ADMIN_PROMPT=1 macos/InputiaInputMethod/install-system.sh
  systemInstallNeedsAdmin=true
  systemInstallReady=false reason=admin-required
  installSystemNoPromptRc=12
  userAppBefore=True
  userAppAfter=True
  userSettingsBefore=True
  userSettingsAfter=True

bash macos/InputiaInputMethod/verify-nongui.sh
  log=/tmp/inputia-verify-nongui-host-tis-icon-filter-rerun-20260708.log
  nonGuiVerificationPassed=true
  awaitUiNotReady.userHostUnchanged=true
  installNoPrompt.userHostUnchanged=true
  residue=false
  tmpResidue=false

INPUTIA_VERIFY_ALLOW_USER_HOST_BASELINE=1 bash macos/InputiaInputMethod/verify-nongui.sh
  log=/tmp/inputia-verify-nongui-user-host-isolation-final3-allow-20260708.log
  verifyRc=0
  verifyUserHostBaselineAllowed=true
  awaitUiNotReady.userHostUnchanged=true
  installNoPrompt.userHostUnchanged=true
  residue=false
  tmpResidue=false

Final path check:
  /Users/minizl/Library/Input Methods/InputiaInputMethod.app|exists=True|isdir=True
  /Users/minizl/Applications/Inputia 设置.app|exists=True|isdir=True

Process residue check:
  no verify-nongui/smoke/post-install-regression/gui-smoke/InputiaInputMethod/TextEdit/Safari process remains
  only Safari framework helper services remain
```

当前结论：

- `post-install-regression.sh` 在 verifier 内部不再读取真实用户级 Host 作为冲突对象，避免非 GUI 验证污染 `~/Library/Input Methods`。
- 用户级 Host/设置启动器已恢复；上一段“最终清理用户级 spike”的状态不再代表当前机器状态。
- 真实 GUI smoke 仍未运行；当前阻塞仍是系统安装版/settings 还是 v40，build/pkg/user 侧为 v41，且系统 TIS enabled matches 仍为 0。

## v41 Mac mini 续修：verify-nongui 默认拒绝用户级 Host baseline

背景：

- 连续 TIS spike 后，用户级 `~/Library/Input Methods/InputiaInputMethod.app` 和 `~/Applications/Inputia 设置.app` 会反复残留。
- 真实 GUI smoke 当前验证的是系统级 `/Library/Input Methods/InputiaInputMethod.app`；用户级同 bundle/source id 会污染 TIS source 选择、post-install conflict 判断和 smoke readiness。
- 之前 `verify-nongui.sh` 只记录 `VERIFY_USER_HOST_BASELINE` 并检查不变；这会把“启动时已经污染”的状态当作可接受基线。

实现：

- `verify-nongui.sh`
  - 新增 `assert_user_host_baseline_absent`。
  - 默认在捕获 `VERIFY_USER_HOST_BASELINE` 前检查三条路径必须不存在：
    - `~/Library/Input Methods/InputiaInputMethod.app`
    - `~/Library/Input Methods/IputiaInputMethod.app`
    - `~/Applications/Inputia 设置.app`
  - 如果存在，快速失败：
    - `nonGuiVerificationPassed=false reason=user-host-baseline-present`
    - 输出 `verifyUserHostBaseline:` 明细。
  - 保留显式 override：`INPUTIA_VERIFY_ALLOW_USER_HOST_BASELINE=1`，仅用于刻意检查用户级安装状态时。
- `verify-nongui.sh` 静态契约新增：
  - `assert_user_host_baseline_absent`
  - `user-host-baseline-present`
  - `INPUTIA_VERIFY_ALLOW_USER_HOST_BASELINE`
  - `verifyUserHostBaselineAbsent=true`

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntaxRc=0

# 在用户级 Host/settings 残留存在时：
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-user-host-baseline-gate-20260708.log 2>&1
  gateRc=21
  nonGuiVerificationPassed=false reason=user-host-baseline-present
  verifyUserHostBaseline: /Users/minizl/Library/Input Methods/InputiaInputMethod.app|41|117dcb3797504ee99805c1f14c57a49c87f92a4c
  verifyUserHostBaseline: /Users/minizl/Applications/Inputia 设置.app|41|78053ac6e1732362661e4ea921ea2580f76b48e3

macos/InputiaInputMethod/uninstall-user.sh > /tmp/inputia-uninstall-user-before-baseline-rerun-20260708.log 2>&1
  uninstallUserRc=0
  userHostExists=false
  userSettingsExists=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-user-host-baseline-clean-20260708.log 2>&1
  verifyRc=0
  verifyUserHostBaselineAbsent=true
  awaitUiNotReady.userHostUnchanged=true
  installNoPrompt.userHostUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 非 GUI 验证现在不会再默许用户级 Inputia baseline 污染。
- 当前机器已清理到 `userHostExists=false`、`userSettingsExists=false`。
- 真实 GUI smoke 仍不应硬跑：系统 host/settings 仍是 v40，build/pkg 是 v41，系统 TIS 仍未 ready，且无非交互管理员安装能力。

补充清理验证：

```text
INPUTIA_VERIFY_ALLOW_USER_HOST_BASELINE=1 bash macos/InputiaInputMethod/verify-nongui.sh
  log=/tmp/inputia-verify-nongui-user-host-isolation-final3-allow-20260708.log
  verifyRc=0
  verifyUserHostBaselineAllowed=true
  user host/settings existed during the explicit allow run
  nonGuiVerificationPassed=true

bash macos/InputiaInputMethod/uninstall-user.sh > /tmp/inputia-uninstall-user-after-allow-baseline-20260708.log 2>&1
  uninstallUserRc=0
  userHostExists=false
  userSettingsExists=false

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-after-allow-cleanup-20260708.log 2>&1
  verifyRc=0
  verifyUserHostBaselineAbsent=true
  user.exists=false
  user settings launcher exists=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

结论更新：

- 显式 allow 路径只作为用户级状态检查工具；默认验证路径已恢复为拒绝用户级 Host baseline。
- Mac mini 当前回到系统级 smoke 前置干净态：没有用户级 Inputia Host/设置启动器，且默认非 GUI 验证通过。

## v41 Mac mini 续修：status/readiness 显式阻断用户级 Host 冲突

背景：

- `verify-nongui.sh` 已默认拒绝用户级 Host baseline，但 `status.sh` 和 `gui-smoke-readiness.sh` 仍需要独立报告同类风险。
- 真实 GUI smoke 目标是系统级 `/Library/Input Methods/InputiaInputMethod.app`；如果 `~/Library/Input Methods/InputiaInputMethod.app` 或用户级设置启动器残留，TIS 可能选择用户域来源，导致系统级 smoke 误判。

实现：

- `status.sh`
  - 新增用户级路径检测：
    - `~/Library/Input Methods/InputiaInputMethod.app`
    - `~/Library/Input Methods/IputiaInputMethod.app`
    - `~/Applications/Inputia 设置.app`
  - 输出 `userHostConflict=` 和 `statusUserHostConflict=`。
  - 当冲突存在时，在 `statusGuiSmokeBlockReasons` 加入 `user-host-conflict`。
  - 增加 `INPUTIA_USER_APP_FOR_TEST` / `INPUTIA_USER_LEGACY_APP_FOR_TEST` / `INPUTIA_USER_SETTINGS_APP_FOR_TEST`，用于非破坏性模拟。
- `gui-smoke-readiness.sh`
  - 同样检测用户级 Host/Settings 冲突。
  - 输出 `userHostConflict=`。
  - 当冲突存在时，在 `guiSmokeReadinessBlockReasons` 加入 `user-host-conflict`。
  - readiness self-check 新增 `case=userhost`，并把 `user-host-conflict` 纳入 all-block-reasons。
- `verify-nongui.sh`
  - 静态契约检查两个入口都包含 `user-host-conflict` 和测试路径覆盖。
  - 运行期用 `/tmp/inputia-status-user-host.*` 与 `/tmp/inputia-gui-readiness-user-host.*` 临时目录模拟用户级 Host，不触碰真实用户输入法目录。

验证：

```text
zsh -n macos/InputiaInputMethod/status.sh
zsh -n macos/InputiaInputMethod/gui-smoke-readiness.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntax=ok

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh
  guiSmokeReadinessSelfCheck case=userhost expected=user-host-conflict actual=user-host-conflict
  guiSmokeReadinessSelfCheck allBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,user-host-conflict,inputia-host-running,screen-locked,textedit-already-running,safari-already-running
  guiSmokeReadinessSelfCheck=true

# 临时路径模拟 status 用户级 Host 冲突：
INPUTIA_USER_APP_FOR_TEST=/tmp/inputia-status-user-host-direct.*/InputiaInputMethod.app macos/InputiaInputMethod/status.sh
  userHostConflict=true
  statusUserHostConflict=true
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,user-host-conflict
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,user-host-conflict

# 临时路径模拟 readiness 用户级 Host 冲突：
INPUTIA_USER_APP_FOR_TEST=/tmp/inputia-readiness-user-host-direct.*/InputiaInputMethod.app macos/InputiaInputMethod/gui-smoke-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  userHostConflict=true
  guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready,user-host-conflict
  guiSmokeReadinessReady=false reason=admin-required

bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-user-host-conflict-readiness-20260708.log 2>&1
  verifyRc=0
  statusUserHostConflict=false
  guiSmokeReadinessUserHostGateSelfCheck=true
  statusUserHostConflictSelfCheck=true
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 直接运行 `status.sh`、`gui-smoke-readiness.sh` 或完整 `verify-nongui.sh` 时，用户级 Host/Settings 污染都会被识别为系统级 GUI smoke 阻断条件。
- 本轮冲突测试使用临时目录覆盖，未创建真实 `~/Library/Input Methods/InputiaInputMethod.app`。
- 真实 GUI smoke 仍未硬跑；当前阻断仍是系统 host/settings v40、build/pkg v41、TIS 未 ready、无非交互管理员安装能力。

## v41 Mac mini 续修：Clipboard recall 清状态后重置事件窗口

背景：

- Clipboard recall GUI smoke 用 debug event log 验证 `clipboardRecallShown` 和 `clipboardRecallCommit`。
- 上一轮真实 recall 曾出现 result mismatch，疑似前一轮 composition / recall 状态污染。
- 旧脚本只在启动前清空 event log；进入 TextEdit 后执行 `clearInputiaState()` 时，如果 Host 写入事件，后续 `assertNoClipboardRecallBeforeTrigger` 会看到非触发窗口内的事件，结果不可解释。

实现：

- `smoke-clipboard-recall.sh`
  - AppleScript 新增 `resetClipboardRecallEventLog(eventLogPath)`。
  - 在 `clearInputiaState()`、`state-clear-leaked-text` 检查之后，触发 `Control-Shift-V` 之前，将 event log `set eof ... to 0`。
  - 回传并打印 `clipboardRecallEventLogResetAfterStateClear=`。
  - shell 层新增断言：若不是 `true`，输出 `clipboardRecallSmokePassed=false reason=event-log-reset-after-state-clear-failed` 并退出。
- `verify-nongui.sh`
  - 静态契约新增：
    - `on resetClipboardRecallEventLog(eventLogPath)`
    - `set eof eventLogFile to 0`
    - `clipboardRecallEventLogResetAfterStateClear=`
    - `event-log-reset-after-state-clear-failed`
  - 验证顺序：event log reset 必须发生在 `assertNoClipboardRecallBeforeTrigger` 和实际 `Control-Shift-V` 触发之前。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-clipboard-recall.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntax=ok

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  clipboardRecallSmokeReady=false reason=inputia-host-running
  rc=10

INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK=1 INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK_RC=23 \
INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
  macos/InputiaInputMethod/smoke-clipboard-recall.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  clipboardRecallCleanupSelfCheck=true phase=after-clipboard-write
  cleanupSelfCheckRc=23

# 首次完整验证被手工诊断遗留的 /tmp/inputia-safari-enter-dir-repro.log 挡住；清理该诊断残留后重跑：
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-clipboard-event-log-reset-rerun-20260708.log 2>&1
  verifyRc=0
  statusUserHostConflict=false
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Clipboard recall 的 debug event log 现在只覆盖“清状态完成之后、真实召回触发开始之后”的事件窗口。
- `clipboardRecallShown` / `clipboardRecallCommit` 的验证不再混入清 IME 状态阶段产生的旧事件。
- 本轮仍没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall GUI smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS matches 0、无非交互管理员安装能力。

## v41 Mac mini 续修：TextEdit smoke 显式计数 IME 状态清理

背景：

- `smoke-textedit.sh` 会在一个 TextEdit 进程中顺序跑默认中文、Enter raw、数字候选、PageDown、ArrowDown、Shift 英/中切换等多个 case。
- 旧脚本每个 case 内部会用 Esc 清 IME 状态并检查当前 doc 为空，但没有把“状态清理成功次数”输出到结果里；真实 GUI smoke 失败时不容易区分是候选/上屏逻辑失败，还是某个 case 前状态没有清干净。

实现：

- `smoke-textedit.sh`
  - 新增 `stateClearPasses` 计数。
  - 新增 `assertDocumentCleared(docRef, labelName)`：读取当前 `docRef`，若非空则报 `<case>-state-clear-leaked-text:...`，否则递增计数。
  - 所有 `runCase` 都通过该 helper 记录清理成功。
  - `runShiftCase` 在 shift 回切中文前新增一次 `clearInputiaState()` + `assertDocumentCleared(..., "shift-chinese")`，避免英文 case 残留影响中文回切。
  - 输出 `textEditStateClearPasses=`。
  - shell 层断言 `textEditStateClearPasses=8`，否则输出 `textEditSmokePassed=false step=state-clear-count ...`。
- `verify-nongui.sh`
  - 静态契约要求：
    - `assertDocumentCleared(docRef, labelName)`
    - `stateClearPasses` 递增
    - `textEditStateClearPasses=`
    - `textEditSmokePassed=false step=state-clear-count`
    - `shift-chinese` 的状态清理必须发生在回切 Shift 之前。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-textedit.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntax=ok

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK=1 INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK_RC=27 \
  macos/InputiaInputMethod/smoke-textedit.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  textEditCleanupSelfCheck=true phase=after-temp-write
  texteditCleanupRc=27

# 首次完整验证被另一个仍在运行的 verify-nongui 锁挡住；等待 owner pid 34207 结束后重跑：
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-textedit-state-clear-count-rerun-20260708.log 2>&1
  verifyRc=0
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- TextEdit 主 smoke 现在会证明 8 个关键清状态点全部执行并成功，包含 shift 回切中文前的额外清理。
- 真实 GUI smoke 仍未运行；当前仍需系统级 v41 安装、TIS ready 后再跑真实 TextEdit/Safari/Clipboard smoke。

## v41 Mac mini 续修：Safari enter 清状态后重置事件窗口

背景：

- `smoke-safari-enter.sh` 和 Clipboard recall 一样依赖 `INPUTIA_DEBUG_EVENTS`：设置 debug env、重启 Host、进入目标 App 后清 IME 状态，再触发真实输入。
- 旧脚本只在启动前清空 event log，并在输入前检查没有 `commit=abc`；如果 `clearInputiaState()` 阶段写入事件，后续 `commit=abc` 判断仍可能混入非触发窗口的事件。

实现：

- `smoke-safari-enter.sh`
  - AppleScript 新增 `resetSafariEnterEventLog(eventLogPath)`。
  - 在 `clearInputiaState()`、`assertStillFrontmost("Safari")` 之后，真实输入 `abc` 前，将 event log `set eof ... to 0`。
  - 输出 `safariEnterEventLogResetAfterStateClear=`。
  - shell 层新增断言：若不是 `true`，输出 `safariEnterSmokePassed=false reason=event-log-reset-after-state-clear-failed` 并退出。
- `verify-nongui.sh`
  - 静态契约新增：
    - `on resetSafariEnterEventLog(eventLogPath)`
    - `set eof eventLogFile to 0`
    - `safariEnterEventLogResetAfterStateClear=`
    - `safariEnterSmokePassed=false reason=event-log-reset-after-state-clear-failed`
  - 验证顺序：event log reset 必须发生在 `assertNoRawCommitBeforeTyping` 和实际 `abc` 按键前。
  - 顺手补齐 `postInstallActiveLockGate` 的 `INPUTIA_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_SETTINGS_APP"`，使 post-install active-lock 自检也使用隔离的用户设置路径。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-enter.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntax=ok

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
  macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  guiSessionCheck=skipped
  safariPreflight=not-running
  InputiaInputMethodPreflight=running
  guiSmokeReady=false reason=inputia-host-running
  safariEnterSmokeReady=false reason=inputia-host-running
  safariEnterHostGateRc=11

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK=1 INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK_RC=26 \
  macos/InputiaInputMethod/smoke-safari-enter.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  safariEnterCleanupSelfCheck=true phase=after-debug-env-write
  safariEnterCleanupRc=26

# 两次完整验证先被正在运行的 verify-nongui 锁挡住；等待 owner 结束后重跑：
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-enter-event-log-reset-final-20260708.log 2>&1
  verifyRc=0
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari enter 的 `commit=abc` 验证现在只覆盖“清状态完成后、真实输入开始后”的事件窗口。
- 本轮仍没有启动 Safari，也没有运行真实 Safari enter GUI smoke；真实阻塞仍是系统/settings v40、build/pkg v41、TIS 未 ready、无非交互管理员安装能力。

## v41 Mac mini 续修：Safari typing/command 输出清状态断言结果

背景：

- `smoke-safari-typing.sh` 和 `smoke-safari-command-shortcuts.sh` 已在真实输入前检查 Safari 页面 title，防止清 IME 状态后仍有残留文本。
- 但旧脚本没有把这些断言成功结果输出到 shell 层；真实 GUI smoke 日志只能看到最终结果，不能直接证明“输入/Command 操作前状态已干净”。

实现：

- `smoke-safari-typing.sh`
  - 在 AppleScript 返回值中新增 `safariTypingStateClearBeforeTyping=true`。
  - shell 层读取并断言该字段；缺失时输出 `safariTypingSmokePassed=false reason=state-clear-before-typing-missing`。
- `smoke-safari-command-shortcuts.sh`
  - 在 AppleScript 返回值中新增 `safariCommandStateClearBeforeCopy=true`。
  - shell 层读取并断言该字段；缺失时输出 `safariCommandShortcutSmokePassed=false step=state-clear-before-copy`。
- `verify-nongui.sh`
  - 静态契约要求两个字段和对应 shell 断言存在。
  - 验证字段输出必须发生在清状态 guard 之后、真实输入/Command-A 之前。

验证：

```text
bash -n macos/InputiaInputMethod/smoke-safari-typing.sh
bash -n macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  syntax=ok

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK=1 INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK_RC=28 \
  macos/InputiaInputMethod/smoke-safari-typing.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  safariTypingCleanupSelfCheck=true phase=after-temp-write
  safariTypingCleanupRc=28

INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK=1 INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK_RC=24 \
  macos/InputiaInputMethod/smoke-safari-command-shortcuts.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  safariCommandCleanupSelfCheck=true phase=after-clipboard-write
  safariCommandCleanupRc=24

# 首次完整验证在 safari-command missing-text clipboard gate 处观测到剪贴板变动；
# 手工复现同命令显示 before/after 相同，随后等待并发 verify-nongui 锁释放后重跑：
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-safari-state-clear-output-final-20260708.log 2>&1
  verifyRc=0
  statusTISEnabledMatches=0
  statusTISInstalledMatches=3
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari typing/command smoke 的真实日志现在能直接证明输入或 Command 操作前状态已清干净。
- 当前系统 TIS 能看到 3 个系统 v40 输入源条目，但 enabled matches 仍为 0；真实 GUI smoke 仍因系统/settings v40、admin-required、tis-not-ready 被阻断。

## v41 Mac mini 复核：默认验证、status 与 TIS readiness 一致

本轮重新确认没有并发验证/安装进程，且用户级 Inputia 已清理：

```text
Process check:
  no verify-nongui/post-install-regression/install/smoke/InputiaInputMethod/TextEdit/Safari process remains
  only Safari framework helper services remain

User-level path check:
  /Users/minizl/Library/Input Methods/InputiaInputMethod.app|exists=False|isdir=False
  /Users/minizl/Library/Input Methods/IputiaInputMethod.app|exists=False|isdir=False
  /Users/minizl/Applications/Inputia 设置.app|exists=False|isdir=False
```

默认非 GUI 验证：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-current-default-20260708.log 2>&1
  verifyRc=0
  verifyUserHostBaselineAbsent=true
  awaitUiNotReady.userHostUnchanged=true
  installNoPrompt.userHostUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前系统状态：

```text
macos/InputiaInputMethod/status.sh > /tmp/inputia-status-current-20260708.log 2>&1
  statusRc=0
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  system host version=40
  system host cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  systemMatchesBuild=false
  system settings version=40
  systemSettingsMatchesBuildVersion=false
  userHostConflict=false
  running=false
  statusAdminInstallReady=false
  statusTISEnabledMatches=0
  statusTISInstalledMatches=0
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
```

当前系统 TIS readiness：

```text
macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.hansIconURL=unknown
  tis.hansIconMatchesApp=false
  tis.readinessBlockReason=target-source-not-installed
  tisReadiness=false
```

当前结论：

- 默认 verifier、`status.sh`、`tis-readiness.sh` 对当前阻塞原因一致：用户级污染已清理，真实 GUI smoke 仍被系统版 v40、settings v40、无非交互管理员安装能力、系统 TIS 未 ready 挡住。
- 本轮没有启动 TextEdit/Safari，也没有运行真实 Clipboard recall smoke。

## v41 Mac mini 复核：常用 Command 快捷键按整类透传

用户反馈：在当前输入法下 `Command-C` / `Command-V` 不可用，疑似被输入法接管。按证据规则先查官方资料再实现/验证：

- Apple Support《Mac keyboard shortcuts》列出 `Command-X/C/V/Z/A/F/G/H/M/O/P/Q/S/T/W`、`Command-Space`、`Command-Tab`、`Command-Delete`、`Command-方向键`、`Shift-Command-3/4/5`、`Option-Command-Esc` 等常见系统/应用快捷键，并说明 App 也可有自己的快捷键。
- 结论：输入法不能按 `C/V` 个别补丁处理；只要 `keyDown` 含 `.command`，默认应透传给宿主 App/系统。Inputia 自己的快捷键不得叠加 `Command`，例如剪贴板召回只接受 `Control-Shift-V`，`Control-Shift-Command-V` 必须拒绝。

当前源码/构建检查：

```text
InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers:) == modifiers.contains(.command)
InputiaInputController.handleKeyDown:
  在剪贴板召回、标点切换、全半角切换、候选导航、composition 字符处理之前先检查 shouldPassThroughCommandShortcut
InputiaHostTextPolicy.shouldPassThroughAppCommand:
  copy:/paste:/cut:/undo:/redo:/selectAll:/saveDocument:/openDocument:/performClose:/terminate:/find:/print:/hide:/showPreferences: 等 AppKit command selector 透传
InputiaHostTextPolicy.recallClipboardMenuKeyEquivalent == ""
```

非 GUI 自检：

```text
./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandXPassThrough=true
  commandZPassThrough=true
  commandAPassThrough=true
  ctrlShiftCommandVRejected=true
  shortcutRc=0

./macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  textPolicyRc=0
```

完整非 GUI 回归：

```text
macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-command-shortcuts-20260708.log 2>&1
  verifyRc=0
  shortcut: shortcutSelfCheck=true
  shortcut: commonAppleCommandShortcutSetPassesThrough=true
  shortcut: anyCommandModifiedKeyPassesThrough=true
  shortcut: allCommandModifierVariantsPassThrough=true
  shortcut: commandCPassThrough=true
  shortcut: commandVPassThrough=true
  shortcut: ctrlShiftCommandVRejected=true
  hostShortcut: commandCPassThrough=true
  hostShortcut: commandVPassThrough=true
  hostTextPolicy: hostTextPolicySelfCheck=true
  postInstallUiPreflightSelfCheck=true
  textEditCleanupSelfCheckNoMutationPassed=true
  safariTypingCleanupSelfCheckNoMutationPassed=true
  safariEnterCleanupSelfCheckNoMutationPassed=true
```

当前安装状态仍需区分：

```text
status.sh:
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  system host version=40
  systemMatchesBuild=false
  user host exists=false
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
```

结论：

- v41 源码/构建已经按“所有带 Command 的 keyDown 组合透传”覆盖，不是只修 `Command-C/V`。
- 当前用户实际遇到的 `Command-C/V` 仍可能来自系统安装版 v40；v41 尚未成为当前系统 TIS 可选/已选输入源。
- 本轮没有运行真实 TextEdit/Safari GUI smoke；GUI smoke 仍被系统安装版 v40、settings v40、管理员安装能力和 TIS readiness 阻塞。

## v41 Mac mini 续修：await-system-install 补齐用户级 Host 冲突门禁

发现缺口：`status.sh` 和 `gui-smoke-readiness.sh` 已经把三类用户级残留都视为 GUI smoke 阻断：

- `~/Library/Input Methods/InputiaInputMethod.app`
- `~/Library/Input Methods/IputiaInputMethod.app`
- `~/Applications/Inputia 设置.app`

但 `await-system-install.sh` 只输出/检查当前拼写的用户级主 Host，未覆盖旧拼写 Host 和用户级设置启动器。已补齐：

- 新增 `USER_LEGACY_APP` 与 `USER_SETTINGS_APP`。
- 新增 `user_host_conflict()`，默认检查三类用户级路径。
- `INPUTIA_AWAIT_USER_HOST_CONFLICT_FOR_TEST=true` 用于自检，不需要真实创建用户目录残留。
- 普通 await 循环输出 `userHostConflict=<true|false>`。
- GUI smoke readiness 通过后、GUI 会话检查前，先用 `user-host-conflict` 阻断。
- `verify-nongui.sh` 增加静态契约和 runtime 自检，防止 await 侧再次漏掉该门禁。

快速自检：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  rc=0
zsh -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=user-host-conflict uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=user-host-conflict uiSmokeBlockReasons=user-host-conflict
  awaitUiStatusSelfCheck=true
```

完整非 GUI 回归：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-user-conflict-20260708.log 2>&1
  verifyRc=0
  awaitUiNotReady.userHostUnchanged=true
  installNoPrompt.userHostUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前 await/status/TIS 复核：

```text
INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_RUN_UI_SMOKE=1 macos/InputiaInputMethod/await-system-install.sh
  userHostConflict=false
  uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready

bash macos/InputiaInputMethod/status.sh
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  system host version=40
  systemMatchesBuild=false
  user host exists=false
  userHostConflict=false
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

bash macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.hansIconMatchesApp=false
  tis.readinessBlockReason=target-source-not-installed
  tisReadiness=false
```

当前结论：

- `await-system-install.sh`、`status.sh`、`gui-smoke-readiness.sh` 已对用户级 Host/legacy/settings 残留形成一致门禁。
- 当前用户级 Host 冲突为 false，非 GUI 验证没有污染用户级状态。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍不能运行；阻断原因仍是系统安装版 v40、settings v40、无非交互管理员安装能力、TIS 未安装/未 ready。

## v41 Mac mini 续修：await UI 自检隔离真实进程并报告组合阻塞

继续复核 `await-system-install.sh` 后发现两个证据质量问题：

1. `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 的 Safari 分支仍会读取真实机器上的 `pgrep -x TextEdit` / `pgrep -x Safari` 状态。如果用户机器刚好已有 TextEdit/Safari 进程，自检会被真实进程污染，导致分支断言不稳定。
2. await 侧虽然已经检查用户级 Host 冲突，但 target/TIS 不 ready 时原先只容易看到主阻塞；组合阻塞证据弱于 `status.sh` / `gui-smoke-readiness.sh` 的全量 blocker 模型。

已补齐：

- `process_preflight()` 增加 `INPUTIA_AWAIT_IGNORE_REAL_PROCESSES_FOR_TEST=1`，自检模式只接受显式 `INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST` 模拟，不读取真实 TextEdit/Safari/Inputia 进程。
- `INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1` 自动启用该隔离变量。
- `ui_smoke_status_line()` 先聚合 `target-cdhash-mismatch`、`tis-not-ready`、`user-host-conflict`，再按优先级返回主因；`uiSmokeBlockReasons` 保留组合原因。
- `verify-nongui.sh` 增加静态契约：await 必须有真实进程隔离测试变量、必须先聚合 target/TIS/user-host blockers，再进入 GUI session / process preflight。
- `verify-nongui.sh` 增加 runtime 断言：`target-tis-userhost` 自检必须输出 `target-cdhash-mismatch,tis-not-ready,user-host-conflict`。

快速自检：

```text
zsh -n macos/InputiaInputMethod/await-system-install.sh
  rc=0
zsh -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=target-and-tis uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready
  awaitUiStatusSelfCheck reason=target-tis-userhost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,tis-not-ready,user-host-conflict
  awaitUiStatusSelfCheck reason=safari-already-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiInputiaHostPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=safari-already-running uiSmokeBlockReasons=safari-already-running
  awaitUiStatusSelfCheck reason=safari-allow uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiInputiaHostPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none
  awaitUiStatusSelfCheck=true
```

完整非 GUI 回归：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-await-isolated-selfcheck-20260708.log 2>&1
  verifyRc=0
  awaitUiNotReady.userHostUnchanged=true
  installNoPrompt.userHostUnchanged=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前 readiness 复核：

```text
bash macos/InputiaInputMethod/status.sh
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  system host version=40
  systemMatchesBuild=false
  user host exists=false
  userHostConflict=false
  statusAdminInstallReady=false
  statusTISEnabledMatches=0
  statusTISInstalledMatches=0
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

bash macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.hansIconMatchesApp=false
  tis.readinessBlockReason=target-source-not-installed
  tisReadiness=false
```

当前结论：

- await UI 自检现在不依赖真实用户进程状态，避免 TextEdit/Safari 已运行时误判其他分支。
- await/status/gui-readiness 对用户级 Host 冲突的组合阻塞证据更一致。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍未运行；当前系统安装版/settings 仍是 v40，TIS 未 ready，且无非交互管理员安装能力。

## v41 Mac mini 续修：post-install 最终入口补齐用户级 settings 冲突门禁

继续盘点真实 GUI smoke 调度入口时发现：`status.sh`、`gui-smoke-readiness.sh`、`await-system-install.sh` 已经把用户级设置启动器视为用户级 Host 冲突，但 `post-install-regression.sh` 只检查：

- `~/Library/Input Methods/InputiaInputMethod.app`
- `~/Library/Input Methods/IputiaInputMethod.app`

它漏掉了：

- `~/Applications/Inputia 设置.app`

这意味着最终真实 GUI smoke 调度入口可能与 readiness/status 门禁不一致。已修复：

- `post-install-regression.sh` 增加 `USER_SETTINGS_APP="${INPUTIA_USER_SETTINGS_APP:-$HOME/Applications/Inputia 设置.app}"`。
- `user host conflict` gate 改为同时检查 user host、legacy user host、user settings launcher。
- 冲突输出增加 `settingsPath=$USER_SETTINGS_APP`，便于证据定位。
- `verify-nongui.sh` 增加 `VERIFY_POST_INSTALL_USER_SETTINGS_APP` 隔离路径，所有 post-install 调用都传入隔离 settings 路径，避免误读真实用户目录。
- `verify-nongui.sh` 增加 settings-only runtime gate：只创建 `/tmp/.../Inputia 设置.app`，验证 `post-install-regression.sh` 以 rc=3 阻断且不进入 system/TIS/GUI 阶段。

定点验证：

```text
tmp_root=$(mktemp -d /tmp/inputia-postinstall-settings-check.XXXXXX)
mkdir -p "$tmp_root/Inputia 设置.app"
INPUTIA_RUN_UI_SMOKE=1 \
  INPUTIA_USER_APP="$tmp_root/InputiaInputMethod.app" \
  INPUTIA_USER_LEGACY_APP="$tmp_root/IputiaInputMethod.app" \
  INPUTIA_USER_SETTINGS_APP="$tmp_root/Inputia 设置.app" \
  zsh macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app

  rc=3
  userHostConflict=true path=/tmp/.../InputiaInputMethod.app legacyPath=/tmp/.../IputiaInputMethod.app settingsPath=/tmp/.../Inputia 设置.app
```

完整非 GUI 回归：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-postinstall-settings-gate-20260708.log 2>&1
  verifyRc=0
  postInstallUserSettingsGate: userHostConflict=true ... settingsPath=/tmp/inputia-post-install-user-conflict.UpGhwl/Inputia 设置.app
  postInstallUserSettingsGate.rc=3
  postInstallUserSettingsGate.clipboardUnchanged=true
  postInstallUserSettingsGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUserSettingsGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUserSettingsGate.debugEnvBefore=unset
  postInstallUserSettingsGate.debugEnvAfter=unset
  postInstallUserSettingsGate.userHostUnchanged=true
  postInstallUserSettingsGateNoMutationPassed=true
  nonGuiVerificationPassed=true
```

当前真实状态复核：

```text
用户级路径：
  ~/Library/Input Methods/InputiaInputMethod.app exists=false
  ~/Library/Input Methods/IputiaInputMethod.app exists=false
  ~/Applications/Inputia 设置.app exists=false

status.sh:
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  systemMatchesBuild=false
  userHostConflict=false
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,safari-already-running
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,safari-already-running

tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app":
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=0
  tis.hansIconMatchesApp=false
  tis.readinessBlockReason=target-source-not-installed
  tisReadiness=false
```

当前结论：

- 最终真实 GUI smoke 调度入口 `post-install-regression.sh` 现在与 status/readiness/await 的用户级 Host 冲突定义一致。
- 本轮没有创建或保留真实用户级 Inputia/Inputia 设置 app；settings-only gate 使用 `/tmp` 隔离路径。
- 真实 GUI smoke 仍未运行。当前阻断包括系统安装版 v40、settings v40、无非交互管理员安装能力、TIS 未 ready；本轮 status 还显示 Safari 当前已运行，因此也会阻断 Safari smoke。

## v41 Mac mini 续修：进程 preflight 自检隔离真实 App 状态

继续检查具体 TextEdit/Safari/Clipboard smoke 脚本后，发现公共 preflight helper 与 post-install preflight helper 仍存在与 await 侧类似的证据稳定性问题：自检用 `INPUTIA_PROCESS_RUNNING_FOR_TEST` / `INPUTIA_UI_PROCESS_RUNNING_FOR_TEST` 模拟运行中进程，但 helper 仍会同时读取真实 `pgrep -x`。如果用户机器当时打开了 TextEdit/Safari，自检分支可能被真实 App 状态污染。

已修复：

- `smoke-common.sh` 的 `inputia_require_process_not_running()` 增加 `INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1`。
- `post-install-regression.sh` 的 `require_ui_process_idle()` 增加 `INPUTIA_UI_PROCESS_IGNORE_REAL_FOR_TEST=1`。
- `INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1` 自动启用 `INPUTIA_UI_PROCESS_IGNORE_REAL_FOR_TEST=1`。
- `verify-nongui.sh` 增加静态契约，要求两个 helper 都保留 ignore-real 测试变量。
- `verify-nongui.sh` 的公共 process preflight helper 自检显式使用 `INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1`，避免未来把测试进程名改成真实 App 名时被机器状态污染。

定点验证：

```text
bash -n macos/InputiaInputMethod/smoke-common.sh
zsh -n macos/InputiaInputMethod/post-install-regression.sh
zsh -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0

INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 zsh macos/InputiaInputMethod/post-install-regression.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  postInstallUiPreflightSelfCheck case=textedit-block TextEditPreflight=running
  postInstallUiPreflightSelfCheck case=safari-block SafariPreflight=running
  postInstallUiPreflightSelfCheck case=inputia-block InputiaInputMethodPreflight=running
  postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflightAllowed=true
  postInstallUiPreflightSelfCheck case=safari-allow SafariPreflightAllowed=true
  postInstallUiPreflightSelfCheck=true

source macos/InputiaInputMethod/smoke-common.sh
INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1 inputia_require_process_not_running Safari helperReady 44 safari-running -
  SafariPreflight=not-running
  smokeCommonIgnoreRealRc=0
```

完整非 GUI 回归：

```text
bash macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-process-ignore-real-20260708.log 2>&1
  verifyRc=0
  processPreflightClearSelfCheck: InputiaFakeProcessForTestPreflight=not-running
  processPreflightBlockSelfCheck: InputiaFakeProcessForTestPreflight=running
  processPreflightAllowSelfCheck: InputiaFakeProcessForTestPreflightAllowed=true
  processPreflightNoAllowSelfCheck.rc=45
  processPreflightHelperSelfCheck=true
  postInstallUiPreflightSelfCheck=true
  nonGuiVerificationPassed=true
```

当前真实状态复核：

```text
用户级路径：
  ~/Library/Input Methods/InputiaInputMethod.app exists=false
  ~/Library/Input Methods/IputiaInputMethod.app exists=false
  ~/Applications/Inputia 设置.app exists=false

status.sh:
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  systemMatchesBuild=false
  userHostConflict=false
  statusTISEnabledMatches=0
  statusTISInstalledMatches=3
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app":
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=1
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  tisReadiness=false
```

当前结论：

- helper 自检不再依赖真实 TextEdit/Safari/Inputia 进程状态；真实 smoke 默认仍会读取 `pgrep` 并阻断已有用户 App。
- TIS 外部状态有漂移：当前能看到系统 v40 的 installed sources，但当前 build v41 仍未安装为系统目标，enabled matches 仍为 0。
- 真实 TextEdit/Safari/Clipboard GUI smoke 仍未运行；继续等待系统版/settings 更新到 v41、TIS readiness 通过、且目标 App 不处于用户既有运行状态。

## v41 Mac mini 续修：debug event log 路径类型错误稳定早退

背景：`smoke-safari-enter.sh` 与 `smoke-clipboard-recall.sh` 都支持外部传入 `INPUTIA_DEBUG_EVENTS`。此前如果该路径是目录，`inputia_prepare_debug_event_log()` 会直接执行 `: >"$event_log"`，由 shell redirection 报出 `Is a directory`；在 `verify-nongui.sh` 的 command substitution gate 中还会出现误导性的 `syntax error near unexpected token ';'`。这不利于判断 cleanup 纪律，也会掩盖真实失败原因。

已补齐：

- `smoke-common.sh`
  - `inputia_prepare_debug_event_log()` 对外部传入路径先检查：存在但不是普通文件时输出稳定 marker：
    - `debugEventLogPrepare=false path=... reason=not-regular-file`
    - 返回 rc=1，不再让 shell redirection error 穿透。
  - 成功准备时输出：
    - `debugEventLogPrepare=true path=...`
- `verify-nongui.sh`
  - 静态契约要求 helper 同时包含 prepare success/failure marker 和 `not-regular-file` reason。
  - Safari enter / Clipboard recall 的 debug-log-prepare failure gate 改为 `run_expect_rc 1`，并断言 marker/reason、剪贴板、当前输入源、debug env、user host、GUI/Host 进程均无污染。
- `await-system-install.sh`
  - 顺手把 `target-cdhash-mismatch`、`tis-not-ready`、`user-host-conflict` 主阻塞输出改成显式分支；仍保留 `uiSmokeBlockReasons` 组合原因，避免回退到隐式 `${block_reasons%%,*}` 难审计路径。

最小复现：

```text
INPUTIA_DEBUG_EVENTS=<directory> smoke-safari-enter.sh build/InputiaInputMethod.app
  safariRc=1
  guiSessionCheck=skipped
  safariPreflight=running|not-running
  InputiaInputMethodPreflight=not-running
  debugEventLogPrepare=false path=/tmp/inputia-safari-enter-dirty-log-repro.* reason=not-regular-file

INPUTIA_DEBUG_EVENTS=<directory> smoke-clipboard-recall.sh build/InputiaInputMethod.app
  clipboardRc=1
  guiSessionCheck=skipped
  textEditPreflight=not-running docs=0
  InputiaInputMethodPreflight=not-running
  clipboardRestorable=true
  debugEventLogPrepare=false path=/tmp/inputia-clipboard-dirty-log-repro.* reason=not-regular-file
```

完整非 GUI 回归：

```text
macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-debug-log-prepare-20260708.log 2>&1
  verifyRc=0
  safariEnterDebugLogPrepareGate: debugEventLogPrepare=false path=/tmp/inputia-safari-enter-dirty-log.* reason=not-regular-file
  safariEnterDebugLogPrepareGate.rc=1
  safariEnterDebugLogPrepareGate.clipboardUnchanged=true
  safariEnterDebugLogPrepareGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  safariEnterDebugLogPrepareGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  safariEnterDebugLogPrepareGate.debugEnvBefore=unset
  safariEnterDebugLogPrepareGate.debugEnvAfter=unset
  safariEnterDebugLogPrepareGate.userHostUnchanged=true
  safariEnterDebugLogPrepareGateNoMutationPassed=true
  clipboardDebugLogPrepareGate: debugEventLogPrepare=false path=/tmp/inputia-clipboard-dirty-log.* reason=not-regular-file
  clipboardDebugLogPrepareGate.rc=1
  clipboardDebugLogPrepareGate.clipboardUnchanged=true
  clipboardDebugLogPrepareGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  clipboardDebugLogPrepareGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  clipboardDebugLogPrepareGate.debugEnvBefore=unset
  clipboardDebugLogPrepareGate.debugEnvAfter=unset
  clipboardDebugLogPrepareGate.userHostUnchanged=true
  clipboardDebugLogPrepareGateNoMutationPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- Safari enter 与 Clipboard recall 的 debug event log 准备失败路径现在是稳定、可断言、无污染的早退路径。
- 本轮仍未运行真实 TextEdit/Safari/Clipboard GUI smoke；系统安装版/settings 仍是 v40，TIS 未 ready，真实 GUI smoke 继续由 readiness gate 阻断。

## v41 Mac mini 续修：post-install 用户级 Settings 隔离与 Clipboard 事件窗口顺序契约

背景：

- `post-install-regression.sh` 已把 `~/Library/Input Methods/InputiaInputMethod.app`、历史 typo host 和 `~/Applications/Inputia 设置.app` 都作为用户级冲突处理。
- `verify-nongui.sh` 的 post-install 隔离根目录原本已经有 `VERIFY_POST_INSTALL_USER_SETTINGS_APP`，但调用 post-install 的部分路径只传了 `INPUTIA_USER_APP` / `INPUTIA_USER_LEGACY_APP`，容易漏读真实用户级 settings launcher。
- Clipboard recall 的 event log reset 已存在，但 verifier 只约束“reset 在 pre-trigger guard 前”，没有明确要求它发生在 `state-clear-leaked-text` 检查之后；若顺序回退，仍可能把清 IME 状态阶段事件混入召回验证窗口。

已补齐：

- `verify-nongui.sh`
  - post-install active-lock gate、post-install non-GUI gate、post-install UI TIS gate 都传入：
    - `INPUTIA_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_SETTINGS_APP"`
  - 新增/保留 `postInstallUserSettingsGate`，用隔离目录里的 `Inputia 设置.app` 模拟用户级 settings launcher 冲突，要求 post-install 返回 rc=3 并输出 `settingsPath=...`。
  - Clipboard recall 静态契约加强：`resetClipboardRecallEventLog()` 必须发生在 `state-clear-leaked-text` 检查之后，且 reset 失败检查必须发生在 `assertNoClipboardRecallBeforeTrigger()` 和 `Control-Shift-V` 之前。

验证：

```text
macos/InputiaInputMethod/verify-nongui.sh > /tmp/inputia-verify-nongui-postinstall-settings-isolation-clean-20260708.log 2>&1
  verifyRc=0
  postInstallUserSettingsGate: userHostConflict=true ... settingsPath=/tmp/inputia-post-install-user-conflict.*/Inputia 设置.app
  postInstallUserSettingsGate.rc=3
  postInstallUserSettingsGate.clipboardUnchanged=true
  postInstallUserSettingsGate.currentSourceBefore=com.tencent.inputmethod.wetype.pinyin
  postInstallUserSettingsGate.currentSourceAfter=com.tencent.inputmethod.wetype.pinyin
  postInstallUserSettingsGate.debugEnvBefore=unset
  postInstallUserSettingsGate.debugEnvAfter=unset
  postInstallUserSettingsGate.userHostUnchanged=true
  postInstallUserSettingsGateNoMutationPassed=true
  clipboardRecallCleanupSelfCheckNoMutationPassed=true
  clipboardDebugLogPrepareGateNoMutationPassed=true
  postInstallNonGuiNoMutationPassed=true
  postInstallUiTisGateNoLaunchPassed=true
  residue=false
  tmpResidue=false
  nonGuiVerificationPassed=true
```

当前结论：

- 非 GUI verifier 不再通过 post-install 路径读取真实用户级 `Inputia 设置.app`，避免污染系统级 GUI smoke 基线。
- Clipboard recall 的事件窗口顺序现在被静态契约锁住：清状态、确认无残留文本、重置 event log、确认 reset 成功、再检查 pre-trigger、最后触发召回。
- 本轮仍未运行真实 TextEdit/Safari/Clipboard GUI smoke；系统安装版/settings 仍是 v40，TIS 未 ready。

收尾状态复核：

```text
macos/InputiaInputMethod/status.sh
  buildVersion=41
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  system host version=40
  systemMatchesBuild=false
  user host exists=false
  user settings launcher exists=false
  includeAllInstalled=true matches=3
  statusTISEnabledMatches=0
  statusTISInstalledMatches=3
  statusUserHostConflict=false
  statusTextEditPreflight=not-running
  statusSafariPreflight=not-running
  statusInputiaHostPreflight=not-running
  statusGuiSmokeReady=false reason=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready

macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  buildCDHash=117dcb3797504ee99805c1f14c57a49c87f92a4c
  appCDHash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
  appMatchesBuild=false
  tis.enabledMatches=0
  tis.installedMatches=3
  tis.targetEnabledMatches=0
  tis.targetInstalledMatches=1
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.hansSelectable=true
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.readinessBlockReason=missing-enabled-source
  tisReadiness=false
```

解释：TIS 现在能看到系统 v40 的 Hans/Hant source，但目标 build 仍是 v41，系统 app CDHash 不匹配；target-filter readiness 仍不通过，所以不能把该状态当作真实 GUI smoke ready。

## 2026-07-08 v43 系统安装去重、稳定选中与 Command 快捷键放行

背景：

- 用户截图中菜单栏出现多条同名 `Inputia`，并且“看起来全都选中但实际不可用”。本机复现到两个层面：
  - 磁盘上不是多个系统 app：最终只应保留 `/Library/Input Methods/InputiaInputMethod.app`。
  - TIS/菜单缓存会在多轮注册、刷新和早期 `defaults -array-add` fallback 后产生错乱；同进程 `TISSelectInputSource` 可返回 `0`，但新进程可能马上看到 `matches=0` 或当前源回到微信输入法。
- 官方参考：
  - Apple Mac keyboard shortcuts: https://support.apple.com/guide/imac/keyboard-shortcuts-apd194062a6d/mac
  - Apple InputMethodKit: https://developer.apple.com/documentation/inputmethodkit
  - Apple NSTextInputContext keyboardInputSources / Text Input Source Services: https://developer.apple.com/documentation/appkit/nstextinputcontext/keyboardinputsources

关键选择：

- 安装主入口继续使用 `macos/InputiaInputMethod/install-system.sh`；pkg 只作为可双击/installer 分发入口，两者都必须安装到 `/Library/Input Methods/InputiaInputMethod.app`。
- `InputiaInputMethod --normalize-hitoolbox` 新增为安装/诊断专用命令：
  - 清掉 `com.inputia.inputmethod.Inputia`、旧 typo `com.iputia.inputmethod.Iputia`、早期 `dev.inputia.inputmethod.Inputia` 的重复偏好项。
  - `AppleEnabledInputSources` 只保留一组 Inputia parent + `com.inputia.inputmethod.Inputia.Hans`。
  - `AppleSelectedInputSources` 只保留当前目标 Hans mode。
  - `AppleInputSourceHistory` 把 Hans 放到第一项，并保留非 Inputia 历史项。
- `install-system.sh` 不再把“同进程 select 成功”当最终结论。刷新 `TextInputMenuAgent` / `SystemUIServer` / `cfprefsd` 后重新 register、normalize、select，并要求连续两次新进程验证：
  - `tisReadiness=true`
  - `tis.currentMatchesTarget=true`
- `Packaging/scripts/postinstall` 同步移除 `defaults -array-add` 累积路径，改为调用 host 的 normalize，并加入 pkg 安装后的稳定选中等待。
- `build-pkg.sh` 仍会 `rm -rf dist` 后重新生成 `InputiaInputMethod-latest.pkg`，所以 dist 只保留 latest 和当前版本 pkg。
- 版本升到 `CFBundleVersion=43` / `0.0.43`，降低 macOS 缓存继续命中 v42 的概率。

快捷键策略：

- 根据 Apple 常用快捷键文档，所有包含 Command 的普通组合键默认放行给系统/App：
  - 覆盖 `Command-C/V/X/Z/A/S/O/W/Q/F/G/H/M/P/T/N/D/E/I/R/J/K/Y/,/Tab/Space/数字/括号/方向键/Delete`
  - 覆盖 `Command-Shift-3/4/5`、`Command-Option-Escape`、`Command-Shift-V`、`Command-Option-V`、`Command-Control-V` 等代表性变体。
- Inputia 自己的快捷键继续限定在无 Command 的组合上：
  - `Control-Shift-V`：剪贴板召回
  - `Control-Shift-S`：简繁切换
  - `Shift`：中英文切换，按设置决定

验证：

```text
macos/InputiaInputMethod/install-system.sh
  installLog=/tmp/inputia-install-system-v43-20260708091854.log
  sourceVersion=43
  sourceCDHash=ac43ad52c2e5f0eb4b68b41c8ee8dad38d33e003
  userHostRemoved=true path=/Users/minizl/Library/Input Methods/InputiaInputMethod.app
  userHostBackupRemoved=true path=/Users/minizl/Library/Input Methods/InputiaInputMethod.app.inputia-smoke-backup-20260708004302
  destVersion=43
  destCDHash=ac43ad52c2e5f0eb4b68b41c8ee8dad38d33e003
  systemInstallVerified=true
  legacyIputiaRemoved=true
  settingsLauncherInstalled=true path=/Applications/Inputia 设置.app
  hitoolboxNormalize=true
  systemInstallTISStableCheck attempt=1 ready=true consecutive=1
  systemInstallTISStableCheck attempt=2 ready=true consecutive=2
  systemInstallPostRefreshTISReady=true
```

```text
macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=true
  tis.enabledMatches=2
  tis.installedMatches=3
  tis.targetEnabledMatches=1
  tis.targetInstalledMatches=1
  tis.hansEnabled=true
  tis.hansSelectable=true
  tis.hansSelected=true
  tis.currentID=com.inputia.inputmethod.Inputia.Hans
  tis.currentMatchesTarget=true
  tisReadiness=true
```

```text
macos/InputiaInputMethod/status.sh
  systemMatchesBuild=true
  system host version=43
  system settings launcher version=43
  systemSettingsMatchesBuildVersion=true
  user host exists=false
  userHostConflict=false
  legacyIputiaPresent=false
  statusTISEnabledMatches=2
  statusTISInstalledMatches=3
  statusGuiSmokeReady=true reason=none
```

```text
defaults read com.apple.HIToolbox AppleEnabledInputSources
  Inputia entries:
    { Bundle ID = com.inputia.inputmethod.Inputia; InputSourceKind = Keyboard Input Method; }
    { Bundle ID = com.inputia.inputmethod.Inputia; Input Mode = com.inputia.inputmethod.Inputia.Hans; InputSourceKind = Input Mode; }

defaults read com.apple.HIToolbox AppleSelectedInputSources
  { Bundle ID = com.inputia.inputmethod.Inputia; Input Mode = com.inputia.inputmethod.Inputia.Hans; InputSourceKind = Input Mode; }

find "/Library/Input Methods" "$HOME/Library/Input Methods" -maxdepth 1 -iname '*inputia*' -o -iname '*iputia*'
  /Library/Input Methods/InputiaInputMethod.app
```

```text
macos/InputiaInputMethod/build-pkg.sh
  buildPkgRC=0
  packageVersion=43
  buildVersion=43
  pkgVerificationPassed=true
  latest.pkg sha256=95af1e8f550bc8c57d362dad297b367abcfbe1de5cc37ac9c0923c9e223eabf1
  versioned pkg=/Users/minizl/services/Handy/macos/InputiaInputMethod/dist/InputiaInputMethod-v43-ac43ad52c2e5.pkg

sudo installer -pkg macos/InputiaInputMethod/dist/InputiaInputMethod-latest.pkg -target /
  installerRC=0
  installer: The install was successful.
```

```text
INPUTIA_RUN_UI_SMOKE=0 macos/InputiaInputMethod/post-install-regression.sh
  postInstallRegressionRC=0
  hostShortcutSelfCheck=true
  commonAppleCommandShortcutSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true
  commandCPassThrough=true
  commandVPassThrough=true
  commandShiftVPassThrough=true
  commandOptionVPassThrough=true
  commandTabPassThrough=true
  commandSpacePassThrough=true
  commandShift4PassThrough=true
  current source id=com.inputia.inputmethod.Inputia.Hans
  uiSmokeSkipped=true reason=disabled
  postInstallRegressionPassed=true
```

收尾状态：

- 未运行 TextEdit/Safari/Clipboard 真实 GUI smoke；默认验证没有抢焦点。
- 已清掉验证残留的 `InputiaInputMethod --bridge-self-check` 进程；最终 `pgrep` 没有 TextEdit、InputiaInputMethod、IputiaInputMethod 残留。
- Safari 只剩系统级 helper/agent 进程，未由本轮打开 Safari 窗口。

## 2026-07-08 v44 签名拒绝导致“允许后仍无法切换”的闭环根因

用户在系统设置里选择 Inputia 时反复看到“允许 TextInputMenuAgent 启用 Inputia?”，点“允许”后仍不能切换，下一次选择又继续弹窗。该现象不是用户没有授权，也不是单纯 TIS 偏好没写入；根因是当前 Inputia 开发构建仍是 ad-hoc 签名，macOS Gatekeeper/系统策略把 `/Library/Input Methods/InputiaInputMethod.app` 判为 rejected。TIS 命令行偏好可以短暂出现 Inputia 条目，但菜单栏 `TextInputMenuAgent` 不会稳定展示/选择它，因此形成“看起来安装了，实际不可用”的闭环。

本轮参考面：

- Apple InputMethodKit 文档：输入法必须作为 macOS 输入源由系统加载，不能只依赖普通 App 自己的状态。
- Apple Gatekeeper/Notarization 文档：分发或系统级使用的 macOS App 需要可被系统信任的签名/公证链；ad-hoc 签名只能满足 `codesign --verify` 的结构校验，不等价于 `spctl --assess` accepted。
- 成熟实现对照：本机 `/Library/Input Methods/WeType.app` 为 Developer ID / notarized 签名，`spctl` accepted；Inputia 为 `Signature=adhoc`、`TeamIdentifier=not set`，`spctl` rejected。

关键证据：

```text
codesign -dv --verbose=4 /Library/Input\ Methods/InputiaInputMethod.app
  Signature=adhoc
  TeamIdentifier=not set
  CDHash=97bd9a98cab276f30b195fe97f92ea247c2da48d

spctl --assess --type execute --verbose=4 /Library/Input\ Methods/InputiaInputMethod.app
  /Library/Input Methods/InputiaInputMethod.app: rejected

spctl --assess --type execute --verbose=4 /Library/Input\ Methods/WeType.app
  /Library/Input Methods/WeType.app: accepted
```

尝试过但放弃的方案：

- 继续写 `AppleEnabledInputSources` / `AppleSelectedInputSources`：会制造“设置里看得到、菜单里不能用”的假状态，且触发重复授权弹窗。
- 对 ad-hoc 构建继续执行 register/enable/select：TIS 层可能显示 enabled，但 `TextInputMenuAgent` 菜单和真实切换仍失败。
- 创建本地自签名证书并导入 System keychain：证书可生成，但当前环境无法通过非交互方式把本地 CA 设为系统信任，`security find-identity -p codesigning` 仍没有有效身份；残留测试证书已清理。

实现修正：

- `Info.plist` / `SettingsLauncher/Info.plist` 升到 v44，显示名统一为 `Inputia`；`InfoPlist.strings` 中 `Inputia.Hans` / `Inputia.Hant` 也都显示为 `Inputia`，不再显示 `Inputia 简体`。
- `InputiaInputMethod` 增加 `--clear-input-source-preferences`，可清理 Inputia 在 HIToolbox / inputsources 中的 enabled、selected、history、third-party enabled 残留。
- `install-system.sh` 和 pkg `postinstall` 在复制并校验 CDHash 后先跑 `spctl --assess --type execute`。如果结果不是 accepted，且没有显式 `INPUTIA_ALLOW_REJECTED_SIGNATURE=1`，则输出 `reason=signature-rejected`、清理 Inputia 输入源偏好、刷新 `TextInputMenuAgent/SystemUIServer/cfprefsd`，并以 rc=14 停止，不再注册/启用/选择。
- `tis-readiness.sh`、`status.sh`、`gui-smoke-readiness.sh`、`smoke-preflight.sh`、`post-install-regression.sh`、`await-system-install.sh` 都加入签名 blocker，拒签时明确输出 `signature-rejected`，不再只报泛化的 `tis-not-ready`。
- 新增 `menu-readiness.sh`，直接读取 `TextInputMenuAgent` 菜单项，避免只看 TIS 命令行状态误判。

验证：

```text
macos/InputiaInputMethod/build-pkg.sh
  buildPkgRc=0
  packageVersion=44
  buildVersion=44
  appCDHash=97bd9a98cab276f30b195fe97f92ea247c2da48d
  pkgVerificationPassed=true
  latest.pkg sha256=1ffccb1cfa893de53293bb89924fbf56a845c21bdacd9d134506cbed071c65db

macos/InputiaInputMethod/verify-nongui.sh
  verifyNonGuiRc=0
  nonGuiVerificationPassed=true
  commandCPassThrough=true
  commandVPassThrough=true
  commonAppleCommandShortcutSetPassesThrough=true
  allCommandModifierVariantsPassThrough=true
```

最终系统状态：

```text
macos/InputiaInputMethod/status.sh
  systemMatchesBuild=true
  system host version=44
  system settings launcher version=44
  legacyIputiaPresent=false
  userHostConflict=false
  statusSignatureAccepted=false
  statusMenuReadiness=false
  statusMenuBlockReason=inputia-menu-item-missing
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing

macos/InputiaInputMethod/menu-readiness.sh
  menuInputiaCount=0
  menuInputiaSelectedCount=0
  menuReadiness=false
  menuReadinessBlockReason=inputia-menu-item-missing

macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appMatchesBuild=true
  appSignatureAccepted=false
  tis.readinessBlockReason=signature-rejected
  tisReadiness=false
```

结论：

- 当前 v44 已阻止继续制造重复弹窗/假启用状态，但因为没有可被系统接受的签名身份，Inputia 仍不能被真实切换使用。
- 要让它在这台 Mac mini 上可用，下一步必须使用可被 `spctl --assess --type execute` 接受的签名：Developer ID Application + notarization，或一个通过 GUI/MDM 正式信任的本地开发 CA/Apple Development 身份。签名接受后，现有安装脚本才会继续执行 register/enable/select 和 GUI smoke。

### 2026-07-08 Mac mini 签名身份缺失只读复核

侧边同步指令要求先停止把主线放在 smoke/verifier/提示文案上，优先确认 MacBook 可用而 Mac mini 不可用的关键环境差异。只读复核时间：`2026-07-08 11:14:06 CST`。

事实：

```text
security find-identity -v -p codesigning
  0 valid identities found

codesign -dv --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  CodeDirectory flags=0x2(adhoc)
  CDHash=97bd9a98cab276f30b195fe97f92ea247c2da48d
  Signature=adhoc
  TeamIdentifier=not set

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  /Library/Input Methods/InputiaInputMethod.app: rejected
```

结论：

- 当前 blocker 不是 TIS 刷新、不是安装包，也不是 GUI smoke。Mac mini 没有 MacBook 上可用的受信任 code signing identity，导致当前系统目录里的 Inputia 只能被 ad-hoc 签名。
- ad-hoc 签名被 `spctl`/Gatekeeper 拒绝后，Text Input/TIS 可以残留或显示部分记录，但 `TextInputMenuAgent` 不会把 Inputia 当作可稳定切换的系统输入源，所以会出现“允许后仍无法切换/反复弹允许”的闭环。
- `INPUTIA_ALLOW_REJECTED_SIGNATURE=1` 只能用于诊断，不是正式解决方案；在 `spctl` accepted 前不要继续运行 TextEdit/Safari/Clipboard 真实 GUI smoke。

下一步推荐：

1. 在 MacBook Keychain Access 中导出此前可用的签名身份，例如 `Codexbar Local Code Signing Leaf v4`，格式为 `.p12`，必须包含 private key。
2. 将 `.p12` 导入 Mac mini 的 login keychain，并在 Keychain Access 中设为信任。
3. 导入后先验证 `security find-identity -v -p codesigning` 能看到有效身份，再用该 identity 重新 build/install。

### 2026-07-08 非 GUI 清理纪律复核

复核时间：`2026-07-08 11:25:09 CST`。

本轮只处理与当前目标直接相关的清理纪律和签名门禁，不运行 TextEdit/Safari/Clipboard 真实 GUI smoke。

事实与修正：

- `verify-nongui.sh` 一次完整运行被大量输出/运行中断打断后，留下了 `/private/tmp/inputia-safari-enter-dirty-log.*`。这不是 `smoke-safari-enter.sh` 的正常退出残留，而是 verifier 被异常终止后的历史残留污染后续运行。
- `verify-nongui.sh` 已新增启动期 stale residue 清理，并把 `VERIFY_TEMP_DIRS` 的 trap 清理前缀从仅 `/tmp/inputia-*` 扩展到 `/tmp/inputia-*` 和 `/private/tmp/inputia-*`，匹配 macOS `/tmp -> /private/tmp` 的实际路径。
- 后续发现一条后台 `verify-nongui.sh` 仍在运行并继续生成 `inputia-safari-enter-dirty-log.*`；已只清理该 verifier/status/menu-readiness 测试进程和假 `InputiaInputMethod`，未触碰系统 Safari 服务。复核后历史 `inputia-safari-enter-dirty-log.*` 已清空。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh \
  macos/InputiaInputMethod/install-system.sh \
  macos/InputiaInputMethod/status.sh \
  macos/InputiaInputMethod/tis-readiness.sh \
  macos/InputiaInputMethod/verify-pkg.sh
zsh -n macos/InputiaInputMethod/Packaging/scripts/postinstall
  rc=0

macos/InputiaInputMethod/verify-pkg.sh
  pkgVerificationPassed=true

isolated gui-smoke-suite blocked gate:
  rc=12
  changed=false
  guiSmokeSuiteReady=false reason=signature-rejected
  guiSmokeSuiteWouldRun=false

isolated status blocker commands:
  rc=0
  changed=false
```

完整 `verify-nongui.sh` 的最新长跑仍不能作为绿色证据：失败点在长时间运行期间的 `*-mutated-clipboard` 检测，且失败位置在不同 gate 间漂移；隔离复现证明对应 blocked gate 本身不写剪贴板。结论是当前验证环境的外部剪贴板状态会干扰长跑 verifier 的全程 `pbpaste` 对比，不能把它解释为某个 Inputia 阻断路径稳定污染剪贴板。

当前系统 blocker 未变：

```text
security find-identity -v -p codesigning
  0 valid identities found

macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false
```

### 2026-07-08 Mac mini .p12 导入尝试

复核时间：`2026-07-08 11:32:49 CST`。

用户已从 MacBook 传输签名身份 `.p12` 到 Mac mini，目标路径：

```text
/Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12
```

只读检查：

```text
ls/stat
  mode=600
  owner=lizhelang
  group=staff
  size=13593

security find-identity -v -p codesigning
  0 valid identities found
```

导入尝试：

```text
security import /Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12 \
  -k "$HOME/Library/Keychains/login.keychain-db" \
  -T /usr/bin/codesign \
  -T /usr/bin/security
```

结果：

- 命令 30 秒未返回，表现为等待 Keychain 或 `.p12` 密码交互；已中止，避免后台挂起。
- 中止后再次验证：

```text
security find-identity -v -p codesigning
  0 valid identities found

security find-certificate -a -c "Codexbar Local Code Signing Leaf v4" "$HOME/Library/Keychains/login.keychain-db"
  no matching certificate output
```

结论：

- `.p12` 文件已到位且权限正确，但签名 identity 尚未成功导入到 Mac mini 当前用户 login keychain。
- 当前不能继续 `build.sh` / `install-system.sh` 的签名安装链路；否则仍会 ad-hoc 签名并回到 `signature-rejected`。
- 下一步需要获得 `.p12` 导入密码，或由用户在 Keychain Access/系统导入对话中完成该 `.p12` 导入并信任。完成后必须先看到：

```text
security find-identity -v -p codesigning | grep -F "Codexbar Local Code Signing Leaf v4"
```

再继续重签、系统安装和 readiness 验证。

### 2026-07-08 v44 签名 blocker 输出补强

复核时间：`2026-07-08 11:31:18 CST`。

本轮针对用户看到的“允许 TextInputMenuAgent 启用 Inputia 后仍不能切换、再次点击又弹窗”的闭环，继续按系统输入法证据规则处理：不绕过 macOS 信任判断，不继续制造假 enabled/selected 状态。

官方依据：

- Apple InputMethodKit 是 macOS 输入法的官方框架入口；真实可用性最终要经过系统 Text Input/TIS 链路。
- Apple notarization / Gatekeeper 路径要求分发软件使用可被系统接受的签名。当前 `spctl --assess --type execute` 拒绝 Inputia，因此不能把 ad-hoc 构建当作可用系统输入法继续 enable/select。

实现补强：

- `install-system.sh` 在 `signature-rejected` 时新增：
  - `systemInstallRequiredAction=sign-with-accepted-identity`
  - `systemInstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts`
- pkg `postinstall` 在同一 blocker 下新增：
  - `inputiaPostinstallRequiredAction=sign-with-accepted-identity`
  - `inputiaPostinstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts`
- `tis-readiness.sh` 新增 `tis.requiredAction=sign-with-accepted-identity`。
- `status.sh` 新增 `statusSigningRequiredAction=sign-with-accepted-identity`。
- `README.md` 补充：签名未被 `spctl` 接受时不要反复手动允许、不要跑真实 GUI smoke；下一步是使用能被 `spctl` 接受的签名身份重建/重装。

验证：

```text
zsh -n install-system.sh Packaging/scripts/postinstall
bash -n status.sh tis-readiness.sh gui-smoke-readiness.sh await-system-install.sh verify-pkg.sh verify-nongui.sh
  rc=0

macos/InputiaInputMethod/build-pkg.sh
  rc=0
  packageVersion=44
  appCDHash=97bd9a98cab276f30b195fe97f92ea247c2da48d
  latest.pkg sha256=9027fb72bf1891bba59a6237555ed7f7dd53821055b665d26a5818594eac41fe
  pkgVerificationPassed=true

macos/InputiaInputMethod/verify-pkg.sh
  pkgVerificationPassed=true

macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appAssessment=/Library/Input Methods/InputiaInputMethod.app: rejected
  appSignatureAccepted=false
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false

macos/InputiaInputMethod/menu-readiness.sh
  menuInputiaCount=0
  menuInputiaSelectedCount=0
  menuReadiness=false
  menuReadinessBlockReason=inputia-menu-item-missing

macos/InputiaInputMethod/status.sh
  systemMatchesBuild=true
  statusSignatureAccepted=false
  statusSigningRequiredAction=sign-with-accepted-identity
  statusMenuReadiness=false
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing

INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 macos/InputiaInputMethod/gui-smoke-readiness.sh
  guiSmokeReadinessSelfCheck signatureBlockReasons=signature-rejected actual=signature-rejected
  guiSmokeReadinessSelfCheck=true

INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 macos/InputiaInputMethod/await-system-install.sh
  awaitUiStatusSelfCheck reason=signature-rejected ...
  awaitUiStatusSelfCheck=true

security find-identity -p codesigning -v
  0 valid identities found
```

完整 `verify-nongui.sh` 仍不作为本轮最终绿色门禁：它的长跑会受桌面状态、全局剪贴板和 timeout 自检影响；本轮已用短验证覆盖实际修改点。后续应把 verifier 拆成更小的稳定分组，再恢复整套长跑作为 release gate。

结论：

- 当前用户看到的授权弹窗闭环根因仍是 `spctl` 拒绝 ad-hoc 签名；安装包、CDHash、Info.plist 显示名、TIS source 枚举不是主要 blocker。
- 现有脚本已避免在拒签状态继续注册/启用/选择 Inputia，下一步必须导入/配置可被 `spctl` 接受的 code signing identity 后再重建安装。

### 2026-07-08 p12 签名身份导入尝试

复核时间：`2026-07-08 11:33:37 CST`。

用户已把 `Codexbar Local Code Signing Leaf v4` 的 `.p12` 传到 Mac mini；本轮切回签名导入/系统安装主线，未运行 TextEdit/Safari/Clipboard GUI smoke。

文件检查：

```text
p12Exists=true
p12Path=/Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12
p12Owner=lizhelang
p12Group=staff
p12Mode=600
p12Size=13593
p12SHA256=a56a21fb1874274d0c350f59a99f653d6e58b7d0cafc4614f4a0b08fc6fa20f4
defaultKeychain=/Users/minizl/Library/Keychains/login.keychain-db
```

导入前状态：

```text
security find-identity -v -p codesigning
  0 valid identities found
```

导入尝试：

```text
security import Codexbar-Local-Code-Signing-Leaf-v4.p12 \
  -k /Users/minizl/Library/Keychains/login.keychain-db \
  -P '' \
  -T /usr/bin/codesign \
  -T /usr/bin/security

security: SecKeychainItemImport: MAC verification failed during PKCS12 import (wrong password?)
```

结论：

- `.p12` 文件已到位且权限正确。
- 空 p12 密码被 PKCS#12 校验拒绝；不能继续猜密码。
- 阻塞点是需要用户提供 `.p12` 导入密码。导入成功且 `security find-identity -v -p codesigning | grep -F "Codexbar Local Code Signing Leaf v4"` 出现有效 identity 后，才能继续 `build.sh` / `install-system.sh` / `spctl` / `tis-readiness`。

### 2026-07-08 p12 导入成功但信任链未完成

复核时间：`2026-07-08 11:43:16 CST`。

用户提供 `.p12` 密码后继续签名主线；本轮未运行 TextEdit/Safari/Clipboard GUI smoke。

导入结果：

```text
security import Codexbar-Local-Code-Signing-Leaf-v4.p12 ...
  5 identities imported.
  securityImportRc=0

security find-identity -v -p codesigning
  0 valid identities found
```

进一步排查：

```text
security find-certificate -c "Codexbar Local Code Signing Leaf v4"
  SHA-1 hash: 12C73495BB5C15FD71C22E823A8A9CBD0CC5243C
  subject: Codexbar Local Code Signing Leaf v4
  issuer: Codexbar Local Code Signing Root v4

openssl pkcs12 -info
  MAC verified OK
  Certificate bag ... Codexbar Local Code Signing Leaf v4
  Shrouded Keybag ... localKeyID F6 F4 0A 9C 46 EB CB E0 65 DF A9 B4 33 8E 90 E2 A0 AA 09 75

security find-certificate -c "Codexbar Local Code Signing Root v4"
  not found in login keychain
  not found in System keychain
```

最小 codesign spike：

```text
codesign --force --sign "Codexbar Local Code Signing Leaf v4" --options runtime /tmp/inputia-codesign-test
  Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  errSecInternalComponent
  codesignSpikeRc=1

codesign --force --sign "Codexbar Local Code Signing Leaf v4" --options runtime --timestamp=none ...
  same chain/root failure
```

尝试过但未成功的非交互信任修复：

```text
security add-trusted-cert -r trustAsRoot -p codeSign -k login.keychain-db leaf.cer
  SecTrustSettingsSetTrustSettings: The authorization was denied since no user interaction was possible.

sudo security add-trusted-cert -d -r trustAsRoot -p codeSign -k System.keychain leaf.cer
  SecTrustSettingsSetTrustSettings: The authorization was denied since no user interaction was possible.

osascript do shell script "... add-trusted-cert ..." with administrator privileges
  same no-user-interaction authorization denial

security trust-settings-import generated plist
  plist format valid after adding modDate
  SecTrustSettingsImportExternalRepresentation: The authorization was denied since no user interaction was possible.

temporary AuthorizationDB allow attempt for com.apple.trust-settings.admin
  denied; authorizationdb write returned NO (-60005)
```

结论：

- `.p12` 密码正确，证书和私钥已导入；当前不是 p12 密码问题。
- `codesign` 能定位 `Codexbar Local Code Signing Leaf v4`，但不能建立到 `Codexbar Local Code Signing Root v4` 的受信任链。
- `find-identity -p codesigning` 仍为 0，因此不能继续 `build.sh` / `install-system.sh`，否则会重新落回拒签/不可用状态。
- 下一步需要二选一：
  1. 从 MacBook 导出并传输 `Codexbar Local Code Signing Root v4` 证书，并在 Mac mini 上通过 GUI/MDM/可授权方式设为 Code Signing 信任；
  2. 允许通过 Keychain Access GUI 设置 `Codexbar Local Code Signing Leaf v4` 或 Root v4 的 Code Signing 信任。

### 2026-07-08 Root v4 缺失复核

复核时间：`2026-07-08 11:45:09 CST`。

追加复核：

```text
security find-identity -v -p codesigning
  0 valid identities found

security find-certificate -a -c "Codexbar Local Code Signing Root v4" login/System keychains
  no result

find /Users/minizl/.inputia /Users/minizl/services/Handy ...
  only /Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12
  no Root v4 .cer/.crt/.pem found

security verify-cert -c /tmp/inputia-leaf-v4.cer -p codeSign
  Cert Verify Result: CSSMERR_TP_NOT_TRUSTED

codesign --force --sign "Codexbar Local Code Signing Leaf v4" --options runtime --timestamp=none /tmp/inputia-codesign-recheck
  Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  errSecInternalComponent
```

结论未变：Mac mini 已有 Leaf p12，但缺 Root v4 或对应 GUI 信任设置；在 `find-identity -p codesigning` 变为有效前，不继续 build/install。

### 2026-07-08 v45 签名导入前置脚本

复核时间：`2026-07-08 11:40:05 CST`。

本轮继续签名导入/系统安装主线，未运行 TextEdit/Safari/Clipboard GUI smoke。

当前只读事实：

```text
stat /Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12
  p12.path=/Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12
  p12.mode=600
  p12.owner=lizhelang
  p12.group=staff
  p12.size=13593

security find-identity -v -p codesigning
  0 valid identities found

codesign -dv --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  Identifier=com.inputia.inputmethod.Inputia
  CDHash=97bd9a98cab276f30b195fe97f92ea247c2da48d
  Signature=adhoc
  TeamIdentifier=not set

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  /Library/Input Methods/InputiaInputMethod.app: rejected

tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appSignatureAccepted=false
  appMatchesBuild=true
  tis.installedMatches=3
  tis.targetInstalledMatches=1
  tis.hansIconMatchesApp=true
  tis.hansEnabled=true
  tis.hansSelectable=true
  tis.hansSelected=false
  tis.currentID=com.tencent.inputmethod.wetype.pinyin
  tis.currentMatchesTarget=false
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false
```

新增脚本：

```text
macos/InputiaInputMethod/import-signing-identity.sh
```

脚本行为：

- 默认读取 `/Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12`。
- 默认导入当前用户 login keychain：`/Users/minizl/Library/Keychains/login.keychain-db`。
- 只有显式设置 `INPUTIA_P12_PASSWORD` 或 `INPUTIA_SIGNING_P12_PASSWORD` 时才运行 `security import -P ...`。
- 缺少 `.p12` 密码时直接退出，不触发 Keychain/PKCS#12 图形交互框。
- 导入后必须再次通过 `security find-identity -v -p codesigning | grep -F "Codexbar Local Code Signing Leaf v4"` 才报告成功。

验证：

```text
chmod 755 macos/InputiaInputMethod/import-signing-identity.sh
zsh -n macos/InputiaInputMethod/import-signing-identity.sh
  rc=0

env -u INPUTIA_P12_PASSWORD -u INPUTIA_SIGNING_P12_PASSWORD -u INPUTIA_KEYCHAIN_PASSWORD \
  macos/InputiaInputMethod/import-signing-identity.sh

  signingIdentityName=Codexbar Local Code Signing Leaf v4
  signingIdentityP12Path=/Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12
  signingIdentityKeychain=/Users/minizl/Library/Keychains/login.keychain-db
  signingIdentityP12Mode=600
  signingIdentityP12Owner=lizhelang
  signingIdentityP12Group=staff
  signingIdentityP12Size=13593
  signingIdentityAlreadyAvailable=false
  signingIdentityImportReady=false reason=missing-p12-password
  signingIdentityRequiredAction=set-INPUTIA_P12_PASSWORD-or-INPUTIA_SIGNING_P12_PASSWORD
  rc=12
```

结论：

- `.p12` 已在 Mac mini 且权限正确，但 signing identity 仍未导入。
- 当前 blocker 仍是 `.p12` 导入密码缺失；不是安装包、TIS 刷新或 GUI smoke。
- 在 identity 出现前，不继续 `build.sh` / `install-system.sh`，也不运行真实 GUI smoke。

### 2026-07-08 v45 p12 导入门禁纳入非 GUI 验证

复核时间：`2026-07-08 11:44:09 CST`。

本轮没有猜测 `.p12` 密码，也没有继续运行真实 `security import`。改动目标是把“缺少 `.p12` 密码时必须非交互失败”纳入 `verify-nongui.sh`，防止以后回退成会挂起的 Keychain/PKCS#12 图形交互。

实现：

- `verify-nongui.sh` 的 `zsh -n` 语法检查加入 `import-signing-identity.sh`。
- `verify-nongui.sh` 的静态契约检查新增：
  - 必须存在 `INPUTIA_P12_PASSWORD` / `INPUTIA_SIGNING_P12_PASSWORD`。
  - 必须输出 `missing-p12-password`。
  - 必须使用 `security import "$P12_PATH" ... -P "$P12_PASSWORD"`。
  - 必须有 `run_with_timeout` 和 `signingIdentityImportTimeoutSeconds`。
  - 必须在导入后用 `security find-identity -v -p codesigning | grep -F "$IDENTITY"` 验证。
  - 不允许使用 `INPUTIA_ALLOW_REJECTED_SIGNATURE` 绕过签名拒绝。
- 新增 `signing identity import self-check`：创建假 `.p12` 和假 keychain 文件，使用不存在的 identity，清空密码环境变量，期望脚本以 rc=12 返回 `missing-p12-password`；不触碰真实 keychain。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/import-signing-identity.sh
  rc=0

fake p12/keychain missing-password check
  signingIdentityName=Inputia Missing Password Self Check Identity
  signingIdentityP12Mode=600
  signingIdentityAlreadyAvailable=false
  signingIdentityImportReady=false reason=missing-p12-password
  signingIdentityRequiredAction=set-INPUTIA_P12_PASSWORD-or-INPUTIA_SIGNING_P12_PASSWORD
  rc=12

extracted verify-nongui static contract
  cleanupPermissionContract=true
  static_contract_rc=0
```

结论：

- p12 导入前置门禁已进入非 GUI 验证覆盖。
- 当前真实系统状态仍未改变：identity 不存在，系统 app 仍 ad-hoc，`spctl` 仍 rejected；继续等待 `.p12` 导入密码或已导入 identity 后再重签安装。

### 2026-07-08 v46 Command 快捷键透传基准与签名链路复核

复核时间：`2026-07-08 12:07:08 CST`。

外部基准：

- Apple 官方 Mac keyboard shortcuts 清单明确把 `Command-C/V/X/Z/A/F/H/M/N/O/P/S/W/Q`、`Option-Command-Escape`、`Shift-Command-3/4/5` 等列为常用系统/App 快捷键。
- Apple 官方 Copy/Paste 文档明确 `Command-C`、`Command-X`、`Command-V`、`Option-Shift-Command-V` 是复制/剪切/粘贴路径。

实现：

- `InputiaShortcutClassifier` 新增 `shouldPassThroughKeyDown(keyCode:modifiers:)`。
- Host `handleKeyDown` 改为用 keyDown 级别 API 判断透传，当前策略仍是：任何含 `Command` 的 keyDown 全部返回 `false`，交还系统/宿主 App。
- 独立 `inputia-shortcut-self-check` 和 Host 内置 `--host-shortcut-self-check` 增加官方常用快捷键 keyCode + modifiers 基准：
  - `Command-C/V/X/Z/Shift-Z/A/F/G/Shift-G/H/Option-H/M/Option-M/N/O/P/S/W/Q`
  - `Command-B/I/U/K/L`
  - `Command-,`、`Command-/`、`Shift-Command-/`
  - `Command-Tab`、`Command-Space`、`Command-\``、`Command-[`、`Command-]`
  - `Command-=`、`Command--`、`Command-Delete`
  - `Control-Command-Q`
  - `Shift-Command-3/4/5`
  - `Option-Command-Escape`
- Host 诊断新增 `--shortcut-self-check` 作为 `--host-shortcut-self-check` 的别名，避免误用短参数时启动 IMK server 长驻进程。
- `verify-nongui.sh` 静态契约新增：
  - 必须存在 keyDown 级别 pass-through 检查。
  - 必须存在官方常用 Command keyDown 基准输出。
  - 必须覆盖 `Command-C/V/X/Z/A/F/S/P/Q/Tab/Space/Option-Escape` 等代表项。

验证：

```text
bash -n macos/InputiaInputMethod/verify-nongui.sh
zsh -n macos/InputiaInputMethod/build.sh
zsh -n macos/InputiaInputMethod/import-signing-identity.sh
  rc=0

extracted verify-nongui static contract
  cleanupPermissionContract=true
  static_contract_rc=0

swiftc ... InputiaShortcutSelfCheck.swift InputiaShortcutClassifier.swift
/tmp/inputia-shortcut-self-check
  shortcutSelfCheck=true
  officialAppleCommandKeyDownSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  appleCommandCKeyDownPassThrough=true
  appleCommandVKeyDownPassThrough=true
  appleCommandXKeyDownPassThrough=true
  appleCommandZKeyDownPassThrough=true
  appleCommandAKeyDownPassThrough=true
  appleCommandFKeyDownPassThrough=true
  appleCommandPKeyDownPassThrough=true
  appleCommandSKeyDownPassThrough=true
  appleCommandQKeyDownPassThrough=true
  appleCommandTabKeyDownPassThrough=true
  appleCommandSpaceKeyDownPassThrough=true
  appleCommandOptionEscapeKeyDownPassThrough=true

swiftc ... Host sources ... -typecheck
  rc=0

./macos/InputiaInputMethod/build.sh
  build_rc=0
  只构建本地 build，未安装系统包；仍有既有 ld warning：
  Rust staticlib object was built for newer macOS version (26.0) than being linked (13.0)

build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  officialAppleCommandKeyDownSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  appleCommandCKeyDownPassThrough=true
  appleCommandVKeyDownPassThrough=true
  appleCommandOptionEscapeKeyDownPassThrough=true

build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --shortcut-self-check
  hostShortcutSelfCheck=true
  officialAppleCommandKeyDownSetPassesThrough=true
```

签名链路新状态：

```text
macos/InputiaInputMethod/import-signing-identity.sh
  signingIdentityAlreadyAvailable=true
  signingIdentityImportVerified=true

security find-identity -v -p codesigning
  1) 12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"
  1 valid identities found

security find-certificate root
  Codexbar Local Code Signing Root v4
  System.keychain
  SHA-1 66003E15593FF649FE1A9E560A65177277BF0722

security verify-cert -c /tmp/inputia-leaf.pem -p codeSign
  certificate verification successful

security set-key-partition-list -S apple-tool:,apple:,codesign:
  rc=0

最小 codesign spike
  Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  /tmp/inputia-codesign-spike: errSecInternalComponent
  codesign_spike_after_policy_rewrite_rc=1
```

本轮还修正 `build.sh`：

- 默认 ad-hoc (`SIGN_IDENTITY=-`) 构建仍允许用于本地非安装验证。
- 一旦显式指定真实 `INPUTIA_CODESIGN_IDENTITY`，codesign 失败必须退出非零。

验证：

```text
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
INPUTIA_CODESIGN_OPTIONS="--options runtime" \
./macos/InputiaInputMethod/build.sh

  warning: codesign failed with identity 'Codexbar Local Code Signing Leaf v4'
  buildSigned=false reason=codesign-failed target=input-method identity=Codexbar Local Code Signing Leaf v4
  signed_build_fail_gate_rc=31
```

结论：

- 用户反馈的 `Command-C/V` 不能被输入法接管这一类问题，代码策略和自检已扩展成“任意含 Command 的 keyDown 透传”并覆盖 Apple 官方常用快捷键集合，不再只靠 C/V 单点。
- Mac mini 签名状态从“identity 不存在”推进到“identity 存在、root trust/verify-cert 通过、partition-list 已设置”，但最小 `codesign` 仍失败为 `errSecInternalComponent`。
- 因 `codesign` spike 未通过，仍不能继续 `install-system.sh`，也不能运行真实 TextEdit/Safari/Clipboard GUI smoke。

## v45 Mac mini root-signing build and Gatekeeper blocker

背景：

- 用户补充 MacBook 导出的 Root/CA 已传到 Mac mini：
  `/Users/minizl/.inputia/signing/Codexbar-Local-Code-Signing-Root-v4.cer`。
- Leaf p12 已导入 login keychain，`security find-identity -v -p codesigning` 能看到：
  `12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"`。

Root/Leaf 验证：

```text
openssl x509 -in Codexbar-Local-Code-Signing-Root-v4.cer -inform DER -noout -subject -issuer -fingerprint -sha1
  subject=CN=Codexbar Local Code Signing Root v4, O=Local, OU=Codexbar
  issuer=CN=Codexbar Local Code Signing Root v4, O=Local, OU=Codexbar
  sha1 Fingerprint=66:00:3E:15:59:3F:F6:49:FE:1A:9E:56:0A:65:17:72:77:BF:07:22

openssl verify -CAfile /tmp/inputia-root-v4.pem /tmp/inputia-p12-cert-05.pem
  /tmp/inputia-p12-cert-05.pem: OK

security verify-cert -v -c /tmp/inputia-p12-cert-05.pem -r Codexbar-Local-Code-Signing-Root-v4.cer -p codeSign
  certificate verification successful
  Certificate chain:
    0: Codexbar Local Code Signing Leaf v4
    1: Codexbar Local Code Signing Root v4
```

关键诊断：

```text
codesign as lizhelang:
  Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  errSecInternalComponent

codesign as root with explicit login keychain:
  signed generic
  Authority=Codexbar Local Code Signing Leaf v4
  Authority=Codexbar Local Code Signing Root v4
```

结论：

- Leaf/Root 证书本身可组成链，Root 信任也可用于 `security verify-cert`。
- `lizhelang` 用户域里仍有 leaf `TrustAsRoot` override；Apple DTS 对 `unable to build chain to self-signed root` 的建议是避免链上证书存在自定义 trust settings。当前非交互环境无法移除该用户级 override：
  - `security remove-trusted-cert` -> `authorization was denied since no user interaction was possible`
  - `osascript ... with administrator privileges` 作为 root 运行时找不到用户域 trust item
  - `authorizationdb` 临时 allow 尝试被系统拒绝
- 为继续验证构建/安装链路，`build.sh` 增加显式 opt-in：
  `INPUTIA_CODESIGN_AS_ROOT=1` + `INPUTIA_CODESIGN_KEYCHAIN=/Users/minizl/Library/Keychains/login.keychain-db`。
  若提供 `INPUTIA_SUDO_PASSWORD`，脚本通过 `sudo -S -p ''` 调用 root `codesign`。默认 ad-hoc/普通签名路径不变。
- root `codesign` 会生成 root-owned `Contents/_CodeSignature/CodeResources`；`build.sh` 已在 root signing 后把 `_CodeSignature` 修回当前构建用户并设为可读，避免 `invalid resource directory`。
- `install-system.sh` 增加 `INPUTIA_SUDO_PASSWORD` 管理员复制分支，避免非交互 `osascript` 管理员弹窗失败。

构建验证：

```text
INPUTIA_SUDO_PASSWORD=... \
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
INPUTIA_CODESIGN_OPTIONS="--options runtime" \
INPUTIA_CODESIGN_AS_ROOT=1 \
INPUTIA_CODESIGN_KEYCHAIN="/Users/minizl/Library/Keychains/login.keychain-db" \
./macos/InputiaInputMethod/build.sh
  buildRc=0
  InputiaInputMethod.app: valid on disk
  InputiaInputMethod.app: satisfies its Designated Requirement
  Inputia 设置.app: valid on disk
  Inputia 设置.app: satisfies its Designated Requirement

codesign -dv --verbose=4 build/InputiaInputMethod.app
  CDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  Authority=Codexbar Local Code Signing Leaf v4
  Authority=Codexbar Local Code Signing Root v4
  Runtime Version=26.0.0
  Sealed Resources version=2 rules=13 files=29

spctl --assess --type execute --verbose=4 build/InputiaInputMethod.app
  rejected
```

系统安装验证：

```text
INPUTIA_SUDO_PASSWORD=... \
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
INPUTIA_CODESIGN_OPTIONS="--options runtime" \
INPUTIA_CODESIGN_AS_ROOT=1 \
INPUTIA_CODESIGN_KEYCHAIN="/Users/minizl/Library/Keychains/login.keychain-db" \
./macos/InputiaInputMethod/install-system.sh
  installSystemRc=14
  sourceVersion=44
  sourceCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  userHostRemoved=true path=/Users/minizl/Library/Input Methods/InputiaInputMethod.app
  systemInstallNeedsAdmin=true
  destVersion=44
  destCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  systemInstallVerified=true
  legacyIputiaRemoved=true
  settingsLauncherInstalled=true path=/Applications/Inputia 设置.app
  systemInstallAssessment: /Library/Input Methods/InputiaInputMethod.app: rejected
  systemInstallInputiaUsable=false reason=signature-rejected
  systemInstallRequiredAction=sign-with-accepted-identity
  systemInstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts
  systemInstallAction=clear-inputia-preferences
```

安装后状态：

```text
./macos/InputiaInputMethod/status.sh
  systemMatchesBuild=true
  legacyIputiaPresent=false
  systemSettingsMatchesBuildVersion=true
  statusSignatureAccepted=false
  statusSigningRequiredAction=sign-with-accepted-identity
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing

./macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  appAssessment=/Library/Input Methods/InputiaInputMethod.app: rejected
  appSignatureAccepted=false
  appMatchesBuild=true
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false

codesign --verify --deep --strict --verbose=2 "/Library/Input Methods/InputiaInputMethod.app"
  valid on disk
  satisfies its Designated Requirement

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  rejected

syspolicy_check distribution "/Library/Input Methods/InputiaInputMethod.app"
  Notary Ticket Missing
```

结论：

- Mac mini 当前已经能构建并复制系统目录里的完整 signed bundle；`codesign --verify` 通过，CDHash 与 build 一致。
- `spctl`/`syspolicy_check` 的 blocker 已从“不能签名/不能构建证书链”推进到“本地 CA 签名缺少 notarization ticket”。在 macOS 当前策略下，本地 Codexbar CA 即使被 trust，也不能满足 `spctl --assess --type execute` 的 Gatekeeper 分发评估。
- 由于 `spctl accepted` 仍未达成，继续遵守 GUI smoke 纪律：未运行 TextEdit/Safari/Clipboard smoke；当前 `status.sh` 也明确 `guiSmokeReady=false`。
- 下一步需要一个能被当前 macOS `spctl` 接受的签名/公证路径，例如有效 Apple Developer/Developer ID 签名身份及 notarization ticket，或重新定义本地系统级 IMK 验证是否可以在非 `spctl` accepted 条件下安全推进。

### 2026-07-08 v47 codesign probe 门禁与本地身份对照

复核时间：`2026-07-08 12:18:43 CST`。

背景：

- v46 后 `security find-identity` 能看到 `Codexbar Local Code Signing Leaf v4`，但最小 `codesign` 仍失败。
- 单看 `find-identity` 会误判签名身份可用，因此本轮把“能否实际签一个临时可执行文件”变成导入脚本门禁。

实现：

- `import-signing-identity.sh` 新增 `codesign_probe()`。
- probe 行为：
  - 创建临时可执行文件。
  - 使用当前 `IDENTITY` 执行：

```text
codesign --force --sign "$IDENTITY" --options runtime --timestamp=none <temp-executable>
```

  - 成功才输出 `signingIdentityCodesignProbe=true` 和 `signingIdentityImportVerified=true`。
  - 失败输出 `signingIdentityCodesignProbe=false`、`signingIdentityImportVerified=false reason=codesign-probe-failed`，并以 rc=17 退出。
- `verify-nongui.sh` 静态契约新增：
  - 必须存在 `codesign_probe()`。
  - 必须输出 probe 成功/失败标记。
  - 必须使用 `--timestamp=none`，避免本地验证依赖网络 timestamp 服务。

验证：

```text
zsh -n import-signing-identity.sh
bash -n verify-nongui.sh
  rc=0

extracted verify-nongui static contract
  cleanupPermissionContract=true
  static_contract_rc=0

macos/InputiaInputMethod/import-signing-identity.sh
  signingIdentityAlreadyAvailable=true
  1) 12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"
  signingIdentityCodesignProbeOutput: Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  signingIdentityCodesignProbeOutput: ... errSecInternalComponent
  signingIdentityCodesignProbe=false rc=1
  signingIdentityImportVerified=false reason=codesign-probe-failed
  signingIdentityRequiredAction=fix-keychain-trust-or-private-key-acl
  rc=17
```

对照 spike：

```text
临时 keychain + 临时 root/leaf + legacy p12 import
  1 identity imported.
  1 certificate imported.
  codesign --force --keychain <temp> --sign "Inputia Spike Leaf" --options runtime --timestamp=none <temp-executable>
  spike_codesign_rc=0
  Authority=Inputia Spike Leaf
  Authority=Inputia Spike Root
```

结论：

- 这台 Mac mini 的 `codesign` API 和本地 CA 方式本身可用。
- 当前失败集中在 `Codexbar Local Code Signing Leaf v4` 这套身份/私钥/信任组合，而不是 Inputia app bundle、build 脚本或 TIS。

本地 fallback 清理：

- 曾尝试生成 `Inputia Mac mini Local Code Signing Root/Leaf v1` 作为 fallback。
- 该 fallback 因 root trust 写入未完成，`security verify-cert` 返回 `CSSMERR_TP_NOT_TRUSTED`，未继续使用。
- 已删除 fallback leaf/root 证书、System/login keychain 中的 fallback 证书，以及 `/Users/minizl/.inputia/signing/Inputia-Mac-mini-Local-Code-Signing-*` 文件。
- 为减少混淆，也删除了 login keychain 中的 Codexbar root 副本；当前只保留：

```text
Codexbar leaf/private key:
  login.keychain-db

Codexbar root:
  /Library/Keychains/System.keychain

/Users/minizl/.inputia/signing:
  Codexbar-Local-Code-Signing-Leaf-v4.p12
  Codexbar-Local-Code-Signing-Root-v4.cer
```

当前状态：

```text
security find-identity -v -p codesigning
  1) 12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"
  1 valid identities found

import-signing-identity.sh
  signingIdentityImportVerified=false reason=codesign-probe-failed
  rc=17
```

下一步不是 TIS/安装包/GUI smoke，而是修复 Codexbar identity 的实际 `codesign` 能力，或换用一套已验证能通过最小 `codesign` probe 且被 `spctl` 接受的签名身份。

## v46 Mac mini final signing/install state after Root v4 transfer

本轮结论日期：2026-07-08。

关键结果：

```text
build.sh with root codesign opt-in
  INPUTIA_SUDO_PASSWORD=...
  INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4"
  INPUTIA_CODESIGN_OPTIONS="--options runtime"
  INPUTIA_CODESIGN_AS_ROOT=1
  INPUTIA_CODESIGN_KEYCHAIN=/Users/minizl/Library/Keychains/login.keychain-db
  buildRc=0
  InputiaInputMethod.app: valid on disk
  InputiaInputMethod.app: satisfies its Designated Requirement
  Inputia 设置.app: valid on disk
  Inputia 设置.app: satisfies its Designated Requirement

install-system.sh with same env
  installSystemRc=14
  sourceVersion=44
  sourceCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  destVersion=44
  destCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  systemInstallVerified=true
  legacyIputiaRemoved=true
  settingsLauncherInstalled=true path=/Applications/Inputia 设置.app
  systemInstallAssessment: /Library/Input Methods/InputiaInputMethod.app: rejected
  systemInstallInputiaUsable=false reason=signature-rejected
  systemInstallRequiredAction=sign-with-accepted-identity
```

安装后状态：

```text
codesign --verify --deep --strict --verbose=2 /Library/Input Methods/InputiaInputMethod.app
  valid on disk
  satisfies its Designated Requirement

codesign -dv --verbose=4 /Library/Input Methods/InputiaInputMethod.app
  CDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  Authority=Codexbar Local Code Signing Leaf v4
  Authority=Codexbar Local Code Signing Root v4
  Runtime Version=26.0.0
  Sealed Resources version=2 rules=13 files=29

spctl --assess --type execute --verbose=4 /Library/Input Methods/InputiaInputMethod.app
  rejected

syspolicy_check distribution /Library/Input Methods/InputiaInputMethod.app
  Notary Ticket Missing

status.sh
  systemMatchesBuild=true
  legacyIputiaPresent=false
  systemSettingsMatchesBuildVersion=true
  statusSignatureAccepted=false
  statusSigningRequiredAction=sign-with-accepted-identity
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing

tis-readiness.sh /Library/Input Methods/InputiaInputMethod.app
  appMatchesBuild=true
  appSignatureAccepted=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false
```

本轮脚本修复：

- `build.sh` 新增 `INPUTIA_CODESIGN_AS_ROOT=1`，用于绕开当前 `lizhelang` 用户域 leaf trust override 导致的 `codesign errSecInternalComponent`。默认签名路径不变。
- `build.sh` 在 root signing 后修复 `_CodeSignature` 权限，避免 root 写出的 `CodeResources` 导致普通用户验证时报 `invalid resource directory`。
- `install-system.sh` 新增 `INPUTIA_SUDO_PASSWORD` 非交互管理员复制分支，避免 `osascript with administrator privileges` 在当前环境失败。

当前 blocker：

- Mac mini 已经能生成完整 Codexbar Leaf/Root signed bundle，并能复制到 `/Library/Input Methods`，但 macOS Gatekeeper assessment 仍拒绝：`Notary Ticket Missing`。
- 因 `spctl accepted` 未达成，按 GUI smoke 纪律没有运行 TextEdit/Safari/Clipboard smoke；进程检查未发现 TextEdit.app、Safari.app、InputiaInputMethod.app 或 IputiaInputMethod.app 残留。
- 下一步需要能被当前 macOS `spctl` 接受的签名/公证路径，例如有效 Developer ID Application identity 加 notarization ticket；仅本地 CA trust 不足以通过当前 `spctl --assess --type execute`。

## v47 Notarization readiness diagnostic

背景：

- v46 已证明系统目录 app 是完整 Codexbar Leaf/Root signed bundle，`codesign --verify` 通过，但 `spctl` 仍 rejected，`syspolicy_check distribution` 报 `Notary Ticket Missing`。
- 为避免后续从“菜单栏没有 Inputia / System Settings 添加后仍不能用”倒推，新增只读诊断脚本：
  `macos/InputiaInputMethod/notarization-readiness.sh`。

脚本覆盖：

- 当前 app 是否存在。
- `codesign --verify --deep --strict`。
- CDHash、TeamIdentifier、hardened runtime、Authority 链。
- `spctl --assess --type execute --verbose=4`。
- `syspolicy_check distribution` 是否报 `Notary Ticket Missing`。
- Keychain 中是否存在 `Developer ID Application:` 和 `Developer ID Installer:` identity。
- `xcrun notarytool` / `xcrun stapler` 是否可用。
- `INPUTIA_NOTARY_PROFILE` 指定的 notarytool keychain profile 是否可用，默认 `Inputia`。

当前输出：

```text
./macos/InputiaInputMethod/notarization-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  notarizationReadinessTool=true
  appExists=true
  codesignVerify=true
  appCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  teamIdentifier=not set
  hardenedRuntime=true
  codesignAuthority=Codexbar Local Code Signing Leaf v4
  codesignAuthority=Codexbar Local Code Signing Root v4
  spctlAccepted=false
  spctlAssessment: /Library/Input Methods/InputiaInputMethod.app: rejected
  syspolicyCheckAvailable=true
  syspolicyNotaryTicketMissing=true
  developerIDApplicationIdentityCount=0
  developerIDApplicationIdentityPresent=false
  developerIDInstallerIdentityCount=0
  developerIDInstallerIdentityPresent=false
  notarytoolAvailable=true
  notarytoolPath=/Library/Developer/CommandLineTools/usr/bin/notarytool
  staplerAvailable=true
  staplerPath=/Library/Developer/CommandLineTools/usr/bin/stapler
  notaryProfile=Inputia
  notaryProfileAvailable=false
  inputiaGatekeeperReady=false
  inputiaGatekeeperBlockReasons=spctl-rejected,notary-ticket-missing
  inputiaNotarySubmissionReady=false
  inputiaNotarySubmissionBlockReasons=missing-developer-id-application,missing-notarytool-profile
  notarizationReadinessBlockReasons=spctl-rejected,notary-ticket-missing,missing-developer-id-application,missing-notarytool-profile
  notarizationRequiredAction=import-developer-id-application-identity
```

验证：

```text
zsh -n macos/InputiaInputMethod/notarization-readiness.sh
zsh -n macos/InputiaInputMethod/build.sh macos/InputiaInputMethod/install-system.sh macos/InputiaInputMethod/notarization-readiness.sh macos/InputiaInputMethod/verify-nongui.sh
bash -n macos/InputiaInputMethod/verify-nongui.sh
  rc=0
```

文档和回归：

- `README.md` 的系统安装诊断段落加入 `notarization-readiness.sh`。
- `verify-nongui.sh` 的 syntax gate 加入 `notarization-readiness.sh`。

结论：

- Mac mini 当前缺少 `Developer ID Application` identity，也没有 `Inputia` notarytool profile；只有 `notarytool` / `stapler` 工具本身可用。
- 当前 blocker 是 Apple 分发链凭据/公证，不是 Inputia bundle 结构、系统目录复制、旧 typo 清理、TIS 脚本或 GUI smoke 清理纪律。
- 在 `spctlAccepted=true` 前，继续不运行 TextEdit/Safari/Clipboard GUI smoke。

## v48 Mac mini signing identity import recheck

时间：2026-07-08 12:35:05 CST

背景：

- 用户已从 MacBook 传输 `Codexbar Local Code Signing Leaf v4` 的 `.p12` 到 Mac mini。
- 本轮只走签名导入 / 系统安装门禁验证主线，不运行 TextEdit/Safari/Clipboard GUI smoke。

只读事实：

```text
git status --short
  dirty worktree preserved; macos/ remains untracked and existing unrelated changes were not reset.

ls -l /Users/minizl/.inputia/signing
  -rw-------  1 lizhelang  staff  13593 Jul  8 11:29 Codexbar-Local-Code-Signing-Leaf-v4.p12
  -rw-r--r--  1 lizhelang  staff    919 Jul  8 11:53 Codexbar-Local-Code-Signing-Root-v4.cer

stat Codexbar-Local-Code-Signing-Leaf-v4.p12
  p12_mode=600 owner=lizhelang group=staff size=13593

security find-identity -v -p codesigning | grep -F "Codexbar Local Code Signing Leaf v4"
  12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"
  identity_grep_rc=0
```

当前用户签名探针：

```text
./macos/InputiaInputMethod/import-signing-identity.sh
  signingIdentityAlreadyAvailable=true
  signingIdentityCodesignProbeOutput: Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  signingIdentityCodesignProbeOutput: .../inputia-signing-probe-bin...: errSecInternalComponent
  signingIdentityCodesignProbe=false rc=1
  signingIdentityImportVerified=false reason=codesign-probe-failed
  signingIdentityRequiredAction=fix-keychain-trust-or-private-key-acl
  rc=17

manual codesign probe
  codesign --force --sign "Codexbar Local Code Signing Leaf v4" --options runtime --timestamp=none probe
  Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  probe: errSecInternalComponent
  manual_codesign_rc=1
```

证书链补充证据：

```text
openssl x509 -in Codexbar-Local-Code-Signing-Root-v4.cer ... -ext basicConstraints -ext keyUsage -ext extendedKeyUsage
  No extensions in certificate

security dump-trust-settings
  Cert 2: Codexbar Local Code Signing Leaf v4
    Number of trust settings : 9
    Code Signing Result Type: kSecTrustSettingsResultTrustAsRoot
  Cert 3: Codexbar Local Code Signing Root v4
    Number of trust settings : 0

security dump-trust-settings -d
  Cert 0: Codexbar Local Code Signing Root v4
    Number of trust settings : 1
    Policy OID: Code Signing
```

尝试排除项：

```text
security import Codexbar-Local-Code-Signing-Root-v4.cer -k login.keychain-db
  1 certificate imported.

manual codesign probe after login root import
  Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  probe: errSecInternalComponent
  codesign_after_login_root_rc=1

security delete-certificate -Z 66003E15593FF649FE1A9E560A65177277BF0722 login.keychain-db
  login root test import removed.
```

构建 / 安装门禁：

```text
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
INPUTIA_CODESIGN_OPTIONS="--options runtime" \
./macos/InputiaInputMethod/build.sh
  codesignOutput: Warning: unable to build chain to self-signed root for signer "Codexbar Local Code Signing Leaf v4"
  codesignOutput: .../build/InputiaInputMethod.app: errSecInternalComponent
  buildSigned=false reason=codesign-failed target=input-method identity=Codexbar Local Code Signing Leaf v4
  signed_build_rc=31
```

因为签名构建没有通过，本轮没有继续执行 `install-system.sh`。当前系统目录里仍是上一轮 root-signing 安装出的 Codexbar signed app：

```text
codesign -dv --verbose=4 /Library/Input Methods/InputiaInputMethod.app
  Identifier=com.inputia.inputmethod.Inputia
  CDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  Authority=Codexbar Local Code Signing Leaf v4
  Authority=Codexbar Local Code Signing Root v4
  Signed Time=Jul 8, 2026 at 12:18:08
  TeamIdentifier=not set
  Runtime Version=26.0.0

spctl --assess --type execute --verbose=4 /Library/Input Methods/InputiaInputMethod.app
  rejected
  system_spctl_rc=3

./macos/InputiaInputMethod/tis-readiness.sh /Library/Input\ Methods/InputiaInputMethod.app
  appSignatureAccepted=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false

./macos/InputiaInputMethod/notarization-readiness.sh /Library/Input\ Methods/InputiaInputMethod.app
  spctlAccepted=false
  syspolicyNotaryTicketMissing=true
  developerIDApplicationIdentityPresent=false
  developerIDInstallerIdentityPresent=false
  notaryProfileAvailable=false
  inputiaGatekeeperReady=false
  inputiaGatekeeperBlockReasons=spctl-rejected,notary-ticket-missing
  inputiaNotarySubmissionReady=false
  inputiaNotarySubmissionBlockReasons=missing-developer-id-application,missing-notarytool-profile
  notarizationRequiredAction=import-developer-id-application-identity
```

结论：

- `.p12` 已到位，`Codexbar Local Code Signing Leaf v4` identity 已在 Mac mini 可见；这一步不再是“文件没传 / identity 没导入”。
- 但当前用户直接 `codesign` 仍失败，错误是证书链构建到 self-signed root 失败并返回 `errSecInternalComponent`。额外证据显示当前 Root 证书没有 CA / KeyUsage 扩展，用户 trust settings 又错误地把 Leaf 设为 `TrustAsRoot`。
- 通过 root-signing 生成并安装的系统 app 仍被 `spctl` 拒绝；当前 `tis-readiness` blocker 仍是 `signature-rejected`。
- 按门禁纪律，`spctlAccepted=false` 前继续不运行 TextEdit/Safari/Clipboard GUI smoke。

## v49 Mac mini trusted-root TIS override spike

时间：2026-07-08 12:38:02 CST

背景：

- 用户反馈已在 Mac mini 上信任 Root/CA 证书后，继续验证系统安装能否切到 Inputia。
- 本轮仍不运行 TextEdit/Safari/Clipboard GUI smoke；只做签名、Gatekeeper、TIS、菜单 readiness 验证。

信任后复查：

```text
./macos/InputiaInputMethod/status.sh
  systemMatchesBuild=true
  legacyIputiaPresent=false
  assessment=/Library/Input Methods/InputiaInputMethod.app: rejected
  statusSignatureAccepted=false
  statusSigningRequiredAction=sign-with-accepted-identity
  statusTISEnabledMatches=0
  statusTISInstalledMatches=0
  statusMenuBlockReason=inputia-menu-item-missing
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing

./macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  appSignatureAccepted=false
  tis.enabledMatches=0
  tis.installedMatches=0
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  /Library/Input Methods/InputiaInputMethod.app: rejected
  spctlRc=3

./macos/InputiaInputMethod/notarization-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  codesignVerify=true
  spctlAccepted=false
  syspolicyNotaryTicketMissing=true
  developerIDApplicationIdentityPresent=false
  notaryProfileAvailable=false
  inputiaGatekeeperReady=false
  inputiaGatekeeperBlockReasons=spctl-rejected,notary-ticket-missing
  inputiaNotarySubmissionReady=false
  inputiaNotarySubmissionBlockReasons=missing-developer-id-application,missing-notarytool-profile
  notarizationRequiredAction=import-developer-id-application-identity
```

本机 Gatekeeper 例外 spike：

```text
spctl --add --label "Inputia Local Development" "/Library/Input Methods/InputiaInputMethod.app"
  This operation is no longer supported. Please see the man page for more information.
  spctlAddRc=4

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  /Library/Input Methods/InputiaInputMethod.app: rejected
  spctlAssessRc=3
```

安装脚本 gate 越过 spike：

```text
INPUTIA_ALLOW_REJECTED_SIGNATURE=1 ./macos/InputiaInputMethod/install-system.sh
  systemInstallVerified=true
  legacyIputiaRemoved=true
  settingsLauncherInstalled=true path=/Applications/Inputia 设置.app
  systemInstallAssessment: /Library/Input Methods/InputiaInputMethod.app: rejected
  registerStatus=0
  id=com.inputia.inputmethod.Inputia.Hans
  bundle=com.inputia.inputmethod.Inputia
  mode=com.inputia.inputmethod.Inputia.Hans
  name=Inputia
  languages=zh-Hans
  enabled=true
  enableCapable=true
  selectable=true
  selected=true
  selectStatus=-50
  systemInstallTISStableCheck attempt=1 ready=false
  ...
  systemInstallTISStableCheck attempt=6 ready=false
  systemInstallPostRefreshTIS: tis.enabledMatches=0
  systemInstallPostRefreshTIS: tis.installedMatches=3
  systemInstallPostRefreshTIS: tis.targetEnabledMatches=0
  systemInstallPostRefreshTIS: tis.targetInstalledMatches=1
  systemInstallPostRefreshTIS: tis.hansIconMatchesApp=true
  systemInstallPostRefreshTIS: tis.hansEnabled=true
  systemInstallPostRefreshTIS: tis.hansSelectable=true
  systemInstallPostRefreshTIS: tis.hansSelected=true
  systemInstallPostRefreshTIS: tis.currentID=com.apple.keylayout.ABC
  systemInstallPostRefreshTIS: tis.currentMatchesTarget=false
  systemInstallPostRefreshTIS: tis.readinessBlockReason=signature-rejected
  systemInstallPostRefreshTIS: tisReadiness=false
  systemInstallPostRefreshTISReady=false
```

越过 gate 后复查：

```text
./macos/InputiaInputMethod/status.sh
  includeAllInstalled=true
  matches=3
  id=com.inputia.inputmethod.Inputia
    enabled=false selectable=false selected=false
  id=com.inputia.inputmethod.Inputia.Hant
    enabled=false selectable=true selected=false
  id=com.inputia.inputmethod.Inputia.Hans
    enabled=true selectable=true selected=true
  menuInputiaCount=0
  menuReadiness=false
  menuReadinessBlockReason=inputia-menu-item-missing
  running=false
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing

./macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  tis.hansEnabled=true
  tis.hansSelectable=true
  tis.hansSelected=true
  tis.currentID=com.apple.keylayout.ABC
  tis.currentMatchesTarget=false
  tis.readinessBlockReason=signature-rejected
  tisReadiness=false

"/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod" --dump-input-source
  id=com.inputia.inputmethod.Inputia.Hans
  enabled=true
  selectable=true
  selected=true
```

脚本修正：

```text
install-system.sh
  修正 INPUTIA_ALLOW_REJECTED_SIGNATURE=1 时的输出：
    systemInstallSignatureAccepted=false
    systemInstallSignatureOverride=true reason=signature-rejected
  不再把本地 override spike 误报成 systemInstallSignatureAccepted=true。

zsh -n macos/InputiaInputMethod/install-system.sh
  rc=0
```

结论：

- 用户侧“信任证书”后，`codesign --verify` 层面仍可通过，但 `spctl` / `syspolicy_check` 仍拒绝系统安装 app；当前明确 blocker 是 `spctl-rejected,notary-ticket-missing`。
- 越过我们脚本里的 `spctl` gate 后，TIS 可以看到并部分标记 Inputia Hans，但菜单栏仍没有 Inputia，当前输入源仍是 ABC，host 进程未运行；这排除了“只是 install-system.sh 没注册输入源”的单一原因。
- 当前 Mac mini 缺少可公证分发链：`Developer ID Application` identity 和 `notarytool` profile 都不存在。继续正式修复应切到 Developer ID 签名 + notarize/staple，或者明确接受只能作为非 Gatekeeper-ready 的本地开发状态。
- `spctlAccepted=false` 前继续不运行 TextEdit/Safari/Clipboard GUI smoke。

清理：

```text
"/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod" --clear-input-source-preferences
  inputSourcePreferencesClear=true
  hitoolboxEnabledBefore=5
  hitoolboxEnabledAfter=3
  hitoolboxSelectedBefore=1
  hitoolboxSelectedAfter=0
  hitoolboxHistoryBefore=3
  hitoolboxHistoryAfter=2

./macos/InputiaInputMethod/status.sh
  tis.installedMatches=3
  id=com.inputia.inputmethod.Inputia.Hans
    enabled=true selectable=true selected=false
  menuInputiaCount=0
  running=false
  statusGuiSmokeReady=false reason=tis-not-ready,signature-rejected,menu-inputia-menu-item-missing
```

## 2026-07-08 v49 - 对照成熟 macOS 输入法项目并确认当前安装 blocker

用户反馈：

- 系统设置中曾出现多个 `Inputia` / `Inputia 简体` 条目，菜单栏只能看到部分条目，选择后反复弹 `TextInputMenuAgent` 允许启用弹窗但无法真正切换。
- 要求参考 GitHub 上成熟 macOS 输入法项目，不要继续凭空试脚本。

对照来源：

- Apple Developer ID: https://developer.apple.com/developer-id/
  - Apple 说明 Gatekeeper 会检查 Developer ID certificate；分发 app / plug-in / installer package 需要签名，notarization ticket 会让 Gatekeeper 知道软件已经公证。
- Rime Squirrel: https://github.com/rime/squirrel
  - `resources/Info.plist` 使用 `ComponentInputModeDict` / `TISInputSourceID` / `InputMethodConnectionName`。
  - `package/sign_app` 使用 `Developer ID Application`、`--options runtime`、`--timestamp`，并立即跑 `spctl -a -vv`。
  - README 说明初次安装后如部分应用无法输入，需要注销并重新登录。
- ToyIMK: https://github.com/eagleoflqj/toyimk
  - 安装到 `/Library/Input Methods`；首次安装后需要 logout/login，再到系统设置添加输入源。
- IMKit sample: https://github.com/ensan-hcl/macOS_IMKitSample_2021
  - `InputMethodConnectionName = $(PRODUCT_BUNDLE_IDENTIFIER)_Connection`，并要求 bundle identifier 包含 `.inputmethod.`。
- vChewing IMKSwift: https://github.com/vChewing/IMKSwift
  - 作为现代 Swift IMK 封装参考；继续实现前应注意 IMKInputController 与 Swift concurrency 的边界。

实现选择：

- 系统层只保留一个可见输入模式：`com.inputia.inputmethod.Inputia.Main`，菜单名为 `Inputia`。
- 不再把“简体/繁体”做成两个系统输入源；简繁应作为 Inputia 偏好设置里的状态切换，与 Shift 中英文切换同级。
- `Info.plist` 删除旧 `.Hans` / `.Hant` 模式，保留 `Hans/Hant/Latn` repertoire。
- TIS 工具、安装脚本、pkg postinstall、readiness/status/smoke 入口统一期待 `.Main`。
- Rime 层新增简繁输出选项映射；`luna_pinyin_simp` 的繁体输出切回 `luna_pinyin + zh_hant=true`，避免 schema 名仍是简体但用户选择繁体。
- `InputiaInputTextRouterSelfCheck` 增加简体/繁体提交验证，作为 GUI smoke 前的无 GUI 防线。

验证：

```text
cargo test --manifest-path crates/inputia-capi/Cargo.toml
  21 passed; 0 failed

plutil -lint macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/SettingsLauncher/Info.plist
  macos/InputiaInputMethod/Info.plist: OK
  macos/InputiaInputMethod/SettingsLauncher/Info.plist: OK

rg -n "Inputia\\.Hans|Inputia\\.Hant|\\.Hans|\\.Hant" macos/InputiaInputMethod/Info.plist macos/InputiaInputMethod/Sources macos/InputiaInputMethod/Tools macos/InputiaInputMethod/*.sh macos/InputiaInputMethod/Packaging/scripts macos/InputiaInputMethod/Resources -g '*'
  no matches

./macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  simplifiedScriptCommit=中国
  simplifiedScriptCommitsSimplified=true
  traditionalScriptCommit=中國
  traditionalScriptCommitsTraditional=true
```

当前 Mac mini 系统安装状态：

```text
security find-identity -v -p codesigning | grep -F "Codexbar Local Code Signing Leaf v4"
  12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"

codesign -dv --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  Authority=Codexbar Local Code Signing Leaf v4
  Authority=Codexbar Local Code Signing Root v4
  TeamIdentifier=not set
  CDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  /Library/Input Methods/InputiaInputMethod.app: rejected

./macos/InputiaInputMethod/tis-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  buildCDHash=a40828c3e019d5bdd8231dbaef15ff1a060a0f9f
  appCDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  appAssessment=/Library/Input Methods/InputiaInputMethod.app: rejected
  appSignatureAccepted=false
  appMatchesBuild=false
  expectedTISModeID=com.inputia.inputmethod.Inputia.Main
  tis.targetInstalledMatches=0
  tis.readinessBlockReason=signature-rejected
  tis.requiredAction=sign-with-accepted-identity
  tisReadiness=false
```

结论：

- 当前没有成功安装/切换不是“没有参考成熟项目”，也不是单纯“缺 pkg”；成熟项目的共同要求仍然是：正确 Info.plist + 安装到 `/Library/Input Methods` + 注册/启用/选择 + 系统可接受的签名。
- Mac mini 现在的关键 blocker 是 Gatekeeper/TIS 不接受当前签名：虽然已有 `Codexbar Local Code Signing Leaf v4` identity 且 app 被该 identity 签过，但 `TeamIdentifier=not set`，`spctl` 仍 rejected，TIS readiness 明确给出 `signature-rejected`。
- 系统目录 app 与当前 build CDHash 也不一致，需要在签名链修好后重新 `build.sh` / `install-system.sh`，再验证 `spctl accepted` 和 `tisReadiness=true`。
- 在 `spctl accepted` 前不运行 TextEdit/Safari/Clipboard GUI smoke，避免继续抢用户焦点并制造假阳性。

## 2026-07-08 v50 - macOS host build/self-check 分组提交前验证

问题：

- 提交 macOS Host 前重跑 `build.sh`，构建预检误判 Codex Desktop 的通知进程为正在运行的 verifier。
- 误判来源是 `SkyComputerUseClient ... agent-turn-complete` 的 command line 中包含本线程历史消息；历史消息里有 `verify-nongui.sh` 等脚本文本，同时 command line 也包含当前 workspace 路径。
- `--bridge-self-check` 一度返回 `bridgeSelfCheck=false`，原因是诊断 bridge 使用 direct session，只传 `rime_user_data_dir`，与真实 host 的 settings session 不一致。

修正：

- `build.sh` 的 verifier 并发检测跳过 `SkyComputerUseClient`、`notify-hook.js`、`agent-turn-complete` 进程，避免被 Codex 通知 payload 里的历史文本误伤。
- `InputiaRustBridge.temporaryForDiagnostics()` 改为写临时 settings 并通过 `inputia_session_new_from_settings` 打开 session；临时 settings 显式包含 bundled `RimeData`、临时 Rime user dir、memory db path，与真实 host 配置路径一致。

验证：

```text
./macos/InputiaInputMethod/build.sh
  /Users/minizl/services/Handy/macos/InputiaInputMethod/build/InputiaInputMethod.app
  /Users/minizl/services/Handy/macos/InputiaInputMethod/build/Inputia 设置.app

./macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --self-check
  bundleIdentifier=com.inputia.inputmethod.Inputia
  connectionName=com.inputia.inputmethod.Inputia_Connection
  classFound=true

./macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-self-check
  bridgeSelfCheck=true
  consumed=true
  mode=Chinese
  firstCandidate=在
  commit=中国

./macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  simplifiedScriptCommit=中国
  traditionalScriptCommit=中國

./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  officialAppleCommandKeyDownSetPassesThrough=true
```

注意：

- build 仍会输出 Rust staticlib 以 macOS 26 object 链到 `arm64-apple-macos13.0` 的 linker warning；当前它不是阻断项，后续可单独收敛 Rust target / deployment target。
- 本轮仍未运行 GUI smoke；系统安装 readiness 仍需先解决 `spctl rejected` / notarization blocker。

## v50 Mac mini：快捷键整类透传、简繁路由修复、签名 ACL 推进与 notary blocker

背景：

- 用户反馈 `Command-C` / `Command-V` 在 Inputia 下不可用，要求按常用电脑快捷键举一反三，不能逐个靠用户测试。
- 用户也要求系统输入法不要显示/堆积 `Inputia 简体` 这类多个入口，简繁应该作为偏好设置里的切换状态。
- 用户已在 Mac mini 上信任 `Codexbar Local Code Signing Root v4`，需要继续验证 signing/import/install 主线。

实现与选择：

- `main.swift` 的 `handleKeyDown` 最前面调用 `InputiaShortcutClassifier.shouldPassThroughKeyDown(...)`，Command 修饰键整类返回 `false`，让系统/App 原生处理复制、粘贴、剪切、撤销、查找、保存、窗口、截图、切 App 等快捷键。
- `verify-nongui.sh` 增加静态合同：Host keyDown 的 Command pass-through 必须早于简繁切换、剪贴板召回、标点切换，并且必须 `return false`。
- Rime 简繁输出从单个 `output_option=true` 改成显式 `(option, enabled)` 列表，切换繁体时会关闭相冲突的 `zh_hans` / `simplification`，并启用 `zh_hant` / `trad_tw`，避免同一 Rime user data 中前一次简体选项污染后续繁体输出。
- `import-signing-identity.sh` 增强：identity 已存在但 codesign probe 失败时，如果提供 keychain 密码，会先 `unlock-keychain` 并执行 `set-key-partition-list`，再重跑 probe。这个覆盖本轮真实遇到的 private key ACL 问题。
- `verify-nongui.sh` 的 system preflight 改为只接受两类安全阻断：`cdhash-mismatch` 或 `ui-smoke-disabled`，避免“当前系统还没安装最新 build”被误判为会启动 GUI smoke 的成功条件。

验证：

```text
INPUTIA_P12_PASSWORD=... INPUTIA_KEYCHAIN_PASSWORD=... ./macos/InputiaInputMethod/import-signing-identity.sh
  signingIdentityAlreadyAvailable=true
  12C73495BB5C15FD71C22E823A8A9CBD0CC5243C "Codexbar Local Code Signing Leaf v4"
  signingIdentityCodesignProbe=true
  signingIdentityImportVerified=true

INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
INPUTIA_CODESIGN_OPTIONS="--options runtime" \
./macos/InputiaInputMethod/build.sh
  buildRc=0
  codesign --verify --deep --strict: valid on disk / satisfies Designated Requirement

codesign -dv --verbose=4 macos/InputiaInputMethod/build/InputiaInputMethod.app
  Authority=Codexbar Local Code Signing Leaf v4
  Authority=Codexbar Local Code Signing Root v4
  TeamIdentifier=not set
  Runtime Version=26.0.0

spctl --assess --type execute --verbose=4 macos/InputiaInputMethod/build/InputiaInputMethod.app
  rejected

syspolicy_check distribution macos/InputiaInputMethod/build/InputiaInputMethod.app
  Notary Ticket Missing
  Severity: Fatal

./macos/InputiaInputMethod/notarization-readiness.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
  codesignVerify=true
  codesignAuthority=Codexbar Local Code Signing Leaf v4
  codesignAuthority=Codexbar Local Code Signing Root v4
  hardenedRuntime=true
  spctlAccepted=false
  syspolicyNotaryTicketMissing=true
  developerIDApplicationIdentityPresent=false
  notaryProfileAvailable=false
  inputiaGatekeeperReady=false
  notarizationRequiredAction=import-developer-id-application-identity

./macos/InputiaInputMethod/build/inputia-shortcut-self-check
  shortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  officialAppleCommandKeyDownSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true

./macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --host-shortcut-self-check
  hostShortcutSelfCheck=true
  commandCPassThrough=true
  commandVPassThrough=true
  officialAppleCommandKeyDownSetPassesThrough=true
  anyCommandModifiedKeyPassesThrough=true
  allCommandModifierVariantsPassThrough=true

./macos/InputiaInputMethod/build/inputia-host-text-policy-self-check
  hostTextPolicySelfCheck=true
  appCommandcopyPassesThrough=true
  appCommandpastePassesThrough=true
  appCommandcutPassesThrough=true
  appCommandundoPassesThrough=true
  appCommandredoPassesThrough=true
  appCommandselectAllPassesThrough=true
  appCommandsaveDocumentPassesThrough=true
  appCommandopenDocumentPassesThrough=true
  appCommandperformClosePassesThrough=true
  appCommandterminatePassesThrough=true
  appCommandfindPassesThrough=true
  appCommandprintPassesThrough=true

./macos/InputiaInputMethod/build/inputia-input-text-router-self-check
  inputTextRouterSelfCheck=true
  simplifiedScriptCommit=中国
  simplifiedScriptCommitsSimplified=true
  traditionalScriptCommit=中國
  traditionalScriptCommitsTraditional=true

cargo test --manifest-path crates/inputia-capi/Cargo.toml chinese_script_selects_schema_and_rime_option -- --nocapture
  1 passed

cargo test --manifest-path crates/inputia-capi/Cargo.toml capi_traditional_script_commits_traditional_chinese -- --nocapture
  1 passed

CARGO_BUILD_JOBS=1 cargo test --lib --manifest-path crates/inputia-rime/Cargo.toml squirrel_default_config_keeps_rime_outside_core -- --nocapture
  1 passed
```

磁盘与回归限制：

```text
df -h /Users/minizl/services/Handy
  Avail: 111M -> 423M after cleaning generated build/target artifacts

cargo test --manifest-path crates/inputia-rime/Cargo.toml squirrel_default_config_keeps_rime_outside_core -- --nocapture
  first failure: No space left on device while creating temp dir
  second failure: linker write() failed, errno=28

CARGO_BUILD_JOBS=1 cargo test --lib ...
  passed
```

结论：

- `Command-C` / `Command-V` 不是单点修复；当前分类器和 Host 路径已把常见 Apple Command 快捷键整类透传，且 `didCommand` 的常见 app command selector 也透传。
- 简繁路由的真实 bug 已复现并修复：繁体模式现在提交 `中國`，不是被前一轮简体 Rime option 污染成 `中国`。
- 用户信任 root 后，签名链和 private key ACL 已推进到 `codesign` 可用：build app 由 `Codexbar Local Code Signing Leaf v4` 签名，并带 hardened runtime。
- 但 Gatekeeper/TIS blocker 没有消失：`spctlAccepted=false`，`syspolicy_check` 明确 fatal 为 `Notary Ticket Missing`，且没有 `Developer ID Application` identity / notarytool profile。继续系统级可用安装需要 Developer ID 签名 + notarize/staple，或者接受本地开发 override 仍不能作为 GUI smoke 前提。
- 本轮没有运行 TextEdit/Safari/Clipboard GUI smoke。

## v51 Mac mini：系统账号目录服务异常导致 signing/Gatekeeper 诊断失真

背景：

- 用户要求按功能小步提交并同步 GitHub，避免最后工作区过脏。
- 在准备继续签名/安装验证时，Mac mini 出现新的基础环境异常：当前进程 UID 501 无法解析为真实用户名，影响 SSH、Keychain、`security`、`spctl`、`syspolicy_check` 和统一日志读取。
- 参考 Apple Developer ID/Gatekeeper 文档和 Squirrel/ToyIMK/IMKitSample/vChewing 等成熟输入法项目后，当前结论不变：系统级 IMK 输入法必须先让签名/信任链稳定，不能在 Gatekeeper/TIS readiness 未通过前继续跑 TextEdit/Safari/Clipboard GUI smoke。

事实：

```text
id -u
  501

id -un
  501

dscacheutil -q user -a uid 501
  no user record returned

security find-identity -v -p codesigning
  0 valid identities found

codesign -dv --verbose=4 --entitlements :- "/Library/Input Methods/InputiaInputMethod.app"
  CDHash=c478b1efda1d34607d3429c3aa953ae147a0dd29
  Authority=(unavailable)
  TeamIdentifier=not set
  Runtime Version=26.0.0
  warning: binary contains an invalid entitlements blob. The OS will ignore these entitlements.

spctl --assess --type execute --verbose=4 "/Library/Input Methods/InputiaInputMethod.app"
  /Library/Input Methods/InputiaInputMethod.app: internal error in Code Signing subsystem

sudo -n true
  sudo: you do not exist in the passwd database

dscl . -read /Users/minizl UniqueID RecordName NFSHomeDirectory
  Operation failed with error: eServerError

git push
  No user exists for uid 501
  fatal: Could not read from remote repository.
```

工具修正：

- `notarization-readiness.sh` 增加 `accountLookupReady` / `accountLookupBlockReason` 输出，先把 UID 解析失败标为一等阻断。
- `spctl` 和 `syspolicy_check` 改为带超时的捕获执行，避免系统策略子系统卡住整个验证流程。
- 当账号目录服务不可用时，最终动作现在明确输出 `notarizationRequiredAction=restore-macos-account-directory-service`，而不是继续误导为单纯导入证书或跑 GUI smoke。

验证：

```text
zsh -n macos/InputiaInputMethod/notarization-readiness.sh
  passed

INPUTIA_SYSPOLICY_TIMEOUT_SECONDS=5 INPUTIA_SPCTL_TIMEOUT_SECONDS=5 \
./macos/InputiaInputMethod/notarization-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"
  accountLookupUID=501
  accountLookupUser=501
  accountLookupDscacheutilUserPresent=false
  accountLookupReady=false
  accountLookupBlockReason=getpwuid-missing
  codesignVerify=false
  codesignVerifyOutput: /Library/Input Methods/InputiaInputMethod.app: CSSMERR_TP_NOT_TRUSTED
  spctlAccepted=false
  spctlTimedOut=false
  spctlInternalError=true
  syspolicyCheckAvailable=true
  syspolicyCheckRc=124
  syspolicyCheckTimedOut=true
  developerIDApplicationIdentityCount=0
  notaryProfileCheckOutput: Error: An error occurred while accessing the keychain. One or more parameters passed to a function were not valid.
  inputiaGatekeeperBlockReasons=account-lookup-broken,codesign-invalid,spctl-rejected,spctl-internal-error,syspolicy-timeout
  inputiaNotarySubmissionBlockReasons=account-lookup-broken,missing-developer-id-application,missing-notarytool-profile
  notarizationRequiredAction=restore-macos-account-directory-service
```

结论：

- 当前 Mac mini 的首要 blocker 不是安装包、不是 TIS 刷新、不是 smoke 脚本，也不是继续手动选择输入法；是系统账号目录服务/Keychain 信任环境异常导致签名身份消失、Gatekeeper 返回内部错误、`syspolicy_check` 卡住。
- 在 `accountLookupReady=true`、`security find-identity` 重新能看到签名身份、`spctl` 不再内部错误前，不运行 TextEdit/Safari/Clipboard GUI smoke。
- 本轮没有运行 GUI smoke。
