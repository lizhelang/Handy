# 候选面板切行宽度修复

日期：2026-09-30。针对展开候选面板上下切换选中行时，窗口宽度及列位置反复变化的问题。

## 原因与修改

原布局只测量当前显示的数字标号；标号随选中行移动，导致各列的最大测量宽度改变。现在为每一行计算其被选中、显示标号时所需的宽度，再取各列最大值。实际显示仍只有选中行带标号，切行不改变列宽和面板尺寸。

同时修复窄屏收缩：第一列已达到最小宽度时，继续收缩其他可缩列，避免过早退出并触发逐行布局回退。新候选列表或字号变化仍会重新计算尺寸。

涉及代码：

- `macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaCandidatePanel.swift`
- `macos/InputiaInputMethod/Tools/InputiaCandidatePanelLayoutSelfCheck.swift`

## 验证

使用截图中的 40 个候选、每行 7 列，运行实际 `InputiaCandidateContentView` 的测量和布局，再比较全部候选子视图的 frame。

- 修复前，14 点字号下切行宽度为 `310,268,268,268,268,263` 点，对应截图前两行的 `620/536` 像素。
- 修复后，逐行向下再向上遍历，始终为 `310` 点，所有候选格的位置与大小保持一致。
- 宽度上限为 280 点时，所有行始终为 `280` 点，候选格不越界。
- 覆盖 12、14、22 点字号、280/1600 点宽度上限、长词分布在不同列、不满一行的尾行。
- 验证更换候选或字号会重新计算宽度，收起模式仍保持单行。
- `candidatePanelLayoutSelfCheckPassed=true`；完整原生构建、三个应用签名验证通过；独立只读代码检查未发现阻断问题。
- 同一原生视图分别渲染第 1 行和第 2 行选中状态，均为 `310×138` 点。这是合成候选的真实 NSView 渲染，不是已安装输入法的实体键盘验收。

## 交付状态

本轮产物为 `1.1.0/build84`，包含此前候选智能化改动。复用此前已验证的控制中心，只重新构建输入法和设置启动器；使用原稳定签名身份并生成匹配新二进制的配对清单。

包目录：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/release-panel-width-20260930-4gjeb6hx`。内含源文件及程序哈希、构建/回归日志、原生视图渲染图。旧安装包保持原样。

构建完成时，本机仍为 `1.0.9/build83`，尚未执行覆盖更新。

## 用户授权后的本机安装

用户明确要求“覆盖安装”后，执行受控更新器 `update-candidate.py --apply`。更新前确认活动语音会话为 0，修复包程序哈希与构建元数据一致，安装与备份路径可写且处于同一文件系统。

- 更新结果：`releaseUpdate=true`、`tccChanged=false`、`previousRecordingsReplayed=false`。
- `/Applications/Inputia.app` 与 `~/Library/Input Methods/InputiaUnifiedCandidate.app` 均更新为 `1.1.0/build84`；更新器核验两个实际运行进程的代码身份通过。
- 原输入源 `com.inputia.inputmethod.Inputia.UnifiedCandidate.Hans` 已恢复，`selectCurrentMatchesTarget=true`。
- 原应用及配对清单备份：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission-update-7torab1j`。
- 完整安装日志：修复包内 `verification/update-apply.log`。

安装已完成；上述结论来自程序身份及更新器验证，安装后的实体键盘体验尚未单独验收。
