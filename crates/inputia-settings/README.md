# Inputia Settings

`inputia-settings` 是 Inputia 输入法偏好的本地 JSON 配置层。它不依赖 Tauri，也不依赖 macOS Host；目标是让系统输入法 Host、未来 Handy 设置页、以及测试工具共用同一份配置契约。

## 更新维护门禁

`maintenance::current_user_context()` 从系统账户获取 euid / home，忽略环境变量；`inspect(home, uid)` 只读固定 `Library/Application Support/Inputia/Updater/maintenance.json`。只有 `Ok(None)` 允许普通启动和附件 GC；存在 marker 或任何读取、格式、权限、归属、链接异常都必须停止写入。无收据的旧 v1 也必须检查。模块不依赖运行时，不创建目录，不修改 marker。

marker 复用更新器的 schema 1 类型，严格拒绝缺失 / 重复 / 未知字段、错误 UUID/release/hash、符号链接和硬链接，文件必须归当前用户且模式为 0600。启动检查不能替代更新器持续停止进程的屏障；已运行进程由原生维护适配器停止和复核。

`inspect_read_only_postcheck` 只检查准确 transaction / epoch / installation / plan 与当前构建的新 release 收据，返回安装元数据；不打开业务库，不创建目录，不给予普通启动豁免。返回值明确标记代码签名和运行时握手尚未验证，不能当作 Security / IMK / TIS 或数据库迁移验收。生产原生适配器仍需提供这些独立证据。

## 输入偏好

当前配置项：

- `schema_id`：Rime schema，默认 `luna_pinyin_simp`，可切到 `double_pinyin_flypy` 等双拼方案。
- `candidate_page_size`：候选页大小，旧格式迁移时钳制到 1 到 9；版本化修改拒绝越界。
- `shift_toggle_enabled`：旧兼容字段；当 `input_mode_toggle_shortcut` 为 `shift` 时为 `true`，否则为 `false`。
- `input_mode_toggle_shortcut`：中英文切换快捷键，支持 `shift`、`control_space`、`none`。
- `punctuation_preference`：`english_in_chinese` 或 `follow_input_mode`。
- `character_width_preference`：`half_width` 或 `full_width`。
- `spelling_correction_enabled`：是否启用拼音纠错候选提升。
- `memory_enabled`：是否打开 Inputia memory/ranker。
- `privacy_learning_enabled`：是否允许学习本地历史。
- `sensitive_bundle_ids`：默认不学习的 App bundle id。
- `rime_dylib_path` / `rime_shared_data_dir` / `rime_user_data_dir`：Rime 运行时路径。
- `memory_db_path`：Inputia 本地记忆库路径。

## 版本化设置协调

`store::Store` 是 Unix/macOS 基础输入偏好的共同写入入口。输入法和离线设置窗口调用不依赖 Rime/主服务的 C ABI；主应用适配器也应复用此入口。现有扁平 `settings.json` 保留业务字段和未知扩展字段，保留键 `_inputia_store` 保存格式版本、store UUID、十进制字符串 revision、值摘要以及最近 256 次操作回执。设置值与成功回执在同一 JSON 原子提交，不另设第二份设置真相。

每次修改发送 `PatchRequest { operation_id, expected_store_id, expected_revision, patch }`。操作 ID 为 `v1:<store_id>:<expected_revision>:<UUID>`，不能把老 toggle 换上新 revision 重放。同 ID/参数重试返回原提交版本，不同参数拒绝；过期回执只返回当前快照和 `outcome_expired`。并发版本冲突返回当前值，调用者保留自己的尝试值并让用户重新选择。

写入在固定目录句柄下进行：独立 0600 `flock` 文件、短时排他锁、随机 create-new 临时文件、文件同步、rename、父目录同步。rename 后确认失败返回 `commit_uncertain`，必须保留相同 ID/参数重试；重放再次同步当前文件、初始化 marker 与父目录后才返回 `saved`。路径、归属、权限、链接、大小与重复 JSON 键均校验。维护门禁在打开、读取与提交前生效；持续停写仍由安装器进程屏障负责。

旧文档首次受控读取会迁移至 revision 0，默认的 Rime 和 memory 路径仍位于该 profile 内。`.inputia-settings-initialized.json` 仅保留格式与 store 身份。它存在时，缺文件或元数据绝不当作新安装静默重置。`InputiaSettings::save` 仅可 create-new 旧格式导入/测试文档，不能覆盖现有文件；生产修改统一走 CAS。

### 外部编辑与修复

手工修改导致值摘要不一致时返回 `external_edit` 并保留原文件。`inspect_external` 只读展示合法业务值与原始文件摘要；用户明确确认后 `import_external` 校验同一 store/revision/原始字节摘要再生成新版本。预览后内容再变化返回 `external_changed`，要求重新预览。坏 JSON、缺失元数据或不安全文件仍需修复，不推断默认值。摘要检测用于并发/完整性核对，不是抵御同 UID 恶意篡改的密码签名。

### 保存与实际应用

保存回执不表示输入引擎已应用。macOS 精确快照 C ABI 先核对 store/revision/digest，再实际创建 Rime session、选择方案并读回 schema 和输出选项；不存在方案不能产生 Applied。Swift 更新自己的快捷键/候选显示后分别确认已应用字段。without-memory 降级明确标记不可用字段。运行时资源覆盖和静态 Rime 路径不冒充持久路径已应用；菜单图标需要独立重启/重装判断。

`.inputia-settings-applications.json` 是有界、原子替换的观察数据，不是耐久业务回执。后台发布每个真实引擎 session 的独立 instance UUID、精确版本和字段结果；设置窗口进程本身不能产生引擎确认。读取核对内核 PID/UID/启动时间与 2.5 秒单调时钟租约，过期/退出进程排除。每进程最多 64 会话、最多 16 个未过期进程，超限报错，不驱逐存活证据来伪报全部应用。观察更新不执行 fsync，崩溃后租约失效；界面只描述收到哪些会话的近期确认，不代表所有运行程序或产品验收。

候选 profile 和正式安装使用安装收据派生的设置路径。日常旧配置默认仍为 `~/Library/Application Support/Inputia/settings.json`。本文不宣称主应用全部偏好、跨产品快捷键冲突检测或真实已装 IMK 热重载验收已经完成。

测试：

```bash
cargo test --manifest-path crates/inputia-settings/Cargo.toml
```
