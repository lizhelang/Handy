# 配对、迁移与固定音频检查点

北京时间 2026-09-05 14:51，接续 `eef101e3`。本报告只证明列出的分项；完整目标保持active。

## 候选构建与数据域

- Inputia输出：`macos/InputiaInputMethod/candidate-builds/trial-20260905/InputiaUnifiedCandidate.app` 及同目录 `Inputia 候选设置.app`。完整build两次成功，没有安装、启动输入法或注册/选择TIS来源。
- 独立部署诊断发现并单变量复现：run_cargo删除 `MACOSX_DEPLOYMENT_TARGET` 导致SQLite对象minos26.5；保留13.0后对象minos13.0。构建脚本现在检查所有静态对象和最终二进制/plist，冲突环境值在删除构建前拒绝。462个对象只出现11.0/13.0，两个app严格签名通过；不冒充13.0实机运行证据。
- Inputia Host CDHash `f65d3d818d662c2f6eda0c7cb3b5a693e0e9ec3c`；Settings CDHash `4afddf85a2cdb3d708b0673773e06ce192f61022`，Settings内ExpectedHostCDHash匹配。完整日志 `/tmp/inputia-candidate-rebuild.lNjVF4`。
- Handy初次输出：`src-tauri/target/release/bundle/macos/Handy Unified Candidate.app`，严格签名与Boolean/profile元数据检查通过。诊断入口确认 `unified-candidate:trial-20260905`，两端数据根只在 `Library/Application Support/HandyUnifiedCandidate/trial-20260905/{Handy,Inputia}`。仅创建专用候选目录，不开日常库，不启动Tauri。
- 初次Handy二进制SHA256 `c6910e8aa979d39ec2a31301af525a4ca6783b574d60ceda1ecd51892cf53ac9`；这是中间构建，**不包含此后所有加固，不是最终候选**。
- Rust候选域新增9项测试：严格Boolean/运行ID、编译和包身份错配、裸候选不能回退日常、链接/硬链接/sidecar、私有根权限、WK专属存储选择。原portable测试保留。InputiaProfile90项合成自检通过。
- macOS的data_directory经依赖源码核对不控制WK存储，已改14+专属UUID、13显式非持久；历史和设置仍在后台持久化。实际多窗口/重启存储隔离、13上的未决操作UI恢复仍未原生验收。

## 迁移边界修复

独立审查发现旧marker及manifest绝对引用可越过当前副本域；已在读取/hash/恢复前约束域、relative path、source根、链接及sidecar。旧副本引用原路径会明确拒绝并要求重定位，不偷偷读原库。

额外实验：VACUUM快照1条记录，配上后来提交的原WAL后读成2条，证明“分别备份主快照和原sidecar再混放”不是一致恢复。新增 `VacuumIntoV1` 自包含快照；新备份不复制其来源sidecar。旧非空WAL/journal默认拒绝自动恢复，显式 `VerifiedSnapshotPoint` 会返回/落盘所选快照hash、保留取证sidecar位置与 `lossless_legacy_wal_merge=false`。

目标旧主库及精确sidecar先移入受管恢复隔离目录，再替换验证的新文件；失败可按隔离副本恢复，其他文件不动。setup失败时不在live manager连接下覆写，保留已验证备份供下次启动、打开manager前恢复。

迁移测试25项通过、1项既有真实数据测试仍忽略；严格Clippy通过。新候选清单包括integration.db、学习密钥、Host outbox/policy/snapshots。**不证明全局停写、跨库原子性或最终兼容回滚已完成**：旧complete marker升级、显式重定位工具、最新删除/遗忘屏障与兼容Host仍待A12全套演练。

## 私有通讯合同和认证原型

- `voice_protocol.rs` 明确Start/Stop/Cancel/Status、server/client实例、目标/controller/字段代数、策略和词库版本；5项契约测试通过。这里只定义消息，不再建立一套录音状态机，实际所有权仍属于TranscriptionCoordinator。
- `native/unified-pair-auth/` 使用Security.framework P256配对公钥和外置签名manifest，避免两个CDHash互嵌循环。签名前嵌公钥，签名后manifest固定两端代码身份；没有把0600文件或同UID当身份根。
- 双向内核audit token→动态SecCode→identifier+CDHash requirement。篡改、重复/未知字段、错profile/run/角色、同UID同identifier另一代码、假服务端均有合成测试。
- 主代理独立执行 `bash native/unified-pair-auth/run-self-check.sh`：`/tmp/uipa-build.M9jjus`，43项通过，`keychain_written=false private_key_persisted=false business_authentication_ready=false`。
- **尚未接入产品**。默认硬化要求true；当前Inputia旧entitlements允许disable-library-validation，动态librime仍依赖外部安装。不能在未验证依赖加载和注入防护前改成UID信任或开放业务。固定公钥嵌入、短socket、Host后台握手/续约与会话仍待实现。

## A11固定音频及真实识别冒烟

- `tools/unified-voice-benchmark/manifest.json`：20个确认测试术语各3句，共60段；另40段普通/未说词负例。实际离线Tingting+say/afconvert生成100个16kHz/mono/16-bit PCM WAV，总304.979625秒，约10MB。
- 音频及逐段hash：同目录 `artifacts/audio-evidence.json`；冻结记录 `freeze.json`；纯格式变更的原文/语义一致性证据 `format-only-reconciliation.json`。实际音频未因模型输出改写。
- manifest文件SHA256：`ca4c352a7bdc57c264b946d11e49bac57f5e76e22493e85eb7623b997beb116e`；语义SHA256：`72dff3d090477d88e1191877355323ec4e83d1145f96324ffbeb9e5b47606fd1`。
- 评分器14项测试通过。两组必须同完整model、binary/weights hash、解码/后处理参数，仅显式terms_prompt可不同；缺字段、错条件、隐藏参数变化拒绝比较。未把评分器夹具当模型预测。
- 依据项目锁定目录下载 Qwen3-ASR-0.6B-Q8_0，revision `e4e16599b900eb0cb36e524514756bb92eb092b7`，850423456字节，SHA256 `f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8`，与目录期望一致。仅存上述候选Handy/models，不改日常模型。
- 初次候选真实headless识别 `term-01-1.wav`：3.208375秒音频、模型冷加载571ms、Metal MTL0推理704ms，输出“明天讨论新蓝计划的测试安排。”；gold为“明天讨论星澜计划的测试安排。”。专名错字如实保留，这是**一段真实ASR冒烟，不是A11通过**。
- 完整基线/统一词库热词组各100段尚未执行。听感复核、总体CER/WER、未说词插入和真实词库→ASR路径仍需完成。

## 下一执行点

1. 冻结并重建包含最新身份/WK/迁移修复的候选，复核编译身份负例；不安装。
2. 解决候选动态依赖与硬化，嵌入配对公钥/manifest并验证真实两端；保持基础输入独立。
3. 将会话合同接到唯一录音协调器和Host主线程交付，把学习/排名/配置扫描移出新增按键路径。
4. 接入统一术语快照，执行已冻结音频的完整质量对照；恢复FunASR并落实Sherpa，不以换名替代。
5. 完成P5及P6全套原生、性能、迁移/兼容回滚、独立终审和最终提交可安装包。

全部A01–A12仍未最终验收；没有日常安装替换、真实数据迁移或远程发布。
