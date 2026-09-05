# P0 基线与构建条件核验

记录时间：2026-09-05 08:15 +0800（北京时间）。
记录范围：基线代码、现有资源与隔离非 GUI 测试；不构成 P0 全部通过，也不构成 P1–P6 或 A01–A12 最终验收。

## 1. 实际基线与工作区保护

- 实施工作区：`/Users/lzl/FILE/github/Handy-unified-input-system`，分支 `codex/unified-input-system`，起点 `b7d7db70`。
- 已核对的上游重接底座：`/Users/lzl/FILE/github/Handy-upstream-reintegration`，分支 `codex/upstream-first-reintegration`，同为 `b7d7db70`，测试前后工作树干净。
- 原工作区：`/Users/lzl/FILE/github/Handy`，分支 `codex/funasr-sherpa-native-hotwords`，HEAD `faea17d3`，37 个 tracked 修改和额外未跟踪文件/目录。未改动、覆盖或提交这些文件。
- `codex/pre-upstream-reintegration-snapshot` 指向 `3407a51740e8d9cd3e3017b96008b9bb5c5c7de5`；该快照不是“所有能力已验证”的证明。其提交正文明确没有执行快照构建/运行测试。
- 本轮只写本报告与独立测试摘要。测试通过来自固定、干净的上游底座；实施工作区同期已有其他代理修改 core/runtime，最终必须对最终提交重跑相关测试。

## 2. 能力保留与明确缺口

- **Inputia：基线代码保留。** `git diff --quiet 3407a517 -- macos crates` 返回 0；底座的 Inputia 与旧工作树恢复快照相同。本轮 Core/Settings/Runtime/CAPI 测试通过，不能替代 IMK 原生跨应用验收。
- **原生 Qwen 热词：保留。** 底座与原工作树的 `src-tauri/src/native_hotwords.rs` 字节相同；`managers/transcription.rs` 消费 `QwenContextPlan`，当前固定 `transcribe.cpp` revision `18718a497750ec51daa102ee0030f4e4000ea627`。这只能确认接线存在；没有实际 ASR 质量通过证据。
- **本地术语纠错：保留。** 底座与原工作树 `custom_words_model.rs` 字节相同，保留代码术语同音纠错及严格替换验证。上游重接提交 `a9877e53` 保留了上游转写管线、模型能力/语言证据、下载镜像和哈希校验；不可用旧版 `transcription.rs` 整体覆盖。
- **文件剪贴板与面板原生交互：保留代码。** `0175ca19` 为原生体验接入，`5a84e9b7` 修复桌面切换误判。`managers/clipboard.rs` 保留文件写入原生表示路径；其失败回退仍可能写为文本，A06 最终验收需要专门核查。`paste_tx/macos.rs` 有 `changeCount` 守护的恢复路径；不能据此声称所有输出路径都满足 A06。
- **快捷键修复：保留代码。** HEAD `b7d7db70` 为“防止重叠转写快捷键重启录音”，前置 `38d7707a` 固定边界。本轮未重跑 Handy Rust 大套件，长按、左右修饰键及 A04 的真实事件测试仍未验证。
- **FunASR：依赖中有引擎，Handy 产品集成被排除，待恢复和运行验收。** 详见下一节。
- **Sherpa：当前可达 refs 未找到实现或依赖，不能声称保留。** 详见下一节。该项仍在用户批准的完整范围内，不能因历史描述不准确而从验收中移除。

## 3. FunASR / Sherpa 历史查证

### FunASR 的准确恢复线索

