# 中文共享候选接线验证

记录时间：北京时间 2026-09-09 23:46；接续基线 `9f6ae9f3`。

## 本批实际到达位置

认证短词缓存已经接到中文候选排序与选择路径。只重排当前页已有候选；不批量写入 Rime，不生成不存在的新候选，不改变原候选的索引、页和组合串。显示重排后，数字键、空格和点击映射回原索引，经异步实时目标校验后调用原 Rime 选择操作。Enter 保持原组合提交；展开候选恢复原排序。

本批没有启动麦克风，没有更新安装包或修改日常数据。尚未到达用户操作的完整原生闭环，不用以下底层验证替代它。

## 新鲜证据

- 核心两个 `shared_order_` 测试通过，覆盖同输入意图槽位、未匹配顺序、96 词预算与超限回退。最初误用过滤器命中 0 项，不算通过；随后已使用正确过滤器实际执行 2 项。
- CAPI 测试强制真实 Rime 会话、Chinese 模式、非空候选；选取当前页可合法提升的同组候选，断言显示顺序确实变化，最后按返回原索引选择，commit 精确匹配该原候选。排序前后核心快照不变。
- 上述 CAPI 测试分别在动态 Rime 和候选同款 `bundled-static-rime` 下通过；静态测试使用本次候选的 RimeData，独立临时用户数据，不允许缺引擎跳过。
- Swift 自检实际输出 `cases=18 selection_intent_checks=10 chinese_mapping_checks=14 synthetic=true native_candidate_insertion_tested=false`。它证明映射和意图合同，不证明 IMK 原生按键链。
- 完整签名候选构建成功：`/tmp/inputia-chinese-shared-build.log`。候选产物位于 `macos/InputiaInputMethod/candidate-builds/trial-20260905/InputiaUnifiedCandidate.app`，CDHash `7c94057baea552a9da0ed973e132b2d86f5bcba5`。这是工作树验证构建，尚非最终提交发布包，未安装。

## 未验证与下一实验

- 独立审查 `review_socket_path` 为 APPROVE，范围仅本批六个实现/测试文件，无阻塞发现。非阻塞建议：点击仍沿用字符串 `firstIndex`，同页若有重复文本且 ID/annotation 不同，无法区分实际点击位置；数字键/空格映射不受此影响。该边角问题保留在剩余清单，不宣称点击任意重复项已验证。
- 尚缺真实共享词的原生中文显示，以及数字、空格、点击、焦点变化、快速连续按键验证。
- 当前测试配置没有已确认的规范学习贡献；不得写假历史来源冒充真实确认或完整语音链。
- 共享词只对已有候选排序，未证明罕见术语候选生成、Rime 既有学习完全撤销或最终识别质量。
- 下一步完成审查后准备匹配配对与恢复包，运行隔离候选。录音便利性未确认前不启动录音。

## 候选61安装与原生分项（北京时间2026-09-09深夜）

- 输入法代码 `5372e075`（功能提交 `4f7f56f1`）、版本61；完整签名构建日志 `/tmp/inputia-ime61-build.log`。安装路径 `/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app`，实际PID65104，运行身份检查通过，CDHash `5272fa6e9885932c0be11e10e9ea25f03f1e282e`。
- 控制中心未换包，仍为 `/Applications/Inputia Candidate.app`、CDHash `dc6338cf32c1fdfaacf46377a58a43af6c2ebc31`；加载新配对后PID64663，运行身份检查通过，15:50:18 UTC日志有listener ready。
- 停止两个已核对身份的候选进程后，完整备份至 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/chinese-ime61-20260910.XGHSiq`。内含 `profile-before`、`control-before.app`、`ime-before.app`、`pair-new.json`。备份history/integration均quick_check=ok，旧语音5项、规范学习贡献0项。没有安装第二个设置启动器，没有修改日常包。
- 原生测试窗口为专用 TextEdit `未命名7.rtf`。原内容 `?n ?你`；首次紧接选源输入ni时尚无新IME进程就绪证据，字母直接进入文稿。进程就绪后再次按n、i，在Inputia自身窗口截图中真实出现“1你 呢 拟 尼 泥 妮”；按space后TextEdit内容为 `?n ?你ni你`。这是基础中文输入分项，不是共享词排序成功。
- TextEdit单窗口截图不会包含Inputia独立候选窗口，不能因前者没有候选就判定候选未显示；本次另外读取了Inputia窗口截图。
- 当前工具无录屏能力，只有会话内原生截图和AX前后值，未交付视频。首次安装选源后的就绪窗口仍需专项验证。
- 读取本进程固定诊断曾出现 `focused_application_mismatch`（15:51:48 UTC附近），与跨App取截图期间存在时序关联，但未证明因果，不放宽焦点校验。需在后续专用文稿保持前台的实验中确认目标注册成功，之后才做共享词或owned语音验收。
- 恢复：先切离测试源并核对/停止候选PID，把当前IME移到新的保留目录，再将 `ime-before.app`复制回原测试安装路径，恢复 `profile-before/pair-manifest.json`，重启控制中心并重新选择测试源。控制中心本批未改包。不要覆盖旧整份profile，以免丢失安装后新增数据；备份数据只用于单独恢复演练。
