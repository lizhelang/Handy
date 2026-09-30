# 候选纠错与诗词质量回放（2026-09-30）

## 环境与隔离

- 基线使用工作区 HEAD 的 `inputia-rime/src/lib.rs`，复制到独立临时 crate；没有回退工作区代码。
- 基线与变更均使用已验证的 bundled static Rime、相同公共共享词典。共享词典从 Inputia 1.0.9/build83 已装包只读复制，再覆盖新版诗词表。
- 每个进程/测试均使用 `tempfile::tempdir()` 用户目录，未读取、复用或修改个人 userdb。
- 此处验证实际 Rime 引擎候选与选择接口；没有安装应用，也不代表 macOS 前台输入法验收。

## 同条件前后对比

| 输入        | 原始 Rime 第一候选 | 旧适配层第一候选 | 新适配层第一候选 |
| ----------- | ------------------ | ---------------- | ---------------- |
| `tainan`    | 太难               | 天安             | 太难             |
| `woaini`    | 我爱你             | 哇哦             | 我爱你           |
| `hainandao` | 海南岛             | 和               | 海南岛           |
| `zhongguo`  | 中国               | 中国             | 中国             |
| `zg`        | 这个               | 这个             | 这个             |
| `nh`        | 你会               | 你会             | 你会             |
| `woain`     | 我爱你             | 哇哦             | 我爱你           |
| `zhonguo`   | 中过               | 中国             | 中国             |
| `dagn`      | 大概你             | 当               | 当               |
| `hoa`       | 后啊               | 好               | 好               |
| `tain`      | 太难               | 天               | 天               |

完整合法分节优先保留原拼写，可信纠错与原候选共同排序。`tainan` 的“台南”仍是第二候选，“天安”仍作为低置信纠错可选。不能形成完整音节的改写（如 `woain → woian`）不引入额外噪声。

纠错保留真实 Rime 地址。已验证完整候选上屏和前缀选择：`tainan` 选择纠错“天”消费原输入前 4 字节，保留 `an`，没有丢掉剩余输入。

## 词库调整

- 诗词表正文由 155,758 条减至 11,266 条。保留来源完整句、分句、独立来源短词及明确手工词，移除穷举 2～8 字连续子串的规则。
- `长风破浪会有时`、`长风破浪` 保留；无独立来源的 `轻舟已`、`风来满` 已移除。
- 仅重新生成 `inputia_poetry.dict.yaml`，其他资源词库不变。`prepare-rime-data.sh` 原有复制链将该词库写入打包资源，不增加联网构建步骤。
- 新诗词表 SHA-256：`7ff1ad489213a32638e67c15ed4433f7daa6ad6505d90eeba54239c3fff69dc0`。

再生成命令（来源版本已固定在生成器，`--cache-dir` 可指定离线源快照）：

```sh
python3 macos/InputiaInputMethod/Tools/generate_inputia_lexicons.py --only-poetry
python3 -m unittest discover -s macos/InputiaInputMethod/Tools -p test_generate_inputia_lexicons.py
```

## 验证结果

- 生成器及实际资源词库：3 项测试通过。
- 静态 Rime 单元测试：4 项通过。
- 完整静态 `schema_smoke`：11 项通过，包含既有自然码、深页热词、增量会话、多个双拼方案、扩展词库，以及新增纠错回归。
- 额外补充纠错前缀消费断言后，相关静态集成测试再次单独通过。
- `cargo fmt --check`、改动文件的 `git diff --check` 通过。
- 初次误用旧 `build/RimeData`，导致既有自然码全拼测试失败和测试锁中毒；换用 build83 公共资源后完整通过，旧环境失败不计入验收。

日志目录：`/var/folders/72/4lw3bh891pzbr5wyqh189xy40000gn/T/inputia-rime-quality-mg37biog`。

- `replay-baseline-current.log`、`replay-after-current.log`：同条件前后回放。
- `rime-tests-current.log`：4 项单元测试和 11 项真实静态 schema 测试。
- `partial-correction-test.log`：纠错前缀地址及消费长度回归。
- `RimeData-current`：可供主线程复用的隔离共享词典，已覆盖新版诗词表。

复跑真实引擎（工具自行创建临时 userdb）：

```sh
INPUTIA_STATIC_RIME_DIR="$PWD/native/static-rime/artifacts/output/arm64" \
  MACOSX_DEPLOYMENT_TARGET=13.0 \
  cargo run --manifest-path crates/inputia-rime/Cargo.toml \
  --features bundled-static-rime --example candidate_quality_replay -- /absolute/path/to/RimeData
```
