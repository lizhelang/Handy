# Inputia permission safety patch

Vendored from crates.io `handy-keys` 0.3.4, with its MIT LICENSE preserved.
The application lockfile resolves this exact local source through `[patch.crates-io]`.

Local changes:

- Process-lifetime atomic permission predicate runs before macOS callback locks or event access.
- Blocking pause advances a generation so tracked presses and modifiers reset without replay.
- Native callbacks use try-locks, pass denied events through, and never reactivate a user-disabled tap.
- Tap creation and explicit retirement have bounded receipts. Drop never joins unfinished workers.
- Live native worker accounting survives timeout/detach; recovery must wait for actual retirement.
- Explicit native worker health distinguishes event-channel inactivity from worker exit.
- Manager 退休直接发送共享停止信号并唤醒原生循环，不依赖转发线程稍后释放 listener；实际工作线程退出计数仍是重建的前提。

Safe verification: `cargo test --manifest-path src-tauri/vendor/handy-keys/Cargo.toml --lib --offline`.
Three upstream tests that create real macOS taps are ignored by default; synthetic input integration
and example programs must not be used for automated permission-revocation checks.
