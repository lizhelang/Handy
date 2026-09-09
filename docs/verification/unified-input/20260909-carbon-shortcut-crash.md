# 候选系统输入源查询崩溃

## 原生发现

权限入口候选 `c52fdc51` 安装后，输入法真实日志在 2026-09-09 22:15:17（Sydney）出现 `target_registered field_observable=true`；随后观察到 provider 无目标。此为更新后连接证据，不代表录音插入或服务端撤销已通过。

尝试准备离线测试时，运行身份检查发现控制中心 PID 668 已不存在，因此没有执行 kill。实际崩溃报告 `/Users/lzl/Library/Logs/DiagnosticReports/handy-2026-09-09-221532.ips` 确认：

- bundle `com.pais.handy.UnifiedCandidate`，路径 `/Applications/Inputia Candidate.app/Contents/MacOS/handy`，PID 668。
- `EXC_BREAKPOINT / SIGTRAP`，faultingThread 23。
- `TSMGetInputSourceProperty → isValidateInputSourceRef → islGetInputSourceListWithAdditions → dispatch_assert_queue → _dispatch_assert_queue_fail`。
- Inputia PID 88734 继续运行，随后日志 `listener_unavailable automatic_trigger_replay=false`。

只提取进程身份和崩溃栈，不复制完整诊断报告或用户正文。

## 原因与修复范围

`shortcut/handler.rs` 从原生后台快捷键回调同步进入 `host_shortcut_broker::current_input_source`，后者没有主线程保护即调用 Carbon。仓库 `input.rs` 已明确同类 TIS API 必须主线程。

修复：macOS 统一快捷键入口异步交给主线程，仅探测 Carbon 后放入单一后台队列；服务查询与动作由同一后台消费者按序处理，取消也走该入口。回调及主线程不等待 IPC/存储；Carbon 查询额外检查 MainThreadMarker，后台直接返回 Unknown；无租约且 Unknown 不再回落普通粘贴。非 macOS 不改调度。

独立首审发现“整个 handler 移到主线程”会被 policy_epoch 服务查询阻塞，已拒绝该方案并收窄到上述主线程探测/后台路由。新增队列仅解除该已观察崩溃及审查阻塞，不是额外服务进程。

`/tmp/inputia-main-thread-shortcut-tests.log`：14 项 broker 测试通过，包括后台查询不调用 Carbon、未知输入源不能降级、活动会话目标冻结与一次消费。

## 未验证

修订后独立复审 `review_carbon_thread_fix` 为 Approve；Tauri 库回归 471 通过、2 忽略，日志 `/tmp/inputia-carbon-fix-full-tests.log`。均不替代候选原生验证。

修改尚未装入候选，需独立审查、重新签名构建/配对，再在原生路径复核。不能因为权限导航曾通过而忽略此发布阻塞。真正麦克风闭环仍需用户方便时验证，不自动录音。

另独立只读定位 A12 得到后续阻塞：当前 snapshot restore 不是保留新增数据的兼容回滚，旧 import 仍直接进入 SqliteMemory.learn，Host shared_terms barrier 不覆盖旧 memory/Rime。此轮因实际崩溃优先，未执行恢复或改变真实数据。
