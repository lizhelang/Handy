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

## 原生写者暂停租约

`native_quiescence::NativeWriterSuspender::suspend` 复用真实 `VerifiedCodeEvidence`，要求旧 release 的
Control、IME、Settings 完整角色集与实际维护标记匹配。内核 UID / 启动时间 / audit token 和运行代码
准确根都需匹配，每个 SIGSTOP 前重新核真实 marker；缺能力时拒绝，没有 PID-only、TERM 或 KILL 回退。
暂停前后核对子树；未知后代使操作失败并恢复本次暂停，不能通过停止未知进程来消除缺口。

返回 `SuspendedWriterLease` 无公开构造、Clone、Deserialize、Send 或 Sync，原生 opaque handle
持有准确实例及原停止状态。同进程禁止重叠租约。`assert_suspended` 重核 marker、代码、实际集合、
子树和暂停状态；`resume(&mut self)` 幂等且失败可重试，只恢复本租约从运行态暂停的实例，
原已停止进程保持停止。Drop 仅尽力恢复；恢复失败有错误且本进程不再接受新租约。

该租约不能转为完整 `QuiescenceReceipt`。它只证明存活期间已核实例暂停，不证明退出、未来不会
启动新实例或 SQLite FD 已释放。TIS 切离、旧路径永久隔离、服务独占 FD 仍是独立门禁。
**此旧接口的 crash / abort / SIGKILL 不触发 Drop，不能直接用于生产交接。下节 guardian 是独立库入口，正式 Updater 接线尚未验收。**
没有生产 `NativeAdapter` 调用，也未搬移用户库；后续快照和交接必须持有并重验活租约。

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml --features native-code-verification native_quiescence
bash native/inputia-install-support/quiescence-self-check.sh
```

测试只对自建临时子进程检查内核暂停 / 恢复、原已停、维护撤销、未知子进程和失败回滚；Rust 合成
元数据测试拒绝缺角色、异 subject / epoch、路径、PID version、签名摘要和伪暂停证据。
真实 Developer ID 三角色的生产暂停成功路径 **NOT_RUN**，不把 RAII 当作崩溃恢复证明。

## 单边崩溃恢复 guardian（库与临时夹具）

`Transaction::guardian_authority()` 只复制已持有的 `Updater/update.lock` 文件描述符，
`guardian::begin_guarded_suspension(authority, updater, roles)` 还要求当前 Updater 与旧
Control / IME / Settings 的真实代码证据及同一维护 epoch。调用者不能用 JSON PID、路径、
`verified: true` 或空角色集合制造授权。新旧原生暂停入口共用进程槽；同一事务复制多个 authority
也不能建立重叠暂停。`GuardedWriterLease` 返回后处于 `Starting`，只有 `Holding` 且
`assert_suspended()` 成功才是当前暂停证据；它不能转换为完整 `QuiescenceReceipt`。

双边分别扫描验证目标，交换的只是准确实例、原始停止状态、index 与绑定摘要。每个原运行实例
先由父保存 ARM 未知效应清单并 ACK，guardian 才能发 audit-token STOP；STOP 回执丢失仍按
ARM 恢复。原本 SSTOP 的实例只观察、永不 CONT。guardian 的唯一串行执行器在恢复开始后
不可逆关闭 STOP，避免先恢复后落入排队的暂停。耗时核验后，双端再次核维护标记、对端身份、
取消与单调期限；过期结果不能返回暂停成功。

父退出、exec 或通道失效时，guardian 封闭 STOP 并恢复全部已 ACK 的 ARM。guardian 退出或
exec 时，父只有在内核确认原执行实例结束后才接管；EOF、超时或活着但无响应均不足以授权
父发 CONT。正常结束为 `Release → Recovering → Resolved → Resumed → DisarmAck`，
并逐条读回真实 running / 原实例已退出；phase 字段不能单独证明恢复。`resume()` 超时返回
`RecoveryPending`，恢复线程保留清单与锁，UI 丢弃句柄也不丢恢复责任。

私有 `socketpair` 只通过固定已验 Updater 的 reexec 入口传递，`posix_spawn` 默认关闭其它 FD，
接收后立即设置 CLOEXEC；没有 PATH helper。父子持有同一 flock open-file-description，
只 close、不显式 `LOCK_UN`，直到恢复真终态。最多 64 个实例、128 KiB 单帧、每方向
512 条消息 / 2 MiB，默认暂停上限 30 秒，不无限续租。消息 budget 耗尽不解除恢复责任。

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml --features native-code-verification guardian:: -- --test-threads=1
cargo clippy --locked --manifest-path crates/inputia-updater/Cargo.toml --features native-code-verification --all-targets -- -D warnings
bash native/inputia-install-support/guardian-self-check.sh
```

