# 候选 RimeData 固定来源

此流程替代候选构建对日常 Squirrel/Inputia 资源的依赖；日常 `prepare-rime-data.sh` 流程保持原样。脚本只展开官方安装包的 SharedSupport，不安装 Squirrel、不执行包中的 postinstall，也不读取日常安装或用户词典。

## 来源与保留内容

- 基础完整 55 个资源文件来自 [Squirrel 1.1.2 官方发布](https://github.com/rime/squirrel/releases/tag/1.1.2)。[官方 release API](https://api.github.com/repos/rime/squirrel/releases/tags/1.1.2) 公布 pkg digest 为 `614746013212937623d5bbab9901e9c43d1ec937aa32307d6b6092a05e308287`，下载后已实际核对。
- 标准、智能 ABC、小鹤、微软、拼音加加、自然码双拼与作者/许可文件来自 rime-double-pinyin 固定提交 `01a13287cbd27819be1c34fa1ddc1b3643d5001b`，归档 SHA 固定在 sources.lock.json。
- 国标双拼及 README 来自方案作者 baopaau/rime-guobiao-quick 固定提交 `1283ba5b6980b54348904be2b582f7cd4d101fd2`，归档 SHA 固定。构建不查询 main/master。
- 仓库 `Resources/RimeData` 中全部普通资源原样复制；五份既有 Inputia 扩展词典缺一即失败，后续新增资源也不会被固定五文件列表漏掉。
- 搜狗双拼直接提取仓库旧脚本的静态 YAML 模板，不执行该脚本。默认方案顺序、朙月词典扩展替换以及旧方案原已禁用的可选 emoji 过滤器处理保持一致；来源 manifest 同时记录原脚本与本脚本 hash。
- 增加官方 Squirrel 发布包 LICENSE 文本与完整文件 hash 清单，不将“随附文本”声称为分发许可审查结论。

## 使用和拒绝规则

候选 `build.sh` 已调用：

```sh
/usr/bin/python3 macos/InputiaInputMethod/candidate-rime-data/prepare.py \
  --run-id trial-20260905 \
  --output /完整工作区/macos/InputiaInputMethod/candidate-builds/trial-20260905/RimeData
```

run ID 只允许 1–64 位 ASCII 字母、数字、短横线和下划线。输出只允许当前工作区候选目录或本脚本 artifacts/outputs 下的对应 run/RimeData；拒绝路径别名、链接、硬链接源文件以及日常安装路径。

下载缓存每次完整核对 SHA；错误缓存直接失败，不回退旧安装。`--offline` 要求全部锁定归档已存在，缺失即失败。每次构建重新展开已核验 pkg 到唯一临时目录；schema 归档只复制明确列出的普通成员，拒绝重复成员或链接，不依赖提取 stamp。

先在临时目录完成资源和 manifest，再切换输出。已有候选资源整体保留为同目录 `RimeData.previous.<UUID>`，切换失败恢复原目录；不删除旧候选中可能存在的运行数据。`--verify-only` 只校验现有文件与源/转换/仓库字典 hash，绝不重新生成 manifest。

产物 manifest 不含时间、机器路径或 run ID，因此相同输入在独立输出目录生成相同资源字节与清单。它用于可信构建过程中的完整性核验，不是抵御控制用户账号后同时修改脚本/清单的安全沙箱。最终候选包仍由外层签名和配对认证固定。

## 本轮实际验证

- 10 项合成单元测试通过，覆盖完整资源/字典保留、默认方案与搜狗转换、缺词典拒绝、模板歧义拒绝、源链接/重复归档成员拒绝、损坏缓存不走网络或 fallback、离线缺失拒绝、输出越界及已有产物篡改拒绝。
- 使用锁定的三个真实归档，离线生成 `artifacts/outputs/resource-audit-20260905/RimeData`：72 个资源文件，加 1 个来源 manifest。
- 另一个全新输出 `artifacts/outputs/resource-repeat-20260905/RimeData` 离线重建，两个完整 manifest 字节相同；每个资源的 SHA/长度因此相同。再次 verify-only 通过。
- 与原 `candidate-builds/trial-20260905/RimeData` 比较：**全部共有业务资源逐字节相同，没有删词库或 schema**。新流程增加 LICENSE 与 manifest；旧目录独有的是运行生成的 `build/`、`installation.yaml`、`user.yaml`。这些旧文件未被修改/删除，也未混入新的可重现资源包。
- 五份 Inputia 词典行数与旧候选完全一致：classical 682、ext_chars 14、idiom 6664、luna_pinyin 18、poetry 155768，总计 163146 行。这是文件行数，不冒称有效词条数。
- 实际运行静态 `CAPIStaticProbe` 读取新资源，合成用户目录 `/tmp/inputia-data-check.xQZ4ly/user`：多 session、free/reopen、全拼/小鹤提交“中国”、合成学习重开保留均通过，没有外部 librime image。该探针本次 SHA 为 `c10d1bf053f09055d165eec48798987da1388c6c310cd5d19a7ff86dcdb54747`。
- build.sh 语法检查与限定 diff 空白检查通过。

以上是资源供应和特定引擎路径验证，不替代最终 Host 签名、每个方案的完整原生回归、真实跨应用输入、迁移回滚或整体验收。需要非作者审查本脚本及外层候选打包后再收口交付。

测试命令：

```sh
/usr/bin/python3 -m unittest discover -s macos/InputiaInputMethod/candidate-rime-data -p 'test_prepare.py'
```

## 自然码全拼兼容（2026-09-27）

仅 `double_pinyin` 增加全拼音节派生，保留原自然码编码；不修改用户选择的方案。先执行原有无效 `xx` 音节擦除，再用首尾标记保护全拼副本，完成双拼转换后恢复副本。原有词典、原生候选地址、选择与用户词典学习继续使用 Rime。

自然码的 preedit 改为原始输入分节，避免旧双拼展开规则误改写全拼字母。候选扫描保留第一页引擎明确音节边界，防止翻到单字候选页时合并的 preedit 使全拼消费长度错误；不按汉字个数推测全拼键数。

独立离线资源 `artifacts/outputs/natural-full-20260927/RimeData`、临时 Rime 用户目录及真实静态引擎已验证：`edu → 额度`、`vsgo → 中国`、`zhongguo → 中国`、`nihaoma → 你好吗`；`zhongguo` 选择“中”消费5键、剩余 `guo`，`vsgo` 消费2键、剩余 `go`；混合 `zhonggo` / `vsguo` 分别消费5/2键，剩余部分继续原生选择“国”。选择非首位“种果”三次，关闭并重开后该候选升到首位，证明 Rime 原生学习仍生效。资源供应11项单测通过；完整静态 schema_smoke 8项通过（132.84秒），包含其他双拼方案、原双拼部分提交、纠错、词典扩展及增量会话回归。将消费长度修正限定于自然码后，综合回归再次通过（5.44秒）。

实际引擎回归命令（资源路径改为当前工作区绝对路径）：

```sh
MACOSX_DEPLOYMENT_TARGET=13.0 \
INPUTIA_STATIC_RIME_DIR="$PWD/native/static-rime/artifacts/output/arm64" \
INPUTIA_RIME_SHARED_DATA_DIR="$PWD/macos/InputiaInputMethod/candidate-rime-data/artifacts/outputs/natural-full-20260927/RimeData" \
cargo test --manifest-path crates/inputia-rime/Cargo.toml --features bundled-static-rime \
  --test schema_smoke natural_code_accepts -- --nocapture
```

这些回归使用独立资源和合成用户目录，不安装输入法、不改已有设置，也不替代最终系统输入验收。
