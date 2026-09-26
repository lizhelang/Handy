# 三步个性化与联想 Goal：实施检查点

用户于2026-09-19明确批准设计方案三步全部实施，并请求goal模式。原目标保持不变；此文件不是完成声明。

## 范围与实现位置

1. 选词反馈、近期/频次/上下文排序、撤销与遗忘：runtime `personalization.rs`、`personalization_wire.rs`，主程序 `personalization.rs`；Swift `InputiaPersonalization.swift`、Main及认证连接适配。
2. 有界32/64候选池、真实Rime候选ID选择、零拼音预测与用户确认：core/rime/capi新增接口；Swift所有点击/空格/数字/展开选择保留原ID。预测仅点击/Tab，不劫持空格Enter。
3. typed/voice/clipboard显式回填与续批：200/500请求、来源修订绑定、删除与关闭传播、状态页只显示已验证正文、后台轮转清理；UI“输入习惯与联想”独立于全文/外部共享。

## 目前证据

- UI TypeScript/ESLint/翻译通过，12项Playwright交互通过。
- 真实静态Rime CAPI回归29项通过，包含页外候选选择、过期文本拒绝、实际全拼消耗长度。初次未设置INPUTIA_RIME_SHARED_DATA_DIR导致环境失败；显式使用候选RimeData后全部通过。
- 真实Rime用户词典实验：合成词候选原第4项，选择后升至第1；确认撤销后恢复第4。`inputia_session_undo_recent_learning`只通知Rime近期事务回退，不删除宿主正文；Swift仅对确实撤销的Rime来源调用，零码预测不调用。
- 公共词库来自已固定Squirrel 1.1.2内的rime-essay，OpenCC转简体后356723条。原始资料、许可证、转换脚本与hash记录在resources/personalization。不是用户资料，不是针对验收词硬编码。
- 实际冷启动12个前缀均有结果；合法拼音候选对+ba、图书+guan、数据+ku的语境增益通过。零码“对”的通用词频首项可能为“于”，不声称词频词典等于语法模型；真实个人选择优先于通用回退。
- runtime 12集成专项、精确并发测试、严格clippy通过。debug参考p95：在线约1.3ms，205条相关回填宽前缀约11ms，公共词库约1.6ms；后台查询数字不冒充原生完整按键延迟。
- 独立审查P1/P2已发现并交作者修复：epoch变更旧响应、管理页旧来源披露、CAS误删新版回填、展开候选ID错位、过期预测插入、文本等值误映射、失败包清空marked。最终独立复核已通过；没有未处理的代码审查阻断。
- Swift全源typecheck及个性化/合并请求/候选身份/预测admission/来源撤销selfchecks通过；Host72已完成签名构建。主程序0.10.3和IME72已成对更新成功；最终文案精确化的主程序亦完成重打包、签名配对更新和原生UI复核。

## 完成前仍需验证

- [x] 最终生产源集与构建产物一致，新增接口通过编译、集成/专项和原生管理页验证；实体键盘链路仍单独待验。
- [x] 返回前学习epoch检查、预测admit、来源删除/CAS清理的独立复核无未解决阻断。
- [x] 主程序0.10.3与IME72固定签名、配对清单、更新预检及原位替换。
- [ ] 原生设置页、本机候选排序与零码预测接受、关闭/遗忘/窗口切换后旧建议拒绝的实际检查。
- [ ] 历史回填UI使用实际后端、重放幂等及源删除后的输出行为。
- [ ] 数据/设置保留、原输入源恢复、临时诊断/测试状态清理。
- [ ] 按原设计逐项审计后，才可将goal标为complete；缺少的原生证据必须明确列出，不能用单测补称完成。

## 构建与现场信息

工作区 `/Users/lzl/FILE/github/Handy-unified-input-system`，保留所有原未提交改动。固定配对元数据与工具位于 `~/Library/Application Support/HandyUnifiedBuilds/loop-20260908.r0xugn`，私钥不得输出。更新使用 `macos/InputiaInputMethod/update-candidate.py`，资料域 `trial-20260905`。

本轮日志：`/tmp/inputia-personal-release.log`、`/tmp/inputia-personal-host-build.log`、`/tmp/inputia-personal-capi-tests.log`、`/tmp/inputia-native-rime-learning.log`、`/tmp/inputia-personal-all-tests.log`。最终已实时复核安装版0.10.3/72，最后日志为`/tmp/inputia-personal-final-release.log`与`/tmp/inputia-personal-final-update.log`。

