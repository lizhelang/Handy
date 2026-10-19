# Inputia 快捷键失联与输入控制恢复：1.1.12/build100

日期：2026-10-16（Australia/Sydney）。用户明确反馈 Control + Shift + Space 完全没有反应，录音界面不出现，并要求修复造成持续失联的程序问题。本轮目标为快捷键与输入控制的可靠恢复；1.1.11 的复制粘贴实际试用仍没有收到确认，不能把本轮工作替代那项验收。

## 现场证据

排查时两组件实际安装版本为 1.1.11/build99；控制 PID 72723、IME PID 72729，进程存活约 8 小时 44 分钟。该时长不表示故障持续同样时间。多次新鲜健康记录显示后台 retiring/epoch2、IME unknown/epoch119，维护标记 active=false；快捷键配置仍为 ctrl+shift+space。主线程四秒采样主要等待事件，没有观测到持续主线程阻塞。

当时文件日志只保留重复的快捷键退休日志，每两秒出现一次“Stopped handy-keys recording mode”。此前首次关闭原因不在保留范围内，不能断言现场最初触发就是下述竞态。选中的 Qwen3-ASR 0.6B 模型属于当前用户设置，本轮保留；故障发生在录音启动之前，没有读取用户转写正文、录音或剪贴板内容，也没有操作微信。

健康发布原逻辑把“资源全部 stopped、worker 已退出、入口关闭”也写成 retiring，因此健康文件持续更新不能证明退休线程正在卡住。独立检查发现，后台 gate=false 足以让 IME 探测返回 unknown；IME unknown 状态本身仍可继续探测，当前没有证据证明是 IME 阻塞了后台恢复。

## 临时恢复的独立证据

保留异常样本和元数据，并另行在线备份四库、两份设置及十个录音后，仅正常重启控制后台。旧 PID 72723 退出，新 PID 65237 在仍为 1.1.11 的情况下 ready/epoch1；没有重启的原 IME PID 72729 从 unknown/epoch119 恢复为 ready/epoch120。没有修改 TCC 或用户设置。这证明当前权限可通过检查，IME 能随可用后台恢复；不能倒推最初故障必然没有权限变化，也不算永久代码修复验收。

## 已复现的代码缺陷

从生产源码提取实际 Lifecycle、launch 和 retire 方法，仅替换原生权限及资源操作，复现以下顺序：旧检查开始于 epoch1；期间租约过期，进入 epoch2 并要求恢复；旧检查迟到返回 stale_permission_probe；错误尾部无归属校验地再次 close_gate，清掉新一代恢复资格；之后三次被动检查都只退休，初始化次数为零，最终 gate=false、busy=false、健康仍 retiring。

另有期限错配：外层 worker 原先 1.5 秒即撤销恢复，内部 fallback 清理允许等待主线程 5 秒，正常偏慢的退休或初始化可能把自己的恢复作废。尚未观测到现场原生死锁，不把有界模拟失败等同本次首次触发。

## 修复与验证状态

已完成以下修复，并经独立复核：

- worker ID、权限 epoch 与关闭 serial 共同限定结果归属，过期检查及 watchdog 不能关闭新一代恢复；已退休的旧快捷键对象也不能先修改全局监听开关，包含同 epoch 的对象替换。
- 1.5 秒只标记缓慢。开放输入的真实权限证据仍最多 3.5 秒；原生构造在 gate 关闭时使用独立 8 秒有界许可，不能据此派发输入。全部组件完成后再次真实检查权限，再发布就绪。
- 监控每 200ms 做轻量调度，原生检查按上次启动时间至少间隔 2 秒且保持单飞；慢检查结束不会因为错过固定 tick 再空等一轮。
- 仅明确白名单的暂时故障以 1/2/4/8/8 秒退避，最多重试 5 次。旧组件必须真正退出并确认退休，才允许重建；真实拒权、维护、明确停用和未知失败保持关闭，旧按键及输出不重放。
- 真正超时但仍未退出的原生工作保留单飞占位，公开需处理状态，不能无限假报恢复中。无恢复计划且已停止的终态不再反复执行原生检查和退休。
- 显式恢复请求不会因监控抢占 worker 而无声丢失；已就绪时的旧检查也不能因显式恢复变更 serial 就拆掉仍可用的组件。
- 健康记录区分 ready/recovering/retiring/paused/restart_required/maintenance，包含固定白名单原因、worker 阶段和重试元数据。维护 ACK 按 IME 实际写入的 maintenance_marker_epoch 核对，兼容旧 marker_epoch，冲突拒绝；该协议错误并非本次 active=false 现场的根因。

