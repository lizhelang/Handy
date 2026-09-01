# Inputia Settings

`inputia-settings` 是 Inputia 输入法偏好的本地 JSON 配置层。它不依赖 Tauri，也不依赖 macOS Host；目标是让系统输入法 Host、未来 Handy 设置页、以及测试工具共用同一份配置契约。

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
