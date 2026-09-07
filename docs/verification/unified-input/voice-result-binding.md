# 语音结果持久关联检查点

接续 `c75733a4`；目标仍未完成，未开放产品socket或触发真实录音/上屏。

`IntegrationStore::prepare_voice_result` 将已claim的同一语音session与一个固定历史修订、原目标和IME输出操作在同一事务绑定。新关联表只保存session/operation ID，不排队复制正文。操作ID由session稳定派生，输出账本继续约束owner/action/item/revision/policy，重复准备不能切换执行路径。

同一事务核验：原服务/Host归属、session未退役或取消/失败、中间事实存在、Start已取得执行资格、源类型为语音、历史仍存在且修订与当前策略有效。此方法不取得派发资格，不能单凭准备成功插入文字。HistoryService适配在准备前同步源变更；实际pipeline调用及Host交付后续接入。

实际验证：

- 5项真实SQLite测试通过：100轮重复只一个IME操作；换item/修订拒绝；关联插入SQL故障连同output准备回滚；换对端/策略/取消/非语音源拒绝；崩溃后已claim输出变Uncertain且不给第二资格；未claim Start/已删历史不能创建结果或输出。
- 本轮runtime完整测试111项通过，0失败/0忽略，日志 `/tmp/handy-runtime-voice-result-20260907.log`。严格Clippy与格式检查通过。
- 表内没有新增转写正文；删除后的旧关联不能通过当前内容校验重新输出。尚未执行完整兼容恢复和删除遗忘全链路演练，不能由这些单库测试宣称A08/A12通过。

下一接线点：保存历史后以确切源记录/修订核对本次文本，准备该固定结果；Coordinator投影记录item/operation及待插入状态；Handy后台只对认证Host派发该操作，Host持久claim和主线程目标检查后报告事实。任何未知回执不得改为平台paste。以上尚未完成，不应安装本次中间包作为日常融合版本。

后续协调器接线：新增异步 `notify_voice_result_prepared`，核对完整原Start身份、当前Processing及未取消状态，再记录固定item/operation为PendingTarget。重复相同通知不升代数；替换结果、外来身份、取消后迟到结果拒绝。正常FinishGuard不再把已准备结果覆盖为Interrupted，旧结果回执不改变下一会话。协调器66项定向测试通过，日志 `/tmp/handy-coordinator-result-20260907.log`；实际pipeline调用与Host交付仍未接入。

生产pipeline追加：Stop时冻结owned Start身份；成功转写保存后通过`prepare_saved_voice_result`按本次history ID定位，并核对当前投影文字。随后在阻塞任务池准备固定IME输出、通知协调器并持久化PendingTarget事实；任何准备失败都不进入旧平台paste分支。普通非owned语音仍保留原平台路径。真实Host接收/claim/目标校验/上屏仍未实现，因此不得启用或宣称完整IME语音可用。

新增真实HistoryService+两个源SQLite测试通过：较新记录不会替代本次ID；错误文字/错误记录拒绝；准备后源被改，旧文字重试不能换新修订。语音结果专项现为6项通过，严格Handy Clippy通过。尚需补原生录音到Host的端到端故障验证、保存失败时完整待处理UI，以及独立接线审查。

整库回归原运行422通过/15失败/2忽略，失败集中在本地HTTP请求连接提前关闭，原日志`/tmp/handy-owned-result-pipeline-20260907.log`保留。仅对测试进程加`NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost`重跑全套，437通过/0失败/2既有忽略，日志`/tmp/handy-owned-result-loopback-direct-20260907.log`。这是回环直连条件下的实际完整重跑，不删除测试、不改系统代理，也不据此宣称已经定位所有系统网络配置。

身份绑定追加：认证连接构造现在必须把VerifiedPeer中的内核audit token与握手client_instance在唯一HistoryService事务内绑定，不能从请求正文提供audit。绑定跨服务重启保留，同实例不同进程拒绝；数据库故障不授予资格。最多16384个身份且不驱逐旧绑定，达到上限明确拒绝新身份，不影响基础键盘；正式产品仍须提供对应容量诊断/维护。store_voice专项5项通过（包含新身份重启/故障测试），Handy严格Clippy通过。实际socket握手仍未接通，不能把此持久层测试当作端到端认证成功。

连接入口追加：`VoiceConnection::accept`消费后台socket，先执行真实原生认证，再读握手；协议/profile合法后持久绑定实例，最后写Accepted。任何失败关闭该socket，连接context尚未确认策略，不授予Start。新增`server_handshake_checked`在成功回包前提供绑定门禁，原基础传输API保留。16项实际UDS传输测试通过，包含绑定拒绝不得出现Accepted及错误profile不调用绑定；Handy严格Clippy通过。此处尚无产品listener/client、策略同步协议或真实两端连线证据。Security与数据库认证阶段的整体准备时限仍需服务生命周期接线时统一约束，不能把帧的2秒deadline称作整条认证流程已测2秒。

词库版本门禁追加：Store现在在Start准备与claim事务内同时核验`learning_generation`，不只比较policy epoch；提供同一只读事务返回策略/词库版本用于后续同步。准备后词库变化会回滚claim，未来版本或损坏版本不会留下新session；Stop/Cancel不受词库版本变化阻挡。store_voice专项7项通过，严格runtime Clippy通过。实际确认词库→模型提示与Host同步仍未完成，这不是A11识别质量证据。

连接策略同步追加：服务端`VoiceConnection::synchronize_policy`先撤销旧Start授权，发送OS随机32字节唯一屏障ID及当前双版本；只接受同ID/版本且明确报告共享缓存已清除、离线队列已重核验的回执，再读服务当前版本确认同步期间没有撤销。失败关闭socket，不沿用旧授权；屏障不含词库正文。voice_protocol专项7项通过，包含跨连接旧回执、缺少任一清理事实、双版本变化拒绝；Handy严格Clippy通过。Host执行清理/落盘/回执和真实socket双端验证仍未实现，不能把协议测试描述为输入法已清除个性化。
