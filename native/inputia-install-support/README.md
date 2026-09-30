# Inputia 原生安装只读验证

本包通过 macOS 公开 Security API 校验 `.app`，是原生安装适配层的一部分。目前只实现静态代码验证，不实现完整 `NativeAdapter`、进程退出、TIS 切换、bootstrap 注册或数据库快照，也不把验证成功当作安装授权。

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

## 一手依据

- Apple [TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements) 说明 Developer ID CA/Application 证书标记与 Team OU；这里不接受兼容表达式里的 Mac App Store 分支。
- Apple [requirement 解释器](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_codesigning/lib/reqinterp.cpp#L202) 与 [公证实现](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_codesigning/lib/notarization.cpp#L80) 表明 `notarized` 经代码摘要查询票据。因此 Developer ID 链和 Team 必须另行显式约束。
- macOS SDK `SecStaticCode.h` 的公开合同说明全架构、嵌套和 strict flag，以及并发修改会使验证结果无效；`SecCode.h` 说明每 slice 的 `Unique`、同 slice 的多摘要算法列表和 entitlement 字典缺失的歧义。
