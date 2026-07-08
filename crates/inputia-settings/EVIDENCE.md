# Inputia Settings 证据

日期：2026-07-06

## 目的

`crates/inputia-settings` 把 Inputia 输入模式、双拼 schema、候选数量、Shift 切换、标点偏好、记忆开关和敏感 App 排除规则集中成可持久化的本地配置。这个 crate 是纯 Rust/serde JSON 层，不绑定 macOS InputMethodKit，也不绑定 Handy Tauri 设置存储。

## 默认配置

`InputiaSettings::default()` 当前语义：

```text
schema_id=luna_pinyin_simp
candidate_page_size=5
shift_toggle_enabled=true
punctuation_preference=english_in_chinese
memory_enabled=true
privacy_learning_enabled=true
sensitive_bundle_ids 包含 com.1password.1password / Bitwarden / LastPass / Proton Mail 等
rime_*_path=None
memory_db_path=None
```

`load_or_create(settings.json)` 会写出同目录本地默认路径：

```text
rime_user_data_dir=<settings-parent>/rime
memory_db_path=<settings-parent>/inputia_memory.db
```

## 验证

命令：

```bash
cargo test --manifest-path crates/inputia-settings/Cargo.toml
```

结果：

```text
running 3 tests
test tests::load_sanitizes_candidate_page_size_and_empty_sensitive_list ... ok
test tests::load_or_create_writes_local_default_paths ... ok
test tests::explicit_overrides_round_trip ... ok

test result: ok. 3 passed; 0 failed
```

覆盖结论：

- 首次加载会创建 settings 文件，并写出本地 `rime/` 与 `inputia_memory.db` 路径。
- `candidate_page_size` 读取时会被限制到 1 到 9。
- 空 `schema_id` 会回退到 `luna_pinyin_simp`。
- 空敏感 App 列表会回填默认排除列表。
- `double_pinyin_flypy`、禁用 Shift、中文标点跟随输入模式、禁用 memory/privacy 等显式覆盖可 round trip。
