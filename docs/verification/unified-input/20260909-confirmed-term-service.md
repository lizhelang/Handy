# 确认词库服务接线检查点

范围：等待实体 Shift 诊断期间，接续批准方案 P3 的已观察缺口。此前 `get_unified_terms` 只读，`update_custom_words` 仅写语音设置，应用层未调用统一快照。未修改用户设置、未安装新控制中心。

本批复用现有 HistoryService 唯一后台写入者，暴露 `contribute_term` 和 `forget_term`，不创建另一份词库。两条入口先撤销旧输出许可、同步源，再调用已有规范库事务；源修订/隐私校验、贡献重放判定、遗忘屏障与 epoch/generation 继续由既有 store/ledger 执行。

隔离 SQLite + 真实服务队列实验 `confirmed_term_queue_replays_once_and_forget_revokes_snapshot` 通过：首次贡献 Applied；相同 contribution_id 重放 Replay；贡献数仍为 1；遗忘推进 epoch 到 2、清空词条并使旧快照失效；旧 epoch 遗忘与旧贡献重放均拒绝。另运行 `store_learning` 六项既有事务/删除/修订回归通过。

验证边界：测试内部创建合成来源，不代表来自真实 UI 的确认；尚未开放前端写命令，不允许客户端把未知来源伪称 verified，也未把普通输入全文送入学习。确认入口、删除/遗忘语义呈现、版本化快照到 Inputia/Rime 的消费和实际 ASR 接线仍未完成。

下一步必须接真实控制中心操作及服务端派生的来源/隐私上下文，再接输入法快照；不能绕过规范库来源验证或改用旧 `inputia_session_learn` 来冒充统一词库。

## 独立审查后的回执修复

审查发现遗忘提交后通知回调阻塞可能让 caller 超时，旧接口没有可靠重试回执。现服务入口要求稳定 operation_id，新增 `learning_forget_receipts` 与遗忘、策略 epoch 在同一事务提交。只保存键控摘要和结果 epoch，不再保存一份词条正文。

相同 operation_id 和参数返回原提交 epoch，即使服务已经重启；同 ID 换参数拒绝。超时明确返回 outcome unknown，必须复用原 ID/参数，不换 ID 重试。回执中的 epoch 是历史操作结果，不能代替读取当前策略。

新验证通过：阻塞 changed 回调直到五秒超时，解除后原 ID 重试得到 epoch 2，重启服务后仍返回同一结果且当前 epoch 不再次推进；回执 INSERT 注入故障时词条删除和 epoch 全部回滚。现有旧 epoch 拒绝规则仍用于不同操作 ID。完整 runtime all-targets 回归通过，日志 `/tmp/inputia-term-receipts-regression.log`。

另修复可复现构建缺口：根目录 `rust-toolchain.toml` 固定本次实际使用的 Rust 1.96.0，runtime 声明最低 1.89（File::try_lock 的版本要求）；未改变全局 rustup 默认值。

回执修改尚待独立复核结论。前端命令尚未开放；实际用户配置/数据库未迁移，不能将服务级实验写成产品入口或兼容回滚验收通过。
