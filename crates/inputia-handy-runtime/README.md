# Inputia Handy Runtime

这个 crate 是 Inputia MVP 和现有 Handy 数据目录之间的桥接层。

它只负责三件事：

1. 固化 Handy 本地数据目录约定：`history.db`、`clipboard.db`、`inputia_memory.db`。
2. 打开 Inputia SQLite 记忆库。
3. 以只读方式导入 Handy 的语音历史和文本剪贴板历史，让候选排序、英文补全和语音热词能共享同一个本地记忆层。

当前它刻意不依赖 Tauri，后续可以从 `src-tauri` manager 里直接调用，也可以先被系统输入法 Host 进程复用。

运行测试：

```bash
cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml
```

只读导入当前 Handy 数据目录并写入 `/tmp` 探针库：

```bash
rm -f /tmp/inputia-handy-runtime-import-probe.db
cargo run --manifest-path crates/inputia-handy-runtime/Cargo.toml \
  --example import_probe \
  -- /tmp/inputia-handy-runtime-import-probe.db \
  "$HOME/Library/Application Support/com.pais.handy"
rm -f /tmp/inputia-handy-runtime-import-probe.db
```
