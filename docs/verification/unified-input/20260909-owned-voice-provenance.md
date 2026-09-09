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

## 候选历史真实副本的双升级

通过SQLite只读backup取得 `/private/tmp/inputia-source-upgrade-20260909.Eub8Qy/history-copy.db`，随后仅对副本运行仓库 `source_upgrade_probe`。输出：schema4、rows=5、repetitions=2；原9字段摘要、store ID、outbox事件数、业务user_version不变，旧来源全unknown；quick_check=ok。原候选history.db复查仍为schema3，未修改。

命令：`cargo run --manifest-path crates/inputia-handy-runtime/Cargo.toml --example source_upgrade_probe -- /private/tmp/inputia-source-upgrade-20260909.Eub8Qy/history-copy.db`。结果日志同目录 result.log，输出不含正文。首次传入/tmp别名被canonical路径保护拒绝，随后使用真实/private/tmp路径，未放宽检查。

这是实际候选历史副本的结构兼容证据，不涵盖新增记录后兼容回滚、录音附件、旧Host重新导入或完整A12；不能从5条记录推断大数据性能。

## 构建完成，未替换当前安装

代码 `8870df79` 的候选构建完成，日志 `/tmp/inputia-provenance-candidate.log`，固定本地证书签名deep/strict校验通过，CDHash `a6ba91411d5f94e164ef1b054ae032919de4d5e0`。独立保留产物 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/provenance-candidate-20260909.ZfTOxZ/Inputia Candidate.app`，未notarize。

安装暂缓：实际旧代码 `9fa26dd3` 的 SourceOutbox::install只接受schema1–3，不能把旧包直接作为schema4恢复构建。需先准备相应兼容恢复构建/验证，不用覆盖旧数据库代替。当前已安装候选历史库复查仍schema3；未更换安装、未重新配对、未启动录音。
