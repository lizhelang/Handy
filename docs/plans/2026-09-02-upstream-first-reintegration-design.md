# Handy 上游优先重接设计

日期：2026-09-02
上游基线：`cjpais/Handy@fbd4e15fa14a721c66c57006ae110428b9e255b3`
现有能力快照：`codex/pre-upstream-reintegration-snapshot@3407a51740e8d9cd3e3017b96008b9bb5c5c7de5`

## 背景

当前 Handy 以官方 `v0.9.0` 之后的代码为起点，增加了剪贴板管理器、Inputia 系统输入法、本地记忆、FunASR/Sherpa 模型和原生热词等能力。与此同时，上游已在录音协调器、快捷键状态机、粘贴事务、Secure Input、音频采集、模型下载、设置迁移和 macOS 生命周期等位置继续演进。

当前快照与实时上游从 `f96a7afb` 分叉：快照侧独有 83 个提交，上游侧独有 142 个提交。直接合并会同时改动大量核心文件，并让旧实现与上游新状态机混用。迁移必须以能力合同和数据合同为中心，不能按旧文件逐个覆盖。

## 目标

1. 以上游固定提交 `fbd4e15f` 为新代码底座，完整获得该点位之前的所有上游行为与修复。
2. 完整保留剪贴板历史、收藏、置顶、命名、图片附件、设置和浮窗行为。
3. 完整保留 Inputia 设置、Rime 用户数据、本地记忆、候选排序、敏感应用排除和 Handy 历史只读导入。
4. 完整保留当前未提交的 FunASR/Sherpa、模型 catalog、native hotwords、自定义词和纠错能力。
5. 在任何持久化结构变更前自动备份；迁移失败时自动恢复旧数据。
6. 通过上游门禁、定制能力测试和真实数据只读验收后，才允许切换主分支或安装新构建。

## 非目标

- 不在迁移过程中重新设计剪贴板或 Inputia 的产品交互。
- 不为了减少冲突而删除上游功能或现有定制功能。
- 不让迁移器静默重置无法解析的设置。
- 不让新版本直接在唯一一份用户数据库上做不可逆试验。

## 方案比较

### 方案一：在当前分支直接合并上游

优点是提交历史连续。缺点是核心文件双向修改多，容易得到能够编译但状态机语义混杂的结果，且难以证明每个上游修复仍然成立。

结论：拒绝。

### 方案二：保留当前底座，只挑选上游提交

优点是短期冲突较少。缺点是无法获得上游录音协调器、粘贴事务和设置结构的整体演进，后续继续累积同步债务。

结论：拒绝。

### 方案三：以上游最新结构为底座重新接入定制能力

优点是上游成为长期可跟进的主干，定制能力通过清晰边界接入；每一阶段都能分别验证上游行为和定制行为。缺点是首次迁移工作量最大。

结论：采用。

## 总体架构

迁移在独立 worktree 和分支 `codex/upstream-first-reintegration` 中完成。旧工作树保持原样，完整快照分支作为代码恢复点。

新底座按以下边界组织：

1. **Upstream Core**：录音协调器、音频、快捷键、Secure Input、粘贴事务、模型下载、tray、overlay、设置和 Tauri 生命周期保持上游实现。
2. **Clipboard Capability**：以独立 manager、commands、store、overlay entry 和设置入口接入，不绕过上游粘贴事务。
3. **Transcription Extensions**：FunASR/Sherpa 和 native hotwords 通过上游 model catalog、capability 和 transcription backend 扩展点接入，不复制旧 coordinator。
4. **Inputia Subsystem**：`crates/inputia-*` 与 `macos/InputiaInputMethod` 保持独立进程/库边界，通过明确的数据路径和只读导入读取 Handy 历史。
5. **Migration Layer**：在 manager 初始化和 schema 变更之前执行备份、校验、迁移和恢复；不把备份逻辑散落到各 manager。

## 数据合同

### Handy 数据目录

运行时必须自动探测普通 AppData 和 portable `Data/`，不能依赖人工选择。至少保护：

- `history.db`、`history.db-wal`、`history.db-shm`
- `clipboard.db`、`clipboard.db-wal`、`clipboard.db-shm`
- `recordings/`
- `clipboard_images/`
- Tauri store/settings 文件
- 模型 catalog 中与本地模型状态有关的持久化信息

### Inputia 数据目录

至少保护：

- `settings.json`
- `inputia_memory.db` 及 WAL/SHM
- `rime/` 用户数据目录
- Inputia 安装与 schema 选择所依赖的用户配置

### 剪贴板不变量

迁移前后必须保持：

- `content_hash` 集合不减少且不重复
- 总记录数、收藏数、置顶数、非空标题数不下降
- `full_text`、`image_path`、`source_app`、`created_at` 和 `size_bytes` 保持
- 所有被数据库引用的图片附件存在
- 已有 ID 在无需重建时保持；需要重建时提供可审计的映射

### Inputia 不变量

迁移前后必须保持：

