# 固定签名后的真实语音闭环复核

2026-09-09，当前代码底座 9b80278f；只以 /Users/lzl/FILE/github/Handy-unified-input-system 的当前代码判断候选，不用其它旧 worktree 否认已有 owned voice 实现。

## 当前运行与基线

- 安装中的固定签名输入法 PID5738，控制中心 PID6019。TIS 确认选中候选 .UnifiedCandidate.Hans，应用路径匹配用户级候选包。
- 专用 TextEdit 文稿为构建根 loop-20260908.r0xugn/native-output-check.rtf，磁盘和实际编辑区均为空；未使用真实聊天或日常文稿。
- 新菜单实验前基线：transcription_history 共5行，最大id=15；unified_voice_sessions 共8行，其中 interrupted=3、pending_target=5。这些旧会话不是本轮结果。
- 工具逐键输入 n、i、空格得到了 n成，不是预期整段拼音结果，不能据此计为键盘完整验收通过。InputMethodKit 的 deactivateServer 会提交组合；工具每次动作的焦点与实际输入方案还需区别验证，未凭猜测修改按键或组合输入逻辑。已仅清除这两个测试字符并保存，恢复空白。

## 对快捷键缺口的当前源码复核

Option+Space 的注册不是当前已证实的阻塞。shortcut/handler.rs 中转写热键调用 TranscriptionCoordinator::send_input；该方法只发送 Command::Input(InputEvent)，不会创建带 target/session 的 VoiceRequest。

已有 Inputia 菜单链确实存在：Command::Voice/VoiceCommand::Start 创建 OwnedVoiceSession、设置 active_voice；actions.rs 根据 voice_output_context 冻结输出归属，owned 分支准备共享历史结果后返回，不再落入平台 paste。这是已实现的分流，不能错误地报告为不存在，也不能仅因代码存在便称真实原框闭环已通过。

最短待接线点是从唯一语音快捷键取得并使用实际 Inputia 目标会话，而不是再注册第二套快捷键或新建协议栈。接线前需要新签名候选的真实菜单录音，确认当前 TextEdit 目标观察与投递边界，避免把旧的 Codex field_unobservable 当作新 TextEdit 证据。

## 当前真实实验

已请用户在该空白文稿中，从 Inputia 系统菜单点“语音输入”，说合成测试句“这是 Inputia 原位置测试”，再点一次停止，不切换窗口。工具没有直接注入 VoiceCommand、没有手工写入最终转写文字、没有以浏览器 mock 替代入口。

截至本记录，基线仍是8个 owned sessions，尚无本轮菜单录音结果；需要收到这次实际操作后核对新 session、TextEdit 来源、field/target 身份、历史记录以及实际编辑区。之后另做切换目标时保留待插入的实验。当前完整目标未完成。
