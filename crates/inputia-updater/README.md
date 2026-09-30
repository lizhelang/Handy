# Inputia 原生更新事务核心

这是可执行的 Rust 文件系统事务库，负责三组件、不可变配对清单和安装收据的协调替换。它不会执行日志中的命令，不调用 shell，不录音，不改 TCC，不复制旧数据库覆盖现有数据。

**当前还不是可交付安装器。** 真实 Security / TIS / SQLite 适配器、已签名 bootstrap、登录恢复入口、受限 postcheck 通道、下载和归档解包器尚需原生 helper 接入。没有默认通过的生产适配器；不能把测试中的合成回执当作签名、数据库一致性或系统验收结果。

## 原生静态代码证据

`native_code::NativeCodeVerifier` 在启用 macOS 特性 `native-code-verification` 时调用预编译的公开 Security ABI。它绑定准确路径、事务、release、逐架构 Team/identifier/CDHash、runtime、受签 Info.plist 和角色 entitlement，显式要求 Developer ID + 公证条件，并在调用前后复核整棵树的预期摘要。结果 `VerifiedCodeEvidence` 不能从 JSON 直接构造。

这部分尚未组合成完整 `NativeAdapter`；它不验证 release/pair 清单、不生成整个阶段的 `VerificationReceipt`，不停止进程或写数据库。不启用特性时明确返回 `NativeUnavailable`。详见 [原生合同](../../native/inputia-install-support/README.md)。临时 ad-hoc 负例通过不代表真实 Developer ID/公证正例已验收。

## 接口与授权边界

```rust,ignore
let updater = Updater::new(system_home, effective_uid)?;
let plan = updater.prepare(request)?; // 只读，不建目录或锁
let mut transaction = updater.begin(plan)?;
transaction.run(RecoveryPolicy::Resume, &mut native_adapter, &mut NoFaults)?;
// 崩溃后，使用同一安装上下文和原事务 ID：
updater.recover(id, RecoveryPolicy::Resume, &mut native_adapter, &mut NoFaults)?;
```

`InstallRequest` 必须由已验证发布授权映射而来。公开字段可用于序列化和测试，**不构成安装授权**。`NativeAdapter::verify_artifacts` 在首次复制前、暂存后、替换后及恢复时验证真实证据，必须绑定 `Subject` 和 `artifact_set_digest`，并区分 `DownloadedNew`、`StagedNew`、`InstalledNew`、`RollbackOld`。未来通过 `inputia-release` 的精确制品授权接入；历史签名在数学上有效不等于可以现在安装。

`PreparedPlan` 固定新旧收据、产品 / installation / profile / UID、三个角色目标、原配对清单和各制品摘要。角色路径由 `inputia-settings::installation` 计算，不能在请求中任意指定目标。新装拒绝覆盖没有收据的已有组件；更新保留 installation、profile、数据位置和作用域，只允许切换 release/channel。当前执行器仅支持 `User` 作用域；`LegacySingleUser` 明确返回 `PermissionRequired`，必须由后续具有独立授权和系统目录信任策略的原生 helper 迁移/接管，不能把 `/Applications` 的系统权限模型当成用户数据损坏。

## 耐久路径与顺序

固定根：`~/Library/Application Support/Inputia/Updater/`。

- `update.lock`：进程级非阻塞 `flock`，跨进程更新互斥。
- `transactions/<UUID>/journal.json`：schema 1 严格类型日志，拒绝未知 / 重复字段；事务目录独占创建，ID 不能复用。
- `maintenance.json`：schema 1，字段为 `transaction_id`、`installation_id`、`old_release_id`、`new_release_id`、`epoch`、`plan_sha256`，不含命令。
- `bootstrap` 与 `versions/<version>/helper`：恢复环境位于被替换组件之外。适配器必须提供真实代码验证与恢复入口凭据，核心复核文件摘要并 fsync。此库不会伪造或安装它们。
- `.inputia-<UUID>-<role>.stage/.backup/.failed/.copy`：位于每个目标的父目录，确保该角色移动在同一卷内。多个组件不宣称全局原子替换。

顺序为：只读预检 → 真实来源及旧套验证 → 复制暂存 → 耐久记录原进程与输入源 → 写维护标记 → 双端和设置进程确认停止 → 一致性快照 → 替换组件和新配对清单 → 静态复核 → 最后切新收据 → 受限 postcheck → 耐久 commit → 耐久记录 `writes_released` intent → 删除维护标记并 fsync → 记录完成。

普通应用启动必须在维护标记存在时关闭写入入口。postcheck 只允许身份、握手及 schema 检查，不能进行用户数据迁移、录音或正文派发。需要迁移时，必须先增加独立的可恢复迁移日志及兼容检查；不能把迁移写入伪装成 postcheck。

