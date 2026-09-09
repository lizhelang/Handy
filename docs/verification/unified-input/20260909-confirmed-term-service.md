# 确认词库服务接线检查点

范围：等待实体 Shift 诊断期间，接续批准方案 P3 的已观察缺口。此前 `get_unified_terms` 只读，`update_custom_words` 仅写语音设置，应用层未调用统一快照。未修改用户设置、未安装新控制中心。

本批复用现有 HistoryService 唯一后台写入者，暴露 `contribute_term` 和 `forget_term`，不创建另一份词库。两条入口先撤销旧输出许可、同步源，再调用已有规范库事务；源修订/隐私校验、贡献重放判定、遗忘屏障与 epoch/generation 继续由既有 store/ledger 执行。

隔离 SQLite + 真实服务队列实验 `confirmed_term_queue_replays_once_and_forget_revokes_snapshot` 通过：首次贡献 Applied；相同 contribution_id 重放 Replay；贡献数仍为 1；遗忘推进 epoch 到 2、清空词条并使旧快照失效；旧 epoch 遗忘与旧贡献重放均拒绝。另运行 `store_learning` 六项既有事务/删除/修订回归通过。

验证边界：测试内部创建合成来源，不代表来自真实 UI 的确认；尚未开放前端写命令，不允许客户端把未知来源伪称 verified，也未把普通输入全文送入学习。确认入口、删除/遗忘语义呈现、版本化快照到 Inputia/Rime 的消费和实际 ASR 接线仍未完成。

下一步必须接真实控制中心操作及服务端派生的来源/隐私上下文，再接输入法快照；不能绕过规范库来源验证或改用旧 `inputia_session_learn` 来冒充统一词库。
