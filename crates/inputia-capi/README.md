# Inputia C API

这是 Swift macOS InputMethodKit Host 调用 Rust Inputia Core 的 C ABI 桥接层。

当前导出：

- `inputia_session_new_luna_pinyin_simp`
- `inputia_session_new_with_schema`
- `inputia_session_new_with_paths`
- `inputia_session_new_luna_pinyin_simp_with_memory`
- `inputia_session_new_from_settings`
- `inputia_session_handle_char`
- `inputia_session_handle_digit`
- `inputia_session_handle_special`
- `inputia_session_snapshot`
- `inputia_session_set_app_context`
- `inputia_session_set_app_context_with_window`
- `inputia_session_learn`
- `inputia_session_voice_hotwords`
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

`inputia_session_new_with_paths` 用于后续设置页切换双拼 schema、打包内置 Rime shared data 或替换 librime dylib 路径。

`inputia_session_new_luna_pinyin_simp_with_memory` 会打开 `inputia_memory.db`，让 Host 看到已经由 typed/voice/clipboard 本地记忆重排后的候选。

`inputia_session_new_from_settings` 会读取或创建本地 `settings.json`，并把 schema、候选数量、中英文切换快捷键、标点偏好、全角/半角、拼音纠错、memory 开关和敏感 App 排除规则传入 Core/Rime/Memory。macOS Host 默认走这个入口。

测试：

```bash
cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
```
