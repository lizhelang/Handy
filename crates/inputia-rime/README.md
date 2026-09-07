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

## 显式静态引擎构建

默认 feature 仍采用已有动态 librime 配置。`bundled-static-rime` 是独立构建选择：

```sh
MACOSX_DEPLOYMENT_TARGET=13.0 \
INPUTIA_STATIC_RIME_DIR=/绝对规范路径/native/static-rime/artifacts/output/arm64 \
cargo +1.96.0 build --manifest-path crates/inputia-rime/Cargo.toml --features bundled-static-rime
```

静态产物先由仓库 `native/static-rime/build.sh` 生成并验证。`build.rs` 不下载、不构建第三方资源，也不自动回退；缺少显式目录、归属/权限/链接异常、manifest/archive/SHA/架构不匹配或部署版本不明确时编译失败。

开启该 feature 后直接引用进程内 `rime_get_api`，`dylib_path` 字段保留兼容但不加载，也不用于区分同一静态运行时。库生命周期以 Static/Dynamic 分开表示；会话、函数表、候选/学习行为保持原接口。

`inputia-capi` 的同名 feature 转发到本 crate。最终 Swift 链接其 `.a` 时仍需 force-load 整个 CAPI archive，以保留原生模块 constructor，并链接系统 `c++`。可复现的成功范例见 `native/static-rime/run-capi-probe.sh`。

静态测试必须设置 `INPUTIA_RIME_SHARED_DATA_DIR` 为明确候选 RimeData；测试用户目录均为临时目录，不允许“找不到 runtime 就 skip”。本机已通过 inputia-rime 15 项与 CAPI 22 项，并通过 hardened ad-hoc Swift→Rust CAPI→静态 Rime 探针。完整 Host、macOS 13 实机与分发许可结论仍未验证；组件许可原文和事实见 `native/static-rime/README.md`。
