# Inputia 专用本地测试签名

用户于 2026-09-09 明确批准创建专用本地测试证书和私钥并存入登录钥匙串；不修改全局信任，不自动授予系统权限。

## 实际创建与验证

- 名称：Inputia Local Test Signing 2026。
- 登录钥匙串：/Users/lzl/Library/Keychains/login.keychain-db。
- 证书 SHA-1：9BFDA2AC249A18FA10FDD0E649AB854B6B536BE7。
- 证书 SHA-256：6C676A482221AEA730893321DF257D8A2129AE33E32FD366FE0DC0982CCF7192。
- RSA 3072、SHA-256 签名；critical digitalSignature/codeSigning，CA:FALSE；有效期至 2029-09-07 UTC。本地测试身份，不是 Apple Developer ID，不代表可公证或正式发布。
- 私钥导入设置 non-extractable，仅为 /usr/bin/codesign 指定访问权限，没有使用允许所有应用访问的选项。
- 第一次导入因 PKCS#8 与指定导入格式不匹配失败；转换同一私钥为 PKCS#1 后成功，没有重建证书或生成第二套身份。导入时两份临时明文私钥均为 0600，完成钥匙串签名验证后已删除；钥匙串里的身份保留。未声称对 SSD 做了安全擦除。
- 没有添加用户/管理员/系统 trust setting；匹配身份显示 CSSMERR_TP_NOT_TRUSTED 与此一致。显式指定此身份仍成功执行代码签名与完整性验证，不需要把证书加入全局信任。

## 身份稳定性初步实验

使用两次独立编译、内容不同的最小 Mach-O 程序验证钥匙串签名。两者的 designated requirement 都是 identifier com.inputia.signing-probe 与同一 certificate root 指纹，不再随二进制内容哈希改变；两者通过严格完整性检查与相同显式 requirement 校验。

两次早期交叉校验命令失败是 codesign -R 参数格式问题（完整 requirement set 不能用作单个表达式，内联表达式必须以等号开头）；查阅本机 man page 后修正调用，没有放宽表达式内容、信任设置或签名要求。

证据和公开证书保留在 /Users/lzl/Library/Application Support/HandyUnifiedBuilds/inputia-signing-20260909.XYwaC8；这里不再包含明文代码签名私钥。探针只验证代码身份，不代表实际 Inputia 更新后 TCC 权限已保持，也不替代真实菜单、录音或插入验收。

后续 Inputia 候选使用同一身份构建并重新签配对清单。首次由用户确认权限后，仍需第二次同身份更新的原生授权保持实验。不得借用其他项目证书，不得为通过测试改 TCC 数据库或自动授予权限。

进一步已构建两个实际 Inputia 输入法候选 A/B（B 修正语音状态文案），二进制 SHA-256 分别为 8167919483df124c26e4bf986d7e2664a99e2efb0ff9ff60d9ace57943e24dca 和 3704ffe8a747a2fd4e464dae54947d4201321b37a5f94f1edf3e9e9420d8ab36。两者均通过完整签名检查及相同的 identifier com.inputia.inputmethod.Inputia.UnifiedCandidate + certificate root 指纹显式要求。这个结果证明实际 Host 构建的代码身份稳定，不等同于安装后的 TCC 授权保持。

控制中心也已用相同身份构建，主程序与嵌套 Qwen helper 签名均通过严格检查；control-center-fixed-build.log 留存实际签名步骤。当前构建沿用原有已准备且推理源码未改动的 Qwen helper 资源，前端增量 beforeBuildCommand 为 bun run build。

后续构建必须显式传入固定身份：Inputia build.sh 使用 INPUTIA_CODESIGN_IDENTITY=9BFDA2AC249A18FA10FDD0E649AB854B6B536BE7；Tauri 候选配置追加 bundle.macOS.signingIdentity 同一值，并继续传入 HANDY_UNIFIED_PAIR_BUILD 的 public-build.json。不得把默认临时签名构建直接覆盖固定身份的已授权测试安装。密钥不进仓库，未来其他机器需自己的签名材料；这个指纹不是可跨机器共享的私钥。

两份候选已实际更新，配对清单 pair-fixed.json 已对安装路径中的新签名重新签署。控制中心 cdhash=9cb6b8b689eb5f0731c20c4e741a0733883f966e，输入法 cdhash=8efedfc5ad3c830a78c8d0819798da5a12ccec72，Authority 都为 Inputia Local Test Signing 2026。新程序先显示权限页（包含当前应用名称和重检按钮），随后实际进入控制中心；工具未点击任何“授予权限”按钮或系统权限开关，不能据此推断以后更新也必定无需用户确认。21:03:29 UTC 的新日志证明 Enigo 初始化成功；第二次同身份安装更新后的授权保持仍未验收。
