# 知识库与外部 AI 接入验收

日期：2026-09-16。实现工作区：`Handy-unified-input-system`。保留此前未提交的输入、权限、录音等改动；本轮没有提交、发布或替换已安装的 Inputia Candidate。

后续用户明确要求替换现有 Inputia，已完成下述原位升级；前述不替换描述仅对应首轮预览验收。

## 已安装升级：0.10.1

- 继续使用 `com.pais.handy.UnifiedCandidate`、原固定代码签名证书和 `trial-20260905` 数据域，输入法组件二进制保持原版本 70。
- 通过 `update-candidate.py` 预检和事务升级，新配对清单验签通过。工具报告 `candidateUpdate=true`、`tccChanged=false`，并恢复升级前的微信输入法。
- 已安装控制中心运行 PID 45553，CDHash `a8b907b5898b5be02a5eae9f1d5f77071bf680b2`；输入法运行 PID 45559，CDHash `8d0739419874d25c76161780d9e73f3c2ab6f4fa`，动态身份校验均通过。两组件健康均为 ready。
- 原生界面确认 v0.10.1、Inputia 输入控制已就绪，知识库和“连接外部 AI”已在现有安装中可见。
- `.md` 与 `.txt` 分别通过导入、关联目录收录；使用实际已安装程序运行外部 skill 的中文搜索及逐条原文读取矩阵测试通过（2 项外部进程测试）。
- 语音历史 4 条完全保留；原 369 条剪贴板记录无 ID 丢失、正文无变化，升级期间正常新增到 372 条，仅 1 条既有记录的 created_at 因重复复制更新。
- 数据备份：`~/Library/Application Support/HandyUnifiedBuilds/knowledge-install-20260916`；程序与清单事务备份：`~/Library/Application Support/HandyUnifiedBuilds/permission-update-2y1fxd_s`。
- 独立预览进程已退出。没有将预览版合成知识文件导入用户知识库，没有自动启用实际历史的外部共享。本轮未新增真实麦克风转写测试，不以健康状态代替语音闭环验证。

## 已交付行为

- 知识库支持关联目录、导入 UTF-8 文件、新建 Markdown 笔记、选择托管目录、刷新与后台 30 秒对账。
- 本地关键词搜索合并文件、现有语音与剪贴板记录；片段携带来源和修订，读取时重新校验文件 hash 或源库身份/版本/记录存在。
- 来源收录与外部读取独立设置，外部默认关闭；撤销后旧引用不可读取。
- “连接外部 AI”生成 skill 和本机脚本，并生成提示词，教对方 AI 根据自己的宿主安装整个技能目录、检查能力并执行查询。不会替其他 AI 猜测安装位置。
- CLI 随主程序交付：主程序 `--knowledge --root PATH status|sources|search|read` 在 Tauri/单实例/录音初始化之前处理。也提供独立 `inputia-kb` 开发工具。
- 引导流程提供“仅使用知识库”，不需要先完成语音权限和模型设置。

## 自动验证

- `cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml`：165 项通过，包含文件、历史、连接包、CLI 真实子进程和既有集成服务测试。
- 上述测试设置 `INPUTIA_KB_NATIVE_APP` 为已打包预览程序，`knowledge_external` 通过导出的查询脚本调用真实应用可执行文件；覆盖文件/语音/剪贴板混合查询、read、撤销、源记录已删但统一索引仍旧、拒绝写操作。
- 新知识库 UI Playwright：6/6 通过，测试 IPC mock 的交互契约，包括无语音初始化的独立入口、导入部分错误、截断提示和复制提示词。
- `bun run build`、完整 `bun run lint`、`bun run check:translations` 通过。
- runtime crate `cargo clippy --all-targets -- -D warnings` 与 `cargo fmt -- --check` 通过。
- 主应用 cargo check/build 与独立身份的 Tauri debug app 打包通过，存在原有 dead-code/future-incompatibility 警告。
- 全仓 `bun run format:check` 未通过：本轮之前已存在的 `src-tauri/vendor/handy-keys` 四个文件不符合 Prettier。主应用 rustfmt 另有既有 `ime_target_broker` 模块位置差异；未为本功能重排这些原有修改。本轮新增文件与前端文件已定向格式化。

## 真实原生流程

启动独立 `com.pais.handy.KnowledgePreview`，数据隔离于该应用 ID，未读取用户原 Inputia 资料。

1. 在权限引导页点击“仅使用知识库”，实际知识库页面可用。
2. 创建合成笔记“外部 AI 接入验收”，保存后索引显示 1 条，搜索结果出现实际文件与行号。
3. 仅为该合成文件来源开启外部读取；生成提示词与真实 skill 路径。
4. 点击“复制提示词”，原生 UI 确认“提示词已复制”（实际 clipboard plugin 权限已配置）。
5. `quick_validate.py` 验证动态生成的 skill 合法。
6. 独立子任务复制技能包到临时技能库，仅按 SKILL.md 运行 status/search/read，从真实应用返回的笔记正确找出 `blue paper airplane`，附对应文件行号。没有直接读 SQLite 或原笔记，也没有依赖预先给定答案。

预览产物：`src-tauri/target/debug/bundle/macos/Inputia Knowledge Preview.app`。预览窗口已打开，可继续试用。

## 明确边界

- 本版为关键词检索，尚无向量索引、PDF/DOCX 解析或 OCR。
- 文件单体上限 5 MiB；分段上限 1600 字符；目录扫描有深度、项数和字节预算，超限提供警告。托管目录切换不会迁移旧文件。
- 现有 SavedSnippet 枚举没有实际可验证的正文生产源，界面显示不可用；键入内容可通过新建笔记收录，不声称已实现全量键盘记录。
- 应用运行时自动对账；应用关闭时只读 CLI 可查已有索引，发生文件变化时拒绝旧引用并提示打开应用同步。CLI 不自动启动语音或索引写入。
- Skill 接入需要 AI 能访问同机文件并执行命令；纯网页/另一台机器不会仅凭本机路径接通。返回片段可能进入外部模型上下文。
- 当前真实原生验证平台为 macOS。PowerShell 适配脚本已生成，但 Windows 原生运行未验证。
