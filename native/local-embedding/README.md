# BGE-M3 MLX embedding worker

这是独立于 Laya typed-decision worker 的 P6 embedding 路径。当前 macOS 默认使用
已验证的 Laya encoder mean pooling，输出 768 维；BGE-M3 4-bit 是后续可选升级，输出 1024 维。

Laya encoder 模式明确标记为 `laya-multilingual-encoder-mean`，不能把它当作 BGE-M3 质量或向量身份。

worker 只负责模型加载和向量计算，不读取 SQLite、不执行检索、不写知识库、不监听网络。模型目录不存在时返回 `embedding_model_missing`；调用方必须回退关键词检索。

验证前需要设置：

```text
INPUTIA_EMBEDDING_MODEL_PATH=/path/to/verified/bge-m3-4bit
python native/local-embedding/worker.py
```

只有模型目录逐文件校验、向量维度/有限值和跨语言检索 fixture 通过后，才允许接入知识索引。

下载默认使用官方 Hugging Face；网络受限时可显式设置 `INPUTIA_HF_ENDPOINT=https://hf-mirror.com`，模型仓库、revision 和 SHA-256 不变。
