# 显式热词同时影响打字与语音

用户编辑的现有 `settings.custom_words` 继续保留原数据格式，不迁移、不合并自动学习词。

## 接线与优先级

- 配对 SharedTerms 请求原来向 `session_hotwords` 传空显式词表，现改为读取当前 `custom_words`。原有目标证明、权限 epoch、策略版本、TTL 与敏感输入检查不变。
- 共享中文排序按服务给出的词顺序提升精确匹配原生候选，因此显式词在学习词之前。同一输入意图和消费长度组内移动原候选索引，不注入不存在的候选。
- 个性化查询完成自动学习与可选本地模型排序后，再应用显式热词优先；完整消费长度和匹配类型组保持不变，候选原生 ID 不变。关闭自动学习时，共享路径仍可使用显式热词。
- 撤下热词后下一次查询不再施加显式优先，不删除或伪造 Rime/个人学习证据；已取得的共享快照仍只在原有短 TTL 内有效。
- 语音现有 Whisper initial prompt、Qwen context 已经优先收录显式热词，本次保留并新增回归。显式词与学习词仍由统一预算分配；学习词只作为原生识别提示，不升级为强制纠错词表。

## 实际验证

真实静态 Rime 使用独立自然码兼容资源、临时用户目录：输入 `zhongguo`，候选“中古”在原候选池第 11 位（索引 10、第三页），个人和共享排序都将它升到首位。其 ID 始终为 `rime:double_pinyin:10:2:0`，用原地址选择后完整提交“中古”，未剩余拼音。未添加该热词时保持原顺序。

- `cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test personalization explicit_hotwords`：显式词压过真实自动学习证据、撤下恢复学习排序、不匹配词无变化、消费组隔离，通过。
- `cargo test --manifest-path crates/inputia-core/Cargo.toml shared_`：3 项通过，覆盖原排序预算、显式单字、消费组、敏感词过滤。
- `cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test voice_protocol shared_terms_`：2 项通过，覆盖显式词顺序与原 lease/peer 合同。
- `cargo test --manifest-path src-tauri/Cargo.toml explicit_hotwords_ --lib --no-default-features`：Whisper/Qwen 提示显式词优先，通过。

真实引擎复现命令：

```sh
MACOSX_DEPLOYMENT_TARGET=13.0 \
INPUTIA_STATIC_RIME_DIR="$PWD/native/static-rime/artifacts/output/arm64" \
INPUTIA_RIME_SHARED_DATA_DIR="$PWD/macos/InputiaInputMethod/candidate-rime-data/artifacts/outputs/natural-full-20260927/RimeData" \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --features bundled-static-rime \
  --test schema_smoke explicit_hotword -- --nocapture
```

## 验收范围

本轮是“匹配到热词时优先展示；语音识别优先参考”。可以把原生后页匹配词提升到前排，但不承诺任意新专名一定出现：当前个人底库没有字音映射，若 Rime 当前候选池中完全不存在某专名，本轮不会注册新拼音词条、伪造 Rime ID 或强行提交该词。没有匹配时保留原输入与候选。当前个人候选池有原有数量上限，池外词也不会被伪称为已匹配。

本轮测试不写用户数据、不安装应用；最终本机配对构建、安装与输入现场验收由发布任务另行完成。
