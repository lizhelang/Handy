# 基线测试证据摘要

日期：2026-09-05 08:11–08:14 +0800。
源码：`/Users/lzl/FILE/github/Handy-upstream-reintegration`，`b7d7db70`，测试前后 git status 干净。
以下为本次工具执行真实输出的精简转录，不是完整原始日志，不是最终实现验收。

```sh
CARGO_TARGET_DIR=/tmp/handy-unified-baseline.xRVDF1/settings cargo +1.96.0 test --locked --offline --manifest-path crates/inputia-settings/Cargo.toml
# test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

CARGO_TARGET_DIR=/tmp/handy-unified-baseline.xRVDF1/runtime cargo +1.96.0 test --locked --offline --manifest-path crates/inputia-handy-runtime/Cargo.toml
# test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

CARGO_TARGET_DIR=/tmp/handy-unified-baseline.xRVDF1/runtime cargo +1.96.0 test --locked --offline --manifest-path crates/inputia-core/Cargo.toml --features sqlite-memory
# test result: ok. 37 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

CARGO_TARGET_DIR=/tmp/handy-unified-baseline.xRVDF1/capi cargo +1.96.0 test --locked --offline --manifest-path crates/inputia-capi/Cargo.toml
# test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 17.22s

CARGO_TARGET_DIR=/tmp/handy-unified-baseline.xRVDF1/capi cargo +1.96.0 test --locked --offline --manifest-path crates/inputia-rime/Cargo.toml --lib
# test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

CARGO_TARGET_DIR=/tmp/handy-unified-baseline.xRVDF1/capi cargo +1.96.0 test --locked --offline --manifest-path crates/inputia-rime/Cargo.toml --test core_flow rime_engine_drives_inputia_core_full_pinyin_flow_when_available -- --nocapture
# Actual librime module initialization output preceded the result; no skip message.
# test rime_engine_drives_inputia_core_full_pinyin_flow_when_available ... ok
# test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.35s

swiftc macos/InputiaInputMethod/Tools/InputiaVoiceInputLauncherSelfCheck.swift macos/InputiaInputMethod/Sources/InputiaInputMethod/InputiaVoiceInputLauncher.swift -target arm64-apple-macos13.0 -framework AppKit -o /tmp/handy-unified-baseline.xRVDF1/voice-launcher-check
/tmp/handy-unified-baseline.xRVDF1/voice-launcher-check
# voiceInputLauncherSelfCheck=true
# candidatesAreOrdered=true
# findsEnvApp=true
# missingWhenExecutableAbsent=true
# runningPlanTogglesImmediately=true
# coldPlanStartsHiddenThenToggles=true
```

上述命令退出码均为 0。前四组和 Rime lib 的 doc-tests 都是 0 个测试，无失败。并发启动时短暂出现 Cargo package-cache lock 等待，随后正常完成。

CAPI 汇总包含历史上按资源可用性提前 return 的用例，不把该测试框架的 passed 数用于声称 A01–A12 全部覆盖。Swift 测试中的 fake app 及 mock fileExists 没有启动用户 Handy。独立 Rime 测试使用临时用户目录，没有写用户 Rime 学习库。
