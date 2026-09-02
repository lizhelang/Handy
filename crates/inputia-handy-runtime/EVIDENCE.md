# Inputia Handy Runtime 证据

日期：2026-07-06

## 已验证内容

`crates/inputia-handy-runtime` 已把 Handy 本地数据目录约定封装成正式 API：

- `history.db`：语音转写历史，只读导入为 `MemorySource::Voice`。
- `clipboard.db`：文本剪贴板历史，只读导入为 `MemorySource::Clipboard`。
- `inputia_memory.db`：Inputia 自己的 SQLite 记忆库。

runtime 不依赖 Tauri，可以先由测试、探针和后续系统输入法 Host 复用，再接入 `src-tauri` manager。

## 测试命令

```bash
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
```

结果：

```text
running 3 tests
test tests::data_paths_match_handy_storage_contract ... ok
test tests::import_existing_sources_tolerates_missing_handy_databases ... ok
test tests::import_existing_sources_reads_handy_databases_without_mutating_them ... ok

test result: ok. 3 passed; 0 failed
```

## 真实 Handy 数据只读导入 probe

命令：

```bash
rm -f /tmp/inputia-handy-runtime-import-probe-real.db
cargo run --manifest-path crates/inputia-handy-runtime/Cargo.toml \
  --example import_probe \
  -- /tmp/inputia-handy-runtime-import-probe-real.db \
  "$HOME/Library/Application Support/com.pais.handy"
rm -f /tmp/inputia-handy-runtime-import-probe-real.db
```

结果：

```text
history_imported=1203
clipboard_imported=1520
inputia_terms=2263
completion_count_for_prefix_中国=1
```

结论：

- 新 runtime 可以从当前 Handy 数据目录读取真实语音历史和剪贴板文本。
- 导入目标是 `/tmp` 下的 Inputia 记忆库，未写入现有 `history.db` / `clipboard.db`。
- 合成数据库测试验证了源表行数导入前后不变。
- 来自 `com.1password.1password` 的剪贴板文本不会进入 completion，敏感 App 默认不学习规则在 runtime 层仍然生效。
