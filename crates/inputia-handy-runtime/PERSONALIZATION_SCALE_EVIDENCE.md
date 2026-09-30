# 个性化候选大数据回放记录

测量日期：2026-09-30。工作区基线 HEAD：`70687eac7861c7f1c48edefe295548f875fa2d5a`。测量对象是此次尚未提交的个性化改动，因此另记录 `src/personalization.rs` 内容 SHA256，避免把 HEAD 当成实际被测源码。

## 结论与边界

在 20,000 条完整合成学习记录、2,742,496 条语境锚点索引的压力库上，64 个候选的有语境查询，release p95 从 **726.61ms 降至 27.34ms**。稀疏有效锚点和仅剩已撤销锚点的场景，最终 p95 均小于 10ms。最终全部场景通过显式 100ms p95 预算。

这些数值测量本机优化编译后的 `personalization::query`，包括读取和排序；不包括 IMK、IPC、候选窗绘制和真实键盘输入。使用全新临时数据库，没有读取、修改或复制个人历史。固定语境回放的 24/24 结果也不代表开放输入的准确率。

压力库最终为 **315.85MiB**，这是必须保留的存储代价：每条记录的近句尽量填满 48 字窗口，生成 131–138 个锚点，属于长中文语境的高负载样本，不等于实际用户数据库体积。本轮没有削减学习记录或锚点来换取速度，也没有迁移 `evidence_anchors` 为 `WITHOUT ROWID`；该迁移的节省尚未测量。

## 可复现样本

入口为 `tests/personalization_scale.rs` 中的 ignored test。测试先使用生产 `feedback` 建立 128 种近期分句模板和真实 schema、锚点、绑定，再用事务 SQL 扩充至 20,000 条；额外调用真实 `feedback`，确认复制的锚点集合与生产生成结果一致。

- 原始 previous 共 20,000 种；近句模板 128 种，前句的合成编号位于实际分句窗口之外。
- 每条 evidence 都有完整 `evidence_anchors` 和 `feedback_bindings`，没有遗漏索引数据来缩短测量时间。
- 包含 `luna_pinyin_simp` / `double_pinyin_flypy`、两个合成 source_app、256 种候选词条及 20,000 条合成公共词组。
- 普通场景分别使用 32 / 64 个候选，有 / 无上下文，每项预热 2 次、测量 20 次。
- 稀疏场景使用 48 个互异汉字形成的 138 个锚点，仅关联 1 条有效记录或 1 条已撤销记录；每项预热 1 次、测量 5 次。候选词均未学习，并断言个性化仍启用，防止关闭学习的快速返回误判通过。
- `evidence_limited` 如实输出；公共语境每次均命中 4,000 条上限，不把窗口以外遗漏当成质量成功。
- SQL 诊断额外比较旧式查询和优化后的密集查询所选 previous 向量完全一致，保留最终统一排序与 4,000 条限额。

从仓库根目录运行：

```sh
CARGO_TARGET_DIR=/tmp/inputia-personalization-scale-target \
  cargo test --release \
  --manifest-path crates/inputia-handy-runtime/Cargo.toml \
  --test personalization_scale -- --ignored --nocapture
```

此 target 独立于应用构建。普通测试默认不会执行压力测试。下面的基线结果来自修复前实际执行日志；当前命令运行的是最终源码，不会自动回退工作区来重建基线。

## Release 基线与最终结果

基线源码 SHA256：`f68db649c2f780927e0761e0ee42bb56b3a9eb55e8bd6b931f204d1312803460`。

最终源码 SHA256：`4394895d09b44a7a6bafab9c262d724884039fae788bfbea94a75cfb710386d7`。

单位均为毫秒。p50 / p95 来自完整 query；SQL 单次仅用于定位主证据 SELECT 耗时，不包括预检、密度探测、评分等步骤，不是另一个总体延迟指标。

| 候选数 | 上下文 | 基线 p50 | 基线 p95 | 最终 p50 | 最终 p95 | 基线 SQL 单次 | 最终 SQL 单次 |
| ------ | ------ | -------: | -------: | -------: | -------: | ------------: | ------------: |
| 32     | 无     |     9.55 |    10.29 |     6.08 |     6.36 |          5.97 |          3.50 |
| 32     | 有     |   716.41 |   721.14 |    22.54 |    24.06 |        542.05 |          7.80 |
| 64     | 无     |    14.35 |    15.03 |    10.58 |    11.04 |          9.32 |          7.68 |
| 64     | 有     |   721.49 |   726.61 |    26.44 |    27.34 |        541.29 |          6.64 |

