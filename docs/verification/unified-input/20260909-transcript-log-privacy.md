# 转写完成日志的调试构建脱敏

实际调用点仅两处：`actions.rs` 与 `managers/transcription.rs` 的转写完成日志。旧 `utils::redact_text` 在 debug_assertions 下返回原文；发布构建已经返回 `[REDACTED]`。

新增合成回归在修改前失败（退出101，`/tmp/inputia-redaction-before.log`），实际返回 synthetic-private-transcript。现删除构建模式分支，统一返回脱敏标记；utils四项测试、格式及diff检查通过（`/tmp/inputia-redaction-after.log`）。独立只读复核Approve，确认原文仍独立用于后处理、返回、历史和插入。

范围仅限这两个完成日志通道，不声称全仓日志审计完成，不删除历史日志。当前安装是发布构建，本来已脱敏，因此本批不重装、不录音，也不新增原生闭环通过项。
