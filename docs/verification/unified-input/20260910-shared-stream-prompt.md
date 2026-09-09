# 流式识别消费共享词并在失效时回退

接续4439c251的批量共享提示。actions开始时从已发布的协调器归属捕获owned请求，交给stream worker；主按键路径不等待词库IPC。worker确认实际模型支持流式及Whisper InitialPrompt/Qwen context后才取共享提示，普通无owned调用不读取共享词。

开始前检查当前双版本、Secure Input和2秒时限。Feed/Finalize前后检查，最多每500ms在线复核同一快照并续租；验证超过2秒、服务错误、版本变化或SecureInput开启则不续租。没有原生family提示时不启用共享续租/提交日志。

失效Feed会reset/drop流、清预览、归还引擎，再使用原drain_until_finalize完成一次None回执，actions回退批量识别。已经取得Finalize的分支直接对该sender回None，不能等第二次Finalize。取消无输出。recorder在回调router.feed之前已将VAD处理后的片段追加到processed_samples，所以回退使用现有整段录音缓冲，不丢已送入流的音频；这不是宣称保留未经VAD的全部原始采样。

Qwen预览先清理完整文本再拆回committed/tentative，避免context回显跨分段漏清；最终结果沿用同样清理。不改强制纠错词表/远程词表或输出所有权。提交日志仅原生调用后记录词数与策略版本，无词正文或稳定词表hash。

验证：转写模块31项、最后流式专项6项通过；完整app库486通过2忽略，日志 `/tmp/inputia-shared-stream-app-tests.log`；格式/diff检查通过。依据固定依赖18718a4的Rust session.stream实现核查RunOptions原生family入口，未猜测新SDK接口。

尚未安装本批；独立审查进行中。真实长流、录制中遗忘/断线、原生质量及Inputia候选消费仍未验证，不能据单元测试称P4或语音闭环完成。

独立审查已完成：review_socket_path 为Approve，无阻塞发现；复核了已捕获Finalize只回一次None、失效Feed的引擎归还/排空以及仍由原owned输出路径处理。另跑shared_stream6项、转写31项、cargo check和diff检查通过。完整app严格静态门禁仍有既有债，不据此宣称全门禁完成。

## 已进入候选安装

代码b16acae5完成固定本地证书签名构建，日志 `/tmp/inputia-shared-asr-candidate.log`。已更新 `/Applications/Inputia Candidate.app`，实际PID46475运行身份匹配CDHash `c690875f912e6f30f96aabab155458ce1437b869`；设置窗口和listener正常启动。IME59/PID37732保持运行，未替换输入法包。

旧候选包和旧/新配对保留在 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/shared-asr-candidate-20260910.jLhcK4`。本次不涉及schema变更，原库仍schema4、5条历史；日常安装未动。未启动麦克风、未新增识别质量成绩、未notarize。原生共享词与长流失效验收仍未完成。
