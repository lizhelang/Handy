# 主历史接线与输出安全检查点

- 北京时间：2026-09-05 13:48 起，本地隔离工作区 `Handy-unified-input-system`。
- 基线：`634292d4` 后的本检查点提交；分支 `codex/unified-input-system`。
- 结论：P1/P2 分项进展，不是 P0–P6 或 A01–A12 完成报告。
- 未安装/替换日常 Handy 或 Inputia；未访问真实业务数据库，未向真实文档输入测试内容，无远程写入。

## 实现位置与合同

1. `crates/inputia-handy-runtime/src/output_ledger.rs`：操作身份绑定 item/revision/target/owner/epoch/action；SQLite 提交 claim 后才允许副作用。相同操作仅一次授权，未知不换路线。重启 prepared 变 rejected，已 claim 变 uncertain；明确未派发有专用终态，不能冒充目标确认。
2. `service.rs`、`store.rs`、`sync.rs`：唯一 writer lease 在恢复前取得；live revision/删除/epoch 约束。短期 OutputPermit 只在 GUI 读取原子代数，不在按键/发键边界等待数据库。源 manager 写入持 RAII 守卫，立即撤销旧许可、阻止新许可；claim 必须追平同步并确认源代数未改变。空同步不无故撤销。
3. `src-tauri/src/unified_target.rs`、`integration_output.rs`：主线程持有进程实例、AX元素/窗口/选择区域及激活代数，UI取得焦点前捕获；目标不明、变化或第三方组合状态不可证明时保留待插入。不读取输入正文作为目标标识。
4. `dispatch_gate.rs`、`commands/integration.rs`：排队超时取消未开始任务；已开始回执丢失保持 uncertain。AX查询前后、剪贴板准备后和发键前核验截止/许可/目标/组合状态。回执查询是独立只读命令，绝不重放 Prepared 插入。
5. `paste_tx/{mod,macos,windows}.rs`、`clipboard.rs`：历史不继承自动发送或尾随空格。Enigo try_lock，注入结果分为未派发/可能派发/已派发，不发生错误自动换路。macOS保存全部 item/type 原始字节，准备恢复对象后再检查 changeCount；不覆盖被观察到的新复制。AppKit不提供原子CAS，未把门控夸大为OS原子保证。
6. `managers/clipboard.rs`：历史复制先解码，再取得短期许可；macOS原生对象/PNG/文件URL准备在最终核验前，随后直接NSPasteboard写入，不通过可能隐式等待的插件锁。准备期间复制代数改变会中止。文件不退化为路径文本。
7. `UnifiedHistoryPage.tsx`、Sidebar及`unifiedOutputStore.ts`：主历史页真实接口接线；操作身份在副作用前持久化纯元数据，无正文/标题/附件路径。重挂载保留执行中状态，刷新后未知操作不自动重试，需明确检查后才能新操作；持久化异常安全封锁。前端状态不是跨原生窗口锁。
8. 历史重转录保留原能力：确认录音受管路径；写回在源事务中检查预期修订，避免覆盖重转录期间用户修改。

## 独立审查

`review_native_dispatch` 初审 Request Changes：多格式降级、排队迟派、慢准备后的焦点变化、吞注入错误、明确未派发误报未知。均按原边界修复。

复审继续发现复制许可、旧源删除即时撤销和AX查询期间撤销缺口；分别加原生复制准备/最终核验、源写守卫与同步屏障、双侧许可验证。最终限定范围源码复审通过，没有新增可确认P1/P2；该结论不替代原生跨应用验证或整项目独立终审。

## 本轮证据

- SQLite账本测试11项、store包装故障/回滚测试7项，均实际临时数据库；含100次重复、双连接争用、删除/修订/epoch、故障注入、重启与未知回执。
- 服务测试覆盖源写开始立即撤销、源写期间禁止新许可、源删除后不显式同步仍拒绝旧claim；许可过期和撤销均有单测。
- 主线程DispatchGate测试5项，包含排队取消、已开始未知、准备过期、目标查询期间撤销。
- Playwright `tests/unified-history.spec.ts`：18项通过（主代理复跑4.8秒）。这是实际React组件加mock Tauri/动作，不是原生输出证据。
- Handy `cargo +1.96.0 test --lib`：本检查点首次完整复跑355通过、2 ignored（原有ignore未新增）；最终复跑结果以随附工作记录为准。
- 前端build、lint、翻译一致性通过。严格Clippy原有4处needless_return在本次涉及的剪贴板模块修正；未添加忽略项放宽门禁。
- `handy --unified-target-self-check`：原生元数据检查通过，`external_focus_read=false text_read=false input_posted=false`。

### 独立原生剪贴板验证

可复现：构建调试二进制后执行 `src-tauri/target/debug/handy --unified-clipboard-self-check`。该入口在设置、Tauri窗口与用户数据库初始化前退出，仅使用 `pasteboardWithUniqueName` 分配的独立命名板，结束清空自己的测试板；不取得 generalPasteboard。

实际结果：

```text
unified_private_clipboard_self_check=pass general_clipboard_accessed=false all_original_formats_preserved=true full_snapshot_restored=true revoked_write_cancelled=true newer_copy_preserved=true
```

最初在Rust测试线程执行发现：系统补充 `public.utf16-external-plain-text`，且提示原生兑现应在主线程。按系统化调试流程复现后，将真实命名板检查移到主线程诊断入口，验证每个原始item/type逐字节保留，并要求恢复后的全部快照与实际捕获快照完全相等。没有删除失败案例或把类型丢失设为允许。

同时修复调试绑定导出的工作目录依赖：使用编译期项目绝对位置，原生诊断不再触发导出。此前本轮从项目根启动产生的生成文件已从 `/Users/lzl/FILE/github/src/bindings.ts` 移至 `/tmp/handy-unified-generated.FngYdU/bindings.ts` 保留，不涉及用户源文件。

## 尚未完成及下一步

- Inputia真实异步会话、IME组合状态证明/单一输出所有权、断线重连未完成（P3）；当前第三方IME平台路线保留待插入。
- 统一浮窗容器、全格式历史采集/召回（HTML/RTF当前明确拒绝）、图片/文件自动插入未完成；不能据此宣称A06通过。
- 共享状态损坏目前安全封锁，仍需专门可理解的恢复入口；跨webview协调仍须后台会话覆盖。
- 自动粘贴的异步剪贴板恢复失败目前记录日志，最终产品仍需可见恢复状态与验收。
- 本轮Windows仅保持接口编译形态，Windows/Linux真实构建/运行尚需最终门禁。
- 三个真实目标应用测试、固定60术语+40普通音频质量对照、性能全套、迁移/兼容回滚及可安装候选包均未完成。

所有必验项仍按批准方案核对；本报告不缩减目标，原生goal保持active。