## 回归证据

Rust 按筛选组通过：权限 25、快捷键 63、安全输入 9、普通口述 24；各组可能有名称筛选重叠，不把相加当作独立用例总数。

新增 `scripts/inputia-permission-recovery-self-check.py` 直接装配生产 Lifecycle、launch、retire 和 monitor，每个场景独立运行，仅原生权限、资源操作、AppHandle 和健康写盘为替身。新源码 5/5 通过；同一脚本针对旧源码的 4 个通用场景全部发生业务断言失败，正常退出，没有用编译错误代替失败证据。

- 旧 probe 结束后保留新一代恢复意图，随后仅初始化一套原生组件并恢复 ready。
- 真实等待的退休、输入设备和快捷键构造分别约 2 秒，总过程不提前开放；1008 次观测没有提前 gate/check 成功，最后权限检查发生在原生构造全部完成之后。
- 连续约 2 秒的权限检查启动于 0/2037/4072/6115ms，gate_drops=0，公开证明最大剩余 3497ms，在途检查没有给旧证明续期。
- 无计划且已停止后观察 4.4 秒，probe 与 retire 次数均不增加。
- 固定恢复错误分类保留拒权、无效配置和未知失败关闭。

脚本已挂入成套构建的预检阶段，读取签名输入前执行。完整构建及安装已完成，证据见下方。没有对用户当前 profile 做 SIGSTOP、真实睡眠或 TCC 撤权注入；生产流程替身的结论不扩大为这些场景已实测。

界面单独回归已完成 18/18，覆盖有界恢复退避、停止、拒权、维护、恢复耗尽和需处理状态，修正终态仍遗留“正在重建”的提示。测试使用独立端口、关闭热更新的模拟后端；TypeScript、定向 ESLint、格式与差异检查通过，不能替代真实快捷键验收。

## 安装证据

主程序、IME 与配套设置制品均为 1.1.12/build100，release ID 为 `inputia-1.1.12-100-fd8978b5600c-34039cfc9d92492298cd53e1deb8ee60`。完整成套构建再次通过新增恢复流程自检及项目原生自检；三个制品的深度严格签名、更新器三次配对、两组件运行身份及输入源恢复均通过。后台 PID 19592、IME PID 19792；更新器报告 `tccChanged=false`、`previousRecordingsReplayed=false`。

安装后五次读取跨 8841ms，两个组件记录均新鲜且 ready；后台 epoch 始终 1、gate_open=true、recovery_pending=false、recovery_attempts=0、error_reason=null，最后 worker 为 idle/busy=false/blocked=false。该观察覆盖几个正常探测周期，不冒充长时间稳定性或故障注入验收。

制品目录 `~/Library/Application Support/HandyUnifiedBuilds/release-Nxv39Etd`；旧应用备份为同目录体系 `permission-update-11lokpuu`。本轮证据目录 `~/Library/Application Support/InputiaBuilds/permission-recovery-20261016-v_5e71hz` 保留原故障、临时后台恢复与最终安装三个阶段的证据。构建冻结清单为 140 文件，SHA-256 `724ee51dfa9a5f35bf456bb9c1c2597fe5071a0090c519015341021680fc893a`，构建后逐文件核对一致。主工作树同步有摘要保护，无关 runtime 修改未变；后续安装文档另存差异，不冒充已编译源码。

安装前在线备份四个数据库、两份设置和十个录音。首次安装后核对四库 quick_check 均通过，语音历史、剪贴历史、个性化原行一致；integration 仅有运行对端及隐私读者元数据变化。两份设置与旧录音字节一致，当前选中的 Qwen3-ASR 模型、快捷键和语音目标模式保留。没有重置系统权限、操作微信或公开发布。

构建日志 `/tmp/inputia-permission-recovery-build100.log`，安装日志 `/tmp/inputia-permission-recovery-install100.log`，均保存到证据目录；连续状态记录为 `postinstall-health-cycles.json`。

## 实体输入验收

2026-10-16，用户针对 1.1.12 在输入框中短按 Control + Shift + Space 开始说话、再次按键结束并等待文字输入的试用，明确回复：“能开始、能结束，文字也能输入”。本次实体快捷键、录音结束和文字插入的完整链路取得实际试用确认。

该确认限定于本次试用，不扩大为长期零故障、真实睡眠或 TCC 撤权后的自动恢复均已验证；自动恢复场景的现有证据仍是上方生产流程替身回归。本轮修复收尾，不因验收确认额外修改生产代码或重新安装。
