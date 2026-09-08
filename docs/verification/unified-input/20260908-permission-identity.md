# 候选更新后权限失效的证据

北京时间 2026-09-08。上一轮分类：有进展（候选代码、安装、签名和实际 UI 证据）；不是完整闭环完成。本轮不修改 goal。

## 真实观察

- 当前候选仍停在权限页；系统“设备控制和数据访问”中的 Inputia Candidate.app 开关为 on。没有操作授权开关。
- 本机 tccd 对实际进程 PID48227 的请求使用旧要求 `cdhash H"653c775aba98f5eebed92f802c8da64936a6f60e"`，报告 `SecStaticCodeCheckValidity status: -67050`。
- 当前安装的 designated requirement 是 `cdhash H"9b95dd2db7f84da1c778978723b92d08a37b8df2"`；保留的更新前包要求与 tccd 使用的旧值完全一致。
- 因此当前新包不满足旧权限身份要求。开关显示 on 不证明新包有效获权，单纯重复重装不会稳定权限身份。不能把本次实际失败归因为缺少 Utility 或普通前端检查故障。
- 当前可用代码签名身份仅有其他项目的 Codexbar Local Code Signing Leaf v4；没有借用它，没有创建证书或修改钥匙串/系统信任，也没有重置 TCC 数据库。

证据日志位于 /Users/lzl/Library/Application Support/HandyUnifiedBuilds/loop-20260908.r0xugn/permission-cdhash-mismatch.log，只筛选本候选的签名匹配失败，不包含转写正文。

## 独立发现的界面缺陷

AccessibilityOnboarding 的初始 Promise.all 任意一项检查失败，就把麦克风和辅助功能一起显示为 needed；缺少不请求授权的重新检查入口。这会把“检查失败”错误引导为“请再授权”。修复应分别保留 granted / needed / error，提供只读重检，只有两项实际 granted 才继续既有初始化。此修复不能冒充解决上述签名失配。

## 下一次有区分力的验证

需要 Inputia 专用、可跨构建保持身份的签名材料，而不是继续临时哈希签名后要求用户每次重授权。Apple 的 [TN2206](https://developer.apple.com/library/archive/technotes/tn2206/) 说明 designated requirement 用于识别更新；固定签名身份是否满足本机各权限策略仍需实际验证，不保证“签了就自动授权”。

取得用户对专用本地测试证书/私钥的授权或可用的 Inputia 开发者签名身份后，先在隔离副本上签两个不同内容的构建，确认其身份要求相互兼容；再一次性安装配对候选，由用户核准所需权限。第二次同身份更新后检查原生有效权限是否保留，再验证真实菜单→浮窗/录音→历史→原框输出。首次授权、签名稳定性和完整原生闭环均不能由静态签名检查替代。

## 本轮局部修复与验证

已在源码修正 macOS 初始检查、轮询和重检的独立状态；检查失败不再显示 Grant。显示当前运行应用名称，名字读取失败不影响权限检查。“重新检查”不发起任何系统授权请求。输入服务初始化或设备刷新失败时保留已确认权限，显示单独初始化错误并允许重试；初始化未完成不提前显示 All set。

5 项定向 Playwright、前端 lint/build、所有语言键一致性通过。日志 permission-ui-tests-final.log。测试仅验证真实组件中的 UI 状态与命令调用约束；它没有核准本机权限，不能作为 native TCC 或语音验收。一次早期测试失败是 fixture 在 store 层替换设备刷新后仍期待底层设备命令，修正了测试断言，未放宽产品行为断言。

随后完整前端 48 项回归通过（permission-full-frontend-tests.log）。独立只读审查这两个源码/测试文件，未发现本次限定权限状态、只读重检和初始化门禁范围内的阻塞；未宣称完成整个产品审查。

本轮没有再次构建或替换原生候选，没有新建证书/私钥，没有借用其他项目签名，没有改任何授权开关或 TCC 数据。已安装仍是 044d3e6e 对应候选，权限页修正尚未安装；待固定签名授权后一起打包验证。
