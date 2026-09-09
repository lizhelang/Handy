# 旧导入重复学习：隔离实验失败

基线 `51935bea`，仅在 `/tmp/inputia-legacy-import-20260909.lw8btX` 创建合成 SQLite 数据库，没有读取日常源目录。

源 history.db 只有一条合成语音文字，表字段为 timestamp / transcription_text / post_processed_text。执行仓库现有 `import_probe` 可执行程序两次，每次均显式提供目标数据库和源目录，不使用其默认日常路径：

```sh
cargo run --manifest-path crates/inputia-handy-runtime/Cargo.toml --example import_probe -- /tmp/inputia-legacy-import-20260909.lw8btX/inputia_memory.db /tmp/inputia-legacy-import-20260909.lw8btX
```

每次后查询 `SELECT count(*),sum(voice_count) FROM inputia_terms;`：首次 `1|1`，再次 `1|2`。日志保留在该目录的 run-one.log / run-two.log。

结论：相同源数据、同一目标库、进程重新打开后重复旧导入会增加学习计数。该结果违反同步重放不重复计数合同；不能把现有规范服务的幂等测试当作这条旧链已安全。

源码路径：inputia-core::SqliteMemory::import_handy_history → learn → UPSERT 累加 voice_count；旧 clipboard 导入同样调用 learn，但本轮未实际验证 clipboard 分支。尚未实验规范 forget 后旧导入复活，不从此次重复计数结果冒称已验证遗忘。

后续必须把实际 Host 旧导入接到带来源身份和遗忘屏障的兼容链，或者提供保留原能力的兼容迁移替代。不得仅删除菜单、禁止同步或恢复旧快照来宣告 A03/A08/A12 成立。当前优先处理已发生的原生 Carbon 崩溃，未修改旧导入行为。