32 候选无上下文选取 2,528 条证据，其余普通场景选取 4,000 条；有上下文各 20 / 20 次均报告 `evidence_limited=true`。最终最高单次为 27.50ms。基线库为 330,522,624 bytes / 315.21MiB，最终库为 331,194,368 bytes / 315.85MiB，新增时间索引约 0.64MiB。

| 稀疏场景                     | 候选数 | 最终 p50 | 最终 p95 |
| ---------------------------- | ------ | -------: | -------: |
| 138 个锚点仅关联一条有效记录 | 32     |     5.03 |     5.15 |
| 138 个锚点仅关联一条有效记录 | 64     |     9.19 |     9.42 |
| 138 个锚点只关联已撤销记录   | 32     |     4.48 |     4.60 |
| 138 个锚点只关联已撤销记录   | 64     |     8.56 |     8.73 |

稀疏场景均 `evidence_limited=false`。第一版优化曾在这些场景退化至 786–796ms，因此最后加入稀疏 / 密集查询分路。这是中间优化版本的退化测量，不是原始基线的稀疏耗时。

Debug 基线的有语境 p95 约 3 秒，仅用于发现问题；最终性能结论全部使用 release。最终压力测试总时长 25.03 秒，其中约 22.32 秒是建库，不计入查询延迟。

## 修复内容及语义保持

原 SQL 的 `(0 OR ...)` 阻碍有效索引利用，语境锚点的全局 `IN` 子查询需要物化大量 posting，再对 evidence 做扫描与临时排序。基线 SQL 单次约 541ms，占据主要耗时。剩余主要开销还包括重复创建当前语境的锚点，以及不同早期前句导致相同近句的相似度缓存失效。

最终生产实现位于 `src/personalization.rs`：

1. 增加 active recency 部分表达式索引，保持 `(origin='') DESC, created DESC` 原排序。
2. 已知候选文本走 normalized 索引；去掉恒假 OR 分支。
3. 最多 144 个当前锚点先做存在性预检，再对有效且未遗忘的 evidence 做 `DISTINCT event_id LIMIT 4001` 探测。4001 只决定执行计划，不是最终证据限额。
4. 探测结果不超过 4000 时，已取得完整匹配 ID 集，使用索引查找；超过时走 recency 索引和逐事件相关 EXISTS，避免物化全部 posting。所有 OR 分支仍共用原排序和最终 4000 条限额。
5. 探测与主 SELECT 放在同一个短读事务内，读取完 rows 后立刻提交；来源验证、可能的清理写入以及发布前 epoch / policy 复核均在该事务之后。
6. 当前语境 anchors 每次查询只创建一次；弱相似度以实际最后分句的 48 字窗口缓存。每条证据仍先判完整 suffix 精确匹配，原相似度公式保持不变，避免弱缓存被误当成精确语境。公共词组也复用当前锚点。

`performance_review` 独立只读审查确认：稀疏完整 ID 分支与原 EXISTS 条件等价、有效性筛选早于 4001 探测、最终排序与 4000 限额保留，短读事务不遮挡原有发布前并发复核。

## 回归证据

```sh
CARGO_TARGET_DIR=/tmp/inputia-personalization-scale-target \
  cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml \
  --lib --test personalization --test personalization_upgrade \
  --test candidate_quality_replay --test learning_contributions \
  --test personalization_wire -- --nocapture
```

共 64 项通过：lib 24、候选质量回放 2、学习来源 6、personalization 19、upgrade 10、wire 3。固定合成语境 24 例的 Top1 从原候选顺序 8 / 24 提升至 24 / 24，Top3 从 22 / 24 提升至 24 / 24；性能修复保留该结果。该小回放最终 debug query p50 1.37ms / p95 1.76ms，与上面的 release 大库测试分开报告。受影响文件 `rustfmt --check`、`git diff --check` 通过。

本机完整原始日志：

- `/tmp/inputia-personalization-scale-20260930.log`：debug 基线。
- `/tmp/inputia-personalization-scale-release-before-20260930.log`：release 修复前。
- `/tmp/inputia-personalization-scale-release-sparse-20260930.log`：第一版优化的稀疏退化。
- `/tmp/inputia-personalization-scale-release-final-20260930.log`：最终 release 全部场景。
- `/tmp/inputia-personalization-scale-regression-final-20260930.log`：最终 64 项回归及 24 例排序输出。

日志位于临时目录，可能被系统清理；核心样本设计、源码 hash、命令和结果已在本文件留存。真实 IMK 输入和应用交付验证由主任务单独执行，不能用此测试代替。
