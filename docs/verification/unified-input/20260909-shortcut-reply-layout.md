# 快捷键回复对象布局门禁

已观察阻塞：runtime严格clippy报告HostShortcutReply的大变体。先增加现有JSON回复形状的往返测试，再仅将Trigger载荷装箱，更新broker构造；不改协议字段、认证、触发归属或Swift解码。

黄金JSON测试修改前通过（`/tmp/inputia-trigger-wire-before.log`），修改后voice_protocol全部12项通过（`/tmp/inputia-trigger-wire-after.log`）。`cargo clippy --manifest-path crates/inputia-handy-runtime/Cargo.toml --all-targets -- -D warnings` 通过，日志 `/tmp/inputia-runtime-clippy.log`。没有用allow或提升警告门槛绕过。

这是runtime该门禁和通信形状证据，不代表全项目所有平台门禁或原生语音验收通过。
