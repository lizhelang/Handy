# Inputia Settings

`inputia-settings` 是 Inputia 输入法偏好的本地 JSON 配置层。它不依赖 Tauri，也不依赖 macOS Host；目标是让系统输入法 Host、未来 Handy 设置页、以及测试工具共用同一份配置契约。

## 更新维护门禁

`maintenance::current_user_context()` 从系统账户获取 euid / home，忽略环境变量；`inspect(home, uid)` 只读固定 `Library/Application Support/Inputia/Updater/maintenance.json`。只有 `Ok(None)` 允许普通启动和附件 GC；存在 marker 或任何读取、格式、权限、归属、链接异常都必须停止写入。无收据的旧 v1 也必须检查。模块不依赖运行时，不创建目录，不修改 marker。

marker 复用更新器的 schema 1 类型，严格拒绝缺失 / 重复 / 未知字段、错误 UUID/release/hash、符号链接和硬链接，文件必须归当前用户且模式为 0600。启动检查不能替代更新器持续停止进程的屏障；已运行进程由原生维护适配器停止和复核。

`inspect_read_only_postcheck` 只检查准确 transaction / epoch / installation / plan 与当前构建的新 release 收据，返回安装元数据；不打开业务库，不创建目录，不给予普通启动豁免。返回值明确标记代码签名和运行时握手尚未验证，不能当作 Security / IMK / TIS 或数据库迁移验收。生产原生适配器仍需提供这些独立证据。

## 输入偏好

当前配置项：

- `schema_id`：Rime schema，默认 `luna_pinyin_simp`，可切到 `double_pinyin_flypy` 等双拼方案。
- `candidate_page_size`：候选页大小，读取时钳制到 1 到 9。
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

`InputiaSettings::load_or_create(path)` 会在配置不存在时写出默认配置。默认派生路径位于配置文件同目录：

```text
settings.json
rime/
inputia_memory.db
```

macOS Host 当前默认读取：

```text
~/Library/Application Support/Inputia/settings.json
```

测试：

```bash
cargo test --manifest-path crates/inputia-settings/Cargo.toml
```
