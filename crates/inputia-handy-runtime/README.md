# Inputia Handy Runtime

这个 crate 连接 Handy 本地源库、Inputia 统一历史、输出账本与附件存储，不依赖 Tauri。

## 存储与写入边界

- `history.db`、`clipboard.db` 是业务源库。源变更与 outbox、操作回执在同一事务提交。
- `integration.db` 保存统一投影、保留修订、输出结果、删除日志和附件引用。`HistoryService` 的唯一后台写入线程持有写者锁。
- `inputia_memory.db` 是既有本地记忆库。`import_probe` 等兼容入口仍以只读方式读取 Handy 源数据；这不代表整个 runtime 只读。
- manager 修改源库前须持有 `SourceWriteGuard`。它撤销旧输出许可，并与物理 GC 的排他守卫互斥。GC 只尝试取得守卫，不能等待持守卫的 manager 再回调服务，避免互锁。

正文删除经过持久 `requested → source_applied → projection_revoked` 阶段。启动恢复核验源事务回执与投影撤销证据；拒绝回执保留原结果，同一操作不会因后续修订变化而重新删除。`projection_revoked` 只确认源记录及统一投影撤销，不等于附件或其他学习域全部遗忘。

## 附件导入与引用

受管目录为数据根下的 `recordings/` 和 `clipboard_images/`。录音及 PNG 写入已从 manager 的直接文件写入改为 `AttachmentStore`：

1. 持久记录导入操作 ID、摘要、临时文件名和空间预留。
2. 使用独占临时文件，写入并 `fsync`，验证 WAV/PNG 格式与摘要。
3. 在同卷上用禁止覆盖的原子硬链接发布，移除已登记的临时链接并同步目录；允许的双链接状态只限该操作的恢复中间态。
4. 在事务里登记物理文件身份；同内容并发导入汇合到同一物理代际，分别保留各自尚未结算的导入保护。
5. 源记录提交后，持久引用绑定 `store_id / record_id / revision`。当前引用与保留修订都会阻止回收。

文件名由内容摘要确定，`attachment_id` 另含物理代际。文件回收后再次导入相同内容会得到新身份，旧租约不能指向新文件。已存在的目标文件必须通过验证，不能覆盖。

`PendingAttachmentImport` 持有至源记录保存或调用放弃，析构时通知后台结算。重启完成源引用审计后，旧进程已关联的导入转为 `attached`，未关联的已发布导入才可安排回收。旧预留只有在源无引用、临时与目标路径均确认不存在时才能退款；损坏或身份不明的 staging 保留为可查询失败，不能以成功清理代替。

识别正文独立于可选 WAV。导入失败仍保存正文，以空 `file_name` 表示没有录音，并记录 `InsufficientSpace`、`InvalidFormat`、`IoFailure` 等附件原因。磁盘已满到连 SQLite 正文都无法提交时，不能保证保存成功；不会伪造附件或绕过输出账本粘贴。

旧附件只依据源记录和保留修订中的精确路径审计登记，不扫描整个目录推断孤儿。外部文件只登记不透明引用、只去引用，不读取其内容或删除原文件。

## 活动租约与维护保护

播放和重转录按源记录及可选预期修订获取租约，返回固定的文件身份与修订。之后修改源记录不会改变已取得租约所钉住的文件。重转录从经过摘要与身份校验的同一文件读取字节，按取得录音时的修订执行更新 CAS。

Tauri 播放接口是：

- `acquire_history_attachment({ id, expectedRevision, operationId }) → { lease, path, revision }`。
- `release_history_attachment({ lease })`：匹配进程实例、附件身份及用途；重复释放幂等，错误可重试。
- `release_history_attachment_operation({ operationId })`：持久取消当前实例的活动获取操作，防止超时或卸载后的迟到请求重新建立租约。

`expectedRevision: null` 表示首次获取时使用当前修订。操作超时不表示未执行，重试必须沿用原操作 ID。旧 `get_audio_file_path` 兼容入口使用保守的进程租约；新音频组件使用显式获取和释放。