验证分层：Rust 使用真实自建 sleep / 测试进程的 audit token，执行 14 个 ARM / STOP / CONT /
终态单方 SIGKILL 或 abort 窗口；测试原 SSTOP、PID version 不匹配、取消交错、存活对端不抢权、
Updater 死后 guardian 仍保持原事务锁，以及阻塞核验期间过期 / 撤权。Swift 自检覆盖 24 个
原生 plan 断言，包含双边独立计划、幂等恢复、不可逆关闭、新旧暂停互斥。角色签名在这些正向
夹具中是合成数据，不替代真实 Developer ID 三角色验证。

**正式 Updater main 尚未调用 `guardian_entry()`；固定发布入口、Developer ID 同入口 reexec
与完整 NativeAdapter 生产路径均为 NOT_RUN，当前禁止生产交接接线。** 双进程同时死亡、OS
失效没有在线自动恢复保证；外部第三方在登记后另发 STOP 没有可区分的内核所有者计数。
意外 fork 的对端使所有权不确定，进入恢复待处理而不猜测。TIS 切离、旧数据库路径 fence、
快照和服务独占 FD 是后续独立门禁；未操作用户真实数据库或日用进程。

## TIS 输入源切离与条件恢复（原生库，系统切换未运行）

`native_input_source::InputSourceLease::prepare(&Transaction, &VerifiedCodeEvidence)` 仅登记观察。
它从真实事务复制同一个锁描述符，绑定 subject / 维护 epoch 与旧 IME 签名证据；不接收调用方
提供的输入源 ID。租约不可 Clone / Deserialize / Send / Sync，要求主线程，同进程只有一个
输入源租约，防止同一 Transaction 重复 prepare 后各自恢复。prepare 与每次操作重验 IME
代码；新功能没有调用已审核 guardian 的暂停/恢复逻辑。

`detach()` 只有当前选择的注册属性匹配已验 IME 合同时才尝试切离；当前为其它输入源则
`AlreadyDetached` 且无选择效应。源与 mode 直接来自同一已验证 `SecStaticCode` 的
`kSecCodeInfoPList` 安全字典，跨架构须一致；没有验签后重新打开路径读取 plist 的窗口。
只对使用的 mode / ID / icon 字段施加预算，Security 自身的元数据加载不由本层限制。
匹配 `TISInputSourceID` / `ComponentInputModeDict`、TIS Bundle ID、准确图标资源路径和唯一
注册对象。**SDK 只承诺 IconURL 是显示资源；这些匹配不证明该 TIS 对象实际执行的组件来自
已验 bundle 根。完整 NativeAdapter 仍需独立组件关联证据，不能把这里的观察升级为该证明。**
没有按名称或 ID 前缀选择任意源，也不调用
register / enable。唯一允许的 fallback 为已启用、可选择、ASCII 的 Apple ABC / US 键盘布局。
其实际布局数据必须具有指向准确 root-owned 系统资源的内核文件映射，资源须通过 `anchor apple`
及固定标识的签名验证，且完整字节匹配映射的文件偏移；同名 ID、相同 heap 字节、第三方文件
均不足以建立 fallback 能力。**SDK 不承诺布局 CFData 必须来自文件映射，缺证据正常返回
`no_verified_fallback`，当前不宣称这一路径在全部 macOS 上可用。**

