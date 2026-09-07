# 静态引擎、会话账本与认证桥接检查点

北京时间 2026-09-05 17:54，接续提交 `bf1b7af5` 的未提交实施。此文只记录分项证据，不表示 P0–P6 或 A01–A12 已完成。

## 原生静态 Rime

动态库在严格运行时下被拒绝后，改用锁定 librime 1.16.0 源码静态合并 Lua、octagram/grammar、predict 与依赖；没有关闭 library validation，没有删除插件。

- `native/static-rime/` 保留来源锁、许可证、源码下载、构建和运行材料；`inputia-rime` / `inputia-capi` 增加显式 `bundled-static-rime` 构建模式。默认动态模式保留。
- 完整 Inputia 候选 Host 已重新构建：`macos/InputiaInputMethod/candidate-builds/trial-20260905/InputiaUnifiedCandidate.app`。该包仍是中间产物，尚未包含完整融合，未安装或注册。
- 此轮 Host CDHash：`1e5a93177d29368f6762a235c0927b168ba70ff5`，ad-hoc + hardened runtime，只有 sandbox=false entitlement；不含 disable-library-validation。最低系统版本13.0，动态依赖只含系统项。
- 实际 Host `--unified-runtime-self-check` 返回 `bundled_static_rime=true pinyin_commit=true double_pinyin_commit=true external_librime_loaded=false imk_server_started=false daily_user_dictionary_opened=false synthetic_profile=true`。真实 CAPI 全拼/小鹤输出“中国”，没有启动 IMKServer 或外部应用窗口。
- 候选 profile 自检扩为95项；编译角色、Host/设置包身份及裸二进制错配在任何数据初始化前拒绝。
- 静态 Rime 15项测试、静态 CAPI 24项及2项编译失败文档测试通过。CAPI 21个导出明确 unsafe/Safety 合同，默认动态模式24项及2项文档测试通过，两个模式严格 Clippy 通过，未改ABI或学习语义。
- 严格签名 Swift→Rust CAPI→静态 Rime 探针完成多session、free/reopen、全拼/双拼及合成术语重开保留。日志：`native/static-rime/artifacts/output/arm64/capi-ffi-hardened-probe.log`，相关测试与导出符号日志在同目录。

以上不是 InputMethodKit 跨应用验收。生产词库全覆盖、grammar真实模型评分、按键延迟、共享词库遗忘及离线输入仍待完成。静态 GPL 组件原文和来源材料保留，不把根 MIT 当作完整分发许可结论；没有外部发布。

## 独立构建审查与修复

独立审查发现“verify-only运行旧探针后给当前库重写manifest”的P1证据绑定漏洞。已修改为：完整链接前记录输入哈希，签名后确认输入未变并绑定探针哈希；验证和Cargo链接只接受精确绑定，verify-only不改manifest。

- `test_artifact_binding.py` 6项合成测试通过：换库、换探针、链接中改头文件、旧无绑定manifest均拒绝，新链接撤销旧回执。
- 重新链接并实际运行原生探针通过，日志 `native/static-rime/artifacts/probe-run.ka1attrr/probe.log`。
- 再次verify-only通过，日志 `native/static-rime/artifacts/probe-run.thsyae0n/probe.log`；运行前后manifest SHA均为 `170abaaac64c9f3ef8c378ca7a12c62aac23d8ae0f23183545229f86d0519bff`。只读Cargo输入门禁通过。
- 另发现解压源码缓存只认stamp，可能将改动的缓存错误归于原来源锁；正在修复，**尚未关闭该P2**。最终可复现性尚未验收。
- 最终候选RimeData还依赖日常Squirrel及浮动上游资源的P2也被发现；已分工实现候选专用锁定官方资源包，不执行其安装程序、不缩减现有词典，产物对账与复核仍进行中。

## 持久会话与原协调器

