# Inputia 当前开发状态

更新时间：2026-10-02

这是当前代码、安装包和本机数据的状态入口。带日期的验证记录保留当时的事实，不因为后续实现而改写；如果旧记录与本页冲突，以本页和最新安装验证为准。

## 产品边界

当前用户产品名称是 **Inputia**。Handy 是历史上游名称和内部兼容标识，不再作为本机用户需要管理的独立产品。

2026-10-02 经用户授权覆盖并核对的已安装本机体验版为 Inputia 1.1.1/build85，源码提交为 `1a63e79f`，由两个 macOS 常驻组件组成（内部路径保留兼容）：

- 控制中心：`/Applications/Inputia.app`
- 系统输入法：`~/Library/Input Methods/InputiaUnifiedCandidate.app`

两者属于同一个 Inputia 产品。输入法组件由 macOS 独立注册，控制中心负责设置、语音服务、统一历史和知识库。

本次升级后，两个运行进程的签名身份和输入源恢复均通过检查，安装收据已绑定新的 release ID。此前中断的启动迁移已恢复并进入 `completed`，控制中心没有再次发生历史服务 5 秒启动超时；`history.db`、`clipboard.db` 和 `integration.db` 的 SQLite 完整性检查均为 `ok`。本机包仍使用本地测试证书，不具备公共分发资格。

## 已实现能力

### 统一历史

Inputia 的“历史记录”页面已经支持：

- 语音转写文本、录音预览和重新转录；
- 剪贴板文本、图片和文件条目；
- 搜索、来源/内容类型筛选、收藏、置顶、标题和正文编辑；
- 统一复制、插入、附件解析和修订记录。

2026-09-24 已将旧 Handy 数据迁移到候选数据域：

- 1,892 条语音记录全部迁移；
- 5,606 条新的剪贴板记录迁移，116 条重复记录按 `content_hash` 去重；
- 1,895 个录音和 1,533 张图片附件校验通过，缺失数为 0；
- 目标 SQLite 完整性检查通过；
- 已在 Inputia 界面现场打开旧录音和迁移后的图片预览。

详细记录见 [Handy 历史迁移验证](verification/2026-09-24-handy-history-migration.md)。迁移工具见 [`scripts/migrate-handy-history.py`](../scripts/migrate-handy-history.py)。

旧 Handy 应用、旧数据目录、旧偏好设置和旧 WebKit 数据已经从原位置移入迁移备份目录，Inputia 当前不依赖它们运行。备份位于：

`~/Library/Application Support/HandyUnifiedBuilds/handy-migration-20260924-retry`

### 知识库与外部 AI

Inputia 已支持统一检索来源：本地 Markdown、TXT、CSV、JSON、YAML、语音历史、剪贴板历史和显式键入片段。外部 AI 连接称为“连接外部 AI”，默认只读、默认关闭，并通过可安装的 Skill 使用 `status/search/read` 查询。

当前文件检索仍是有界关键词检索，不是自动向量 RAG；PDF、DOCX、OCR 不属于当前文件索引能力。

### 输入法个性化

已实现选词反馈、近期/频次/上下文排序、可撤销与遗忘、零拼音后续联想，以及键入/确认语音/剪贴板历史的显式幂等回填。Rime 原生用户词典和 Inputia 个性化层分开管理。

上下文排序已用真实小鹤双拼候选池验证“图书 → 馆”可以升到首位。该结果是机制和固定候选池验证，不等于真实用户留出集准确率，也不代表达到搜狗或微信输入法的整体质量。

1.1.0/build84 的候选升级包括：及时排序和选择顺序锁定、准确码个人词召回、分段整词学习、多词上下文、应用偏好、语境拒绝、合法拼写保护及诗词碎片清理。实现和验证状态见 [候选智能化验证](verification/2026-09-30-candidate-intelligence.md)。本机现已覆盖安装同时包含候选面板切行宽度修复的构建；两个运行组件身份及原输入源恢复均已验证，见 [候选面板修复与安装记录](verification/2026-09-30-candidate-panel-width.md)。实体键盘体验与自动化验证仍分别记录。

## 仍未完成的边界

- 既有原生能力已获得的用户确认保持有效；本轮新增的召回词快打、分段整词学习及语境拒绝仍需实体键盘验收。自动化状态机与 Rime 接口测试不能代替这项验收。
- 没有用户实际选词记录时，不能宣称个性化已经改善日常输入命中率。
- Rime 原生用户词典仍按自身规则学习；Inputia 个性化开关不会清除 Rime 原生词典。
- 尚未进行真实用户时间留出集的输入法准确率评测。

## 数据和恢复原则

- Inputia 候选数据域：`~/Library/Application Support/HandyUnifiedCandidate/trial-20260905`
- 迁移前后的完整数据备份必须保留，不能用旧快照覆盖当前目标库来冒充回滚成功。
- 源码工作区 `/Users/lzl/FILE/github/Handy-unified-input-system` 与用户数据分离，不因卸载应用删除。
- 当前只从系统常用位置移除了旧 Handy，迁移备份仍可恢复。

## 本机正式版升级

本机体验版保留已有配对身份、权限和数据目录，中文输入法、语音服务及剪切板控制中心完整配对构建。发布脚本为 `scripts/build-inputia-release.sh`；最新安装及验证结果见 [1.1.1/build85 体验版交付清单](verification/2026-10-01-preview-build85.md)。Apple Developer ID 签名、公证、独立安装器、干净机器验收与对外分发未在本次完成。

## 验证入口

- [历史迁移验证](verification/2026-09-24-handy-history-migration.md)
- [个性化与联想验证](verification/2026-09-19-personalization-goal.md)
- [知识库与外部 AI 验证](verification/2026-09-16-knowledge-external-ai.md)
- [候选版使用与恢复说明](verification/unified-input/candidate-user-guide.md)
