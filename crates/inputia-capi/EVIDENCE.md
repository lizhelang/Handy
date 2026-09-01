# Inputia C API 证据

日期：2026-07-06

## 目的

`crates/inputia-capi` 是 macOS Swift `InputiaInputMethod` Host 与 Rust Inputia Core/Rime Adapter 的桥接层。它避免在 Swift Host 中重写输入状态机，也避免把 Rust 类型直接暴露给 Swift。

## ABI 设计

导出函数：

```text
inputia_session_new_luna_pinyin_simp(user_data_dir, candidate_page_size)
inputia_session_new_luna_pinyin_simp_with_memory(user_data_dir, memory_db_path, candidate_page_size)
inputia_session_new_with_schema(schema_id, user_data_dir, candidate_page_size)
inputia_session_new_with_paths(schema_id, dylib_path, shared_data_dir, user_data_dir, candidate_page_size)
inputia_session_new_from_settings(settings_path)
inputia_session_handle_char(session, unicode_scalar)
inputia_session_handle_digit(session, digit)
inputia_session_handle_special(session, special_key)
inputia_session_snapshot(session)
inputia_session_set_app_context(session, bundle_id)
inputia_session_learn(session, source, text, bundle_id)
inputia_session_voice_hotwords(session, limit)
inputia_session_free(session)
inputia_string_free(value)
```

`handle_*` 返回 JSON outcome。当前 Swift Host 使用字段：

```text
ok
consumed
commit
mode
composing
page
visible_candidates[].text
```

## Rust C ABI 测试

命令：

```bash
cargo test --manifest-path crates/inputia-capi/Cargo.toml -- --nocapture
```

结果：

```text
running 6 tests
test tests::capi_can_open_double_pinyin_schema_when_prepared ... ok
test tests::capi_drives_core_with_rime_full_pinyin_when_available ... ok
test tests::capi_loads_candidate_count_and_punctuation_from_settings_file ... ok
test tests::capi_loads_shift_setting_from_settings_file ... ok
test tests::capi_memory_reranks_candidates_and_respects_sensitive_apps ... ok
test tests::capi_typed_commits_respect_current_app_context ... ok

test result: ok. 6 passed; 0 failed
```

覆盖路径：

```text
C ABI -> InputiaCore<RimeEngine> -> librime -> 候选 -> Core 分页 -> Space 上屏
```

断言：

- Shift 后 `mode=Chinese`
- 输入 `zhongguo` 后首候选为 `中国`
- PageDown 后页码为 1 且首候选不再是 `中国`
- PageUp 后回到 `中国`
- Space 后 `commit=中国`
- `inputia_session_new_with_paths` 可用临时 shared data 打开 `double_pinyin_flypy`
- 小鹤双拼输入 `vsgo` 后首候选为 `中国`，Space 后 `commit=中国`
- `inputia_session_new_luna_pinyin_simp_with_memory` 可打开 SQLite memory。
- 学习 clipboard 来源 `种过` 后，输入 `zhongguo` 首候选从 `中国` 重排为 `种过`，且 candidate source 为 `clipboard`。
- `com.1password.1password` 来源的 `密码 候选` 被标记为 `excluded`，不会出现在 voice hotwords。
- voice 来源 `语音 热词` 和自动 typed commit `种过` 会出现在 `inputia_session_voice_hotwords`。
- `inputia_session_new_from_settings` 会读取 settings 文件；禁用 Shift 后 `KEY_SHIFT` 不再切到中文。
- settings 中 `candidate_page_size=2` 会让 `visible_candidates` 只返回 2 个候选。
- settings 中 `punctuation_preference=follow_input_mode` 会让中文模式下逗号提交 `，`，而默认 `english_in_chinese` 会保留英文标点。
- `inputia_session_set_app_context` 会影响自动 typed commit 学习；当当前 app context 为 `com.1password.1password` 时，上屏 `中国` 不会进入 voice hotwords。

测试过程中曾复现一个重要约束：两个 C ABI 测试并发初始化/析构 librime 会触发进程崩溃；加入进程内测试锁后全绿。这支持当前架构判断：正式 Host 应集中管理 librime 生命周期，不能让多个 Host session 自行并发 initialize/finalize。

## Swift Host 桥接自检

命令：

```bash
./macos/InputiaInputMethod/build.sh
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-self-check
```

结果：

```text
bridgeSelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=中国
commit=中国
```

Memory-enabled 自检：

```bash
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-memory-self-check
```

结果：

```text
bridgeMemorySelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=种过
commit=种过
```

Settings 自检：

```bash
macos/InputiaInputMethod/build/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod --bridge-settings-self-check
```

结果：

```text
bridgeSettingsSelfCheck=true
mode=Chinese
composing=zhongguo
firstCandidate=中国
candidateCount=5
commit=中国
```

结论：

- Swift Host 已能链接 Rust staticlib。
- Swift 进程内可以调用 C ABI。
- C ABI 继续调用真实 Rust Core/Rime。
- Swift Host 默认通过 `inputia_session_new_from_settings` 从 `Application Support/Inputia/settings.json` 初始化 session，失败时才回退到直接 memory-enabled session。
- 诊断命令用临时库验证本地记忆可以影响候选排序；设置自检用临时 `settings.json` 验证 Host 能从配置创建 session。
- Swift Host 已按 Squirrel 的成熟路径调用 `IMKTextInput.bundleIdentifier()`，并把宿主 app bundle id 传给 `inputia_session_set_app_context`，用于 typed commit 的敏感 App 排除。
- 下一步是把 IMK 真实 `inputText`/`didCommandBySelector` 事件进入该 bridge 的行为，在系统 enabled input source 通过后做宿主 App 验证。