`assert_detached(&VerifiedCodeEvidence)` 用当前版本重新绑定合同并读回；它与
`restore(&VerifiedCodeEvidence)` 均允许同 subject / Bundle ID / 安装根下已验新版本或回滚版本，
因此不要求已被替换的旧 artifact 继续存在。restore 只恢复本轮捕获的原 mode，不能传任意 ID。只有本次确实
切换且当前仍为该 fallback、未观察到任何选择通知时才条件尝试。用户选择其它源得到
`PreservedUserSelection`；fallback → 其它 → fallback 的 ABA、包括自身迟到通知在内的任何
通知，都使恢复资格变为 `Uncertain`。通知代数从 prepare 起从不重置，不把事件猜作“自己产生”。
每次慢校验后紧邻真实 `TISSelectInputSource` 前再核当前对象 / 代数 / 维护标记 / 期限，调用后
读回并再次核验；默认两分钟期限不因重试续期。失败或未知结果不自动重放。

结果为 `Prepared`、`AlreadyDetached`、`DetachedObserved`、`PreservedUserSelection`、
`RestoredObserved` 或 `Uncertain`，并带原始/最近观察/备用源 ID 与是否尝试效应。
`ownership_exact` 恒为 false：TIS 没有原子 CAS、所有者计数或可靠带序号的通知，最终检查到
选择之间仍有不可消除的竞争。Observed 只表示当次读回，不能当作退出、长期排他切离或完整
NativeAdapter 回执。Drop **不切换输入源**；TIS 尚未与 guardian 的进程崩溃恢复联动。

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml --features native-code-verification native_input_source::
bash native/inputia-install-support/input-source-self-check.sh
```

Swift 使用注入后端验证安全字典与路径分离、跨架构一致、字段预算、验签失败不交付合同，
以及正常切离/条件恢复、用户选择保护、ABA/迟到通知、两侧超时/撤权、失败
不重放、禁用/伪装 fallback、互斥和 Drop；没有构造系统后端或执行真正的 TIS 切换。Rust 检查
事务/角色/版本/epoch 与严格观察结果合同。**真实系统选源、Apple fallback 来源链正向、
Developer ID IME 正向与正式 Updater / NativeAdapter 接线均 NOT_RUN。** 未启停日用 Host、
未启用或安装输入源、未修改用户设置。公开 SDK 合同来自 Xcode `TextInputSources.h`，尤其是
Bundle ID 可缺、布局 CFData 无来源承诺、`TISSelectInputSource` 与 distributed change 通知的限制。

## 旧学习库路径隔离与合作文件租约（origin 发行者未接）

`legacy_handoff::LegacyMemoryHandoff::prepare(&Transaction)` 从事务的真实 home / uid 重新读取安装
收据，必须与本事务 old / new receipt 精确匹配后再解析固定 profile。调用方不能提供数据库路径
或反序列化的 location 当作授权。固定源集为 `inputia_memory.db`、`-wal`、`-shm`、`-journal`；
旧 Core 未强制 WAL，所以热 rollback journal 同样必须保留。源缺失返回 `LegacySourceMissing`，
不推断成新 profile，也不新建空库冒充迁移成功。

文件协议由 `advance(&mut VerifiedLegacyOrigin)` 执行：

1. 绑定原 Transaction OFD / subject / maintenance epoch，并取得新的独立操作锁 OFD。
2. 耐久登记完整源文件实例（device / inode），预建非空目录 fence 及其记录。
3. 通过 macOS `RENAME_SWAP` / Linux `RENAME_EXCHANGE` 把旧主路径原子替换为 fence；没有
   “旧文件移走、新文件尚未放入”的空路径窗口。同步两侧父目录，随后按原 basename 归档全部
   sidecar。fence 不自动撤销，失败也不把旧版本重新指向新库。
   正常移动与“rename 已发生但目录 fsync 失败”的重入执行同一耐久确认：重同步实际文件及
   两侧父目录；同步仍失败时不得因目标已经存在而继续成功。
4. **fence 持久后，对实际归档的完整实例集合重新取得无旧 FD / 未知写者证明。** 在此之前
   不产生可发布快照；暂停租约、一次 fence 前扫描或单纯 TIS Observed 都不满足这一条件。
5. 把冻结源集复制到独占工作区的原 basename，在那里执行 SQLite 恢复 / backup，不修改归档。
   目标关闭所有连接、切为 DELETE journal、通过完整性检查且没有 sidecar 后同步；使用
   no-replace rename 发布至 `managed-memory-v1/memory.sqlite`。
   每次 SQLite open 前核完整四文件集合，原集合 absent 的 sidecar 必须仍 absent；staged
   target 旁的任何未登记 WAL / SHM / journal 也拒绝并保留，不能先交给 SQLite 消费后再检查。
6. 返回前再次核原生授权、文件集合、fence 与目标实例，获取固定 `service.lock` 的合作排他
   租约。日志 `ready` 只记进度，不能直接发行 origin 或省略真实盘面核验。

**当前生产 `origin_authority()` 明确返回 `OriginProofRequired`。** `VerifiedLegacyOrigin` 没有
公开构造 / Deserialize / Clone；只有 `cfg(test)` 私有夹具发行者。正式原生 writer exit、未知
子进程及归档 inode 的 FD 集合审计尚未接入，因此本包不能启动生产迁移或解除 runtime 的
`handoff_required`。全套 NativeAdapter、实际用户库迁移均 **NOT_RUN**。

`inputia-settings::memory_domain::OwnedMemoryDomainLease` 持有 `service.lock` 的独立 OFD 与
目标 / fence / fence-record 的私有文件描述符，逐次核固定名字仍指向相同实例，拒绝链接、
越权路径、非私有文件、重复租约和 fork 后使用。它不创建缺失文件。`MemoryFileBinding` 只是
文件事实，JSON / UUID 不是来源授权；`requires_origin_and_connection_validation()` 始终为 true。
数据库 FD **不额外持 flock**：macOS 实测该锁会挡住同进程 SQLite 的 fcntl 锁。单写者合作
排他由 `service.lock` 提供，SQLite 自己管理数据库锁。该租约不阻止任意同 UID 程序直接 open。

`bind_resource(Connection)` 只封装析构顺序：先关闭 Connection，再关闭租约 FD，没有提前
拆出 / 释放租约的接口。未来 runtime 必须在打开 SQLite 后核其真实 handle / `HAS_MOVED`，
并验证 domain UUID / key ID / profile / epoch 与第三域真实回执，不能只检查数据库路径字符串
或把此文件租约写进旧 `exclusive: bool`。文件租约允许正常内容更新，不把迁移时摘要当作未来
内容永远不变的条件。已开始业务写入后，不能重跑快照迁移来覆盖新正文；应走运行时域审计。

恢复采用实例和摘要证据，不覆盖或删除未知文件。已知持久检查点可继续；若进程死在文件创建
与实例登记之间，或工作区在 SQLite 恢复途中留下不能核实的内容，返回 `RepairRequired` 并
保留原物和 fence，不猜测归属。源集合总量上限 256 MiB，backup 分页且有五秒循环期限；
完整性检查和哈希仍为同步磁盘工作，不承诺整个迁移五秒完成。归档包含私有学习数据，保持
0600 / 私有目录；本包不删除归档，后续须接保留策略及真正遗忘/GC，不能宣称所有痕迹已遗忘。
目录 fence 阻止旧 SQLite 常规打开/创建，不是防止同 UID 恶意递归删除目录的系统沙箱。

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml legacy_handoff:: -- --test-threads=1
cargo clippy --locked --manifest-path crates/inputia-updater/Cargo.toml --all-targets -- -D warnings
```

临时夹具覆盖 WAL / 热 rollback journal、一项测试中的十个真实自建进程 SIGKILL 持久边界、
fence 后归档 inode 仍被旧子进程打开时拒发布、伪造 ready、同 inode 内容恢复、inode 替换、
未知目标、符号/硬链接、跨进程合作锁及 SQLite 实际 `HAS_MOVED`。此外覆盖 rename 后目录
同步持续失败及工作区/目标的六种未知 sidecar 保留。测试中的来源发行者只验证本次自建子进程
的状态，不冒充生产全系统 FD 扫描。没有切换输入源、扫描日用进程或操作用户库。
