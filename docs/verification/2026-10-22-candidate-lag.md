# 旧输入会话阻塞候选更新

## 现场证据

用户在 1.1.26/build114 再次报告候选不出现或更新慢半拍。现场两组件来源均为
`340758b93652018449a2d6822c91f080e2a76985`，进程已持续运行约 26 小时。
2026-10-22 16:18（悉尼时间）五秒采样中，输入法主线程 3936 个样本有 2303 个
位于 `clearManagedMemoryDisplay → selectedRange → IMK XPC`。

同时段日志先记录活动控制器清理（flags=3），约 0.6 毫秒后开始非活动控制器
清理（flags=2），约 3.1 秒后才记录恢复结果。多个周期重复，每轮约 11.5 秒。
旧控制器没有候选可恢复，仍被全局内存屏障逐个查询宿主选区和 marked range。
输入法状态曾为 unknown、权限代数 327，而后台为 ready、代数 4。

发起该次屏障的是菜单状态连接握手。握手持连接单飞门，服务端等待 ACK 上限
为 2 秒；清理中的宿主查询超过此期限，造成连接失败和再次握手。探针期间
得到 connection_gate，无法补充短期权限证明。不能靠延长权限或 ACK 期限掩盖。

## 修复边界

- 旧控制器仍同步撤销全部本地增强候选、目标与选择映射，但不再取得宿主 client
  或查询选区；不能操作当前控制器的窗口。
- 已完成 CAPI 清理、没有基础候选或联想可恢复的活动控制器也不查询宿主。
  尚未清理时不根据旧空列表否定清理后可能得到的新基础候选。
- 旧恢复闭包和已排队联想回调先检查控制器与展示 token，再访问宿主。
- 当前有效输入会话的选区、marked range、敏感来源、输入源、权限和代数校验保留。
  未改动权限证明时限、ACK 时限、配对认证或输入重放规则。

## 验证

- 75 项真实生产方法的候选恢复回归通过，覆盖旧控制器零宿主查询、空基础快照、
  失焦后旧恢复闭包、撤销清理和清理后新基础候选恢复。
- 同一回归执行已安装版本 `340758b9` 的源码，成功编译，在旧控制器不应取得
  宿主 client 的运行断言失败；不是接口或编译失败。
- 46 项普通物理输入优先级回归、候选面板刷新、联想生命周期和权限探针整链检查通过。
- 完整原生 Swift 输入法编译链接及输入法权限 API 边界检查通过。

本轮使用现有当前工作树 `/Users/lzl/.codex/worktrees/inputia-player-duration/Handy`；
项目说明中的 candidate-visibility 目录已不存在，历史基线目录没有参与构建。
用户随后明确授权构建配对包并更新、重启当前应用。

## 本机安装与观察

已安装 **1.1.27/build115**，主程序 PID 49839、输入法 PID 49852，更新器的
配对校验与运行身份校验通过。两组件 designated requirement 与旧版一致，
更新器记录 `tccChanged=false`、`previousRecordingsReplayed=false`。

该制品来自 `c20744018dd2f80b798b88c1fb35bfdeab868139` 加本轮未提交修复，
构建上下文明示 `working_tree_clean=false`；私有证据目录保存 `source.patch`
及 `source-state.json`，记录补丁及关键源码 SHA，不把基准提交当作完整修复身份。
首次成套构建完成主程序后因未显式提供静态 Rime 路径停止；补齐前次使用的
已验证依赖路径后，从同一 release context 继续完整输入法构建、检查、签名及配对，
最终退出 0。未跳过输入法检查，也未触碰运行中的旧版直至更新预检通过。

安装后 7 次新鲜采样覆盖约 30 秒，两端均 ready，IME 代数保持 4、后台代数保持 1，
后台 gate_open=true。启动期有连接暂失和一次迟到证明，之后才进入上述稳定观察段；
不能声称整个启动过程无重试。此前的约 3 秒旧控制器等待未在该短观察段重现。

数据检查：原 10 条语音和 8608 条剪贴历史逐条保持，期间新增 7 条剪贴记录；
两份设置、11 个 WAV 和词库目录原有 40 个文件字节一致，5 个数据库完整性均为 ok。

更新器恢复的是安装前用户选择的微信输入法 `com.tencent.inputmethod.wetype.pinyin`，
没有自动切回 Inputia。因此本轮证明修复回归、成套安装和短期后台状态，
**仍需用户切回 Inputia，在原输入框实际键入并选词确认候选体验**。

制品目录：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/release-VUBmeqp6`。
旧程序备份：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission-update-xk8wzyxp`。
未提交、推送或公开发布。

私有采样、日志、旧版失败对照及构建产物目录：
`/Users/lzl/Library/Application Support/InputiaBuilds/candidate-lag-20261022`。
