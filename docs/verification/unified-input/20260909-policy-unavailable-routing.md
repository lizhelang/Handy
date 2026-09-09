# 策略服务失败不能改路插入

基线 `76ea02e0`，接续实际 Carbon 崩溃修复的快捷键调用链。工作区保持候选隔离。

检查发现 `route_inputia_host_shortcut` 原先对 manager 缺失或 policy_epoch 查询失败使用 `?`，返回 None；外层随后调用普通 coordinator.send_input。这与已选择 Inputia 的输出唯一性合同冲突：查询失败不能冒充非 Inputia 输入源并改路。

修复：路由不再以 Option 表达“跳过判断”；broker 接受可缺失 epoch。没有版本时不选用预备租约、不建立新会话，也不回落 Legacy；已有 active lease 仍保留原 session/target/epoch，允许发送停止边沿。broker 本身缺失时返回 HostPending。

复审发现必须保留普通录音（非 Host-owned）的停止边沿。因此 HostPending 仅发送 LegacyContinuation，协调器在自己的串行队列内确认没有 active_voice、当前 Recording 且 binding 相同才处理；Idle/Processing 不启动新录音。没有用跨线程 is_recording 快照来授权新操作。新增测试覆盖 Toggle/PTT 的空闲不启动、已有录音停止及处理阶段不启动。

`/tmp/inputia-unavailable-policy-tests.log` 中15项broker测试通过，新增用例真实检查：有预备租约但版本未知不发Start；正常Start后版本服务不可用仍转发相同session/target的非Start停止触发。它是路由回归，不是故障注入原生录音证据。

旧导入重复学习本轮未修改。旧事件缺少来源记录身份，不能从相同正文推断同一来源；简单正文去重会误合并不同合法贡献，给旧事件补猜测来源又会破坏升级数据。需要继续接入规范来源身份与遗忘屏障，而非通过停用同步绕过问题。

当前已安装的代码仍为 `51935bea`；本修复尚未重建安装。真实录音、焦点变化、故障重连和旧数据回滚均仍未完成验收。

最终 Tauri 库回归 473 通过、2 忽略，日志 `/tmp/inputia-policy-final-tests.log`。PTT 首次测试误把释放宽限期当成立即停止，已按既有 RELEASE_GRACE 模拟到期，不改产品宽限规则。独立复审确认执行逻辑阻塞解除，并要求修正该测试；现已通过。无原生故障注入通过声明。

## 候选安装接续

代码提交 `322df3b1` 已完成固定本地测试证书签名构建，日志 `/tmp/inputia-policy-fix-candidate.log`。产物 `src-tauri/target/release/bundle/macos/Inputia Candidate.app` 更新到 `/Applications/Inputia Candidate.app`，实际 PID 11454 的运行身份校验匹配 CDHash `151f13eb9ce70888602e582eba6f2dcd75a4d16e`。CUA 实际打开设置窗口正常，22:43:39 Sydney 日志显示 unified_voice_listener_ready。

旧候选包与 pair-before.json、重新签署的 pair-new.json 保留在 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/policy-fix-20260909.txgqB1`。替换前核验旧 PID 10342 身份，停止并确认退出，再替换候选；未改日常安装、真实数据库或输入法包。未 notarize，非正式发布。

上述只验证构建/安装/启动，不代表本故障条件已原生注入或录音闭环已通过。
