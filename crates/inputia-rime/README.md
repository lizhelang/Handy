# Inputia Rime Adapter

这是 Inputia 的第一版中文候选引擎适配层。它动态加载 librime，并把候选输出转换为 `inputia-core::Candidate`，避免 Core 直接依赖 librime session、context 或 userdb。

默认开发配置使用本机已安装 Squirrel 的 librime：

- `/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib`
- `/Library/Input Methods/Squirrel.app/Contents/SharedSupport`

运行单元测试：

```bash
cargo test --manifest-path crates/inputia-rime/Cargo.toml
```

运行本机 probe：

```bash
cargo run --manifest-path crates/inputia-rime/Cargo.toml --example rime_probe -- luna_pinyin_simp ni
cargo run --manifest-path crates/inputia-rime/Cargo.toml --example core_flow_probe -- luna_pinyin_simp zhongguo 2
```

小鹤双拼 probe 需要先运行 spike 的数据准备脚本：

```bash
./spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  cargo run --manifest-path crates/inputia-rime/Cargo.toml --example rime_probe -- double_pinyin_flypy vsgo
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  cargo run --manifest-path crates/inputia-rime/Cargo.toml --example core_flow_probe -- double_pinyin_flypy vsgo 2
```
