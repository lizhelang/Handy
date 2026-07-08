# Inputia macOS IMK Host 阻塞 Runbook

日期：2026-07-06

更新：2026-07-08

当前只处理 macOS Text Input Source / InputMethodKit Host 入口问题。不要把这个阻塞归到 Core、Rime、候选窗、Settings 或记忆层。

## 结论先行

当前最可信的判断：

1. Inputia 源码里的 Host baseline 已经走到 Squirrel-style mode-enabled input method 结构：parent `com.inputia.inputmethod.Inputia`，visible modes `com.inputia.inputmethod.Inputia.Hans` / `com.inputia.inputmethod.Inputia.Hant`。
2. Apple TIS 规则决定了 mode-enabled input method 的 parent 本来就不可选；真正要进入菜单栏和 TextEdit 的是 visible mode。
3. 旧的 `/Library/Input Methods/InputiaInputMethod.app` 缺 `Inputia.icns` 和 `InfoPlist.strings`，不能用它证明最新版 Host 方案失败。
4. `TISEnableInputSource` 返回 `0/noErr` 不是成功标准。成功标准只能是 `TISCreateInputSourceList(nil, false)` 能枚举出 Inputia mode，System Settings 已启用列表能看到它，且 `TISSelectInputSource` 返回 `0/noErr`。
5. ToyIMK 和本机 probe 现象都指向同一个 macOS 行为：首次安装新的 IMK app 后，System Settings 的可添加输入源列表可能不会热刷新。先关闭 System Settings 并刷新 `TextInputMenuAgent` / `SystemUIServer`；仍不出现时再 logout/login。
6. 历史上 v4 机器曾经解除过 Host 入口阻塞：`Inputia 简体` 已添加到 System Settings 已启用输入源列表，`includeAllInstalled=false` 能枚举到 Inputia parent 和 Hans mode，Hans mode `enabled=true/selectable=true/selected=true`，`TISSelectInputSource` 返回 `0`，TextEdit 真实按键能上屏。
7. 但当前 v41 状态必须以 `status.sh` / `gui-smoke-readiness.sh` 为准：构建版是 v41，系统目录 `/Library/Input Methods/InputiaInputMethod.app` 和 `/Applications/Inputia 设置.app` 仍是 v40，TIS enabled/installed matches 为 0。真实 TextEdit/Safari/Clipboard GUI smoke 仍被正确阻断。
8. 菜单栏真实 Host 仍取决于 `/Library/Input Methods/InputiaInputMethod.app`。临时取消旧 LaunchServices 记录并注册 build app 后，TIS source 的 `iconURL` 仍指向 `/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf`；因此不能靠手动注册 build app 代替系统目录安装。

## 外部证据

- Apple Text Input Source Services Reference：
  - mode-enabled input method 的 parent 不直接 selectable。
  - input mode 只有在自身和 parent 都 enabled 时才能被 selected。
  - 否则 `TISSelectInputSource` 返回 `paramErr/-50`。
  - 参考：https://leopard-adc.pepas.com/documentation/TextFonts/Reference/TextInputSourcesReference/TextInputSourcesReference.pdf
- Apple Support Input Sources：
  - 菜单栏切换的是已添加/已启用输入源，不是 all-installed list。
  - 参考：https://support.apple.com/guide/mac-help/change-input-sources-settings-mchl84525d76/mac
- Squirrel：
  - `resources/Info.plist` 使用 `ComponentInputModeDict` 暴露 `im.rime.inputmethod.Squirrel.Hans/Hant`。
  - `scripts/postinstall` 顺序是 kill、register、build、再以登录用户 enable/select。
  - `sources/InputSource.swift` 默认只 enable/select primary mode，不把 parent 当选择对象。
  - 参考：https://github.com/rime/squirrel
- ToyIMK：
  - 文档明确写着首次安装后需要 logout/login，然后在 System Settings 添加输入法。
  - 参考：https://github.com/eagleoflqj/toyimk

## 本机事实

源码 baseline：

```text
bundle id: com.inputia.inputmethod.Inputia
parent source: com.inputia.inputmethod.Inputia
primary mode: com.inputia.inputmethod.Inputia.Hans
secondary mode: com.inputia.inputmethod.Inputia.Hant
connection: com.inputia.inputmethod.Inputia_Connection
controller: InputiaInputMethod.InputiaInputController
```

当前系统目录旧安装曾经只有 mode PDF 图标资源：

```text
/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/inputia.pdf
```

旧安装曾缺少：

```text
Inputia.icns
en.lproj/InfoPlist.strings
zh-Hans.lproj/InfoPlist.strings
zh-Hant.lproj/InfoPlist.strings
```

因此早先 System Settings 搜不到 Inputia，最小解释不是 Info.plist 又错了，而是系统目录没有最新版 bundle，且 Add list 没有刷新到 probe / user-level 安装。历史上最新版系统安装并刷新 UI 后，System Settings 的简体中文添加列表曾出现 `Inputia 简体`。后续菜单图标改为只使用 `TISIconLabels`，不再把旧白色 PDF 作为 mode 图标。若 `/Library/Input Methods` 仍是旧包，TIS `iconURL` 会继续指向旧 `inputia.pdf`，菜单和真实输入链也会继续表现为旧 Host。

当前 v41 事实：

