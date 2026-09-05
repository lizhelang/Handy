# 基础实现检查点：阶段证据，非完成报告

工作区：`/Users/lzl/FILE/github/Handy-unified-input-system`。
起点：`b7d7db70`。执行计划提交：`2a59f35d`；领域合同提交：`a5aede7c`。
时间：2026-09-05。本检查点的其余代码在本地工作树，后续提交需引用本报告并重新运行受影响检查。

## 当前已实现

- Core integration：稳定身份/序列、唯一输出所有者、目标进程/控件/焦点/编辑代数、未知回执不重派、2 秒个性化租约、独立隐私授权、受限热词。
- Runtime protocol/transport：大端长度分帧 JSON、256 KiB 上限、2 秒操作 deadline、profile/epoch 握手、同 UID 验证、私有端点、不删除替换 inode。
- Runtime source/store：源表事务触发 outbox、源操作幂等回执、索引原子消费、稳定来源记录身份、删除墓碑、正文修订、游标恢复和完整快照对账。
- Runtime sync/service：后台唯一连接所有者，规范索引提交后确认源事件，ACK 后索引丢失恢复快照，源缺失明确失败而不创建替代空库。
- Learning ledger：调用者事务内的可撤销贡献、来源删除屏障、私有词身份标记、forget/relearn/epoch 检查；尚未与 store/Host/模型的业务调用合并。
- Handy：启动源 outbox 前调用一致性备份；注册统一历史、修订、刷新接口与变更通知。debug 导出绑定不启动 Tauri 或读取用户数据。

## 实际命令和结果

所有命令在上述隔离工作区执行，未启动日常 Handy/Inputia，数据库测试使用临时合成数据。

1. `cargo +1.96.0 test --manifest-path crates/inputia-core/Cargo.toml --features sqlite-memory`：58 passed，0 failed；包含新增 integration 21 项。
2. `cargo +1.96.0 test --manifest-path crates/inputia-handy-runtime/Cargo.toml -- --nocapture`：当时 49 passed，0 failed。分组：旧 runtime 3；history_service 2；integration_store 18；protocol_transport 14；source_outbox 10；sync_pump 2。
3. `cargo +1.96.0 test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test learning_contributions`：6 passed，0 failed。该文件在上一条全套运行之后补齐，因此不把其覆盖计入上一条。
4. 两个 crate 的 `cargo +1.96.0 clippy ... --all-targets -- -D warnings`：通过；Core 使用 sqlite-memory feature。
5. `CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo +1.96.0 check --manifest-path src-tauri/Cargo.toml --lib`：通过，约 1m15s。
6. 在 `src-tauri` 中执行 `CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo +1.96.0 run --bin handy -- --export-bindings`：编译及导出成功，约 1m32s；随后 Prettier 格式化自动生成文件。
7. `CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo +1.96.0 test --manifest-path src-tauri/Cargo.toml --lib`：335 passed，0 failed，2 ignored。保留既有跳过项，不能将其视作通过。
8. 前端 `bun run lint`、`bun run check:translations`、`bun run build`：已通过基线；绑定更新后 build 另行复核。Vite 仍有原有 chunk 大小提示，Rust 依赖 block 有 future-incompatibility 提示。

## 数据与性能证据边界

- Source 50000 记录完整快照：单独运行 331 ms；与其他构建并发时 402 ms。查询计划从 SCAN source_row 改为 SEARCH source_row USING INTEGER PRIMARY KEY。
- Store 50000 混合记录批量导入与三次重放测试通过；并行构建负载下初次导入记录约 11015 ms。没有据此声称 A10 的 200 次搜索/500 次事件与界面交互 p95 门槛通过。
- 100 次真实 Unix 握手/status/断开：本次约 33 ms。使用同一测试进程里的真实 socket 与线程；不是跨应用输入，也不是按键性能测试。
- 快照可恢复当前投影与源删除状态；过往修订仍需要规范库备份恢复，不能把“重建索引”宣传为任意数据库损坏后的全历史无损恢复。

## 独立审查与修复

1. 显式热词绕过控制标记检查：已提取共同检查，trim 前拒绝控制符，新增原反例测试；独立审查明确关闭。
2. ACK 回收后游标丢失返回空：已加 SnapshotRequired、with_snapshot 及索引 restore_source_snapshot；真实 SQLite 恢复后接收下一序列测试通过。
3. 快照 LEFT JOIN CAST 主键使恢复 O(N²)：已更改到整数主键查询，增加 50000 行完整快照和 EXPLAIN 验证。

后续 service、learning ledger、Tauri接线、Swift profile 仍要独立复审；现有结论不等于最终发布审查通过。

## 所有必验项仍按原范围保留

P0 的真实 IMK 目标、完整签名认证及来源覆盖率仍未验收；P1 的全业务事务、策略重核、旧库映射、兼容回滚构建尚未闭合；P2–P6 尚未交付。A01–A12 全部未达到最终完成标准。尤其真实三应用输入、100 段音频质量对照、最终安装包和回滚演示没有新证据，不得以本检查点替代。