1. `faea17d3:src-tauri/src/catalog/catalog.json` 第 509 行附近和第 789 行附近有 `Fun-ASR-MLT-Nano-2512`、`Fun-ASR-Nano-2512`，`model_capabilities.rs` 有 `funasr_nano`。该旧 catalog 尚不含新版完整 revision/hash 字段。
2. `3407a517` 恢复快照实际上删除了这两个 catalog 项及 `KNOWN_ARCHES` 中的 `funasr_nano`。该快照的 `scripts/gen_catalog.py` 注释是“intentionally no longer shipped or offered”，并新增 `EXCLUDED_FAMILIES = {"fun"}`。所以快照提交正文“保留 FunASR/Sherpa”与实际树并不一致。
3. `9bd088dee251211ae5ee09d15880913b404e9c9a`（`codex/reintegration-agent-import-snapshot`）提供最合适的**选择性数据恢复参考**：`src-tauri/src/catalog/catalog.json` 两个 FunASR 项具有模型 revision、分量化文件大小和 SHA256，`model_capabilities.rs` 有 `funasr_nano`。MLT revision 为 `0b8f9c7bc545a219658aeb1dd4eeaa55d1cf89f3`；Nano revision 为 `30360f003f99929c1e25c3f6222d02eccb04663c`。该提交自述未通过整仓编译，明确禁止整体覆盖上游主干。
4. `a9877e53f35e168bbd7a4252ed34d7e45db15139` 重接 Qwen/纠错时继续排除 FunASR，并将生成器注释改为需要通过相同 runtime/data-safety gates；`catalog/mod.rs` 测试明确要求 catalog 不含 `funasr_nano`。这表明排除由集成验收不足造成，不是证明底层引擎不存在。
5. 当前固定的依赖缓存 `/Users/lzl/.cargo/git/checkouts/transcribe.cpp-6a28d06a2913d687/18718a4/src/arch/funasr_nano/` 有 encoder、adaptor、decoder、weights、model、capabilities 实现。其 `capabilities.cpp` 声明 16 kHz、取消与 ITN，不支持翻译、时间戳；没有在本次核验中证明其支持运行时热词提示。
6. 同一依赖的 `docs/porting/families/funasr_nano.md` 自述两个变体已完成数值对照与 LibriSpeech WER 验证。这是**依赖仓库既有报告**，不是用户这台 Mac/Handy 的本轮测试结果。当前不能标记 A11 通过。

因此，恢复应在当前 transcribe.cpp ABI 上接入 catalog/能力探测，并执行模型下载、实际加载、取消、错误恢复及识别测试；不能只删除隐藏开关就宣告完成。无需为已有 FunASR 引擎先引入另一个依赖，也不应回退旧 transcribe.cpp。

### Sherpa 的证据边界

已对所有本地可达 refs 执行 `git log --all -i -S sherpa -- src-tauri`，没有找到文本变更记录；对 `3407a517`、`9bd088de` 的 src-tauri/crates 以及现工作树搜索，也没有 Sherpa backend 或 Cargo 依赖。能找到的只是计划与提交说明中的“FunASR/Sherpa”。

没有证据支持“现成成熟 Sherpa 实现可直接从某提交恢复”。后续应将其登记为需要查清并实现/接通的能力缺口，由实施主代理决定独立恢复阶段；本次不添加依赖，不用其他 ONNX 后端改称 Sherpa。

## 4. 本机工具链、资源及安装状态

- Apple M2 Max，内存 34359738368 bytes（32 GiB）；macOS 27.0，build `26A5425a`。
- Bun `1.3.13`；默认 Cargo/Rust `1.88.0`。另外已安装 Rust `1.89.0`、`1.96.0`、`1.97.1`。本轮固定 `cargo +1.96.0`；不要假定默认 `cargo` 等于 Host 要求的工具链。
- Apple Swift `6.3.3`；Xcode 路径 `/Applications/Xcode.app/Contents/Developer`。
- 实施工作区在本次初检时没有 `node_modules`，前端门禁需要本工作区安装锁定依赖后再跑。
- 仓库 `src-tauri/resources/models/silero_vad_v4.onnx`：1807522 bytes；`gigaam_vocab.txt`：2007 bytes。
- 在已检查的 `/Users/lzl/Library/Application Support/com.pais.handy/models` 中仅发现纠错模型 `custom-words-qwen3-0.6b/Qwen3-0.6B-Q4_K_M.gguf`：484219808 bytes。没有找到 Qwen3-ASR 1.7B 或 FunASR 音频模型；这只是限定目录的检查结果，不是全磁盘不存在的断言。未读取历史正文或凭据。
- `/Applications/Handy.app` 已存在，Info.plist 标示版本 `0.9.6`；系统 `/Library/Input Methods/InputiaInputMethod.app` build `50`。这些版本号不能证明对应本次源码提交。本轮未启动、替换或重签日常安装。
- Squirrel 的 `/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib` 已存在，7177888 bytes，SharedSupport 目录存在；独立 Rime 测试能使用它。当前 Host 默认依赖这个系统外部路径，候选交付需明确处理依赖随包与新装可用性，不能因开发机有 Squirrel 就宣称新装完整。
- 当前 `inputia-rime` 未见已接出的词级 forget/delete API；P0 应核对原生能力并按设计呈现 Rime 撤销状态，不能把共享词库已忘记等同 Rime 已清除。

