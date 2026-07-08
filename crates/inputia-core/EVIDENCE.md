# Inputia Core 证据

日期：2026-07-06

命令：

```bash
cargo test --manifest-path crates/inputia-core/Cargo.toml
cargo test --manifest-path crates/inputia-core/Cargo.toml --features sqlite-memory
```

结果：

```text
running 14 tests
test tests::english_mode_commits_characters_directly ... ok
test tests::backspace_updates_composition ... ok
test tests::chinese_mode_can_force_english_punctuation ... ok
test tests::digit_selects_candidate_on_current_page ... ok
test tests::chinese_punctuation_is_available_when_configured ... ok
test tests::escape_clears_composition ... ok
test tests::chinese_mode_builds_composition_and_candidates ... ok
test tests::shift_toggles_between_english_and_chinese ... ok
test tests::paging_changes_visible_candidates ... ok
test tests::space_commits_first_candidate ... ok
test tests::memory_reranks_engine_candidates_from_typed_history ... ok
test tests::voice_and_clipboard_history_can_create_completion_candidates ... ok
test tests::user_committed_terms_become_voice_hotwords ... ok
test tests::sensitive_apps_do_not_learn ... ok

test result: ok. 14 passed; 0 failed
```

SQLite feature 结果：

```text
running 17 tests
test tests::sqlite_memory_persists_terms_and_reranks_candidates ... ok
test tests::sqlite_memory_imports_handy_history_read_only ... ok
test tests::sqlite_memory_imports_clipboard_text_and_skips_sensitive_source_apps ... ok

test result: ok. 17 passed; 0 failed
```

本机真实 Handy 数据只读导入 probe：

```bash
rm -f /tmp/inputia-memory-import-probe.db
cargo run --manifest-path crates/inputia-core/Cargo.toml \
  --features sqlite-memory \
  --example sqlite_import_probe \
  -- /tmp/inputia-memory-import-probe.db \
  "$HOME/Library/Application Support/com.pais.handy/history.db" \
  "$HOME/Library/Application Support/com.pais.handy/clipboard.db"
rm -f /tmp/inputia-memory-import-probe.db
```

结果：

```text
history_imported=1200
clipboard_imported=1513
inputia_terms=2257
completion_count_for_prefix_中国=1
```

结论：

- Inputia Core 可以作为平台无关状态机独立测试。
- Core 不需要知道 macOS IMK、Windows TSF、Linux IBus/Fcitx 或 librime 的内部类型。
- 第一批测试已经锁住英文直通、中文 composing、候选选择、分页、退格、清空、Shift 切换和标点偏好。
- 本地记忆层已证明 typed history 可以重排候选，voice/clipboard 可以产生 completion，用户 typed 新词可以进入语音热词候选。
- 敏感 bundle id 默认排除学习，当前测试使用 `com.1password.1password`。
- SQLite-backed `SqliteMemory` 已验证 `inputia_terms` / `inputia_events` schema、持久化重开、只读导入 Handy `history.db` / `clipboard.db`、剪贴板敏感来源跳过。
- 本机真实导入只写 `/tmp/inputia-memory-import-probe.db`，没有改写 Handy 的 `history.db` 或 `clipboard.db`。

后续正式化要求：

- 把 stub engine 替换为 `Chinese Engine Adapter` trait 实现。
- 通过 `crates/inputia-handy-runtime` 接入 App 数据目录中的正式 `inputia_memory.db`；下一步是把 runtime 挂到 `src-tauri` manager 或 macOS Host。
- 扩展敏感 App policy，增加浏览器隐私窗口、银行、医疗等默认排除规则。
- 增加真实 key event 映射层测试，避免 Host 误吞系统快捷键。
