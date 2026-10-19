# Inputia 普通输入与系统快捷键优先：1.1.11/build99

日期：2026-10-15（Australia/Sydney）。用户在 1.1.10 运行约 9 小时后反馈，使用 Inputia 时 Cmd+C / Cmd+V 有时没有反应，普通输入也偶尔失败。本轮处理已复现的增强选词等待和后续按键丢弃；不将合成回归冒充本次现场已完全定因。

## 证据

排查时主程序和 IME 均为 1.1.10/build98，两组件 ready，短时 CPU 约 1.5% / 2.2%。IME 三秒线程采样的主线程主要等待系统事件，没有捕获到持续阻塞。累计 permission_epoch 2126 只说明历次边界变化，不能据此单独推断当前系统权限异常。

仅提取固定白名单诊断，不读取输入或剪贴板正文、不操作微信。最近二十分钟有个人服务 timeout、policy/no_reply 和目标忙的记录，这些能证明等待路径有实际失败，不足以直接绑定某次用户的 Cmd+C/V。已记录的普通语音结果均 dispatched；源码表明该日志在标准粘贴事务清理后才写出，剪贴历史轮询只读系统剪贴板，未发现监控覆盖普通复制内容。

用生产 `deferPersonalInput`、`finishDeferredPersonalInput` 以及真实 Deferral/BoundaryWait 方法，仅替换 IMK 客户端和原字段证明，复现：增强选择 pending → 普通字母被 return true 消费并排队 → 准入无回复 → Cmd+C 等待约 751ms、Cmd+V 约 752ms → 系统快捷键最终透传，但之前消费的字母没有重放且队列已清空。nil 证明、范围失效、同步重入也能触发丢弃；有效证明对照可以重放。权限/缓存清理另有直接清队列分支。该试验没有发送真实系统按键，也不宣称等同用户现场。

## 修复语义

后续真实输入优先于尚未完成的增强选择。新的普通打字或宿主命令到来时，取消未执行的增强选择，继续当前原始 IMK 事件；不再暂存该物理输入，也不等待最多 750ms 的远程准入。中文按键继续经过 Rime，复制粘贴按既有宿主命令路径透传。

选择键自身的 keyUp 与修饰旗标变化直接继续原路径，不排队、不等待，也不误取消选择；仅在真正的独立 Shift 模式切换入口取消未完成选择，并保留当前 Shift 手势。迟到的准入和目标回执必须通过原 token 检查，取消后不能插入。保留原字段、权限、隐私代数、一次提交与重放检查，不允许凭缺失证明把旧键补到未知输入框。

已知要因命令或编辑边界清空的自有上下文直接清空，不再先同步查询输入框选区与组合范围；普通文字仍保持原有检查。取消时缺少 proof 的路径先短路，不为一个必然不可重放的结果继续查询原字段。

行为边界：如果增强候选还未核验上屏就继续打字或使用系统命令，尚未完成的那次选择会取消；原 Rime 组合保持，新的字按当前组合处理，不伪造候选代替用户选择。已经上屏后的连续推荐仍按原流程继续。本轮不更改语音目标模式或 Rust 粘贴事务。

检查还发现按键注入层忽略释放失败的错误路径，可用错误注入替身复现，但没有取得真实释放失败证据；它不在本次最小修复中，也不能用已派发日志把它完全排除。

## 验证、安装与试用

专项脚本 `physical-input-priority-self-check.py` 抽取实际生产方法和 admit/validate/managed 回调，使用原生输入边界替身：新代码 46/46 通过，旧代码同一脚本 36 项行为失败，旧源正常退出而非编译失败或崩溃。最小 C/V 对照为原先各等待 750ms、普通字交付 0，修后各等待 0ms、普通字交付 1；这是替身环境中的路径耗时，不代表任意真实应用的端到端延迟。

完整配对 IME Swift 类型检查、候选生命周期（含完成后连续推荐）、25 项面板回归、个性化流程、构建脚本语法及差异检查通过。独立复核确认旧字段/权限/安全输入准入没有放宽，取消后的个人推荐与托管记忆迟到回调均被 token 拒绝。新增专项已挂入完整原生构建。

对应日志为 `/tmp/inputia-physical-input-priority-current.log`、`/tmp/inputia-physical-input-priority-before.log`、`/tmp/inputia-physical-priority-typecheck.log`、`/tmp/inputia-physical-priority-lifecycle.log`、`/tmp/inputia-physical-priority-presentation.log`、`/tmp/inputia-physical-priority-personalization-flow.log`。Rust 粘贴和语音目标路径没有改动，无须把重复语音测试当作本次复制粘贴验收。

## 安装证据

1.1.11/build99 已完成完整构建和本机安装，release ID 为 `inputia-1.1.11-99-fd8978b5600c-eb008a8b50b643ea858b6ddb6d8a3c25`。完整构建再次执行新增 46 项专项及全部项目原生自检并通过。三个制品的深度严格签名检查、更新器三次配对检查、两组件运行身份和输入源恢复均通过；后台 PID 72723、IME PID 72729，安装后均 ready。更新器报告 `tccChanged=false`、`previousRecordingsReplayed=false`。未公开发布或宣称公证完成。

制品目录 `~/Library/Application Support/HandyUnifiedBuilds/release-h84vzJmG`；旧应用备份为同目录体系 `permission-update-sorrcnh5`。源码、数据及诊断证据保存在 `~/Library/Application Support/InputiaBuilds/input-priority-20261015-qbn91ahx`。最终清单 135 文件，SHA-256 为 `bab29d92fdb4efd167e5aa709a508e72faed8edd99e4fd26dd7e36297ac6e243`。为并行完成检查，在生产代码冻结后开始构建，期间只完成新测试脚本及本验证文档；构建开始摘要与最终差异均已保存，生产源码没有中途改变。主工作树有摘要保护的同步保留了无关 runtime 修改。

安装前在线备份四个数据库、两份设置和 10 个录音。安装后四库 quick_check 通过，语音历史、剪贴历史、个性化原行一致，两份设置和旧录音字节一致；安装后首次核对显示 integration 的 learning_sources 既有行更新；随后单独核对确认既有行只改变 revision、无删除，同时新增 1 行来源记录，另有运行对端和隐私读者元数据变化。语音目标缺字段继续使用已验证默认 `current_cursor`，没有重写用户设置。

构建与安装日志分别是 `/tmp/inputia-input-priority-build99.log`、`/tmp/inputia-input-priority-install99.log`，均复制到证据目录。安装记录属于构建后的文档增量，不冒充已编译源码。未操作微信、修改系统权限或自动重放旧内容。

## 真实试用

已请求用户保持使用 Inputia，正常打字和空格选词后，用 Cmd+C / Cmd+V 复制粘贴一段选中文字，并试选词后立即粘贴。目前仍待用户确认；上述替身回归和成功安装不能替代真实 IMK 验收。