## 2026-09-19 安装现场核验

- 更新器返回 `candidatePairUpdateVerified=true`、`candidateUpdate=true tccChanged=false previousRecordingsReplayed=false`。恢复更新前ABC输入源。运行签名由更新器验证，安装版主程序0.10.3，IME build72。
- 原生设置页显示新管理入口、默认启用的个性化策略；选键入正文→确认→实际后端返回本轮检查/导入/跳过均0（安装版当前无键入正文）。未自动导入用户语音或剪贴板历史。非空回填、续批、删除传播由隔离真实数据库测试覆盖。
- 安装版个性化数据库加载356723条公开基础词；学习证据0条，未伪造用户学习数据。
- 原生Inputia settings.json与更新前备份一致；history.db转写4→4、clipboard.db历史531→531，来源版本与操作表数量保持。
- 数据备份：`~/Library/Application Support/HandyUnifiedBuilds/personalization-0.10.3-3l_27ejj`；更新器另保留完整双应用/清单回滚包。

## 尚不能宣称的结果与边界

- 自动化键盘投递在TextEdit产生字母，未复现物理键盘的IMK候选会话。已请用户进行原生选词、重复拼音、零码Tab接词验收；未收到结果前不宣称硬件键盘端到端通过，goal仍保持active。
- 新的开关/forget/clear只控制Inputia个性化数据库；Rime原生用户词典继续按自身行为学习。最终UI已补充此说明，以及来源仅部分验证时的统计提示。
- 预测Admit拒绝授权检查之前发生的禁用/遗忘；已经通过Admit授权的短时在途插入以授权时点为准，不承诺之后任何策略变更都能零窗口取消。
- 未进行真实用户时间留出集命中率评测，不承诺达到搜狗/微信的预测质量。公共前缀词频回退不等于语法模型。

## 2026-09-19 自动化机制补证：跨进程持久化、取消与固定候选池回放

本补证轮仅修改 `crates/inputia-handy-runtime/tests/personalization.rs` 并追加本节；未修改生产 Rust、Swift、UI、配置或安装产物。以下命令在工作区 `/Users/lzl/FILE/github/Handy-unified-input-system` 执行，均通过：

```bash
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test personalization learned_preferences_survive_process_exit_and_database_reopen -- --nocapture
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test personalization cancellation_changes_neither_evidence_nor_rank_and_match_groups_stay_eligible -- --nocapture
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test personalization fixed_candidate_pool_replay_has_separate_training_and_evaluation_phases -- --nocapture
```

- **跨进程持久化**：`learned_preferences_survive_process_exit_and_database_reopen` 启动两个独立操作系统子进程，分别调用 `separate_process_persistence_child` 的 train/verify 阶段。首进程写入12个确认选择事件后退出；次进程重新打开同一个临时 SQLite 库，核对12事件仍存在、epoch未变且预期候选仍居首。父进程再次读取核对计数。此证据强于同一进程内重复调用 API，但不替代真实 IME 重启后的物理按键验收。
- **取消不学习与短码资格**：`cancellation_changes_neither_evidence_nor_rank_and_match_groups_stay_eligible` 核对拒绝 `operation=cancel` 后，学习计数及查询顺序完全不变。再建立强个人选择证据，分别把目标候选设为较短 `consumed_len`、`completion` 和 `abbreviation` 类别，核对其不挤占其他资格组槽位，且返回 ID 无新增或重复。它验证服务接口及重排约束，不声称已验证原生 Escape/退格的全部事件路径。

固定回放的预先定义如下：

- 候选池原顺序为 `[吧, 八, 巴, 把, 爸]`，对应 ID `0..4`；拼音均为 `ba`，初始 `consumed_len=2`、`match_type=exact`。这是固定机制测试池，**不是本轮实时 Rime 采集的原排序**。
- 训练阶段共有26个事件：`我→爸`（原rank4）6次、`请→把`（rank3）6次、`数字→八`（rank1）6次、`好→吧`（rank0）8次。调用的是实际反馈与持久化模型；没有直接修改权重或写入测试答案到排序结果。
- 训练结束后单独评估8个上下文：`这是我/还有我→爸`、`麻烦请/劳驾请→把`、`这个数字/那个数字→八`、`那就好/这样好→吧`。评估期间只调用 query，并核对学习事件计数始终为26，未继续反馈测试答案。
- 实测固定原顺序 **Top1=2/8（25%），Top3=4/8（50%）**；个人排序 **Top1=4/8（50%），Top3=8/8（100%）**。仍有4个首选未命中，不能声称“训练后全部首选正确”。

