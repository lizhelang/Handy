# 统一历史动作与组件检查点

> 当前状态请以 [docs/CURRENT_STATUS.md](../../CURRENT_STATUS.md) 和本页末尾的 2026-09-24 修订为准；本页前半段保留组件接线阶段的历史证据。

时间：2026-09-05。范围：P1/P2 分项；主导航/浮窗容器、真实插入和完整验收尚未接通。

## 后台动作

- `SourceOutbox::update_record` 使用源事务中的操作 ID 回执和预期 revision；重复修改返回原结果，过时修改拒绝，不覆盖新版本。
- `HistoryService::update_item` 路由到对应源连接，等待索引消费到该修订后才确认 UI 更新。
- 源 outbox schema 升到 3：语音置顶和显式文本覆盖存于自己的注解表。空文本修订不会回退显示原始转写；原始转写列保留，重转写可取代旧覆盖。
- DELETE 触发器在源事务内删除注解，覆盖手动删除和自动清理，不遗留完整修订正文。
- Handy 注册 copy/update/asset 三个统一接口；附件路径 canonicalize 后必须位于受管录音或图片目录，调用方不能提交任意路径。
- 统一文件复制使用 strict 原生文件入口；无效路径、空文件集合和原生写入失败直接返回失败，不使用旧路径文本回退。旧 legacy 入口暂时保留，最终浮窗切换必须走统一严格入口。
- HTML/RTF 完整原生复制及多格式表示仍需接入；现统一接口明确报不支持，不能算作 A06 完成。

## 统一组件

`UnifiedHistory.tsx` 与 `unifiedHistoryStore.ts` 调用真实生成的查询/修订/刷新接口并订阅更新事件；提供搜索、来源/类型/收藏筛选、分页、预览、修订及编辑。

在该阶段，复制/插入/更新/附件解析经必需 typed callbacks 接入；组件不自行构造本地附件路径或把文件变成文本。当时容器与真实插入 callback 尚未接入 App/Sidebar，因此那份记录不代表当前日常可用状态。

### 当前状态修订（2026-09-24）

上面的段落是组件接线阶段的历史记录。现在 `App/Sidebar` 已接入统一历史页面，真实候选安装已验证语音录音和剪贴板图片预览；完整迁移和附件校验见 [2026-09-24 迁移记录](../2026-09-24-handy-history-migration.md)。

非成功输出反馈区分 pending_target、uncertain、dispatched、rejected、failed。插入 promise 丢失也按 uncertain 处理，阻止自动重试；用户检查旧结果并明确允许后才可发起另一操作。最后一个组件卸载或选中项消失时清空旧修订缓存，迟到请求不写回。

## 新鲜验证

- `source_mutation` 3 项通过：文本修订、重试/冲突、文件不可文本编辑、空语音修订及删除注解。
- `history_service` 2 项通过；其中覆盖收藏/置顶/命名、重复操作返回同 revision、过时修改失败、正文版本保留、重启。
- `source_outbox` 10 项通过（schema 升级后的基础回归）。
- Handy lib 定向 `unified_files_never_fall_back_to_text_on_invalid_payload_or_native_error` 1 项通过；该测试使用文件写入错误注入，不触系统剪贴板，不证明原生格式实际粘贴成功。
- Runtime Clippy `--all-targets -- -D warnings` 通过；Handy cargo check 与 debug-only 绑定导出成功。
- `bun run test:playwright tests/unified-history.spec.ts --reporter=line --workers=1`：主代理复跑 13 passed（5.7s）。真实 React 组件搭配 mock Tauri/动作回调，仅证明界面交互，绝不能替代原生 IMK/系统粘贴证据。
- 前端 build、lint、翻译键一致性通过。新中文文案已补充；多数其他语言暂使用英文回退，不能宣传人工本地化质量验收完成。

浏览器截图：[合成组件预览](./history-component-browser.png)。主代理已查看截图，确认列表/预览和动作区无明显遮挡。数据为合成 Voice fixture/report.pdf。

## 独立审查

审查报告定位两个问题：注解正文删除残留（P1）和文件复制静默降为文本（P2）。两处已实现修复及回归。最终集成发布时仍须对最终提交进行完整独立审查，并复核实际原生行为。

## 后续

接上实际目标捕获、持久输出账本、唯一执行者和容器 callbacks 后，再切换主历史/浮窗入口。随后执行带真实后台的 UI 流程与三应用原生测试，不能用本报告的 mock 交互结果填满 A01–A12。
