# Inputia 类 JEV 本地判断模型执行进度

要求矩阵与完成边界见 `docs/verification/2026-09-24-local-decision-model-audit.md`。

日期：2026-09-24。对应计划：`P20260924-183531` / 模型明确修订版。

## 已完成

- 目标机器已核对：Apple M2 Max、32 GB、arm64。
- Rust 判断协议已加入 `crates/inputia-handy-runtime/src/decision.rs`：有界 state、问题数、候选数、deadline、协议版本、request_id、概率范围和候选身份校验。
- Rust sidecar 管理已加入 `src-tauri/src/decision_worker.rs`：只有设置 `INPUTIA_DECISION_WORKER` 才启动；worker 不配置时不改变任何旧路径；启动、写入、读取、JSON、模型错误均回退。
- MLX JSONL worker 已加入 `native/local-decision/worker.py`，默认模型 ID 为 `laya-multilingual-mlx`；缺少 `laya_mlx` 时返回 `decision_dependency_unavailable`。
- 可重复的分块下载、逐文件校验和原子发布脚本已加入 `native/local-decision/install_model.py`；目标目录已存在时拒绝覆盖。
- 安装脚本的 `--verify` 模式已对 `/tmp/laya-multilingual-mlx` 重新检查主权重大小和 SHA-256，通过后才允许作为 fixture 使用。
- 模型清单和 P0 golden fixture 已加入 `native/local-decision/models.json`、`golden.jsonl`。
- 已从 Hugging Face 的模型 manifest 固定 source revision、主权重大小和 SHA-256；随后通过 HTTP Range 分块下载并完成本机逐文件校验。
- 后处理路由已接入：只有显式配置本地判断 worker 时才询问是否运行 custom-word correction；没有 worker、低置信度、错误或拒答均保留原有纠错路径。
- P2 当前只改变“是否调用 custom-word correction”的路由，不改变纠错器本身；没有配置 worker 时默认保持旧行为。
- P3 第一阶段已接入知识库查询意图标签：worker 显式配置时为搜索请求附加 `exact_lookup/procedure/definition/comparison/history_lookup`，未知或失败不改变现有关键词检索；结果中带回 `decision_intent` 供后续有限重排使用。

## 已验证