- `inputia_terms` 与 `inputia_events` 记录和关键计数
- typed、voice、clipboard 来源语义
- app policy 与敏感应用不学习规则
- schema、双拼、候选数量、标点和中英文切换设置
- Rime 用户词典和部署状态

## 自动备份与恢复

备份目录采用时间戳和迁移 ID，包含：

- `manifest.json`：源路径、备份路径、类型、大小、mtime、SHA-256、迁移阶段和应用版本
- SQLite 一致性副本：优先使用 SQLite backup API；不能使用时先确认写入已暂停，再连同 WAL/SHM 捕获
- 附件和配置目录快照
- `status.json`：`prepared`、`migrating`、`verified`、`restored` 或 `failed`

流程为：

1. 获取单实例迁移锁并阻止相关 manager 开始写入。
2. 自动探测所有数据路径并生成 manifest。
3. 创建一致性备份，逐项计算校验和。
4. 对备份副本做可打开性检查。
5. 执行幂等 schema 迁移。
6. 校验数据库统计、关键字段和附件引用。
7. 成功后标记 `verified`；失败时保留失败副本并自动恢复备份。

恢复必须幂等。备份不能在一次成功启动后立即删除，由显式保留策略管理。

## 迁移阶段

### 阶段一：建立上游底座和恢复护栏

- 新建独立 worktree/分支。
- 保持上游测试原样通过。
- 实现数据清单、备份 manifest、SQLite 一致性备份、校验和恢复。

### 阶段二：剪贴板能力

- 移植数据库 schema、manager、commands、store、设置入口和 overlay。
- 将“复制/粘贴”接到上游 paste transaction，避免覆盖上游的剪贴板恢复与 modifier 修复。
- 迁移测试锁住收藏、命名、置顶、图片和单击复制行为。

### 阶段三：FunASR、Sherpa 与热词

- 按上游 catalog、download manager、model capabilities 和 transcription backend 接口重新接入。
- 保留 native hotwords 的上下文生成、回显清理、去重和越界替换防护。
- 重新生成 bindings 和所有语言键，不能用旧生成文件覆盖上游。

### 阶段四：Inputia

- 移植 `crates/inputia-*`、macOS Host、资源、安装与验证工具。
- Handy 历史和剪贴板继续只读导入；Inputia 只写自己的 memory 与 Rime 用户目录。
- 保留候选、分页、双拼、隐私、设置和安装 readiness 自检。

### 阶段五：全量验证与切换

- 对真实数据先做只读清单和备份演练，再在副本上执行迁移。
- 运行前后端、Rust、翻译、格式和 Inputia 专项门禁。
- 生成可安装构建并在隔离数据副本上 smoke。
- 所有门禁通过后才允许更新主分支和替换本机应用。

## 错误处理

- 获取迁移锁失败：不迁移，不启动会写入目标 schema 的 manager，返回明确诊断。
- 备份或校验和失败：不触碰源数据。
- schema 迁移失败：保留失败副本，恢复备份，记录阶段和错误链。
- 设置解析失败：保留原文件，报告字段级错误；禁止自动写回全默认值覆盖用户配置。
- 附件缺失：迁移判失败，不把数据库标记为 verified。
- Inputia 或模型运行时不可用：不删除现有配置和模型，报告能力不可用原因。

## 测试与验收

### 数据迁移

- 临时目录覆盖空数据、旧 schema、当前 schema、WAL 未 checkpoint 和半迁移状态。
- 备份 manifest 的路径、大小和 SHA-256 可复算。
- 注入失败后自动恢复，恢复后的 SQLite 可打开且关键统计一致。
- 对真实数据只读记录迁移前统计，在副本上验证迁移后统计。

### 剪贴板

- 总数、hash、收藏、置顶、标题和图片引用一致。
- 单击复制、按钮事件隔离、收藏筛选、标题编辑和标题/内容搜索可用。
- 与上游 paste transaction 协作后，原剪贴板非文本内容仍可恢复。

### FunASR/Sherpa/热词

- 模型出现在 catalog 与 onboarding；下载配置和 capability 正确。
- backend 可选择并完成测试转写路径。
- 热词去重、上下文长度、回显清理和代码术语纠错测试通过。

### Inputia

- Core、Rime、C API、settings、Handy runtime 测试通过。
- 全拼、双拼、候选分页、上屏、隐私排除和 memory 排序通过。
- macOS Host 的非 GUI 自检、安装 readiness 和构建验证通过。

### 项目门禁

- `bun run lint`
- `bun run format:check`
- `bun run check:translations`
- `bun run build`
- `cargo test`（`src-tauri/`）
- Playwright smoke（Vite 服务可用时）
- Inputia `dev-fast` 与 release full-check 中不需要系统权限的层级

## 切换与回滚

迁移分支通过全部验收前，不改变现有 `main` 和本机安装。切换时保留：

1. 代码快照分支和提交；
2. 用户数据迁移前备份及 manifest；
3. 上一个可运行应用包；
4. 失败构建和失败数据副本的诊断路径。

回滚时先停止新应用写入，按 manifest 恢复旧数据，再恢复旧应用包。恢复完成后复核数据库可打开、关键统计一致、录音和图片附件存在。
