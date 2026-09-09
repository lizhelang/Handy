# 新认证录音的来源保存接线

接续 `d2347010` 的真实阻塞：历史投影固定unknown，确认入口无真实可学习来源。本轮未更新安装或修改真实数据。

## 实现路径

`TranscriptionCoordinator.voice_output_context` 的已接纳会话 → actions成功转写保存 → VerifiedVoiceSource（Start/首次HostShortcut、有字段和来源App、非敏感应用）→ HistoryManager单次INSERT正文及来源 → SourceOutbox → 统一索引 → 用户逐词确认。

来源标记不是前端可提交字段，不接受普通CLI转写或来源不明的录音自报Verified；从内部已认证会话读取的是录音开始时的来源上下文，不把识别文字本身当作已确认术语。用户仍须逐词确认才能学习。

SourceOutbox schema4为语音表添加inputia_source_app/trust，旧行默认unknown。旧outbox负载、源store ID、事件计数、注解和业务user_version保持；clipboard仍unknown。列及触发器在同一事务升级，安装幂等；app启动按SOURCE_SCHEMA_VERSION检查并通过既有备份迁移函数升级。正文和来源同一INSERT只生成一个完整事件，不先发unknown再覆盖。

## 证据

- source_outbox 14项通过：schema1/2/3升级、schema3双升级、旧unknown、新verified outbox/快照、故障时DDL/trigger/meta回滚。
- app库回归476通过2忽略，日志 `/tmp/inputia-voice-source-app-tests.log`；原fixture无列名INSERT因新增列失败，已改显式原字段，不放宽断言。
- `/tmp/inputia-voice-source-chain.log` 验证生产历史写入器产生事件、正文与来源共同回滚、一个源事件进入规范索引并完成一次确认；不是手工向索引塞Verified。
- runtime完整回归通过，日志 `/tmp/inputia-voice-source-runtime-tests.log`。

## 未完成边界

以上是隔离数据库与生产函数调用链，不是实际录音的原生来源证据。用户未确认方便录音，未启动麦克风。现有旧记录/剪贴板仍不允许提升来源trust；共享词库消费、旧导入替换与兼容回滚仍未完成。

schema4需要相应兼容构建，旧schema3代码不能被假定可以直接回滚启动。安装前必须保留数据副本并验证兼容路径，不能仅恢复旧数据库来抹去新增记录或遗忘屏障。

独立复审 `review_socket_path`：Approve，无阻塞发现。另跑source_outbox14项、history6项、认证/策略/断线定向测试、cargo check与双crate格式检查通过。确认信任由已接纳认证会话派生，而不是仅有一个目标字符串就授权；未据此宣称实际新录音已原生验证。
