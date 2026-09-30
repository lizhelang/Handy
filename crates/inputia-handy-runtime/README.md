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