`Active` 租约在新进程完成源审计后回收。`Export`、`Update` 事务 pin 不能仅因进程退出而清除，必须显式结算。当前提供这些 pin 的真实能力，没有虚构不存在的导出流程。

物理 GC 还复用 `inputia-settings::maintenance` 的共享门禁，通过系统用户目录检查固定 `Library/Application Support/Inputia/Updater/maintenance.json`。只有明确不存在标记才放行；存在、损坏、权限或链接异常、无法检查均暂停。更新事务的完整调度由 UpdateService 接入，不能把附件层已有 pin 能力说成更新流程已完成。

## 删除与 GC 状态

源删除的成功回执在同一次源修订 CAS 事务中保存附件清单。统一侧同时保存保留修订的原始附件清单、摘要、物理代际映射和失败原因；投影删除后仍可恢复核查。

| `attachment_cleanup` | 含义                                                                           |
| -------------------- | ------------------------------------------------------------------------------ |
| `not_started`        | 尚未建立附件处理证据。                                                         |
| `scheduled`          | 该 owner 引用已撤销，待处理文件仍受共享引用、租约及维护门禁约束。              |
| `completed`          | 该操作涉及的受管物理代际已回收，或文件仍有其他合法引用；外部文件仅完成去引用。 |
| `blocked`            | 缺少旧清单、文件身份变化或验证失败，不能宣称附件处理完成。                     |

`completed` 不是“其他 owner 的共享文件也被删除”。旧成功回执没有附件清单时记录 `LegacyManifestUnknown`；它不永久阻断正文读取，也不据此删除未知文件。

所有历史及剪贴板单删、清空、保留策略清理都通过相同的耐久源删除路径，生产 manager 不再直接 `unlink`。GC 在真实排他门禁、完整引用审计和 pin 检查后，每次处理一个已登记文件：持久安排隔离、验证文件身份与摘要、删除、同步目录，再确认完成。轮转游标避免坏文件饿死后续项。单改阶段或删掉清单 IDs 不能充当完成证据；已缺失且没有最终 unlink 回执的文件保持未完成。

Unix 检查包含受管根/子目录归属及权限、单链接常规文件、禁止符号链接和物理身份比较。实现没有声称可以抵御同 UID 攻击者任意并发替换目录的所有情形。

## 容量、暂停与查询

`AttachmentBudget` 默认容量 4 GiB、单文件上限 256 MiB、最低保留可用空间 512 MiB。容量计算受管唯一文件与尚未结束的 staging 预留；磁盘可用空间是预检，最终写盘仍处理实际 ENOSPC。

服务提供：

- `configure_attachment_budget`：修改持久预算。
- `pause_attachment_gc`：设置持久暂停原因；当前同一暂停也阻止新附件导入。
- `attachment_health`：占用、容量、暂停原因、待回收数、失败导入数、受阻删除数。
- `attachment_import_status(operation_id)` / `attachment_deletion_status(operation_id)`：查询原操作结果，不返回正文。
- `acquire_attachment_maintenance` / `release_attachment`：创建和结算全局维护 pin。

这些是后端能力，尚未提供完整容量设置与附件修复 UI。历史条目保留策略已接统一删除；历史 SQLite 正文、模型、索引和备份的独立字节预算尚未纳入该容量计算。

## 当前验收边界

- 新导入的编码、完整格式/摘要校验、发布，以及主动租约读取的文件摘要工作在调用者阻塞任务中执行，数据库队列保存元数据。
- 旧资源首次审计、启动导入恢复及每轮一个文件的物理 GC 仍可能在唯一服务线程执行文件读取/校验。大库或大文件会增加请求延迟；当前 5 秒请求超时不取消已排队操作，需按同一操作 ID 查询或重试。尚未完成分批异步审计与最大队列延迟验收。
- 物理文件归属、链接与空间检查当前仅在 Unix 实现；非 Unix 保守返回 typed 暂停，不能宣称 Windows 附件功能已完成迁移验收。
- 未做已安装应用的真实音频设备、系统播放或更新现场验收。自动测试使用临时目录与 fixture 数据库。

运行测试：

```bash
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --offline
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --offline --lib attachment_
```

只读导入示例仍可通过 `examples/import_probe.rs` 使用单独的探针输出库；不要把探针目标指向实际用户库。

