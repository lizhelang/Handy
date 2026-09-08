# 剪贴历史快捷召回接线

执行时间：2026-09-08，北京时间。继续使用 codex/unified-input-system，底座 51174918；不修改原生 goal。

## 实际缺口与本批修复

原 Inputia 菜单 History 打开控制中心历史页，未打开原 ropy 浮窗。原浮窗只查询 clipboard 来源，确认操作不等待复制结果就关闭，未消费 confirm_mode。

本批将真实菜单 History 路由到 show_clipboard_overlay，先捕获目标、不先打开控制中心。原浮窗保留搜索、预览、收藏、置顶、标题、分页、帮助及复制来源设置；列表改为共享历史 source=all，保留 item_id、revision 与完整文件路径，图片通过受管资源解析。语音保留录音引用，没有把录音当图片或伪造附件尺寸。

复制/插入复用输出账本及回执。成功确认才关窗；未知、仅派发、待目标继续保留，不自动重派。显式纯文本复制使用独立 CopyPlainText 后端身份；修改与删除使用源事务 operation UUID。采集关闭状态不被自动打开。

主要中文入口统一为“剪贴历史”，保留 Inputia 青绿色和已修复的标识比例。内部 unified 标识、历史数据域及兼容身份不为改名而迁移。

## 独立审查与隐私修正

审查发现新的源记录删除不会清理录音/图片，直接称删除会误导用户。已把所有条目删除入口（D、Delete、按钮）统一接入站内确认框，明确“仅删除记录；录音、图片仍留在本机，原始文件不删除”。取消不调用后端；确认使用原记录/修订快照；未知事务仍使用同一 UUID。

此修正只解除“用户以为附件也被清除”的接入阻塞，不代表完整附件生命周期或遗忘验收通过。完整可选附件清理、引用计数及遗忘原生验证仍未完成。

## 验证边界

- 前端测试证明共享查询、语音身份、受管图片解析、快捷键、确认模式、回执阻重放及删除确认；不替代原生运行。
- Rust source_mutation、history_service、output_ledger 定向验证通过，覆盖重复删除、修订及身份冲突、普通复制与文本转换身份隔离、未知回执不重派。
- 原生 Inputia 候选构建、签名和最低系统版本检查通过。控制中心候选构建及实际菜单浮窗验证继续记录在下方运行记录。
- 全量翻译键一致性通过。其他语言新增提示暂采用英文，不把键一致性称为本地化质量验收。

未完成：图片/文件直接插入仍安全拒绝；源没有可信 HTML/RTF 原字节，普通富文本复制仍拒绝，仅可显式转为文字；完整语音原框插入、焦点变化、快捷键所有权整合及 P0–P6/A01–A12 总验收不因此通过。

## 证据位置与运行记录

持久构建根：/Users/lzl/Library/Application Support/HandyUnifiedBuilds/loop-20260908.r0xugn。

日志：recall-product-build.log、recall-inputia-build.log、recall-playwright.log、recall-runtime-tests.log。构建使用现有 public-build.json 配对信任；私钥不输出、不随候选分发。

截至接线提交：尚未新增真实菜单打开浮窗或实际复制/插入证据，不能把构建和 mock 测试写成已运行通过。

## 候选更新与恢复约束

只更新 /Applications/Inputia Candidate.app 与用户级 /Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app；日常应用及系统级输入法不变。更新前退出候选并备份候选包、数据和旧配对清单，再对两份实际安装包签配对清单。

普通应用回退只恢复旧候选包和它对应的配对清单，保留当前数据。不得为了回退 UI 覆盖旧数据库，使新发生的删除/遗忘复活。数据备份仅用于隔离故障演练，不能直接覆盖现用数据；完整跨版本兼容回滚仍须独立验收。

## 本机候选安装结果（北京时间 2026-09-08 20:49）

实现提交 044d3e6e。完整构建后，为纳入审查补丁和最后文案，再执行同一配对配置的前端增量打包（复用本批已完整重编译的 Qwen helper）；最终日志 recall-product-final-build.log。最终 43 项 Playwright、runtime 全量 131 项测试通过；这些仍不是原生输出验收。

旧包保存为 Inputia-before-recall.app / InputiaIME-before-recall.app，实际被替换包另外留为 Inputia-replaced-recall.app / InputiaIME-replaced-recall.app。候选数据克隆 profile-before-recall 的三个数据库 quick_check 均为 ok。日常包和数据不变，无删除文件。

两份新安装严格签名验证及包含配对信任的 profile 自检通过，pair-recall.json 已签名并用于现候选。控制中心 cdhash=9b95dd2db7f84da1c778978723b92d08a37b8df2；输入法 cdhash=44498732b16e31dfddcb2e8b201938802a46d597。本机成对候选保存于构建根 package-recall-044d3e6e；未公证、未发布，配对材料仅供此隔离测试配置。

TIS 恢复候选输入源后 selectCurrentMatchesTarget=true，控制中心 PID48227、输入法 PID48240。12:49:13 UTC 的新日志出现 unified_voice_listener_ready。实际新控制中心截图保留 Inputia 标识和青绿色，但权限页显示麦克风/辅助功能均需授予；不能把旧版已授权或启动日志替代新版本有效权限。

已请求用户仅从真实系统输入法菜单点击“剪贴历史”，不录音、不插入文稿。工具之前无法可靠取得系统输入源菜单，未用底层 invoke 或新增旁路冒充菜单验证。截至本次记录尚未收到这个新菜单操作的结果；原生浮窗、图片/文件复制及原框插入仍未验证。
