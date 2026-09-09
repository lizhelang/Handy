# Shift 手势接续检查（2026-09-09）

范围：接续未提交的 Shift 手势接线，不修改已安装候选、不录音、不操作用户文稿。

## 本轮实际证据

在现有 `InputiaShortcutClassifier.shiftInputModeGestureSelfCheckResults` 中增加两条事件序列，先运行失败，再修改原状态机：

- 普通键始终按住，连续按下/松开两次 Shift：`shiftGestureHeldKeyAcrossTwoGesturesRejected=false`。原因是第一次 Shift 松开清空了普通键状态。现改为仅由普通键 keyUp 移除，结果 true。
- 全局 Shift 松开留下待领取结果，之后普通 keyDown，再收到本地 flagsChanged：`shiftGestureInterveningKeyCancelsDeferredToggle=false`。现由普通 keyDown 取消待领取结果，结果 true。
- 现有检查仍通过：`shortcutSelfCheck=true`；`git diff --check` 通过。

可重跑命令（仓库根目录）：

```sh
swiftc macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaExpandedCandidateGridNavigation.swift macos/InputiaInputMethod/Tools/InputiaShortcutSelfCheck.swift -o /tmp/inputia-shift-selfcheck
/tmp/inputia-shift-selfcheck
```

首次编译命令漏列 ExpandedCandidateGridNavigation，补上该已有依赖后才得到上述行为测试结果；编译错误不算行为复现。

## 未完成及下一实验

### 最新批次：独立复核后候选 53

安装与实际实验补记：

- **后续回归**：用户通过侧聊报告单独 Shift 无法切换中英文，优先级高于先前单测。问号场景的确认不撤销，但整体 Shift 功能未通过。
- 为区分本地事件缺失与会话取消，候选 54 仅增加 OSLog 手势阶段/布尔诊断，没有改变判定行为。已安装，实际 PID 42337/CDHash `aec88c03078c84dba09a8499e78ebf9bbe0cbe45` 校验通过；测试控制中心已重启加载新配对。备份及旧配对在 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/shift-diagnostic-20260909.1fENsU`。
- 诊断子系统 `com.inputia.shift` 只记录 global/local flags、toggle 和 session-reset 原因，不记录普通按键、正文、窗口标题；未启用旧 INPUTIA_DEBUG_EVENTS 日志。
- 初始原生记录显示 activate/deactivate 交替且 shift=false。尚未收到对应用户单独 Shift 的确认，不能据此判定回归原因。下一实验是实体单独 Shift 的 local/global/reset 顺序。
- 后续 54 日志在 17:18:05、17:18:09（悉尼时间）出现 global/local 的 Shift down/up，local 阶段 armed=false，未执行 toggle。这排除“所有本地 flags 都收不到”的假设，但尚不足以区分残留键、修饰键组合与设置状态。日志保存在上述备份目录 `phase-events-54.log`。
- 55 仅增加 held/baseline/configured/blocked 布尔诊断（不记键值），完整构建后已安装；PID 44259、CDHash `71d2bdeb39c75ba691af0f4efc38fda24e05f99a` 实际校验通过。旧 54 包仍在备份目录，配对为 `pair-55.json`，测试控制中心已重启。
- CUA 在专用文稿调用 Shift+/ 后显示新增问号，但对应观察窗口没有 flags 记录，只有 session-reset；因此不能用 CUA 送键替代本次实体修饰键诊断。仍等待 55 的实体单 Shift 事件，不归因、不放宽判断条件。

- **最新用户实体键盘确认**：`Shift+/` 能输出 `?`，且中文模式保持。该组合键场景已通过用户原生确认，取代下方自动送键实验对该场景的未决结论。不据此推断任意松开顺序、独立 Shift、焦点变化或所有应用均通过；无需再次请求用户重复此项。

- 实现提交 `9f4651ef` 已安装至用户级测试包，输入法实际 PID 26157 经 running-identity 校验为 `6f9f115aa6e93b40548cb40d9f67b5411784f33c`，不是旧进程。配对重签成功；测试控制中心由 PID 87379 正常退出后重启为 PID 26383，未替换控制中心二进制。
- TIS 当前确认 `Inputia (Test)` / `.UnifiedCandidate.Hans`。日常输入法包和 Handy.app 未修改。
- CUA 在 TextEdit 新建空白文稿进行测试：`shift+slash` 输出 `?`，随后 `n`、空格得到 `?n `，没有足够证据证明中文候选状态。因此本轮原生验收为**未通过验证**，不能以问号已出现声称组合键修复完成。
- 下一区分实验尝试通过 CUA 独立 `shift`，工具拒绝：`keyPressIncludedNoNonModifierKeys`，没有发出该键。现有接口不能完成独立修饰键与任意松开顺序对照，必须取得实体键盘证据；不使用另一种事件注入工具绕过限制。
- 新建测试文稿由 TextEdit 自动命名 `未命名7.rtf`；只写入上述合成按键，没有操作原有文稿。未启用包含正文的事件日志、未录音。
- 下一最小人工实验：在该测试文稿中独立按一次 Shift，再输入 n 和空格，观察候选是否出现；之后才比较 Shift+/。当前不判断是模式切换失败还是自动送键路径差异。

- 独立审查发现两项阻塞：焦点断开漏收 keyUp 后状态不能恢复；全局待领取切换缺少事件归属。已修复并经同一独立审查者复核：原两项消除，限定范围内无新增阻止安装验证的问题。
- 删除 deferred 切换。全局 flagsChanged 仅否决组合参与，本地事件独占手势起止和切换；`shiftGestureDelayedGlobalCannotCreateSecondToggle` 先失败后通过。
- 新增会话 reset，与同一手势 invalidate 分开。会话切换清普通键记录，保留系统当前修饰键作为基线，不让已按住 Shift 获得新资格。缺 keyUp 后恢复序列先失败后通过；跨会话已按住 Shift 不切换。
- 完整候选 53 构建及严格签名通过，CDHash `6f9f115aa6e93b40548cb40d9f67b5411784f33c`，日志 `/tmp/inputia-shift-53-build.log`，构建产物快捷键自检通过；`zsh -n build.sh` 与 `git diff --check` 通过。
- 候选 52 备份及旧配对清单位于 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/shift-20260909.Sr8J5m`。更新只涉及测试输入法与测试配对，日常安装和用户数据不迁移；恢复时选择离开测试输入法，恢复备份应用及旧配对，重启测试控制中心，并校验实际运行进程签名。
- 下一步：安装 53、重签测试配对并重启测试控制中心加载，再在新建 TextEdit 测试文档验证。此处仍不宣称已安装或原生通过。

