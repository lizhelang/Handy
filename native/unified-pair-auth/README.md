# 候选配对签名与 Unix 对端代码身份

此目录提供候选离线配对合同、Swift 共享认证实现及独立合成实验。**它不是已经通过验收的产品认证链路**：没有接入 Handy/Inputia 服务，没有启动输入法、申请权限、安装注册应用、读取用户内容或修改钥匙串。生产发行的 Apple 链、实际 Team ID 和 identifier 允许列表仍需单独实现。

## 信任前提与构建顺序

可信构建代码、可信启动的两端可执行文件、嵌入的公钥及构建私钥未被攻击者替换或注入，是本方案前提。0700/0600 仅减少误访问，不构成抵御已经控制用户账号的沙箱。攻击者删除 socket/manifest 可以造成拒绝服务；不能靠握手修复或自动重建信任。

1. 每次独立 run 由构建工具生成 P256 配对密钥，不指定 `kSecAttrIsPermanent` 为 true，不访问用户 Apple 证书。
2. 两端签名前嵌入公钥 X9.63 字节、key ID、run ID、profile ID、protocol major、固定本端角色和候选模式。公钥不从对端、manifest 或可写设置中取得。示例工具中的文件公钥仅服务合成实验。
3. 完成两端构建/签名，提取每个计划支持架构的最终 CDHash。
4. 签名外部 manifest；不得把它放回已签名 bundle 导致 hash 循环。每端 manifest 均以嵌入公钥认证。
5. 构建私钥只留在专用临时 0600 文件或内存；不进入安装包、源码、命令行内容或日志。本工具拒绝覆盖已有输出，读取私钥拒绝文件自身符号链接、非属主、组/其他用户权限、硬链接。构建流程必须提供可信专用父目录并负责私钥生命周期；这不抵御已控制账号的攻击者替换整个目录树。
6. 任何端点重新签名或变化，都重新构建一整对 run。旧 run 不共享 profile，不靠时间戳或可写文件宣称抗回滚。

## 稳定 API

- `PairBuildKey()`：创建内存 P256 构建密钥；`publicKeyX963` 可嵌入两端；`sign(payload)` 产生外部 envelope。
- `PairManifestPayload.canonicalSigningBytes()`：供构建脚本取得规范 payload。默认 schema=1、mode=candidate、protocol major=1。
- `PairTrust(...)`：只接受可信编译配置。`requireHardenedRuntime` 默认 true；false 是合成代码身份实验，不得用于开放真实业务。
- `SignedPairManifest.verify(envelopeBytes, trust:)`：先验原始 payload 签名，再解析和检查完整字段合同。
- `PeerAuthenticator.authenticate(socketFD:manifest:expectedRole:)`：双方后台队列调用，从连接内核凭据确认身份；同时核验本端代码属于 manifest 的本端角色，返回 `VerifiedPeer`。

`VerifiedPeer` 包含角色、32 字节 audit token、UID、run/profile 与 hardeningEnforced。它不等于 Recording、policy barrier 已确认或 IME 会话所有权许可。每次重新连接都要重新认证；运行中 exec/FD 转移、长期连接身份失效需要调用方重新验证并断开，不可把 PID 或上次连接的身份对象当永久权限。socket 应 close-on-exec，不转交不可信进程。

## 字节合同

Envelope 仅有 `payloadBase64`、`signatureBase64`，最大 16 KiB；原始 payload 最大 8 KiB；签名是 P256 ECDSA X9.62 DER/SHA256。

签名消息为 UTF-8 `Handy-Inputia-Candidate-Pair-Manifest-v1`，跟一个 NUL 字节，再跟原始 payload 字节。验签时不得重序列化 payload 来替代原字节。签名正确后，Swift Codable 规范重编码必须与原字节完全相同，从而拒绝未知字段、重复字段、不同空白及键序。Envelope 同样要求规范字节。构建器使用 sortedKeys + withoutEscapingSlashes，无换行；其他语言可转交原始 payload 给本 Swift 实现，不需要各自实现一套签名 JSON 规则。

Payload 仅包括 schemaVersion、mode、keyID、runID、profileID、protocolMajor、peers。两角色恰为 handy/inputia，各自 identifier 不同；每角色 1–4 个唯一、40 位小写十六进制 CDHash。identifier/keyID 仅 1–128 字节 ASCII 字母、数字、点、下划线、短横线，禁止 requirement 注入字符。runID 严格为 1–64 字节 ASCII 字母、数字、下划线、短横线；profileID 必须精确等于 `unified-candidate:<runID>`，与 Handy 实际候选数据域一致，例如 `unified-candidate:trial-20260905`。不以放宽通用 ID 字符集容纳冒号。当前构建工具 `identity` 提取宿主实际架构，通用候选打包器仍需逐架构明确收集允许 hash，不能拿单架构结果冒充通用验证。

