# 从真实历史确认本地术语：首次接线

接续 `62df8e0e`。本批从主历史预览接到已有规范学习服务，不再增加独立学习账本。

## 实现

- ConfirmHistoryTerm：文本历史项内手动输入短词、逐词勾选本地授权；不预填全文。失败重试保留相同正文对应的operation ID，编辑术语取消勾选，成功后不重复提交。
- confirm_unified_history_term：只接聚焦的main窗口、明确同意和非Secure Input；后端读取真实item/revision/source和策略版本，使用现有短词/敏感模式过滤。Unknown/Observed不提升为Verified，rich/image/file拒绝，explicit_relearn=false、remote=false。
- 直接调用现有HistoryService::contribute_term及规范事务；没有把“同步历史”误当学习。新增仅为已有仓库inputia-core的直接路径依赖，不引入外部库。
- 成功文案只声称本地词库保存，明确共同消费仍在接入；不冒充已经影响输入法候选或识别质量。

## 验证

- `/tmp/inputia-confirm-term-backend.log`：5项通过。包括真实合成SQLite贡献、同请求重放仅一份贡献、忘记后重启不复活，以及来源/过滤/确认门槛。
- `/tmp/inputia-confirm-term-ui.log`：逐词勾选、初始空输入、相同操作重试、编辑重新确认通过。组件截图 `test-results/confirm-history-term.png`，文字与控件可见；不是原生UI验收。
- 前端build、lint、翻译键检查通过。非中英文新增文案使用英文回退，不称逐语言翻译完成。绑定通过export-bindings生成，未手写。
- 独立审查 `review_confirm_term_entry`：未发现提权、远程授权、重复计数或遗忘复活；发现下述重试缺口。

## 必须接续的缺口

若首次贡献成功却回执丢失，随后其他遗忘操作推进epoch，重试重新读取epoch会与原贡献digest冲突。必须补原操作回执查询或固定首次授权载荷，不能去掉epoch检查。这项尚未修复，故本批不安装为已验收词库功能。

焦点与Secure Input检查目前在入口和入队前，实际服务会先同步再写入；如需提交时门槛，应在writer写入前重核验。旧导入重复学习、旧数据来源迁移、Inputia与ASR规范快照消费、原生确认和语音闭环均未因此完成。
