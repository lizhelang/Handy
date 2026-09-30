# Inputia 原生安装验证与写者暂停租约

本包通过 macOS Security API 校验 `.app`，并提供受管旧写者的身份核验与存活期间暂停租约。它是原生安装适配层的一部分，不实现完整 `NativeAdapter`、TIS 切换、bootstrap 注册或数据库快照，也不把代码验证成功当作安装授权。

## ABI 与输入合同

`iuis_verify_code(bytes, length)` 接受最多 32,768 字节的规范 UTF-8 JSON，返回独立 JSON 字符串，由 `iuis_string_free` 释放。输入字段由 Rust `CodeExpectation` 定义，包含完整事务 `Subject`、产品、角色、准确 bundle 路径、版本/build/release/source commit、Team、架构与 CDHash 集合。所有期望值必须来自编译分发策略和已验证发布清单，不能从待验证 app 的 Info.plist 反向建立信任。

`purpose=new_release` 必须匹配事务目标 release；`purpose=previous_release` 必须是不同的旧 release，供旧套验证与回滚使用。两种用途都绑定同一完整事务 subject；旧套期望值必须由上层从已验证旧发布清单提供，`previous_release` 不构成旧版本安装或数据兼容授权。

Swift 对固定结构重编码并逐字节比较，拒绝重复、未知、缺失字段和非规范数字；没有任意 requirement、关闭公证或忽略错误的选项。错误仅返回固定分类与 OSStatus，不回显证书内容或用户路径。

只有以下条件同时满足才返回只读代码证据：

- bundle 根与祖先按目录句柄逐级 `O_NOFOLLOW` 打开，并检查每级归属与模式；只允许 root/current-user 安全祖先（root 的 sticky 临时目录例外），树本身须由当前用户拥有且禁止组/全局写入和特殊模式。根准确路径在前后仍是同一 device/inode；内部只允许可解析到包内的相对链接，保留标准 Framework 版本链接。
- 使用固定 Developer ID Application 证书链、准确 Team OU、bundle identifier、CDHash 集合及 `notarized` 的联合 requirement。主应用标识/Team/CDHash 都是独立条件。
- `SecStaticCodeCheckValidity` 开启全部架构、标准嵌套代码和 strict 校验，保留默认可执行文件及资源校验。
- 有界解析主 Mach-O 的真实架构集合，并逐 slice 创建 Security 对象核对 CDHash、Team、identifier、runtime flag 和 entitlement 策略。`kSecCodeInfoCdHashes` 不是多架构列表，不能拿它代替这一步。
- 逐 slice 使用受签名保护的 Info.plist 检查 release、source commit、版本和 build。角色 entitlement 只允许仓库已声明的键与布尔类型；原始 entitlement 存在但无法取得字典时拒绝。

结果包含请求完整绑定、逐架构身份/flags/entitlement 摘要、根 device/inode 和三个实际验证条件。没有“允许安装”“Gatekeeper 全部通过”或“已强制联网撤销检查”的状态。

## 时效与范围

Security 的成功只在被验代码没有改变时有效；公开 API 不提供对整棵 bundle 的原子锁。Rust `NativeCodeVerifier` 在调用前后执行安全树扫描与完整摘要比较，绑定调用方预期摘要；持有根目录句柄，并把原生返回的 device/inode 与 Rust 前后根身份相等绑定，拒绝混合两个同路径目录的证据。安装核心还必须在每次恢复和最终替换后重验，并通过受管 staging、进程停止与维护门禁约束并发写入。回执不能豁免后续验证，也不能证明同 UID 主动篡改下完全没有竞态。

`CheckNestedCode` 覆盖系统认可的标准嵌套代码位置；它不能代替完整的可执行文件发布清单，也不表示已经逐个审计所有 framework 的角色 entitlement。主可执行文件的全部 slice 执行本包策略，整树路径/资源与摘要由原生检查及 Rust 文件系统层共同验证。

公证 requirement 可能使用系统票据缓存或系统服务，不能据此宣称本次已强制联网查询撤销。实现不导入私有 `SecAssessment` API，不枚举 Keychain，不调用 `spctl` 的本机 allow 规则，也不需要用户机器上存在 Python、Swift 编译器、Git 或 Xcode。Swift 编译器仅用于开发时生成预编译静态库。

