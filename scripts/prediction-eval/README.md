# 空输入联想离线评测

用于比较上屏后联想引擎的改动。测试集、切分规则、基线与验收门槛固定在
[`manifest.json`](../../crates/inputia-handy-runtime/tests/fixtures/prediction_eval/manifest.json)，调参只能使用 `*-dev` 集合。

## 出题与命中

- 用 jieba 0.42.1（仅评测依赖，不随应用分发）切词，只在两侧都是汉字的词边界出题；每条记录最多 40 个位置。
- 前文取该位置之前最多 512 字，与输入法传给服务的上限一致。
- 某个联想恰好是剩余文字的开头即为命中，不要求评测分词与引擎词表一致。
- 指标：首选/前 N 命中率、MRR、平均省字数、空框率、"有候选但全错"率、服务内查询耗时。

## 测试集

| 集合 | 来源                                                                                | 存放                                                                   |
| ---- | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| T1   | 独立 agent 在无仓库访问下撰写的 400 句日常句，固定哈希切 100 句开发集               | 入库 `t1_everyday.jsonl`                                               |
| T2   | 中文维基 2600 个随机条目导语中的简体句子，2500 句，前 500 句为开发集                | 正文按 CC BY-SA 4.0 只存 `target/prediction-eval/`；入库页面与修订清单 |
| T3   | 本机语音与键入历史，各取时间上最后 20% 作测试；学习库副本只保留键入切分点之前的证据 | 只在临时私有目录，结束后删除；只记录汇总数字                           |

## 复现

```bash
python3 -m venv /tmp/prediction-eval-env && /tmp/prediction-eval-env/bin/pip install jieba==0.42.1
cargo build --release --offline --manifest-path crates/inputia-handy-runtime/Cargo.toml --example prediction_eval
/tmp/prediction-eval-env/bin/python scripts/prediction-eval/build_cases.py \
  --items crates/inputia-handy-runtime/tests/fixtures/prediction_eval/t1_everyday.jsonl \
  --set-prefix t1 --dev 100 --out target/prediction-eval/t1_cases.jsonl
crates/inputia-handy-runtime/target/release/examples/prediction_eval \
  --root target/prediction-eval/root-public \
  --lexicon src-tauri/resources/personalization/base-lexicon.tsv \
  --cases target/prediction-eval/t1_cases.jsonl --limits 3,5
```

问号与表情（1.1.34）另有三组公开用例，门槛与结果同样记在 `manifest.json` 的 `punctuation_emoji` 下：

| 集合  | 来源                                                                         | 用法                                                      |
| ----- | ---------------------------------------------------------------------------- | --------------------------------------------------------- |
| T4    | 独立 agent 撰写的 240 句问句与非问句（含难例），按句式分层切 60 句开发集     | 句末用例看问号召回与误出，词边界用例看句中误出            |
| T5    | 首次测试判定后另一个独立 agent 按中性说明新写的 240 句，全部作测试           | 问号与感叹号门槛的终验                                    |
| emoji | 独立 agent 撰写的 60 个表情词（带可接受表情）与 60 个不该出表情的词，各切 15 | 以该词为前文，看前 3 项是否含可接受表情、反例是否误出表情 |

```bash
python scripts/prediction-eval/build_cases.py --items .../t4_punctuation.jsonl --set-prefix t4 --dev 60 \
  --stratify form --out target/prediction-eval/t4_cases.jsonl --ends-out target/prediction-eval/t4_ends.jsonl
python scripts/prediction-eval/build_cases.py --emoji --items .../emoji_cases.json --set-prefix emoji --dev 30 \
  --stratify polarity --out target/prediction-eval/emoji_cases.jsonl
```

`--ends-out` 把"汉字后紧跟标点"的位置写成句末用例（答案为该标点），T1/T2/T3 同样适用；原有词边界用例文件不变。回放工具对句末用例输出 `ends`（各标点处问号、感叹号是否出现及是否排第一），对词边界用例输出 `middle`（句中误出标点），对表情用例输出 `emoji`。联想标点可能是半角，统计时统一按全角比较。

T2 用 `fetch_wikipedia.py --cache <原始缓存> --out <句子文件> --sources <清单>` 按入库清单重新取回同一批页面；维基摘录接口返回页面当前版本，条目被编辑后 `items_sha256` 会对不上，此时以首次生成的本地缓存为准，不能把重取结果当作同一测试集。T3 用 `export_local_history.py` 生成到私有临时目录，回放时不得传 `--dump`，评测后删除整个目录。
