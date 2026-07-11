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
