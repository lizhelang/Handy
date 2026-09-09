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

## 剪贴板分支实际复现（2026-09-10；代码649810a0）

隔离目录 `/tmp/inputia-clipboard-replay-20260910.l0naZS`，fixture.sql只创建一条带固定ID的合成text剪贴记录，source_app为TextEdit。两次独立进程运行现有import_probe，均显式指定该目录与其独立inputia_memory.db；没有读写日常或候选数据。

两次输出均为history_imported=0、clipboard_imported=1、inputia_terms=1；`SELECT count(*),sum(clipboard_count) FROM inputia_terms`从`1|1`变为`1|2`，quick_check=ok。因此剪贴板旧导入同样确定违反重放不重复计数，而不只是源码推测。

当前配对候选的统一菜单不展示旧syncHandyMemory入口；旧设置导入、C ABI和兼容runtime仍保留此路径。不能把本次独立CLI复现说成“本轮候选正在自动重复导入”，也不能凭统一菜单未暴露旧入口就关闭兼容迁移缺口。

下一修复需要真实源库/记录身份、版本与遗忘屏障一起约束兼容导入；仅按正文去重会错误合并不同来源记录，仅保存最大时间戳又无法处理修改/删除。此次没有实施这样的不完整修复。