## 跨域真正遗忘与 Host 撤销

`HistoryService::begin_privacy` 是遗忘操作的协调入口。`PrivacyRequest` 包含
`operation_id`、`expected_epoch` 和 `forget_term` / `clear_learned` 范围；旧的
`HistoryService::forget_term` 委托此入口，返回的 epoch 只表示已接受。
普通候选抑制（reject/undo）仍保持原有上下文语义，不冒充跨域遗忘。

- 接受事务在 `integration.db` 同时撤销共享贡献、保存重放屏障与域回执、推进一次全局 epoch，
  并写入 `PrivacyOperation` 的必需域集合。独立数据库分别提交，由真实回执恢复进度。
- 个性化域独立保存 `operation_id + HMAC 请求摘要 → 本域 epoch` 回执；域提交后、协调日志更新前
  崩溃可幂等恢复。失败保留已完成进度，后台每 250 ms 尝试恢复。
- 恢复载荷用既有学习密钥 AES-256-GCM 加密，完成后清除；操作摘要使用独立域前缀 HMAC。
  个性化旧明文遗忘标记迁移成本域密钥 HMAC，保留旧事件拒绝回执，不再次保存被遗忘正文。
- 启动每页核验最多 32 条终态的所有必需域独立回执。审计未完成时不发布“完成”；
  已完成操作缺少域回执时标记部分失败并暂停学习，不重新删除后来新写入的证据。
- Host 每秒刷新个性化策略，个人候选租约最长两秒；共享词条原有最长一秒租约也登记到协调服务。
  在线回复携带现有 `VoicePolicyBarrier`，主线程先清理候选、上下文和在途策略票据，再发送 ACK。
  ACK 只结算严格早于该 barrier epoch 的租约，避免同 Host 多连接的迟到 ACK 消除新租约。
  断连不冒充 ACK；服务重启后旧租约从启动时刻再保守等待两秒。

状态为 `accepted`、`processing`、`partial_failure`、`completed`。只有必需域回执及旧读者全部
结算才显示完成。控制中心通过现有 `knowledge_request` 的 `privacy_status`、`privacy_begin`、
`privacy_operation` 查询持久摘要，返回 operation ID、范围种类、epoch、各域状态和原因码，不返回词正文；
超时重试必须复用原操作 ID 与参数。前端会展示启动恢复、完成范围和缺回执的修复提示。

`coverage` 明确区分 `primary_only`（统一与个性化域）、`all_domains`（另含旧派生学习域）和
`legacy_coverage_unresolved`。旧两域完成记录已清除恢复载荷时，不能反推出原词或补造第三域回执；
升级后显示覆盖待修复，不自动执行范围更大的清理。第三域单独缺失交接或覆盖证据时，旧派生功能暂停，
历史正文和基础输入仍可使用；整项操作不显示完成。

此范围移除必需学习库中的学习证据及共享贡献；原始历史、附件、公开基础词典及设置中手动添加的词条
会保留。磁盘备份、已发送给外部模型的上下文和磁盘介质物理擦除不在此操作范围。
当前验收使用临时 SQLite、合成 socket/Swift 状态机与浏览器 IPC 夹具，不代表已安装 Host 的现场验收。

## 旧派生学习域

`HistoryService::start_with_memory` 接收后台解析的固定 profile 路径。`LegacyMemoryContext` 的公开生产入口
目前只允许 `unconfigured` 或 `handoff_required`：没有 NativeAdapter 证明旧写者停止并交接所有权，
不会打开 `inputia_memory.db`，也不会另建空库作为迁移成功证据。当前独占路径只由临时测试夹具构造。

- `memory_query` 在同一个 worker job 内完成 epoch/启动审计/源同步门禁、精确 query 快照和读者登记；
  租约最多两秒。缺域、域替换、运行中 epoch 或 generation 回退均阻止派生读取。
- 计数沿用 Core 的 u32 饱和语义；快照 generation 从 1 开始。业务 tick 独立持久，并继承旧词与事件
  的最高 tick，迁移后新词仍有正确的最近使用顺序。旧计数仅归于一次迁移来源，不伪造历史记录身份。
