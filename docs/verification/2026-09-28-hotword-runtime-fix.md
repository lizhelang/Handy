# 1.0.5 热词实际输入链路修复

## 实际故障与根因

用户确认 1.0.4 中文输入模式输入 `lll` 仍不显示邮箱热词。

匹配函数合成回归通过，没有覆盖实际候选接线。旧实现要求语音目标快照和辅助功能字段校验：

1. 每次原生 `keyDown` 在候选分发之前执行 `discardPreparedVoiceTarget()`，把 `shortcutPreparedSnapshot` 清空，`sharedTargetReady` 置为 false。
2. 热词查询依赖 `liveSharedTerms()`；连续按键会使后台目标捕获赶不上当前输入状态。
3. 即使已经显示热词，空格、Tab、数字按键也先清空目标，后面的 `selectHotwordOverlay()` 必需快照检查无法通过。
4. 本机诊断还显示字段选区无法观测时，语音目标根本不能准备；手动词库因此完全不能读取。

自动化 TextEdit 按键未进入 IMK 的限制与用户的真实验收结果分别记录，不把前者当作功能通过。本轮依据可证实的源码分发顺序修正候选路径。

## 修复边界

显式热词属于用户手工设置的本地输入词库，仅从当前 profile 的 `Handy/settings_store.json` 的 `settings.custom_words` 读取。后台刷新内存缓存，不读取历史、知识库、自动学习词，也不向外部服务发送这些词。

基础热词候选和选择不再要求语音目标快照、AX 字段可观测性或语音服务已就绪。自动学习、历史和语音的原有目标及权限校验保持。

显示和选择仍验证当前 IMK 输入上下文、客户端、输入源、组合/模式/当前拼音方案、安全输入状态和词库 generation。词库变更或文件失效使旧候选失效。原生候选身份与消费长度保留。

## 验证结果

- 使用已安装的 Command Line Tools，进程级设置 `DEVELOPER_DIR`，未修改全局 Xcode 选择或接受许可。
- 全源 Swift 类型检查与原生 release 构建通过；原有 weak capture 警告不属于本轮改动。
- 新缓存回归覆盖每次按键清空共享/语音状态、Space/Tab 选择合同、英文 3/4 字母及退格、中文全拼/双拼、词库删除后旧候选拒绝、光标变化和安全输入拒绝。
- 固定配置读取回归覆盖文件/父目录符号链接、硬链接、超大文件、缺失文件和失败清空。
- 新构建程序通过与实际运行相同的 loader 读取当前 profile：`loaded_count=65 prefix_matches=1 voice_snapshot_required=false`。诊断不输出词汇正文。
- 这些检查验证接线与数据读取；本轮不声称已完成用户实体键盘的 IMK 提交验收。

## 安装核对

主程序与原生输入法均已签名成对安装为 `1.0.5 / 79`。更新器返回 `candidateUpdate=true tccChanged=false previousRecordingsReplayed=false`，运行程序身份与当前选中的 Hans 输入源核对通过。安装后的原生程序再次执行实际词库自检，结果为 `loaded_count=65 prefix_matches=1 voice_snapshot_required=false`。原词库保留，无需重新录入热词。

本轮只创建本地提交，没有推送或发布 GitHub release。
