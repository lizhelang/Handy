# Inputia 本地判断模型 worker

第一版固定模型目标：`aac6fef/laya-multilingual-mlx`，MLX 多语言 typed-decision checkpoint，Apple Silicon 本地运行。当前清单已固定 source revision `052592a15d198d9ad47da779604259b10b47b7aa`、主权重 643,835,426 bytes、SHA-256 `7fc5834af4d8fdfb268d272a9d1a66e5819a0daac98241651c4c888cc43adff1`；隔离 fixture 已完整校验并在本机 M2 Max 实测，冷加载约 636 ms，常驻 warm 判断约 12 ms。这些数字不是完整应用端到端延迟，也不是生产安装完成证据。

当前 worker 只提供有界 JSONL 协议适配，不提供网络监听，不接收 API key，也不会自行写入历史或知识库。缺少 `laya_mlx` 时返回 `decision_dependency_unavailable`，调用方必须回退确定性路径。

生产 sidecar 构建使用 `build_sidecar.sh`；它要求已验证的 Python/MLX 环境和模型目录，显式收集 `laya_mlx`/`mlx` hidden imports。冻结 worker 的真实 inference smoke 通过后，才允许进入应用资源打包；权重不进入 Git。

开发环境安装和验证：

```text
python3 -m venv .venv
. .venv/bin/activate
pip install laya-mlx
export INPUTIA_DECISION_MODEL_PATH=aac6fef/laya-multilingual-mlx
python native/local-decision/worker.py
```

模型 revision、SHA-256、许可证和本机 M2 Max 性能结果在 P1 验收前必须补入模型清单；未完成前不能随应用自动下载或宣称已可用。
