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

`validate_archive_entries` 是解包前完整目录表校验，限制路径、重复项、文件作为父目录、链接及总大小；解包实现仍必须使用独占创建和固定目录句柄，不能在校验之后调用宽松的 shell 解包命令。真实 ZIP 实现见下节；本库不提供 DMG 挂载或解包。

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


## 真实 ZIP 解包：`inputia-zip-v1`

`archive::extract_zip` 消费调用方提供的 `File`，以及从已验证发布描述取得的
`ArchiveDigest { sha256, size }`、`required_root`、全新目标目录、安装 UID、`ArchiveLimits` 和取消标记。
实现不会重新按源路径打开文件，不调用 shell / Python / 系统解压命令。返回的
`ExtractedArchive` 字段不可直接构造，包含源摘要、解包根目录的 `Fingerprint`、持有的
根 fd 身份及读取预算计数。macOS 新根若继承扩展 ACL 会拒绝，避免只看 0700 误判私有目录。**它只证明本次解包一致性，不证明 Apple 签名、公证、组件角色、
发布授权或允许安装。** `required_root` 是调用者从受信 role→组件根映射取得的单段名称，
不能从 ZIP 自行选择；本库要求该目录显式存在、所有条目仅在该根内，链接也不能跨根。
当前 manifest 尚未定义 `bundle_root`，正式 writer 应加入版本化布局合同；不猜测 role 布局。
proof 绑定该精确根名。调用方仍须按授权 role 检查精确布局，在受控 stage 对实际 `.app`
做原生验证；恢复时重验，旧 proof 不豁免这些步骤。

处理顺序：同一个源 fd 的 SHA256 与 size → 全量中央目录、局部头和 descriptor 一致性 →
完整路径/链接图 → `mkdirat` 独占建立 0700 根 → `openat(NO_FOLLOW|EXCL)` 建普通文件 →
CRC/实际长度/Deflate 终点检查 → 文件 fsync → 根内链接 → 目录 fsync → 同 fd 再算 SHA256
并核 inode/mtime/ctime → 可取消的树摘要 → 返回 proof。任何错误均不返回 proof，不覆盖
已有目标，不清理未知文件；失败留下的部分目标必须由拥有该事务的调用方明确处理。

### 支持范围和拒绝条件

- 单卷、非 ZIP64、Unix/macOS 创建者、UTF-8 路径原始字节；仅 Store / Deflate。
- 普通文件、目录和最终指向包内现有节点的相对链接。Framework `Versions/Current`
  结构保留；普通文件/目录的执行位和模式保留，链接模式按平台链接语义处理。
- 目录表保留全部条目，不合并重名。拒绝重复项、大小写与 Unicode 分解/折叠别名
  （含隐式父目录）、绝对/越界/反斜线/控制字符路径、文件或链接充当父目录、硬链接与设备。
- 拒绝加密、未知 flags/压缩、局部/中央身份或 CRC/尺寸不一致、重叠数据、隐藏条目、
  SFX 前缀、尾部内容、多卷与未知 metadata。权限拒 setuid/setgid/sticky、组/全局可写。
- 明确允许的说明性 extras 仅 Info-ZIP Unix 原始时间/UID、扩展时间戳、新 UID/GID，
  校验结构但不恢复归属/时间。归属固定当前安装 UID，不从包里任意 chown。
- 条数、隐式目录条数、深度、单文件/总展开量、压缩比、总源读取量及原始/比较路径内存均有限制。
  摘要/解压/落盘/树摘要循环都检查取消。默认最大源 <4 GiB、10 万节点、64 层、
  单文件 2 GiB、总展开 8 GiB、压缩比 200、源累计读取 16 GiB、路径字节 16 MiB；预算可收紧。
- `__MACOSX`、`._*` AppleDouble、资源叉、xattr/ACL/fork extras 整包拒绝；不会静默跳过。
  下载 ZIP 文件自身的 quarantine 不由解包器删除；本 profile 不授权绕过 Gatekeeper。

采用固定版本 `rawzip 0.5.1` 负责 ZIP 解析和 CRC/尺寸验证，`flate2 1.1.9` 负责标准
Deflate；不调用其宽松路径规范化或高层自动解包。流式中央目录避免依赖库按文件名
合并重复项；安全路径操作与目录图由本库控制。

### 发布生产者仍必须补齐的门禁

当前 `scripts/build-inputia-release.sh` 的 `ditto` 仅复制三套 `.app`，还没有最终 ZIP
writer。合成夹具确认本机 `ditto -c -k --keepParent --norsrc --noextattr --noacl --noqtn`
可保持普通文件、执行位与 Framework 链接；默认 `ditto` 则会把 xattr 写成 AppleDouble，
解包器按合同拒绝。**这不是实际已签名产品 ZIP 的兼容验收。**

生产者须在签名/公证/装订后的冻结源树逐节点检查并拒绝本 profile 不能承载的 xattr、
ACL、resource fork；不能为了通过解包而删除它们。非 Mach-O 脚本签名可能在 xattr 中，
丢失会破坏签名；有此情况须先升级归档 profile 和实现。参考
[Apple TN2206](https://developer.apple.com/library/archive/technotes/tn2206/) 与
[签名操作指南](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html)。

正式门禁顺序是：嵌套代码从内到外签名 → 公证 → 对 `.app` 装订 → 创建最终下载 ZIP →
用本解包器解到全新受限目录 → 比较树/模式/链接/CDHash → 原生 Team/identifier/runtime/
entitlements/notarized 检查与装订验证 → 最后计算 ZIP hash/size 并签入 manifest。
ZIP 自身不能装订，不能先绑定摘要再修改 `.app`。真实 Developer ID、公证、装订成品的
完整往返验证当前 **NOT_RUN**，本包没有读取证书或执行产品安装。

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml --test archive
```

测试使用真实临时 ZIP，包含 Stored/Deflate/Framework 链接和 mode、目录重名/Unicode、
路径逃逸、恶意头/descriptor/metadata、CRC 与尺寸损坏、压缩炸弹/预算/取消、源 fd
变化、已有目标和硬链接拒绝。macOS 专属测试只对自行创建的临时夹具调用 `ditto/xattr`，
用来证明上述生产 profile 行为；生产解包路径没有这些外部工具依赖。
