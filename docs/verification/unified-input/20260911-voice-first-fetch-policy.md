# 录音已保存但未自动插入：首次取结果前刷新策略

用户截图：9月11日15:35两条新转写存在于历史，但原输入框没有文字。只读核对实际候选运行签名为配套63，未重装权限、未读取日志正文或重放用户结果。

## 根因证据

- 两条对应owned会话有完整field_id及来源，view为pending_target；输出账本为Prepared，不是已派发或回执不明。
- 开始会话记录的learning_generation为16/17，当前版本为18，policy_epoch仍1。
- 服务端Status允许旧generation查询已拥有会话，但Fetch要求连接实际完成当前完整策略版本屏障。握手只在连接初建时完成屏障。
- 原Host复用录音开始连接取结果；保存/同步造成版本更新后，Fetch会在claim前被拒绝，Host关闭连接，账本留下Prepared。因此不能归因为用户焦点移动或麦克风权限。
- 新Rust回归实际证明：旧连接Status返回pending_target，Fetch Unauthorized且仍Prepared；新屏障完成后首Fetch变Dispatched。不是通过放宽服务端版本检查修复。

## 修复与安全边界

首次Fetch之前开同配对服务的新认证连接、完成已有策略清理屏障，重新Status核对session/target/item/output操作与旧view一致且仍pending_target，然后仅Fetch一次。回执沿同一新连接。旧连接此时关闭；已Fetch后失败绝不重新Fetch或改路。

刷新失败关闭连接并结束本地等待，但保留shortcut ownership与持久Prepared结果，避免下一次录音被非nil等待状态卡住。新会话重置一次门；重复poll不干扰已在进行的派发/回执。原输入框实时校验、组合冲突与敏感字段保护未改。

## 验证

- Rust voice_dispatch 18项通过，包括上述新generation回归。
- paired Swift整体类型检查通过；既有launcher自检新增首Fetch门及失败释放等待覆盖，输出 `preFetchFailureReleasesWait=true shortcut_owner_retained=true`。
- 第一次候选构建因实现文件在编译中变更而失败，已弃用；停止所有写入后重新构建，不能把第一次当成功。
- 本轮没有启动麦克风、没有向真实聊天插入文字。最终原生新录音上屏仍需在新包安装后验证；历史中的旧录音不自动补发。

## 安装状态与外部阻塞

功能提交88667e48，最终停止所有写入后的IME64签名构建通过（/tmp/inputia-ime64-build.log）。独立审查APPROVE，无HIGH/CRITICAL；paired失败清理自检已单独运行，尚未接入标准build的launcher自检编译分支，此为记录的非阻塞覆盖局限。

候选profile与旧IME备份到 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/voice-fetch64-20260911.Rs33zD`，包含profile-before、ime-before.app、pair-new.json。两库quick_check均ok。无进行中会话后，核对旧候选PID1141/1074并停止；只替换候选IME，控制中心包未改，更新匹配配对并重新注册候选包。日常包未动。

启动验收时CUA明确报告Mac已锁定且无法解锁，未绕过锁屏或使用其它工具启动GUI。当前只证明64文件已安装，不证明64进程已运行；测试源临时切离至微信输入法。需用户解锁后恢复Inputia(Test)、核验实际运行签名并进行新录音上屏测试。

恢复方式：核对并停止候选进程，保留64包后将ime-before.app复制回候选安装位置，恢复profile-before/pair-manifest.json，重新注册并重启原控制中心；不要整份覆盖旧profile以免丢失新增数据。此回滚步骤未在本轮执行。