每次制品 rename 都执行：写 intent / 文件 fsync / 日志目录 fsync → 内核 no-replace rename → 源和目标目录 fsync → 写 result / fsync。收据和 marker 都受 0600、UID、单硬链接及逐级无链接路径约束。日志采用同目录临时文件和原子替换；残留临时文件保留，不执行或据此覆盖制品。

## 恢复与数据边界

恢复先重新取得锁、验证日志绑定、外置恢复环境和真实签名，再根据 `(destination, stage, backup, failed)` 的实际旧 / 新摘要继续。日志声称的阶段不是单独的放行条件。未知文件、缺失备份、改变的制品、冲突 marker 或不认识的事务会停止并保留盘面，不能盲重放或自动删除。中断的部分复制可以留下 `.copy`；若其摘要不完整则保留并要求修复，旧安装尚未移动。

`BinaryRollback` 只移动程序、配对清单和收据。新套放入 `.failed`，原下载源保留；所有用户数据、更新后的新增 / 修订和删除 / 遗忘账本均保留。该接口不提供数据恢复功能。

`writes_released` 在移除 marker **之前**耐久记录，崩溃后保守认为新写入可能已发生。此后回滚必须先进入维护并停止双端，再提供绑定本轮 `epoch` 的当前数据逐域读 / 写 / outbox / 隐私兼容回执；每次恢复重新检查，静态版本能力声明不能代替它。新装无旧套时拒绝此类回滚。`rollback_requested` 独立持久化，避免回滚中断后错误继续安装新套。恢复完成前不得清理备份，库目前不实现 GC。

麦克风 / 辅助功能授权缺失通过 `PermissionState` 报告，输入源恢复失败通过 `NeedsUserAction` 报告，不混同制品损坏。恢复服务的角色不得超出原快照，原先退出的语音服务不被自动复活。

## 文件与归档约束

路径操作使用逐级 `openat(O_NOFOLLOW)` 和持有的目录句柄；制品树摘要覆盖相对路径、类型、文件模式、长度、正文和符号链接原始目标。只允许能够解析到包内现有节点的相对符号链接，保留 Framework 链接结构；拒绝越界、绝对、悬空和循环链接，拒绝硬链接、设备节点、异常归属、外部可写内容及超额树。

`validate_archive_entries` 是解包前完整目录表校验，限制路径、重复项、文件作为父目录、链接及总大小；解包实现仍必须使用独占创建和固定目录句柄，不能在校验之后调用宽松的 shell 解包命令。该库不包含 ZIP/DMG 解包实现。

目录句柄、禁止覆盖和摘要复核减少竞态影响，但不声称抵抗拥有同一 UID 且能够任意篡改整个进程或所有证据的攻击者。生产 helper 必须从自己的编译信任根验证代码 / 发布授权，不能从日志导入新信任根。

## 适配器合同

| 阶段            | 必须提供的真实证据                                                                                                     |
| --------------- | ---------------------------------------------------------------------------------------------------------------------- |
| recovery        | 外置 bootstrap/helper 的准确代码身份、文件摘要、兼容日志版本和恢复注册凭据                                             |
| verify          | 当前安装授权、全部角色的 Security / hardened runtime / release / pair / metadata 验证和准确集合摘要                    |
| capture/quiesce | 停止前的进程身份和原输入源；对应 marker epoch 的停止回执；每次移动前重新确认                                           |
| snapshot        | 所有数据库与 WAL、一致附件清单、Rime / 配置 / 输出 / 删除账本；最多 256 个证据文件，可使用封闭清单文件承载大量附件记录 |
| postcheck       | 准确收据、release、profile、受限私有握手与 schema；不自动测试录音或文本插入                                            |
| rollback        | 目标旧版本对当前数据的读写、outbox、删除和隐私语义兼容实测                                                             |
| restore         | TIS 恢复结果和受原进程快照限制的服务状态，失败明确可见                                                                 |

所有阶段回执绑定 transaction / plan / installation / release，不能跨事务套用。适配器动作须可重入；核心已先保存原运行状态，恢复时不能用“当前已停止”覆盖原来运行的记录。

## 验证

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml
cargo clippy --locked --manifest-path crates/inputia-updater/Cargo.toml --all-targets -- -D warnings
```

测试仅操作临时目录。覆盖新装/更新、每次更新与回滚 rename 前后故障、日志边界故障、重复恢复、锁冲突、收据顺序、缺失权限、未知文件和坏备份保留、原源不删除、用户新数据不覆盖，以及 Framework 内部链接与归档越界。测试适配器名称明确为 `SyntheticNative`，不承担真实系统证明。