## 5. 本轮实际测试

所有以下命令的工作目录均为固定基线 `/Users/lzl/FILE/github/Handy-upstream-reintegration`，构建输出使用 `/tmp/handy-unified-baseline.xRVDF1`，保持源与 Cargo.lock 不变，均加 `--locked --offline`。

- Inputia Core + `sqlite-memory`：37 passed，0 failed。
- Inputia Handy Runtime：3 passed，0 failed。
- Inputia Settings：4 passed，0 failed。
- Inputia CAPI：测试框架报告 22 passed，0 failed。多个用例有资源缺失时提前返回的历史模式，因此不能将汇总数等同完整原生覆盖；输出实际加载 librime，九方案测试也运行。没有真实输入框上屏。
- Inputia Rime `--lib`：3 passed，0 failed。
- Inputia Rime 定向全拼 `core_flow`：1 passed，0 failed；使用临时 user 数据，实际加载现有 Squirrel librime，确认输入、候选、翻页与提交状态。不是 IMK Host 验证。
- Swift `InputiaVoiceInputLauncherSelfCheck`：编译并运行通过。该旧测试断言固定等待启动方案，只证明旧逻辑，不证明新的异步会话协议。

完整命令及输出摘要见 `baseline-test-evidence.md`。本轮未跑完整 Handy Rust、前端、完整 Host 打包或 GUI 套件；未做模型音频对照、迁移/回滚、100 次通讯循环或跨应用输入。

## 6. 可复用的安全构建方式与后续门禁

建议为最终候选创建新的临时根目录，并按 `handy-target`、`inputia-target`、`host-bundle`、`profiles`、`test-data` 分离；通过显式参数向被测程序传递 profile，不能仅换构建目录就认为数据也隔离。

现有 `macos/InputiaInputMethod/build.sh` 的 CAPI 输入固定为 `crates/inputia-capi/target/release/libinputia_capi.a`。若使用 `CARGO_TARGET_DIR`，必须同时修正/覆盖该链接路径，否则可能读取旧静态库或构建失败。脚本还会删除本工作区旧 Host bundle，拷贝/下载 Rime 资源并签名；本轮没有调用它。

`verify-nongui.sh` 默认直接转到 `dev-fast.sh`，后者会先执行完整 `build.sh`。`INPUTIA_VERIFY_NONGUI_FULL=1` 的路径还启用 GUI readiness 等探测，不应因文件名含 nongui 就无审查运行。需要系统安装的原生验证必须使用准备好的候选包和恢复步骤，按用户授权边界处理。

后续最终门禁至少包括：`bun run lint`、`bun run format:check`、`bun run check:translations`、`bun run check:model-languages`、`bun run build`、Playwright；Handy Rust test/clippy；五个 Inputia crate 测试与编译后的 Host 自检。当前 `.github/workflows/test.yml` 直接运行真实 `cargo test`，不再复制 transcription mock，不能沿用旧 AGENTS 中“CI 使用 mock”的描述。

仍需完成的 P0：认证/ready 原型、有效与失效目标 token、真实三应用测试、剪贴板多格式与来源覆盖率、Rime 撤销、模型动态提示验证和协议冻结。以上缺口不因本报告基线测试通过而关闭。
