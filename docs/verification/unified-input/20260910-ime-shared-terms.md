# 认证短词快照到输入法英文补全

本批接通服务端取词、输入法内存短缓存与英文补全；中文Rime候选排序尚未接入，不能称完整P4。

## 实际调用链与合同

- server Hello新增可选shared_terms_v1能力；旧server缺能力时Host不发送新请求，普通输入/语音协议保持。
- SharedTermsRequest绑定client/server/policy和已经注册的lease ID/epoch，不能自报目标；broker拒绝不完整字段/来源、敏感App、错误身份和过期租约。现有认证及policy-applied通过后才读规范快照，取后复核当前版本和同一目标/剩余lease，返回寿命≤1000ms且≤剩余lease。
- Host独立取词队列与独立认证连接，单个in-flight，≥500ms调度；慢词库查询不阻塞语音poll。primary与取词连接需同server/epoch；生成号由取词连接真实barrier更新，不能伪提升primary或无限重连。
- 内存cache绑定目标/版本/身份/代数，TTL从请求发送monotonic时间计算并受原注册lease截止约束。barrier、断线、目标撤销和TTL清cache/UI；旧ticket回复不得复活缓存。不把词写回旧SqliteMemory/Rime。
- 英文补全合并符合前缀的共享短词；共享选择沿用后缀插入，不调用旧learnTyped重复计数。默认键盘刷新不直接展示共享词，只coalesced异步实时AX/敏感窗口/SecureInput/原snapshot gate通过后展示。
- 共享Tab/点击立即消费并排队，异步复核client/controller/activation/target/prefix/mode/cache/TTL后至多插入一次；失败只清共享建议，保留文稿/组合，不换路重放。按键回调不等待新增AX/IPC。

## 审查修复

首审指出纯缓存/observer检查存在焦点刚变化时的窗口，已改成上述异步实时gate及独立选择意图状态。新增10项意图自检覆盖变化取消、gate拒绝、重复开始和重复consume。没有通过放宽目标观察器修复：输入改变后仍需后台重新准备目标。

## 已验证与未验证

- Rust协议13项、broker16项及app库487通过2忽略；runtime完整回归及严格clippy all-targets -D warnings通过。
- paired完整Swift typecheck通过（`/tmp/inputia-shared-terms-final-typecheck.log`为空）；真实framed socket/短缓存自检18项+选择意图10项通过，输出明确synthetic=true/native_candidate_insertion_tested=false。
- 新自检及既有VoiceService自检已接入build.sh编译执行步骤；脚本语法检查通过。完整dev-fast仍有已报告handyMemorySyncSelfCheck失败，未宣称全门禁绿灯。
- 未安装本批；原生英文候选展示、点击/Tab、焦点变化与延迟未验证，更未达到完整中文词库/语音/历史闭环验收。

独立复审review_socket_path为Approve（限本轮接线/HIGH修复）。复审实际运行标准build.sh通过，包含18项短缓存/帧与10项选择意图检查；没有新的隐私/目标校验/重复插入阻塞。其构建不是原生插入验收，也不替代已知dev-fast其它失败。