```text
buildVersion=41
buildCDHash=6d7e1033ef95597258f7c9c30f7d361f1b3dee2f
system.version=40
system.cdhash=8d4f473adcc2f7c093b5629b9b1e742dcba184f8
systemMatchesBuild=false
settings.systemVersion=40
systemSettingsMatchesBuildVersion=false
tis.includeAllInstalled=false matches=0
tis.includeAllInstalled=true matches=0
latestPkgSHA256=e6af057c5199c590a6eba4529439738cd4919e0d1be42530b7776ddc3b16858c
```

当前不要硬跑真实 GUI smoke。先把系统目录和设置启动器更新到 v41，再等 TIS readiness 通过。

## 正确验证顺序

1. 构建最新版 Host：

```bash
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
  INPUTIA_CODESIGN_OPTIONS="--options runtime" \
  ./macos/InputiaInputMethod/build.sh
```

2. 自检 build app：

```bash
./macos/InputiaInputMethod/verify-system.sh macos/InputiaInputMethod/build/InputiaInputMethod.app
```

要求看到：

```text
resourcePresent=true path=.../Inputia.icns
resourcePresent=true path=.../en.lproj/InfoPlist.strings
resourcePresent=true path=.../zh-Hans.lproj/InfoPlist.strings
resourcePresent=true path=.../zh-Hant.lproj/InfoPlist.strings
classFound=true
```

3. 构建安装包：

```bash
INPUTIA_CODESIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
  INPUTIA_CODESIGN_OPTIONS="--options runtime" \
  ./macos/InputiaInputMethod/build-pkg.sh
```

`build-pkg.sh` 输出 `dist/InputiaInputMethod-v<version>-<cdhash>.pkg`。不要在 Installer 已经打开后覆盖同一个 pkg 路径，否则 Installer 会因 digest mismatch 拒绝安装。

4. 安装到系统目录。此步必须有管理员授权：

```bash
pkg_path="$(./macos/InputiaInputMethod/build-pkg.sh | tail -n 1)"
sudo /usr/sbin/installer -pkg "$pkg_path" -target /
```

或运行 `./macos/InputiaInputMethod/open-installer.sh` 走 Installer UI。

5. 验证系统目录确实是最新版：

```bash
./macos/InputiaInputMethod/verify-system.sh "/Library/Input Methods/InputiaInputMethod.app"
```

6. 关闭 System Settings 并刷新 Text Input UI 缓存：

```bash
osascript -e 'tell application "System Settings" to quit' || true
killall TextInputMenuAgent >/dev/null 2>&1 || true
killall SystemUIServer >/dev/null 2>&1 || true
open 'x-apple.systempreferences:com.apple.Keyboard-Settings.extension'
```

如果添加列表仍不出现 Inputia，再 logout/login，然后打开：

```text
System Settings > Keyboard > Text Input > Edit > Add > 简体中文
```

搜索或查找 `Inputia` / `Inputia 简体`。

7. 手动添加后，再验证 enabled list：

```bash
"/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod" \
  --dump-source-prefix com.inputia.inputmethod.Inputia
```

成功时 `includeAllInstalled=false` 中必须出现 Inputia mode，并且：

```text
enabled=true
selectable=true
```

历史 v4 机器曾验证过的 ready 形态如下；当前 v41 不能沿用这段作为现状证明，必须重新以 `status.sh` / `gui-smoke-readiness.sh` 输出为准：

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
```

8. 选择输入源：

```bash
"/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod" \
  --select-input-source
```

成功标准：

```text
selectStatus=0
```

历史 v4 机器曾验证 `selectStatus=0`。当前 v41 仍未满足该前置状态。

9. TextEdit 或 Notes 里选择 Inputia，按键应进入 `InputiaInputController` 并通过 `insertText` 上屏英文字符。

历史 v4 机器曾验证 TextEdit smoke：

```text
current source = com.inputia.inputmethod.Inputia.Hans
System Events keystroke "xyz"
TextEdit text value = abcxyz
InputiaInputMethod log = InputMethodKit Inserting text
```

当前 v41 仍未运行真实 TextEdit/Safari/Clipboard GUI smoke；系统安装版/settings 仍需先更新到 v41，且 TIS readiness 必须通过。

## 不再重复尝试的方向

- 不把 all-installed list 里能看到 Inputia 当成功。
- 不把 `TISEnableInputSource == 0` 当成功。
- 不手写 `AppleEnabledInputSources` 当正式方案。
- 不继续扩展 Core/Rime/Settings 来绕开 Host。
- 不在旧系统目录 bundle 上继续猜字段。

## 后续动作

历史 v4 的 macOS enabled/selectable 入口阻塞曾经解除，并验证了物理键盘 Shift 的 `flagsChanged` 事件能进入 IMK，在 `mode=English` / `mode=Chinese` 之间切换。当前 v41 不能沿用这条历史状态：必须先更新系统安装版和 TIS 状态。

当前正确下一步：

1. 用管理员权限安装当前 v41 pkg 或运行 `install-system.sh`。
2. 跑 `status.sh`，要求系统 host/settings 都变成 v41，CDHash 对齐 build。
3. 跑 `gui-smoke-readiness.sh "/Library/Input Methods/InputiaInputMethod.app"`，要求 `guiSmokeReadinessReady=true reason=none`。
4. readiness 通过后再跑 `INPUTIA_RUN_UI_SMOKE=1 gui-smoke-suite.sh "/Library/Input Methods/InputiaInputMethod.app"`。
5. 只有 GUI suite 通过后，才把注意力转回候选窗、设置实时重载和 Core/Rime/Settings 集成验证。