## 构建与验证

```sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml --features native-code-verification native_code
bash native/inputia-install-support/self-check.sh
```

`native-code-verification` 是显式 macOS 编译特性；不启用时返回 `NativeUnavailable`，没有模拟成功适配器。Swift 自检与 Rust 原生集成测试只生成临时 ad-hoc 应用并验证拒绝，纯策略夹具单独验证字段/架构/entitlement 判断。不会执行这些临时 app，更不会退出、替换日常安装或读取签名凭据。

**真实 Developer ID + 公证成功路径为 NOT_RUN。** 尚需使用正式签名制品，在支持 OS 矩阵中验证票据、全架构、嵌套代码与恢复环境；当前测试不构成产品安装验收。

## 已核写者的暂停租约

Rust `NativeWriterSuspender::suspend` 只接收真实 `VerifiedCodeEvidence`，要求旧 release 的
Control、IME、Settings 三角色齐全且绑定同一 `Subject` 与维护 epoch。维护文件通过共享
`inputia-settings::maintenance` 合同读取；缺失、变化或角色不足都失败。原生
`iuis_writer_suspend` 接受最多 131,072 字节规范 JSON，不接受客户端 PID 或跳过签名的选项。

原生按当前 UID 枚举，通过受管根或签名 identifier 发现目标；PID 查询仅用于发现。
目标必须取得内核 `TASK_AUDIT_TOKEN`，绑定 UID、启动时间、PID version、实际可执行路径，再以
该 token 校验运行代码的 Developer ID / Team / identifier / CDHash / 公证 requirement。
元数据读取失败不能当作目标不存在，准确根以外的旧套副本也会拒绝。暂停前与暂停后核对子树，
任何未纳入已验角色的后代都使操作失败；不向未知 helper 发信号。

只有动态查找的 `proc_signal_with_audittoken` 能发送 SIGSTOP / SIGCONT；无 PID-only 回退，
没有 TERM / KILL 退出功能。每个 STOP 前同步回调 Rust，重新读取真实维护文件。
原生保留真实 audit 实例和观察到的原始停止状态；原本已 SSTOP 的进程不会被租约恢复。
失败路径恢复本次实际暂停的实例，同进程禁止重叠生产租约。能力不足时暂停前拒绝。

`SuspendedWriterLease` 无 Clone / Deserialize / 公开构造，且非 Send / Sync。
原生 handle 由唯一拥有者保留，Rust 在返回解析、末次门禁或重验失败时也会释放并恢复。
`assert_suspended` 重新检查维护标记、角色代码、实例集合、子树及内核暂停状态；它只证明当时
已核实例被暂停，不是未来新实例的启动锁，也不能转换为完整 `QuiescenceReceipt`。

正常结束必须调用 `resume(&mut self) -> Result<()>`，它幂等、失败保留 handle 可重试，
不因维护标记撤销而拒绝恢复。原生检查 CONT 后实例已恢复或结束。Drop / deinit 仅尽力兜底；
若恢复仍失败，记录错误并拒绝本进程后续新租约。**crash / abort / SIGKILL 不执行 Drop。**
因此此旧接口不能用于崩溃恢复；下节 guardian 库提供单边恢复，但正式发布入口仍未接线，不能解除生产门禁。

另须完成 TIS 切离、旧路径永久隔离与服务独占 FD。被暂停写者仍可能持有旧 SQLite FD，任意其他
同 UID 程序仍可能打开数据；暂停证据本身不能授权服务接管。后续快照、fence 与交接检查必须在
租约仍存活且重新核验成功时执行；真实优雅退出另由合作 shutdown / 用户迁移边界解决。

```sh
bash native/inputia-install-support/quiescence-self-check.sh
cargo test --locked --manifest-path crates/inputia-updater/Cargo.toml --features native-code-verification native_quiescence
```

自检只创建自己的临时 sleep / shell 父子进程，验证真实内核身份、签名不匹配不发信号、
marker 在验证后撤销、Drop / 显式恢复、原已停状态保留、未知后代及失败回滚。
测试不枚举或停止日用 Inputia / IME，不调用生产三角色 suspend；真实 Developer ID 旧套的
暂停成功路径仍为 **NOT_RUN**。这些夹具证明局部 primitive，不能替代生产交接验收。

