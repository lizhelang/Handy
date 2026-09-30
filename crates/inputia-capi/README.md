# Inputia C API

这是 Swift macOS InputMethodKit Host 调用 Rust Inputia Core 的 C ABI 桥接层。

当前导出：

- `inputia_maintenance_startup_check`：无参数、无 session 的固定维护门禁；只有 `ok=true` 且 `normal_start_allowed=true` 才能继续普通启动。采用系统身份，不允许路径或环境变量覆盖。返回值由 `inputia_string_free` 释放。
- `inputia_installation_load`：只读安装收据定位。
- `inputia_session_new_luna_pinyin_simp`
- `inputia_session_new_with_schema`
- `inputia_session_new_with_paths`
- `inputia_session_new_luna_pinyin_simp_with_memory`
- `inputia_session_new_from_settings`
- `inputia_session_new_from_settings_without_memory`
- `inputia_session_handle_char`
- `inputia_session_handle_digit`
- `inputia_session_handle_special`
- `inputia_session_snapshot`
- `inputia_session_set_input_mode`
- `inputia_session_set_app_context`
- `inputia_session_set_app_context_with_window`
- `inputia_session_learn`
- `inputia_session_import_handy_history`
- `inputia_session_import_handy_clipboard`
- `inputia_session_voice_hotwords`
- `inputia_session_clipboard_candidates`
- `inputia_session_completion_candidates`
- `inputia_session_free`
- `inputia_string_free`

`handle_*` 返回 JSON outcome，包含：

- `consumed`
- `commit`
- `mode`
- `composing`
- `page`
- `visible_candidates`

这个 crate 依赖 `inputia-core` 和 `inputia-rime`，但 Host 只通过 C ABI 看见稳定函数和 JSON，不直接绑定 Rust 类型。

所有 session 构造的共同入口及会创建 settings 的两个入口均先检查维护门禁；基础 fallback 不绕过。该检查不代替更新器对已经存活的 session / 进程执行停写和退出，也不构成可启动普通 writer 的 postcheck 许可。

## FFI 内存与生命周期合同

所有 21 个需要外部指针保证的导出在 Rust 中声明为 `pub unsafe extern "C" fn`，并各自附有 `# Safety`。这不改变 C/Swift 符号名、ABI 或参数；Rust 调用方须使用明确的 `unsafe` 块承担下列义务：

- 非空输入字符串必须在本次调用期间可读、NUL 结尾且不被并发修改。函数会复制需要保存的字符串；不是要求调用后继续保留该缓冲区。
- 非空 session 必须是本库成功创建、尚未释放的原始指针，独占使用。同一活跃 Rime 运行时的创建、操作与释放应在单一所有者线程串行执行，不与其他 session 操作并发。
- session 结束使用后只调用一次 `inputia_session_free`。不能传内部地址、伪造指针、已释放指针或存在其他借用的指针。
- 返回的 JSON 字符串由调用方持有，只能交给本库 `inputia_string_free` 一次；不得改变首个 NUL 的位置、偏移指针或使用其他分配器释放。
- 原有 null 行为不变：构造返回 null，操作返回错误 JSON，两个 free 接受 null 并无操作。null 检查不能使任意非空地址安全；本库没有加入伪装安全的运行期地址猜测。

启用 `unsafe_op_in_unsafe_fn` 拒绝隐式不安全操作，测试使用存活 CString、真实返回对象和串行锁，并在每个 Rust CAPI 调用处显式标注 unsafe。另有 C ABI 类型和所有 null 路径回归测试。

`inputia_session_new_with_paths` 用于后续设置页切换双拼 schema、打包内置 Rime shared data 或替换 librime dylib 路径。

`inputia_session_new_luna_pinyin_simp_with_memory` 会打开 `inputia_memory.db`，让 Host 看到已经由 typed/voice/clipboard 本地记忆重排后的候选。

`inputia_session_new_from_settings` 会读取或创建本地 `settings.json`，并把 schema、候选数量、中英文切换快捷键、标点偏好、全角/半角、拼音纠错、memory 开关和敏感 App 排除规则传入 Core/Rime/Memory。macOS Host 默认走这个入口。

测试：

```bash
cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
```
