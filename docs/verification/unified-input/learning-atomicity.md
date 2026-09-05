# 学习贡献与内容事务接线

范围：P1 分项，2026-09-05；不是 A01/A08 或完整融合完成报告。

## 实现

- `IntegrationStore::enable_learning` 在规范库安装学习表及删除/改文撤销触发器，沿用唯一后台数据库连接。
- 删除、正文修订、快照对账删除均在原索引事务内撤销对应贡献；收藏和标题的变更保留原合法贡献。
- `contribute_term` 对照实际来源记录、规范信任程度及当前记录版本，贡献绑定 `content_revision`；不能靠调用者上下文把 unknown 来源提升为 verified。显式独立用户词需后续独立授权写入路径，不复用自动学习入口。
- `forget_term` 同一事务删除贡献、写遗忘标记、前移统一 policy epoch。策略写入失败时，贡献与标记一并回滚。
- `private_key` 使用系统随机源 getrandom（复用 Handy 已有 0.2.17 版本）创建 32 字节、0600 密钥文件；拒绝异常长度、符号/硬链接、非私有权限。已有账本缺密钥时失败，不悄悄生成替代密钥。
- 后台服务初始化该账本；Handy 注册只读 `get_unified_terms`，绑定由 debug-only 导出命令生成。

## 实现细化：双版本撤销

内容删除和正文修订经常发生。若它们每次都提升全局隐私 epoch，将使源数据库正常排队的历史事件持续过期。因此在批准的撤销语义下，增加独立 `learning_generation`：

- 全局隐私变化、忘记操作仍前移 policy epoch。
- 新贡献、删除、正文修订、忘记同时在事务中前移 learning generation。
- `TermSnapshot` 返回两个版本及术语；`term_snapshot_is_current` 在一致性读事务内检查两者。
- 服务的 `session_hotwords` 返回完整版本快照，不仅返回字符串列表。
- P3 消费者仍必须接入在线版本通知及 2 秒租约；本轮不把存储层的版本失效当成已经完成 Host 的即时撤销。

## 新鲜验证

- `cargo +1.96.0 test --manifest-path crates/inputia-handy-runtime/Cargo.toml --test store_learning`：6 passed，0 failed。
- 独立审查代理另行执行同一测试：6 passed，0 failed；三项发现均在限定 runtime 范围关闭。
- `cargo +1.96.0 test --manifest-path crates/inputia-handy-runtime/Cargo.toml private_key`：2 个密钥单测通过，其他测试被该过滤器排除，不计为覆盖。
- `history_service`：2 项通过，含后台同步及重新打开。
- Runtime `clippy --all-targets -- -D warnings`：通过。
- Handy debug-only 绑定导出编译及执行成功；重新格式化生成文件后前端 build 通过。

故障注入覆盖：源删除后事件回执写入失败、忘记后策略更新失败，确认内容/贡献/策略版本均一起回滚。其他反例包括收藏后再贡献第二词、unknown 来源伪提升、旧快照被撤销且下一源序列仍可消费。

## 剩余必做

用户词增删的受控命令与 UI；学习来源和内容修订的完整来源链；全局策略前移后源旧 epoch 重核；受管密钥的备份/恢复清单与真实兼容回滚；Host 租约/通知；模型会话消费与100段实际音频质量对照。以上均未由本轮测试代替。