## 一手依据

- Apple [TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements) 说明 Developer ID CA/Application 证书标记与 Team OU；这里不接受兼容表达式里的 Mac App Store 分支。
- Apple [requirement 解释器](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_codesigning/lib/reqinterp.cpp#L202) 与 [公证实现](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_codesigning/lib/notarization.cpp#L80) 表明 `notarized` 经代码摘要查询票据。因此 Developer ID 链和 Team 必须另行显式约束。
- macOS SDK `SecStaticCode.h` 的公开合同说明全架构、嵌套和 strict flag，以及并发修改会使验证结果无效；`SecCode.h` 说明每 slice 的 `Unique`、同 slice 的多摘要算法列表和 entitlement 字典缺失的歧义。

## Guardian 原生计划与双边恢复

`InputiaWriterGuardian.swift` 向 updater 的私有 guardian 库提供 opaque plan 与只读 peer 句柄。
生产 `prepare` 首先占用与旧暂停租约共用的 `WriterSuspensionSlot`，验证当前固定 Updater 代码，
再独立扫描三角色；同进程两个 authority 无法重叠暂停。plan 保存真实 audit 实例与原停止状态，
协议只能传 index，不能传任意 PID 取得效应能力。STOP 前同步核真实维护授权；关闭 STOP 后
永不可重新开启。原本停止实例不允许恢复，已恢复 / 退出可幂等查询。

plan 的 `free` **只释放元数据，不发 CONT**：父的备份 plan 可能从未拥有 STOP 权。
恢复必须经过 Rust 双边 ARM 账本与准确 peer EXIT / EXEC 判断，不能将原生 action ABI 单独
当作公开进程控制功能。Rust 持有 plan 到真实恢复终态，失败继续保留清单、槽与共享事务锁。
父子 reexec / flock / 协议状态见 [`inputia-updater` 文档](../../crates/inputia-updater/README.md#单边崩溃恢复-guardian库与临时夹具)。

```sh
bash native/inputia-install-support/guardian-self-check.sh
```

自检在自建 sleep 进程上执行 24 个断言，不扫描日用程序。Rust 另在自建进程中执行 14 个单边
crash / abort 窗口。真实 Developer ID 三角色、正式 Updater 同发布入口与完整生产交接仍
**NOT_RUN**；双边同时死亡、外部第三方另发 STOP、TIS / fence / 独占 FD 不由本 primitive 证明。

## TIS 切离与恢复观察

`InputiaInputSource.swift` 提供独立的输入源租约，主线程限定、同进程单租约，prepare 不选择源。
真实选择由绑定事务 / 维护标记 / 已验 IME 的 Rust 句柄显式调用。mode 与图标资源路径直接
取自同一已验 SecStaticCode 的 `kSecCodeInfoPList` 安全字典，所有架构一致后才交付，不重开
plist 路径；使用字段有界，Security 内部元数据加载不由本层限制。TIS 注册属性与已签合同匹配
仅是条件观察；IconURL 是显示资源，不能证明执行组件来自已验根，后续 NativeAdapter 尚需
独立组件关联证据。观察与恢复可重绑定同事务根下的已验新/回滚版本。
fallback 仅接受启用的 Apple ABC / US 布局及其可核实文件映射 / 签名 / 内容来源；
复制相同字节、伪造 Bundle ID 或缺来源都不能授权，普通系统缺映射时也可能不可用。

选择前后均读回并核期限 / 维护；没有启用或注册 API。通知代数不重置；任何已观察到的新选择、
ABA 或迟到通知都会阻止自动恢复。TIS 无 CAS，Observed 不是排他证明；free / Drop 不选源，
与 guardian 崩溃恢复尚未联动。详细字段及边界见 updater README 的 TIS 小节。

```sh
bash native/inputia-install-support/input-source-self-check.sh
```

本自检只注入 backend，不构造真实 Carbon 后端、不读取或切换用户当前输入源。真实 TIS、
Apple 映射来源正向、正式 IME / Updater 签名正向均 **NOT_RUN**。