- `memory_learn` 只接收 Rust 服务构造的 `VerifiedMemoryEvidence`，不接受客户端自报已验证。
  operation、event、源 store/record/revision 都有幂等约束；贡献、事件和回执同事务提交。
- `memory_import` 保留每源最多 2000 条的用户请求，每轮最多读取 128 条，贡献与游标同事务提交。
  同源同修订再次导入不重复计数；任务绑定接受时的源库身份与 epoch，撤权后不换 epoch 自动续作。
- 遗忘屏障使用独立命名空间和固定空白折叠/小写规范的 HMAC。显示文本保留大小写；新操作、新修订或
  手动重导入不能复活已遗忘词。第三域提交后协调日志写入前崩溃，由原回执恢复，不二次删除新数据。
- 源修订/墓碑先撤销旧贡献。删除摘要的 `legacy_memory_cleanup` 独立报告 `pending`、`completed`、
  `blocked`、`coverage_unresolved` 或 `not_configured`；源/投影删除不等于第三域完成。旧迁移计数无法
  证明属于哪条源记录时，报告来源覆盖待修复，不能把未知贡献当作已经清除。

当前实现仍会在独占迁移时读取完整旧词表，并在派生查询前核对全部已登记源贡献；大库性能尚需后续有界
审计/迁移优化。生产所有权交接、实际 Host 输入端到端验收另行完成。上述安全暂停不代表旧学习功能已上线。

### 连续英文的有界读回证明

`memory_word_span::WordSpanRegistry` 在输入之前读取真实折叠光标、字段实例和左右各最多 32 个 UTF-16
单位的锚点。已有英文词中间不能开启新段，也不接受客户端发送既有前缀来补造许可。取得许可后，只记录
严格连续 sequence 的追加与尾部回删；重复 sequence 必须保持原事件。正文上限为 8192 UTF-16 单位、
64 KiB，每个活跃段最多 128 个已结束词、1024 个编辑事件和 32 次确认。

`checkpoint` 核验整段真实读回、两侧锚点、原字段/焦点/光标/文档长度和读取前后稳定性。只有此步骤可
续最长 1.5 秒的许可，重放旧确认不会重新计时。ASCII 字母、数字、下划线和短横线组成词；长度至少为
2 且包含字母，尾部未出现本次新增结束边界的词不产生学习证据。

- `memory_apply_word_span` 按稳定 span 来源和递增修订，在同一事务中替换全部词贡献、事件和操作回执。
  同词不同位置保留各自次数；跨多次 checkpoint 的尾部回删会撤销旧集合，不叠加旧权重。
- 普通确认使用 `word-span:<span_id>:<sequence>`；`finish=true` 使用附加 `:seal` 的操作身份。
  finish 仍须重新完整读回并看到真实闭合尾边界。域的 sealed 状态与最终贡献同事务提交后，调用方才能
  `acknowledge_seal`。sealed 记录是已经证实发生的输入事件，不随普通失焦或空闲到期删除。
- 未封存许可到期、字段/权限/epoch 变化、读回不支持或预算超限时，registry 清除正文并保留有界撤销
  token。后台调用 `memory_revoke_word_span` 幂等执行，再确认 token；不依赖下一次 checkpoint。
  finish 已提交而 ACK 丢失时，域的真实 sealed 状态阻止误删。服务重启先撤销上次实例留下的所有
  unsealed 贡献并留下 tombstone；迟到的旧证据不能恢复它。
- sealed 重试正文缓存也仅存活于原短租约内；隐私撤销立即清除缓存。真正 Forget/Clear 仍清除 sealed
  学习证据。`memory_snapshot_is_current` 只核快照身份、修订与当前来源/隐私门禁，不返回正文或续租。

当前只支持一个有界活跃段内的跨 checkpoint 回删，不支持独立 sealed 段之间的反向编辑追踪或无限段
滚动。预算耗尽时明确停止该段学习，正常输入继续；只有新的真实词边界可以取得新许可。测试使用合成
Observer 与临时 SQLite，原生 broker、Host 事件接线和现场输入验收不能由这些单元测试代替。
