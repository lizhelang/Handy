# 1.0.6 Shift 标点组合不切换模式

## 用户反馈和根因

用户在中文或英文模式按 Shift+/ 输入问号，模式被反向切换。主程序的 deferredShiftToggle 是 IMK 漏交付 Shift 松开事件的补救逻辑，却只排除 Shift 大写字母，没有要求当前按键的 Shift 标志已消失。问号等带 Shift 的非字母因此被提前当作独立 Shift 切换。

另外，中文有未提交拼音时 flagsChanged 在 Shift 按下时立即提交了拼音，这时还无法判断用户是单按 Shift，还是准备输入标点或大写字母。

## 参考与解决规则

微信输入法的行为由用户实体键盘反馈给出，未取得其内部源码，不能声称确认了微信实现。参考 [Rime 的原生 ascii_composer](https://github.com/rime/librime/blob/master/src/rime/gear/ascii_composer.cc)：修饰键释放才执行切换；其他普通按键参与时清除修饰键切换资格。

本项目规则：

- 独立按下、松开 Shift：中英文切换；中文未提交拼音先按原始字母提交。
- Shift 与任何普通键组合：取消本次切换资格，保持当前模式；保留标点、大写及 Shift+Space 等已有功能。
- 松开事件丢失时，仅下一次不含 Shift/Command/Control/Option 的按键可以领取仍有效的独立手势。
- 组合键已取消资格后，即使松开事件丢失，后续普通键也不能触发补救切换。

候选、热词及语音状态不参与该判断。

## 验证

CLT 快捷键自检共 67 项通过，包括新增 13 项。左右 Shift 分别覆盖 Shift+/ 普通键先松、Shift 先松和 Shift 松开事件丢失；已有独立 Shift 双向切换、IMK 普通键 keyUp 丢失恢复、大写组合和重复/迟到事件检查保持通过。全原生构建通过，运行其产出的 `inputia-shortcut-self-check` 再次得到 `shortcutSelfCheck=true`，67 项均为 true。

主程序接线移除按下 Shift 就提交中文拼音的提前动作，改为确认独立手势后才提交原始拼音并切换模式。自检不等同于实体键盘的宿主 IMK 验收。

## 安装结果

主程序和输入法均已签名成对安装并重启为 `1.0.6 / 80`，运行身份校验通过，Hans 输入源已恢复为选中状态。更新器返回 `candidateUpdate=true tccChanged=false previousRecordingsReplayed=false`。安装后 `--shortcut-self-check` 返回 `hostShortcutSelfCheck=true`，实际词库检查仍为 `loaded_count=65 prefix_matches=1 voice_snapshot_required=false`。

无权限重置，无热词丢失。已创建本地提交，未推送或发布 GitHub release。最终实体键盘验收为：中英文分别按 Shift+/ 后继续输入，确认模式保持；独立 Shift 双向切换；中文拼音组合中单按 Shift 按原始字母提交并切换英文。
