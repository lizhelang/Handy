# A11 固定离线语音夹具

仅用于 Handy 融合验收的原创合成文本。不得自动加入真实用户词库；不读取录音、不录麦克风、不调用付费或远程服务。当前只准备音频和评分工具，不声称识别质量通过。

## 冻结合同

`manifest.json` 固定 20 个显式测试术语，各 3 个不同句子，共 60 段；另有 40 段普通语句，其中 10 段含近音/近形干扰。包含中文、英文和中英混合句。每段明确列出 `expected_terms` 和所有其他 `forbidden_terms`。普通组全部 20 个术语均为“未说词”，任何插入都应报告。

音色固定为本机已列出的 `Tingting (中文（中国大陆）)`，语速按 manifest 为 160/180/200 words per minute。macOS `/usr/bin/say` 输出 AIFF，再由 `/usr/bin/afconvert -f WAVE -d LEI16@16000 -c 1` 转成 16 kHz 单声道 16-bit PCM WAV，不做增益归一化。音色的中英发音、术语读音和自然度尚须听感复核；不能根据 ASR 输出修改 gold transcript。

首次运行会在生成任何音频之前写入 `artifacts/freeze.json`，冻结 manifest、生成器、系统版本与工具 SHA-256。续跑验证冻结合同及已有音频 hash，不静默覆盖。不同系统或声音资产版本可能产生不同波形，因此实际固定测试集以已保存 WAV 和其 hash 为准，而不承诺跨系统逐字节重新合成一致。

本次生成后遵循项目 Prettier 格式要求调整了 JSON 排版。`reconcile_format.py` 逐项证明格式化前后 JSON 值相同，将生成时文件/冻结/音频账本分别保存在 `artifacts/manifest.before-format.json`、`freeze.at-generation.json`、`audio-evidence.at-generation.json`；`format-only-reconciliation.json` 同时记录原始文件 SHA、当前文件 SHA 和共同语义 SHA。当前 `freeze.json` 与 `audio-evidence.json` 引用格式化后的文件 SHA，并明确关联原始证据。该工具拒绝任何句子、术语或参数变化。

## 执行

在此目录运行：

```sh
python3 -m unittest -v test_score.py
python3 generate.py
```

生成器仅使用所列本机声音。`artifacts/audio-evidence.json` 包含逐段 hash、帧数、时长、峰值、RMS 和完成数；这些只证明格式/非全静音，不证明声音正确或模型准确。`artifacts/` 不进入 Git；交付候选证据时需另行保留这份实际固定音频集。

## 接实际识别输出

基线与热词条件必须使用同一份 WAV、模型版本、解码参数与后处理配置，只改变已确认术语提示；不得把 gold 给模型。每个预测文件使用以下 JSON 合同，完整覆盖 100 个 ID，空识别结果写空字符串：

```json
{
  "manifest_sha256": "从生成证据复制",
  "model": { "id": "实际模型", "revision": "实际版本" },
  "condition": "baseline",
  "run_parameters": {
    "binary_sha256": "实际识别二进制的64位小写十六进制SHA256",
    "model_weights_sha256": "实际模型权重或完整权重清单的64位小写十六进制SHA256",
    "decoding": { "language": "zh", "beam_size": 1 },
    "post_processing": { "enabled": false }
  },
  "terms_prompt": [],
  "predictions": [
    {
      "id": "term-01-1",
      "audio_sha256": "对应音频hash",
      "text": "真实模型输出"
    }
  ]
}
```

这里展示结构而非可直接用于验收的假运行记录：hash、模型版本和全部配置必须由实际 runner 填写。多文件模型的 `model_weights_sha256` 应是固定排序、包含每个权重文件相对名称及内容 SHA 的清单摘要，并保存对应清单；不能只散列模型名称。

评分器强制 `--baseline` 文件的 `condition` 为 `baseline`，`--hotwords` 文件的 `condition` 为 `hotwords`。`model` 必须有非空 `id`/`revision`，两组完整对象一致，包括额外后端字段。`run_parameters` 必须有有效二进制/权重 hash、非空 `decoding` 和 `post_processing` 配置，且两组完整 JSON 对象相同；额外配置也参与比较。未启用后处理仍需明确记录 `{ "enabled": false }`，不能省略。

唯一允许区别是顶层 `terms_prompt`：基线必须为空列表，热词组必须是 manifest 已确认术语的非空去重子集。共同参数内不允许嵌入 `terms_prompt`、`terms` 或 `hotwords`。实际提示模板、偏置权重等非术语配置应作为共同解码参数完整记录，两组保持相同。错条件、换模型、换权重、换解码或后处理配置、缺失元数据均拒绝比较，不会产生热词门槛报告。

```sh
python3 score.py --evidence artifacts/audio-evidence.json --baseline artifacts/baseline.json --hotwords artifacts/hotwords.json
```

评分先核对 manifest、完整音频集合、实际 WAV hash 和预测所引用的 hash，再进行纯离线文本计算：NFKC、忽略大小写；CER 去除空白、标点和符号。混合 token WER 明确定义为英文/数字连续串算词、中文逐字算 token，不冒充中文分词 WER。不使用同音字替换或模型专属纠错。输出总体/含术语/普通组的 micro CER、WER、术语正确召回及逐段未说词插入。未说词插入数量按“音频、不同术语”配对计数，同段重复插入同一术语计一对。

比较报告检查专名召回是否严格提高、普通组 CER/WER 绝对恶化是否各不超过 1 个百分点，以及未说术语是否零插入。它不签发完整 A11 通过结论：仍需实际模型运行证据、模型/参数记录、听感与实际识别复核。单测中的完美/错误文本只是评分器夹具，不是识别结果。
