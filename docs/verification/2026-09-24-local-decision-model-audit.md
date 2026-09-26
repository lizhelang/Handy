# 类 JEV 本地模型融合要求矩阵

对应 Goal：`01a0d2e3-9f2d-73f3-adc4-f4a16db2fc6c`。

| 阶段                | 当前证据                                                                                                       | 状态                 | 缺口                                                                     |
| ------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------- | ------------------------------------------------------------------------ |
| P0 基线/golden      | `native/local-decision/golden.jsonl`、`run-golden.py`；M2 Max 实测，3/4，低置信拒答                            | 部分完成             | golden 规模仍小，需要更完整中文/英文集和人工标签复核                     |
| P1 DecisionProvider | `crates/inputia-handy-runtime/src/decision.rs`、`src-tauri/src/decision_worker.rs`、MLX JSONL worker、模型清单 | 已完成（候选包）     | 已随候选 `.app` 打包并由资源发现；生产安装切换仍需单独执行               |
| P1 模型管理         | Laya 分块安装/verify 脚本、revision/SHA-256、冷/热测量                                                         | 隔离验证完成         | 生产资源目录、应用更新/回滚和原生安装未完成                              |
| P2 后处理           | `actions.rs` route decision；无 worker/低置信/错误回退旧路径                                                   | 已接线，未默认启用   | 真实转写场景和产品设置入口仍需验收                                       |
| P3 知识库           | 意图标签、低置信回退、有限标题/查询词重排                                                                      | 部分完成             | embedding 未接入；真实资料集检索质量未验收                               |
| P4 历史/剪贴板      | 敏感凭据/短噪声预筛；幂等、revision、撤销既有测试                                                              | 部分完成             | 本地模型分类尚未接入非热路径批处理 UI                                    |
| P5 语音触发         | `voice_intent.rs` 确定性预筛测试                                                                               | 未完成产品功能       | 当前 Inputia 没有自动语义委派入口，需先定义用户可见行为                  |
| P6 embedding        | `embedding.rs` 合同、`native/local-embedding/worker.py`、Laya encoder fixture、BGE manifest                    | Laya fallback 已验证 | 持久索引和生产打包未完成；BGE 仍是未验证的可选升级                       |
| P7 候选二次重排     | `apply_rerank`、服务端可选 worker rerank、候选资格组校验                                                       | 服务端接线           | 尚未进入 IME 默认热路径，缺真实键盘验收                                  |
| 隐私/权限           | worker 不开网络端口；输入权限、epoch、字段证明和外部知识只读保留                                               | 已完成（候选包）     | 已通过候选包深度签名校验和包内 sidecar 推理；仍需真实 IME 热路径长时验收 |
| 最终交付            | Rust/前端/专项测试、模型资源、固定身份签名和原生 smoke                                                         | 候选交付完成         | 生产安装/切换与真实键盘长时验收属于后续发布步骤                          |

## 当前可以声称的结果

- 本地 typed-decision 协议、MLX sidecar、模型校验、低置信回退和 P7 服务端安全门已经有代码和专项测试。
- M2 Max 上 Laya multilingual worker 已完成隔离真实推理；warm 模型判断约 8–12 ms，首次加载约 636 ms。
- 没有 worker、模型缺失、超时或低置信时，Inputia 继续使用原有确定性路径。

## 当前不能声称的结果

- 不能声称生产 Inputia 已随包携带并默认启用 MLX 模型。
- 不能声称 BGE-M3 embedding 已经在知识库生产索引中工作。
- 不能声称 P5 自动语义语音触发已经实现。
- 不能声称 P7 已经通过实体键盘端到端验收。
- 不能声称已经达到主流输入法、Jev 云模型或 LiveCopilot 的整体质量。

## 2026-09-24 最终回归快照

- Runtime integration tests：全部通过（含 embedding、voice_intent、personalization rerank、knowledge/history）；
- Tauri actions tests：10/10 通过；
- MLX Laya fixture：模型目录 verify 通过，golden 3/4，普通会议文本因低置信拒答并回退；
- BGE-M3：manifest 已固定，完整权重尚未通过本机校验；
- 原生候选 Inputia 包已完成签名和包内 sidecar 推理 smoke；IME 热路径和实体键盘/语音长时验收仍未宣称完成。
