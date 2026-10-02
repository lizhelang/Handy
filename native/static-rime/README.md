# Inputia 静态 Rime：本地可行实现与交付材料

本目录解决 hardened ad-hoc Host 无法加载外部 librime dylib 的技术限制，不关闭 library validation，不删除 Lua、grammar 或 predict。没有修改系统输入法、Squirrel、真实 Rime 用户目录或仓库根 MIT LICENSE。当前证明的是独立静态引擎和合成探针，**不是完整 Inputia Host 验收或法律合规结论**。

## 来源与构建

`sources.lock.json` 锁定所有下载 URL、SHA-256、librime 和依赖提交。两个官方 macOS release 归档的 SHA 与 GitHub release API 公布的 digest 一致：

- librime 1.16.0：`a251145d3aafa33871824a40bbec04c966bd8b56`。
- 官方主归档只有 dylib，未把它冒充静态库；实际由该提交源码编译 `rime-static`。
- 官方 deps 归档提供 universal 静态 glog、leveldb、marisa、opencc、yaml-cpp。本实现取出对应架构后与引擎合并，不依赖 Homebrew dylib。
- 插件采用 release 的 `version-info.txt`：Lua `68f9c364…`、octagram `dfcc1511…`、predict `920bd41e…`。
- Lua 源码另锁定 thirdparty 提交 `0752fb32…`，实际版本 5.4.8。原上游安装脚本读取浮动 thirdparty 分支，本实现改为下载固定归档。
- Boost 1.89.0 与官方 release workflow 一致，源码头文件在隔离目录内，不引用全局 Boost。

在仓库根执行：

```sh
/bin/bash native/static-rime/build.sh
```

需要机器已有 CMake、Ninja 和 Apple SDK/clang；脚本不安装任何系统依赖。默认架构为本机架构，也接受 `STATIC_RIME_ARCH=arm64` 或 `x86_64`。**目前实际编译和运行验证仅有 arm64**。

构建使用 `BUILD_SHARED_LIBS=OFF`、`BUILD_STATIC=ON`、`BUILD_MERGED_PLUGINS=ON`，合并 Lua/octagram/predict；不启用外部 Rime 插件发现。Lua 自身接口没有被裁剪，但本探针未加载任意外部 Lua C 模块。测试和生产 schema 未被相互替换。

下载、解压缓存、构建目录和合成用户目录都位于被 gitignore 排除的 `artifacts/`。缓存归档每次核对 SHA；要验证无缓存重建，应另建干净工作区运行脚本，不在并行集成过程中删除正在使用的产物。

## 产物契约

`artifacts/output/arm64/` 包含：

- `lib/libinputia_rime_static.a`：引擎、三插件及五项静态依赖的单一 archive。
- `include/rime_api.h` 等同版本公共 API 头文件。
- `link-flags.txt`：`-Wl,-force_load,<archive>`、系统 `-lc++`、最低 macOS 13.0。
- `static-rime-probe`：没有 library-validation 放宽 entitlement 的 hardened ad-hoc 探针。
- `manifest.json`：库/探针/来源锁/许可证 SHA、所有 Mach-O 对象最低系统版本、签名、依赖及制品根内的运行日志引用。schema 1 的文件引用均为受限相对路径，整套输出可复制到隔离工作树复验。
- `codesign.txt`、`dependencies.txt`、`licenses/`、`sources.lock.json`。

调用端可直接包含头文件并调用 `rime_get_api()`。静态模块依赖 constructor 注册，**必须 force-load archive**；仅按普通 archive 链接可能丢失未直接引用的模块注册器。当前 archive 已包含其非系统依赖，不要再链接外部 librime、Lua 或 OpenCC dylib。

只复核既有产物并运行新的合成 session：

```sh
/bin/bash native/static-rime/build.sh --verify-only
```

## 实际证据

2026-09-05 arm64 首次完整脚本构建、签名和探针均成功：

- 静态 archive 的 263 个 Mach-O 对象最低版本只出现 11.0/13.0；探针最低版本为 13.0。
- 探针 CodeDirectory 标记 `0x10002(adhoc,runtime)`，严格签名验证通过。
- 链接依赖只有 `/usr/lib/libc++.1.dylib` 和 `/usr/lib/libSystem.B.dylib`。
- 运行时检查 `rime_get_api` 地址来自探针本体，并检查所有已加载 image；没有 Squirrel、Homebrew 或其他外部 librime image。
- Lua、octagram、grammar、predict 四个模块注册成功。
- 全拼 `nihao` 与完整小鹤拼写运算的 `nihc` 均在真实 Rime 引擎中产生并提交 `你好`；两条路径都执行了真实 Lua filter。
- 只使用四个词条的合成词典和新建临时用户目录，不向系统应用发送按键。此小词典是探针 fixture，不是生产词典替代品。

首次小鹤试验的键序误写为 `nich`，实际未出现目标候选；依照 schema 的 `hao -> hc` 修正为 `nihc` 后通过。早期 fixture 缺 ascii bindings 的日志也已修正，最终验证器遇到该日志会失败。上游 OpenCC 的 `std::iterator` 弃用警告、部分空对象的 libtool 警告仍保留，没有隐藏它们。

## 组件许可证事实与随附材料

实际收集的原始文本和各自 SHA 在输出 `licenses/` 与 `manifest.json`：

