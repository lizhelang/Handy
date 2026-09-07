# 语音结果持久关联检查点

接续 `c75733a4`；目标仍未完成，未开放产品socket或触发真实录音/上屏。

`IntegrationStore::prepare_voice_result` 将已claim的同一语音session与一个固定历史修订、原目标和IME输出操作在同一事务绑定。新关联表只保存session/operation ID，不排队复制正文。操作ID由session稳定派生，输出账本继续约束owner/action/item/revision/policy，重复准备不能切换执行路径。

同一事务核验：原服务/Host归属、session未退役或取消/失败、中间事实存在、Start已取得执行资格、源类型为语音、历史仍存在且修订与当前策略有效。此方法不取得派发资格，不能单凭准备成功插入文字。HistoryService适配在准备前同步源变更；实际pipeline调用及Host交付后续接入。

实际验证：

- 5项真实SQLite测试通过：100轮重复只一个IME操作；换item/修订拒绝；关联插入SQL故障连同output准备回滚；换对端/策略/取消/非语音源拒绝；崩溃后已claim输出变Uncertain且不给第二资格；未claim Start/已删历史不能创建结果或输出。
- 本轮runtime完整测试111项通过，0失败/0忽略，日志 `/tmp/handy-runtime-voice-result-20260907.log`。严格Clippy与格式检查通过。
- 表内没有新增转写正文；删除后的旧关联不能通过当前内容校验重新输出。尚未执行完整兼容恢复和删除遗忘全链路演练，不能由这些单库测试宣称A08/A12通过。

下一接线点：保存历史后以确切源记录/修订核对本次文本，准备该固定结果；Coordinator投影记录item/operation及待插入状态；Handy后台只对认证Host派发该操作，Host持久claim和主线程目标检查后报告事实。任何未知回执不得改为平台paste。以上尚未完成，不应安装本次中间包作为日常融合版本。
