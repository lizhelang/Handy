# Handy Rust → Swift 配对认证桥

这是独立桥实现与合成验证，不包含产品 socket、业务 dispatcher、Host 配对接线或录音。共享认证的信任/注入边界继续遵循 README；严格 runtime 的合成程序正例不能替代 Handy/Inputia 候选动态依赖验证。

## ABI 与信任来源

`PairAuthBridge.h` 是 C ABI 合同：

- `uipa_manifest_load(...)`：接受待验证 envelope、**构建时嵌入**的 65 字节公钥、key/run/profile 常量和本端 role，返回 opaque handle。固定协议 major=1、requireHardenedRuntime=true，没有放宽参数。
- `uipa_authenticate(handle, fd, expected_role, out_peer)`：真实认证 socket 的内核 audit token 及动态签名，成功只输出 audit_token[32]、uid 和 role。fd 不转移所有权。
- `uipa_manifest_free(handle)`：释放一次，NULL 无操作；禁止对无效/已释放 handle 调用，也不能与认证并发。

固定状态码：0 成功、1 参数无效、2 manifest 拒绝、3 对端身份/硬化拒绝。失败会清空 out_handle/out_peer；不跨 ABI 返回异常、错误正文或私钥。

`src-tauri/src/native_pair_auth.rs` 提供 `EmbeddedPairTrust::from_build_constants`、`PairManifest::load`、`PairManifest::authenticate` 及 RAII Drop。信任参数要求静态生命周期，无配置读取和 Deserialize；产品不得用泄漏可写配置为静态引用的办法规避此约定。`VerifiedPeer` 字段私有且没有公开构造器，仅成功调用真实认证才可获得。handle 不 Send/Sync，每个后台认证队列独立加载，避免跨线程释放竞态。认证结果仍须由调用方绑定到具体连接，并完成策略屏障才可进行业务。

## 链接与部署版本

`src-tauri/build.rs` 新增独立函数，仅在 Cargo 实际目标 OS 为 macOS 时调用。Swift 目标架构来自 CARGO_CFG_TARGET_ARCH；显式 MACOSX_DEPLOYMENT_TARGET 原样用于 bridge triple。未设置时使用现有 tauri.conf.json minimumSystemVersion：Intel 10.15；Apple Silicon 受平台最低版本约束为 11.0。候选显式设置 13.0。没有改变 Apple Intelligence 的原分支或原函数。

桥以 Swift whole-module-optimization 编译成单个 object，再 libtool 打包静态库；链接 Foundation、Security、工具链/SDK 的 Swift 库搜索目录以及 `/usr/lib/swift` rpath。SDKROOT/SWIFTC 覆盖沿用已有构建习惯，工具失败即构建失败。未引入第三方依赖。父模块应以 `#[cfg(target_os = "macos")]` 注册 Rust wrapper。

已实际编译并由 vtool 读取 LC_BUILD_VERSION：x86_64 minos 10.15、arm64 minos 11.0、arm64 minos 13.0；对象在 `/tmp/uipb-build.vEsJj2`。这是 deployment 元数据与编译证据，不等于在三个 OS 版本实机运行。

## 独立桥实验

从仓库根运行 `bash native/unified-pair-auth/run-bridge-check.sh`。

脚本先在唯一 0700 `/tmp/uipb-build.XXXXXX` 中生成 0600 临时私钥；公钥通过 Rust `include_bytes!` 真正嵌入两端，编译/签名结束后才产生包含最终 CDHash 的 manifest。私钥只用于离线签名，脚本 EXIT 清理，不访问或导入钥匙串，也不替换用户证书。其他构建及证据文件保留在唯一目录。

实验分别签署带 runtime 的 Handy/Host Rust 程序及不带 runtime 的同 identifier Host 副本；manifest 明确列入两种 Host CDHash，因此弱签名拒绝确实覆盖硬化门禁，不仅是 hash 不在允许列表。测试错误公钥/profile/空 envelope、普通文件 FD、错误角色、50 次 RAII load/drop、真实严格 runtime 双向正例及弱签名双向拒绝。数据仅为合成元数据和单字节结果。

首次桥实际通过目录 `/tmp/uipb-build.vEsJj2`；严格双向认证约 4/4 ms，弱签名拒绝约 9/3 ms。测量只代表本机本次小型合成程序，不是 A09 或产品端到端延迟。排错发现 Darwin accept 继承监听 fd 的 O_NONBLOCK，实验已明确切回有界阻塞 I/O；没有放宽 5 秒实验超时。

原 43 项共享协议测试仍独立保留并在 `/tmp/uipa-build.ELHGpY` 重跑通过；桥测试不替代它。后续源码修订应重新运行两个脚本，并由非作者审查桥 ABI/所有权、构建和信任传递。

本轮最终修订补齐 Security API 返回的 CFError 所有权释放，避免反复失败验签泄漏错误对象。最终桥重跑 `/tmp/uipb-build.AQ6ynz` 全部通过（严格允许 9/4 ms、弱签名拒绝 6/5 ms）；原 43 项协议重跑 `/tmp/uipa-build.Cf3cHZ` 全部通过。已确认专用临时私钥不存在，脚本语法及限定 diff 空白检查通过。整个 Handy 应用最终构建、独立审查及实际候选端认证尚需主代理集成验证。

## 非作者审查与运行时例外回归

独立审查在 `/tmp/uipa-review-weak.CAkdwH/reproduction.log` 实际复现：带 runtime 且CDHash已在manifest允许列表中的Host，若包含 `com.apple.security.cs.disable-executable-page-protection`，原检查错误接受。现已把该例外加入拒绝集合，并保留专用测试entitlements（只供隔离夹具，不用于候选App）。

主代理重新执行完整桥脚本 `/tmp/uipb-build.46gKyn`：正常严格程序双向通过，不带runtime的已允许程序双向拒绝，关闭可执行页保护的已允许runtime程序双向拒绝。50次RAII、错key/profile/FD等检查同时通过。私钥退出清理保持不变；这是实际本地原生签名与socket测试，不是静态断言，也不是完整产品认证完成。
