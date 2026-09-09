# 逐词确认的跨版本回执恢复

接续 `d9dec83d` 的独立审查发现：旧app重试重新读取epoch，与首次成功贡献的摘要不同，会永久冲突。首次确认入口尚未安装，因此没有用户操作需要从该旧入口迁移。

## 修复合同

- 复用规范学习账本，仅新增确认操作回执表，不另建学习计数系统。摘要绑定操作ID、历史item ID、请求修订与规范短词，使用已有私有密钥摘要，不存明文词或公开词散列。
- 回执与贡献写入在同一事务。先查原操作回执，存在则仅返回历史Replay，不再要求源仍存在或epoch未变；不同载荷同操作ID冲突。原contribute_term的epoch/遗忘规则不变。
- 服务在同步前即可查已存在回执，运行中源同步失败不妨碍恢复已有结果。没有回执才同步、读取真实来源/修订/epoch，并在事务写入前调用确认guard。
- 新写仍要求纯文本、Verified来源、短词过滤、非敏感来源；只授权逐词本地确认，history=false、remote=false、explicit_relearn=false。
- app移除自己拼装ContributionInput的重复逻辑，直接调用HistoryService::confirm_history_term；对应数据库验证移至runtime真实事务测试。
- 前端Replay显示“这次确认操作已处理，不会再次学习。之后的删除或遗忘仍然有效”，不谎称词当前仍有效。

## 验证与限制

定向confirmation实验覆盖：忘其他词后重试、忘同词后重启重试、删除源后重试、异参同ID、错误密钥、回执插入失败回滚、guard拒绝、非纯文本/不可信来源/无效短词，以及运行中同步失败后恢复回执。

日志：`/tmp/inputia-confirm-receipt-runtime-tests.log`（runtime完整回归）、`/tmp/inputia-confirm-receipt-final-app.log`（app库回归）、`/tmp/inputia-confirm-receipt-ui-tests.log`（19项历史及确认组件交互）。前端build/lint/翻译键检查日志同前缀。原生确认、Inputia/ASR共同消费和语音闭环仍未验证，未安装本批候选。

严格clippy仍报告原有 voice_protocol.rs 的 HostShortcutReply large_enum_variant，未用allow抑制，也未将质量门禁标为全通过。它与本批回执修复分开处理。

独立复审 `review_socket_path` 已完成：Approve，无阻塞发现；复核了原子事务、异参冲突、删忘后Replay不复活和当前有效性文案。复审另跑runtime/app编译、store_learning及confirmation测试、前端build/lint/翻译与确认交互通过。最终完整runtime回归通过，app库474通过2忽略，历史/确认交互19通过，格式检查通过。未据此宣称原生确认或共同消费通过。
