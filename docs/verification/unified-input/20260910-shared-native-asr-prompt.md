# 已确认词接入owned batch本地识别

实际调用从actions停止时冻结的owned_voice进入TranscriptionManager::transcribe_with_voice_context。普通transcribe、文件CLI和历史重转写仍传None，不从全局协调器猜测另一个会话。

支持范围为本地Whisper InitialPrompt与支持Qwen context的引擎。通过现有session_hotwords读取快照，epoch/generation必须与本会话一致，提交前再确认服务当前版本、Secure Input及从获取开始计算的2秒时限。来源App/字段缺失、敏感应用、版本/服务失败或超时不附加共享词；保留原有显式提示，不改变输出所有者。

native_prompt_words与settings.custom_words分离。共享词不写入后者、不进入apply_custom_words或远程/后处理。Qwen不支持原生context仍走原显式词行为；原生返回后的上下文回显清理沿用已有路径。提交日志仅含prompt词数、epoch/generation，在原生调用返回后记录，避免日志I/O插入租约校验与提交之间；不输出词表或转写正文。

Host已有capture先拒绝SecureInput和AXSecureTextField、主控制器检查敏感应用及窗口标题。本接口只信任由协调器接纳的owned开始请求，不依据任意field字符串给外部调用授权。

验证：转写模块25项测试通过（含4项新增隐私/版本/TTL/提示与纠错隔离）；app库480通过2忽略。日志 `/tmp/inputia-shared-native-prompt-tests.log`、`/tmp/inputia-shared-native-prompt-app-tests.log`。测试为代码与合成上下文，不是实际模型识别质量或原生录音证据。

未完成：流式识别仍使用原路径；Inputia候选尚未消费规范共享词；本批未构建安装，未执行真实owned录音或A11共享词质量重测。不得将此次batch接线宣称为完整P4。

独立首审发现旧Qwen日志会在最终租约检查前输出词集合的稳定version摘要。已统一移除两条Qwen日志中的version及plan.version()调用，仅留词数；提交后日志只含词数/策略epoch/generation，不记录可枚举词集合指纹。转写模块25项重跑通过，未通过关闭日志门禁或减弱策略检查解决。

流式入口已定位：actions::start_stream在录音开始前开启后台worker，最终调用session.stream(&RunOptions, &StreamOptions)。当前没有传owned上下文或原生词提示。后续应沿同一会话接入并处理长流中的撤销/租约，而非关闭流式以改走batch。

隐私修复复审已Approve。原QwenContextPlan的version字段/摘要生成及getter只有上述日志使用，确认无实际消费者后移除；保留原生context与word_count，相关过滤/回显测试保持。最终app库480通过2忽略，日志 `/tmp/inputia-shared-native-prompt-final-tests.log`。全app严格clippy仍有既有lint债；只有runtime严格clippy已通过，不混称整体质量门禁通过。