## 动态身份与硬化边界

内核链路为 getpeereid → LOCAL_PEERTOKEN → SecCodeCopyGuestWithAttributes(kSecGuestAttributeAudit) → **SecCodeCheckValidity(动态 SecCode, identifier + cdhash requirement)**。静态 signingInfo hash 比较不作为运行身份决定；本机 SDK 明确说明 SecCodeCopyStaticCode 的文件来源关联不保证安全，而动态 CheckValidity 抵御源文件修改。

默认硬化门禁还检查 runtime 签名标记，拒绝 get-task-allow、disable-library-validation、allow-dyld-environment-variables、allow-unsigned-executable-memory。硬化静态元数据先以同一固定 requirement 校验，再重新检查动态对象。**这些是元数据门禁，不证明注入已被操作系统阻止**。尚未验证 debugged 动态状态、所有已加载库、候选 librime 加载以及 DYLD/调试注入负例。

Handy/Inputia runtime 在完成以下检查前必须失败关闭集成；Host 基础本地输入应继续独立工作：

1. 对两端主程序和所有 Frameworks/dylib 的最终文件清单、签名、实际架构、rpath/install_name 留证；拒绝日常安装、可写开发路径或未经固定的外部库回退。
2. 在候选进程读取 runtime/entitlement/dynamic status，确认不靠关闭 library validation 解决 librime 加载问题。
3. 在专用合成工具执行 DYLD_INSERT_LIBRARIES、DYLD_LIBRARY_PATH、同 UID debugger/替换 dylib 的负例；记录拒绝发生的层级及进程退出状态，不触碰日常进程。
4. 同时运行真实候选冷启动及 Rime/语音依赖加载正例；不能只用没有第三方依赖的本工具证明候选兼容硬化。
5. 真正两端在已嵌入公钥而非工具文件公钥的情况下完成双向允许/拒绝；之后才开放版本握手与策略屏障，最后开放业务。

## 可复现合成实验

从仓库根运行 `bash native/unified-pair-auth/run-self-check.sh`。脚本只在 `/tmp/uipa-build.XXXXXX` 创建三个合成可执行文件并 ad-hoc 签名，保留该唯一构建目录用于复核。可信 Handy 与 Host identifier 不同；不可信程序故意使用与 Host 相同 identifier、同 UID，但编译内容及 CDHash 不同。

自检生成内存构建密钥，在自己 0700 `/tmp/pair-auth-UUID` 内创建 0600 socket、公钥和 manifest，退出只清理该专用目录。测试包括签名允许、错误公钥/profile/run/protocol、域隔离、签名后字节篡改、**正确签名的重复/未知字段**、角色重复、超长 envelope；真实 socket 双向允许、假客户端拒绝、可信客户端拒绝假服务端、默认硬化策略拒绝普通 ad-hoc。进程间只交换单字节实验判定，不发送正文或调用录音/输入。

构建工具还提供：`keygen PRIVATE PUBLIC`、`sign PRIVATE PAYLOAD MANIFEST`、`identity ROLE BINARY`。创建路径由可信构建流程指定；运行时不得调用 keygen/sign，产品也不得开放 `--fixture-*` 工具入口。

首次实际结果：2026-09-05，Swift 6.3.3，arm64 macOS 13 deployment target + warnings-as-errors 编译通过；`/tmp/uipa-build.GB019s` 执行 23 项通过。此结果是初版源码的合成链路证据，不覆盖后续修订或整体验收；后续每次修改需重跑并记录实际新路径。

本轮最终源码重跑：`/tmp/uipa-build.Rlvh9E`，23 项全部通过；包含 SO_TYPE 检查、硬化静态元数据以固定 requirement 验证后再次绑定动态对象的修订。输出 `keychain_written=false private_key_persisted=false socket_fixture_only=true business_authentication_ready=false`。`bash -n` 与目录范围 `git diff --check` 通过。独立审查、真实候选嵌入公钥/握手、注入/动态依赖硬化及产品接线仍未完成。

跨组件 profile 合同修订后：`/tmp/uipa-build.wS7dAg`，43 项全部通过。实际 socket 夹具使用 `runID=trial-20260905`、`profileID=unified-candidate:trial-20260905`；新增 1/64 字节有效 run 边界，正确签名但非法的点/冒号/斜杠/空白/非 ASCII/65 字节 run、非派生 profile 及代码 identifier 注入字符拒绝回归。identifier/keyID 原规则不变，业务硬化默认仍为 true。