- librime、librime-lua、predict、glog、leveldb：BSD 类许可原文。
- octagram/grammar：GPLv3 许可原文。
- 双拼测试拼写运算所来自的 rime-double-pinyin：GPLv3 原文及固定提交的 schema 文件。
- Lua 5.4.8：`lua.h` 中的版权与许可原文。
- Boost：Boost Software License 1.0。
- OpenCC：Apache 2.0。
- yaml-cpp：MIT 原文。
- marisa：上游 `BSD-2-Clause OR LGPL-2.1-or-later` 双许可原文。
- darts-clone 与 X11 构建头文件：自带版权/许可文本。

本机候选交付应同时保留：上述许可证文本、`sources.lock.json`、下载归档（尤其完整 librime/插件/Lua 源码）、本目录脚本与探针源码、最终 Host 工程提交及其构建步骤、构建/签名/运行证据。官方静态 deps 使用其官方归档，精确 submodule 提交另载于锁文件；本次未重新从源码构建全部 deps。

**没有修改整个项目的许可声明，没有对静态组合的分发合规作法律结论，也未对外发布。** 最终分发材料和义务仍须独立核查，不能只附根 MIT 文件便声称已解决。

## 尚未验证

- 完整 Host、InputMethodKit、候选 UI、真实应用按键和断线退化行为。
- octagram 注册已验证，但没有加载真实 grammar 模型验证评分；predict 注册不等于预测质量已验证。
- 生产全量 Rime schema、词库、用户学习、遗忘、同步与迁移回归。
- x86_64、universal 最终 Host 和 macOS 13 实机兼容性。
- 全部第三方源码缓存污染防护、字节级可重现构建、分发许可合规。

## Rust/CAPI 静态适配层追加证据

`inputia-rime` 和 `inputia-capi` 已增加显式 `bundled-static-rime` feature。构建期需要 `MACOSX_DEPLOYMENT_TARGET=13.0` 与规范绝对路径 `INPUTIA_STATIC_RIME_DIR`。校验脚本只读检查产物所有权、权限、无链接、SHA、架构和来源锁，不自动下载或 fallback。缺少目录的负向编译试验退出 101。

静态模式从当前进程调用 `rime_get_api`，保留配置里的 dylib 路径但完全不加载。Static/Dynamic 生命周期分开；同一静态运行时只比较实际数据域，不因未使用的 dylib 字符串不同而拒绝第二个 session。默认动态模式仍保留原动态加载及错误行为。

本机候选 RimeData + 临时用户目录的实测结果：

- inputia-rime：3 单元 + 1 静态生命周期 + 4 core flow + 7 schema smoke，共 15 项通过，包含全拼/各双拼、稀有字与扩展词库、纠错、分段候选消费、增量/冷查询一致性；0 skip。
- inputia-capi：22 项全部通过，包含学习/敏感来源排除、分页、双拼和关闭上一 session 后继续使用下一 session；0 skip。静态测试遇到原 skip 条件会失败。
- 默认动态模式：3 单元 + 1 缺失外部库错误回归通过；未将这 4 项称为动态全量识别回归。
- inputia-rime 静态 all-targets 严格 Clippy 通过。
- 初次接线时 inputia-capi 严格 Clippy 暴露生产 C FFI 的 29 处裸指针安全签名问题；原始诊断保留在 `capi-clippy.log`。后续专门分工已完成修复，结果见下方，不使用 allow 掩盖。

运行带严格 runtime 签名的实际 Swift → Rust CAPI → 静态 Rime 探针：

```sh
INPUTIA_RIME_SHARED_DATA_DIR=/候选安装包/Contents/Resources/RimeData \
/bin/bash native/static-rime/run-capi-probe.sh
```

成功探针验证多 session、free/reopen、全拼和小鹤 `中国` 提交、合成学习术语重开保留。刻意配置不存在的 dylib 路径仍成功，且运行时仅加载本体和系统 image。`CAPIStaticProbe` 的 CodeDirectory 为 `0x10002(adhoc,runtime)`，未提供放宽 entitlement，严格签名通过，minOS=13.0。

证据位于 `artifacts/output/arm64/capi-probe-codesign.txt`、`capi-probe-dependencies.txt`，运行日志在 `artifacts/capi-probe-run.*/probe.log`。这仍不是 InputMethodKit 全 Host 或跨应用验收。

## FFI 安全签名门禁收口

21 个生产导出已声明 `pub unsafe extern "C" fn`，每个函数有 `# Safety`，模块统一说明输入字符串、session 独占/生命周期/串行所有者线程，以及返回字符串的释放合同。符号、ABI 参数、null 行为、学习和排序逻辑不变。既有 Rust 调用点显式标注 unsafe，且启用 `deny(unsafe_op_in_unsafe_fn)`；没有增加 allow 或猜测非空指针有效性的运行期检查。

- 默认动态：24 项 CAPI 测试通过，0 skip/0 ignored；2 项 compile-fail 文档测试通过。
- bundled-static-rime：24 项 CAPI 测试通过，0 skip/0 ignored；2 项 compile-fail 文档测试通过。
- 两种模式的 all-targets 严格 Clippy 均通过；原 29 处门禁已清除。
- 重新构建、hardened ad-hoc 签名的 Swift CAPI 探针通过，确认 Rust unsafe 声明没有改变 C 调用。

最终记录为输出目录中的 `capi-ffi-dynamic-tests.log`、`capi-ffi-static-tests.log`、`capi-ffi-dynamic-clippy.log`、`capi-ffi-static-clippy.log`、`capi-ffi-hardened-probe.log`。调用方仍必须兑现 Safety 合同；声明 unsafe 并不是运行期内存保护、全局线程协调或完整 Host 验收。