- `voice_ledger.rs` 记录请求语义、唯一Start执行资格、目标归属和单调事实投影；`IntegrationStore`事务和`HistoryService`唯一写入者接线。100个不同request ID重发同session只获一次Start，SQLite第二语句失败完整回滚，策略和已应用屏障在claim再次核验。
- 启动恢复仅在独占writer lease后运行，旧Preparing/Recording/Processing变Interrupted，既有输出事实保留，不重开录音。
- 真实后台服务延迟claim超过5秒：调用方没有执行权限；即使迟到数据库任务提交，重试也不再次授予。此项是本地服务故障测试，不冒充产品断线场景。
- 本轮新鲜完整 runtime 测试 **105 passed / 0 failed / 0 ignored**（包括实际SQLite、UDS和上述队列超时）。
- 已本地提交为 `d6935c66`；严格Clippy与格式检查通过。后续新增无音频条目回归另计，未把后续测试数写回旧提交。
- 上游 `TranscriptionCoordinator` 仍是唯一录音状态机。显式Start/Stop/Cancel、进程归属、15秒准备时限、首帧Recording、单调view generation、重复请求及延迟旧回调均加入覆盖；64项协调器测试通过。
- 取消入口只入队，由原actor顺序执行真实清理；删除“清理后采样最新generation再通知”的迟到取消竞态。动态Cancel注册转独立单worker，旧注销完成后才处理最新需求。
- Handy最近整库测试 **414 passed / 2 ignored**，严格Clippy通过；这是后述SecureInput修复之前的结果，不替代其回归。
- 独立审查确认SecureInput后台持锁等主线程、GUI设置命令又等同锁的P1死锁。正在把完整reconcile的锁和系统操作统一到主线程，保持shadow→primary同步屏障；**尚未关闭该发现**。
- 主线程调度修复及5项注入测试已落盘，随后全Handy测试419通过/2忽略、Clippy通过；主代理又发现切换失败后已保存new_impl导致同实现重试假成功，正在修复。真实Carbon/GUI矩阵仍未执行。
- 成功文字不依赖WAV已改生产分支，空file_name映射无音频附件，重转写和删除避免把录音目录当文件；SQLite持久重放新增用例通过。独立审查要求补足实际保存分支的WAV失败注入回归，正在补测，尚不声明P1验收通过。

## 原生配对桥

`native/unified-pair-auth/PairAuthBridge.*` 与 `src-tauri/src/native_pair_auth.rs` 已构建到Handy库，使用真实Rust→Swift→Security.framework调用。公开原生API不等于WebView命令；没有从设置/环境/握手建立信任根。

- 编译期公钥接口限定静态公钥/key/run/profile，Swift handle由Rust RAII独占，不跨线程；VerifiedPeer只能从实际认证成功构造。
- 真实桥接诊断 `/tmp/uipb-build.AQ6ynz`：双向严格对端接受、同已允许CDHash但弱运行时拒绝、错key/profile/fd与50次生命周期检查。43项原配对检查另在 `/tmp/uipa-build.Cf3cHZ` 通过。
- 临时私钥未写入Keychain，构建实验结束移除；没有发行Team ID、公证或安装。
- 非作者独立审查实际发现关闭可执行页保护的已允许CDHash程序仍会被接受，原失败证据 `/tmp/uipa-review-weak.CAkdwH/reproduction.log`。已增加该entitlement拒绝及真实弱签名回归，主代理重跑 `/tmp/uipb-build.46gKyn` 全部通过，独立源码复核确认修复。部署元数据另由非作者重编复核通过；不是旧OS实机证明。

## 必须接续的产品工作

1. 关闭上述P1/P2并复核，分组提交当前源码和证据。
2. 产品双端构建期公钥、签名manifest、私有socket认证和策略/遗忘屏障尚未接通；不能开放UID-only业务服务。
3. Coordinator已有owned会话API，但Host仍是旧启动器；ASR完成仍走旧paste分支。接线前必须把owned结果保存为统一内容、交给持久输出所有者并走Host最终目标检查，不能让两条路线并行输出。
4. Host按键路径去同步DB/学习、异步outbox与2秒快照租约、统一浮窗、控制中心、确认术语→本地ASR、FunASR/Sherpa仍待完成。
5. 固定100段双组音频、真实三应用、性能、迁移/恢复/兼容回滚、最终独立审查及最终提交候选包均未验收。

所有A01–A12保持未最终验证；没有修改日常安装、唯一真实数据或远程分支，goal保持active。