### 接续批次：本地事件接线及候选构建

- 新增无全局监听序列，先得到 `shiftGestureLocalHeldKeyRejectedWithoutGlobalMonitor=false`；将本地 keyDown 纳入现有按住集合并注册/处理本地 keyUp 后为 true。keyUp 返回 false，不消费宿主事件、不送入 Rime。`shiftGestureLocalKeyUpRestoresIndependentShift=true`。
- 取消期间另一路重复 Shift down 的序列先得到 `shiftGestureCancelledHoldCannotRearmFromDuplicateDown=false`；取消改为保留物理按住状态、仅撤销切换资格后为 true。
- 最后一次完整候选构建成功，构建日志 `/tmp/inputia-shift-build-final.log`；产物内快捷键自检 `shortcutSelfCheck=true`，严格签名验证通过。
- 构建产物 `macos/InputiaInputMethod/candidate-builds/trial-20260905/InputiaUnifiedCandidate.app`；CDHash `a485958e1052ea888a0b48a9de560a1379a97601`。源码尚未提交；未安装、未更新配对清单、未重启候选或日常程序。
- 已安排独立只读审查；仍须处理事件归属、焦点变化及未收到 keyUp 时的恢复问题，再决定可安装性。构建通过不是实际原生手势通过。

以下为前一批次的缺口快照，本地 keyUp 项已由本批接线，原生行为仍未验证：

- 主机本地 recognizedEvents 仍只有 keyDown/flagsChanged，普通键 keyUp 依赖全局监听。必须检查无全局键盘事件权限时的行为，不能以状态机自检代替接线验证。
- 待领取结果尚未绑定具体原生事件身份；需要验证全局/本地事件交错、焦点取消后迟到事件，不使用时间阈值掩盖重复。
- 未构建/安装本轮候选，未验证实际 Shift+/ 两种松开顺序、长按、组合输入提交、同窗口输入框变化。
- 下一步先补齐本地按键状态观察与事件归属验证，再独立审查并构建候选。不得将本记录当作原生验收或 goal 完成证据。