- `cargo test --manifest-path crates/inputia-handy-runtime/Cargo.toml decision --lib`：4/4 通过。
- `python3 -m py_compile native/local-decision/worker.py`：通过。
- worker health JSONL：通过。
- `cargo test --manifest-path src-tauri/Cargo.toml actions::tests --lib`：10/10 通过。
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`：通过；现有 dead-code warning 保留，不影响构建。
- Runtime 全部 integration tests：全部通过；Tauri actions tests：10/10 通过。

## 尚未完成

- 本机 Python/MLX 依赖安装成功；普通整包下载曾在未认证 Hugging Face 连接上停滞，后改用 HTTP Range 分块下载，完整 fixture 已通过 manifest SHA-256 校验。
- M2 Max 真实 worker 中文判断通过：重复扣款/退款被选为 `billing`，第二次常驻判断 `elapsed_ms=12`，首次加载约 `636 ms`。这仍不是应用端到端延迟，也不代表模型质量已通过完整 golden set。
- 11 次连续 benchmark 中 10 次 warm 判断的模型耗时 P50=8 ms、P95=9 ms；另有一次性进程 wall time 约846 ms，包含首次加载和进程启动，不能当作输入热路径延迟。
- 完整隔离模型目录 `/tmp/laya-multilingual-mlx` 已补齐并通过逐文件 manifest SHA-256；worker 已将 Laya 原始 `choice/noul/score` 响应归一化为 Inputia 协议。
- 初始 golden fixture 的 4 条样本中，敏感凭据识别稳定；全中文 criteria 会造成业务路由低置信，已改成中文 state + 英文 typed criteria 后再评测。
- 将生产提示改为“中文 state + 英文 typed criteria”后，`run-golden.py` 当前结果为 `golden_pass=3/4`：自定义词路由、知识库 procedure、敏感凭据均通过；普通会议文本低置信而安全拒答。该拒答会回退旧路径，不能算业务分类完全通过，但已经证明中英混合提示比全中文 criteria 更稳定。
- 权重已复制到 `src-tauri/resources/local-decision/laya-multilingual-mlx`，随候选 `.app` 分发；生产安装仍需用户侧更新流程完成后才算切换。
- sidecar 已随候选 `.app` 打包；资源路径由 `DecisionWorker::configure_from_resources` 自动发现，未发现或启动失败仍安全回退旧逻辑。
- PyInstaller 冻结版 sidecar 已通过候选 `.app` 内真实中文 inference smoke；构建脚本额外复制 `mlx.metallib`，规避 Tauri 展开 `libmlx.dylib` 符号链接后的 Metal 资源路径问题。
- P5 范围审计：当前 Inputia 没有 LiveCopilot 式“对话问题自动委派”管线，现有语音路径是录音/VAD/ASR/转写/后处理与手动快捷键；因此不能伪造“自动语义触发已完成”。后续如增加自动触发，必须先定义用户可见行为和误触回退，再接入 `question/request/statement/cancellation/incomplete` 判断。
- P5 的确定性预筛模块已加入 `crates/inputia-handy-runtime/src/voice_intent.rs` 并通过 3 个单元测试；它目前不接入自动发送或转写行为，只为未来边界判断提供 fail-closed 第一层。
- P6 已具备当前 macOS 的 Laya encoder embedding fallback；P7 已完成服务端可选二次重排，但尚未进入 IME 默认热路径。
- P6 第一阶段已冻结独立 embedding 合同 `crates/inputia-handy-runtime/src/embedding.rs`：模型身份、revision、维度、有限值、零向量和 cosine identity mismatch 均有测试；BGE worker 已有，但尚未接入索引。
- P6 当前 macOS 默认候选改为已验证的 `laya-multilingual-encoder-mean`（768维），复用已下载 Laya checkpoint；BGE-M3 4-bit（1024维）保留为可选升级，关键词检索仍是失败回退。
- P6 独立 worker 已加入 `native/local-embedding/worker.py`；无完整模型目录时 health 明确返回 `ready=false`，不会生成伪向量。
- P6 Laya encoder mode 已成为当前 macOS 默认 fallback：实测中文/英文对应句余弦相似度约0.827；身份明确为 `laya-multilingual-encoder-mean`，不冒充 BGE-M3。
- Laya embedding worker 最新实测输出 768 维、中文/英文相似度 0.8273、单批 elapsed_ms=39；仍是 fixture/临时 fallback，不代表知识库生产召回率。
- 最终审计重跑：Runtime integration tests 全部通过、Tauri actions 10/10 通过；Laya embedding worker 最新单批 elapsed_ms=72、768维、cosine=0.8273。
- 主程序最终 0.10.3 release bundle 已重新构建并通过 `codesign --verify --deep --strict`；包内 sidecar、Laya 权重和补齐后的 Metal 资源均已通过实际推理验证。
- P6 BGE-M3 4-bit manifest 已固定 repository、revision、319,903,668 bytes、1024维和 SHA-256，记录在 `native/local-embedding/models.json`；权重仍未完成本机下载/校验。
- P6 分块下载/逐文件补齐/原子发布/`--verify` 工具已加入 `native/local-embedding/install_model.py`；当前生产仍不会自动下载或启用 embedding。
- 2026-09-25 重试镜像下载：20 个分块最终返回并完成合并，但 BGE-M3 SHA-256 与固定 manifest 不匹配；安装脚本拒绝发布并清理 staging。当前阻断是镜像 Range 数据完整性失败，不是“已下载但未接入”。
- P6 已增加可选查询时语义重排：知识库只对已授权的最多24条结果调用 embedding worker，按 cosine 排序并回传模型身份；worker 缺失、失败或维度不符时保持原关键词顺序。Laya encoder 仅作临时 fallback，BGE 未验证前不默认启用。
- P7 已冻结候选二次重排验证函数 `personalization::apply_rerank`：只接受真实候选 ID，禁止重复/未知 ID 和跨 consumed_len/match_type 资格组重排；服务端模型请求已接线，IME 热路径仍未默认启用。
- P7 已接入服务端 personalization query 的可选本地 typed-choice 二次排序：只有 worker 显式配置、置信度≥0.5、未拒答且候选 ID/资格组校验通过时才修改 ordered_ids；否则保持原有个性化顺序。IME 现有 admission、epoch 和字段校验仍在更后面执行。
- P3 已完成意图接线、低置信安全降级和有界确定性重排；尚未完成本地 embedding。
- P3 已增加有界的确定性意图重排：只有 worker 返回高置信意图时才按标题/查询词/意图标记对已授权候选排序；模型失败或低置信度仍保持关键词顺序。
- P4 已增加本地安全预筛：明显凭据、过短噪声不进入个人学习回填；代码和普通文本保留原有回填路径，模型不可用不扩大权限。
- 现阶段没有把模型输出用于权限、输入提交、Rime 事务或外部知识库写操作。

## 当前安全结论

模型未验证前，普通输入、ASR、Rime、知识库关键词检索和现有后处理仍使用原逻辑。判断 worker 不配置时不会启动；本地模型失败不会静默切换云端；模型下载未完成不代表“本地模型可用”。
