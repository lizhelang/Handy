# v69：权限查询软超时与硬截止

## 范围

用户明确要求在已安装 v68 上补硬期限，并验证恢复不会重放旧操作。本轮仅修改 IME 的 InputiaPermissionLifecycle.swift、其自检和构建版本号；保留 v68 的快捷键路由与控制中心二进制，不改系统权限。

## 故障复现

注入无系统调用的 fake permission query：第一次成功，第二次阻塞。v68 在第二次查询超过5秒后仍 ready，旧epoch仍通过准入（红测exit1）。因此仅从unknown启动时验证timeout的旧自检不覆盖该漏洞。

## 修复

- 软查询超时仍为750ms：已就绪状态允许短暂检测延迟，但不续有效期。
- 从最后一次有效确认算起5秒硬截止，不受queryInFlight影响；snapshot在准入处检查，另有独立100ms watchdog在无人读取状态时主动失效。平台调度存在延迟，100ms是轮询间隔而非实时系统保证。
- 结果接纳与硬截止更新共用model锁，过期查询即便在软时限内返回，也不能跨旧epoch重新打开；请求开始epoch和回执必须一致。
- 在途查询不退出就不创建替代线程；迟到结果丢弃，必须下一次fresh查询恢复，旧操作代际不再有效。

## 验证

专门回归证明短延迟仍允许、硬截止暂停、旧epoch拒绝、单飞不膨胀、迟到结果不恢复、fresh恢复后旧epoch继续拒绝。补充无人读取snapshot时独立watchdog通知的断言。所有查询都是合成闭包，未触发或切换macOS权限，也没有录音或重放用户文字。

独立只读审查未发现新增阻断，指出watchdog主动失效验证缺口后已补测。完整构建和安装证据待本轮追加。

## 最终证据

完整签名构建通过；build内回归输出 hard_deadline_paused=true、old_epoch_rejected=true、recovery_no_replay=true、fresh_query_required=true、singleflight=true、autonomous_watchdog=true。既有maintenance ACK/路径验证/语音派发合成检查也通过。

受控更新脚本经过当前v68维护ACK后完成安装，恢复原Inputia(Test)输入源。IME69 PID82742，动态CDHash d79360477b622eb4367d5b7182996b09334b2f81 与新包匹配；控制中心PID82736/CDHash45088d0c147e0d754dfe2c05adbd778127a9298c，二进制SHA256与更新前完全一致。两个组件健康均ready。

系统辅助权限的auth_value/auth_reason/last_modified/csreq两条记录更新前后完全一致。未切换/删除/新增系统权限，未启动麦克风或重放旧录音。

回滚目录：/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission-update-uis82kc9。补丁准备及公开授权匹配快照：/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission69-20260912。

恢复不重放的验证为真实lifecycle实现注入fake查询+旧epoch动作准入回归；没有以真实撤销系统权限冒险复现整机卡死，因此不宣称macOS权限撤销下整机绝不会再卡死。