限制：这8例是与训练前文存在明确后缀关系的小型、合成、确定性机制回放；训练和评估的调用阶段分开，不等同真实用户时间留出集。它只说明这一固定池中的排序改善，不能外推总体中文输入准确率，也不能与搜狗、微信或实时 Rime 命中率横向比较。Rime 原生学习提升/撤销证据另见 `/tmp/inputia-native-rime-learning.log`，不与本节固定池百分比混为一项结果。物理键盘、完整窗口切换与宿主应用端到端验收仍保留前文所述缺口。

最终UI现场复核：安装版展示“启用 Inputia 个性化学习”及Rime范围说明，输入控制已就绪；更新器恢复ABC并报告tccChanged=false。未收到实体键盘验收回执，Goal保持active，未声称全链路验收通过。

## 2026-09-24 原生阻塞根因

在重新选择 `Inputia (Test)` 后，输入法进程已启动（Host73），但目标字段捕获仍失败。最新诊断为：

- `personalization.sqlite` 的 `evidence=0`、`imports=0`；
- 输入法诊断：`target_capture=unknown_code`、`capture_reply=no_reply`、`commit=missing_selection`；
- Handy 日志连续记录 `unified_target_attribute_failed attribute=AXFocusedApplication status=-25212`；
- 权限健康文件显示 ready，但该状态只反映生命周期探针，不能证明当前候选控制中心已经能够完成目标 AX 字段读取。

因此当前未形成原字段 token，选词反馈和零码 Tab 链路没有机会执行。这是候选控制中心的实际 Accessibility/AX 目标读取阻塞，不是“馆”的排序算法失败；排序算法已经通过隔离真实小鹤候选池测试。需要用户在系统设置中确认当前 `/Applications/Inputia Candidate.app` 的辅助功能条目有效后，再进行实体键盘验收。自动化 `typeText/pressKey` 仍然不能替代该验收。

## 2026-09-19 用户原生验收后的缺陷修复（仍在进行）

用户实际反馈：重复拼音排序有变化，但提交图书后的Tab不接词；图书+小鹤gr仍为管/关/官/观/馆/灌/冠。现场Inputia个性化库events=0，因此只能确认原生Rime记词，不能确认新增反馈链路通过。

隔离的真实静态Rime跨层probe复现了gr原序。旧模型收到图书上下文后仅将馆升到第三，零码预测馆第一。修复采用1.5\*ln1p(rank)作为名次先验，公共词频证据保留上限3并按frequency/(frequency+50)收缩，负反馈占优不叠公共增益。新模型真实32候选池馆首位、无上下文逐ID原序不变，18项回归及严格clippy通过；独立审查通过。报告`/tmp/inputia-flypy-model-probe.ARUKYh/run-fixed/report.json`；这是机制与真实引擎候选池验证，不是用户留出准确率。

Host73修复正文收录暂时失败错误清空个性化的耦合，只有已知原字段失效才清context；增加固定阶段/错误类别/布尔/计数诊断，不含输入正文、拼音、窗口名或token。独立隐私/门禁审查通过，已与主程序诊断包签名更新（`/tmp/inputia-personal-diagnostic-update.log`）。最终模型修复主程序正在构建，仍等待现场诊断回执以确认新增上下文入口的具体失败位置；不能因算法单测通过宣称Tab已修复。

本轮最终安装：修复后的主程序已与Host73完成固定身份配对更新，更新器返回candidateUpdate=true、tccChanged=false，恢复原输入源。构建日志`/tmp/inputia-personal-context-fix-release.log`，更新日志`/tmp/inputia-personal-context-fix-update.log`。现场尚未产生新的personalization阶段日志或学习事件；已发出一次用户诊断复现请求。两个已证实缺陷已修复并安装，但上下文入口根因与Tab端到端仍未闭环，不标记完成。

## 用户确认的原生验收结论（2026-09-24）

用户明确确认：这一步的实体键盘候选排序、上下文联想和 Tab 接受已经自行验证通过，不要求继续重复测试。此前自动化输入只能产生 Latin 文本，不能作为原生验收证据；本条以用户实际操作结果为准。原生验收的 AX 诊断日志保留为故障定位历史，不再作为当前未完成项。
