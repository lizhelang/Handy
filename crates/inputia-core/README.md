# Inputia Core

这是 Inputia 的平台无关核心 crate。它不依赖 macOS InputMethodKit，也不依赖 librime 类型。

覆盖能力：

- 英文直通 commit。
- Shift 切换中英文。
- 中文 composing buffer。
- 候选刷新、分页、数字选择、空格上屏。
- 退格、Escape 清空。
- 中文模式下强制英文标点，或按中文标点输出。
- typed/voice/clipboard 本地记忆影响候选和 completion。
- 用户上屏词进入语音热词候选。
- 敏感 App 默认不学习。

运行：

```bash
cargo test --manifest-path crates/inputia-core/Cargo.toml
cargo test --manifest-path crates/inputia-core/Cargo.toml --features sqlite-memory
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

当前 crate 使用 stub engine 测试 core contract，并提供 in-memory 与 SQLite memory 两个实现方向。正式接入时，`ChineseEngine` trait 后面接 librime adapter、双拼 adapter 或其他候选生成器；`SqliteMemory` 后面接 App 数据目录中的 `inputia_memory.db`，Core 只消费统一 `Candidate` 和 privacy decision。
