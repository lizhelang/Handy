# Safari 交错激活时候选窗不显示（1.1.40）

日期：2026-10-26。分支 `codex/inputia-safari-candidate`，基于产品分支 `codex/unified-input-system` 的 `8477918f`（改名合并），工作树 `/Users/lzl/FILE/github/Inputia-safari-candidate`。代码提交 `63f0a06b`，版本提交 `8445f1aa`（1.1.40/build128）。

## 现象与定位

用户在 Safari 网页输入框里输入时只看到带下划线的拼音，没有候选窗；稍后又自行恢复。

- 两端健康状态都是就绪；已安装输入法的合成数据自检（`--unified-runtime-self-check`、`--bridge-self-check`、`--bridge-default-chinese-self-check`）能正常出候选；Rime 用户目录已在新位置部署，方案与设置和改名前一致。排除了引擎、数据与改名迁移。
- 输入法日志（只记阶段、固定原因与数量）显示：组字期间排期结果为 `not_active`；随后 Safari 激活了新会话，旧会话停用时清掉的候选数为 7，且它当时不是活动会话。也就是引擎已给出候选，但候选窗只归活动会话，接收按键的会话不是活动会话。
- 原因：Safari 网页输入框会交错激活多个输入会话（激活 A、激活 B、停用 B，再把按键发给 A）。B 停用时清掉了活动标记，A 没有再次被激活，候选窗就一直不归任何会话。B 停用还会关掉全局的键入采集。候选窗只归活动会话的限制自 `3666802d`（2026-10-19）起存在，与改名无关。

## 修复

- 新增 `InputiaSessionActivation`：每个会话记录自己的激活期。
- 会话收到宿主按键时，若仍在激活期而活动标记不是自己，就收回活动标记、重新挂上候选窗设置回调并恢复键入采集，同时记录诊断 `session_reclaim/host_key`（只含候选数量）。
- 已停用或从未激活的会话不能收回，原有防止旧会话抢占候选窗的保护不变；停用时仍先补完延后的回车上屏，再清除激活期。

## 自检

- 新增 `InputiaSessionActivationSelfCheck`（构建时编译运行），按控制器相同规则重放四类序列：交错停用后无人持有候选窗、收到按键的激活会话收回、已停用会话不能收回；两个会话并存激活时归属随按键转移；正常切换后旧会话不能抢回；从未激活的会话不能收回。11 项全部通过。
- 主程序按构建脚本文件列表做类型检查，配对与非配对两种变体都通过；IMK 事件路由检查通过；构建中的 15 个 Python 自检全部通过（其中回车上屏自检 74 项）。

## 构建与安装（1.1.40）

- 成套构建 `scripts/build-inputia-release.sh --build-local`（构建目录已改为 `InputiaUnifiedBuilds`），三组件 1.1.40/128，严格签名校验通过，构建内自检全部通过（含新增的会话激活自检与回车上屏自检 74 项）。制品目录 `~/Library/Application Support/InputiaUnifiedBuilds/release-gt9G95ME`。
- 安装前在线备份五个数据库与两份设置（副本完整性均 ok），预检 `updatePreflight=true permissionRecordsUnchanged=true`；`--apply` 一次成功，两个运行组件身份校验通过，`tccChanged=false`、`previousRecordingsReplayed=false`；旧应用备份 `permission-update-mreif95v`。
- 安装后两端健康状态均为 ready，控制中心输入模拟、快捷键与目标均为 ready，CPU 约 2%。安装瞬间的 8 条"快捷键注销通道掉线"警告出现在旧进程进入维护之后、新进程语音监听就绪之前，来自 1.1.39 退出过程。
- 历史与学习数据核对（只比对条数、字数合计与最大编号，不读正文），安装前后完全一致：语音 2050 条 / 166278 字 / 最大编号 4179，剪贴 9023 条 / 1751257 字，键入 1520 段 / 5677 字，录音 2051 个，剪贴图片 2991 个，学习事件 3917 条，历史词对 71991 行，导入来源 2948 条，联想表登记不变；录音保留仍为"从不"，各库完整性 ok。
- 本版同时首次装入产品分支上的 `d1fac07e`（统一历史显示语音输入目标与复制来源）。
- 证据目录 `~/Library/Application Support/InputiaBuilds/safari-candidate-1140-20261026`（构建日志、安装前后快照、预检与安装日志、数据库在线备份）。

## 未覆盖

- 自检重放的是激活顺序规则，没有驱动真实的 Safari；实际效果以用户在 Safari 网页输入框中的使用为准。
