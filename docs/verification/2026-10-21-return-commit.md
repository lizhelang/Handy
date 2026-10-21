# 回车上屏不触发宿主发送

## 用户问题与定位

用户反馈在部分输入框中，按回车原本只想把正在组合的英文直接上屏，却同时触发发送。
排查时本机主应用与输入法均为 1.1.25/build113，安装代码来源为 `bbf53187`。

原生 Host 的 Return / 数字键盘 Enter 分支先调用 `bridge.enter()` 上屏，随后判断
“无引擎候选、提交内容等于原组合串、全部为可打印 ASCII”，并在满足时返回 false。
这会把同一次回车继续交给宿主应用，聊天框可以因此执行发送。
历史 v37 曾为表单一次回车提交专门加入该例外；本次用户要求改为先确认组合输入。

Rust 的中文 Enter 已在有组合串时返回 `consumed=true`，空组合才返回 false，
CAPI 保留该结果。本次修正 Host 的消费结果，不改变引擎和上屏内容。

## 修复边界

- 有组合串时，Return / 数字键盘 Enter 完成原文上屏并消费这次按键。
- 上屏后组合清空，下一次独立回车继续交给宿主处理。
- 无组合串、英文直输模式和原有修饰快捷键仍沿用宿主行为。
- Shift 英文组合的回车上屏，以及空输入联想不拦截回车的规则保持。

## 验证与交付状态

- 新增真实 Host Return 分支和 `apply` / `insertCommittedText` 回归，50 项通过。
  覆盖两个回车键、无候选 ASCII、有原生候选、非 ASCII、一次插入及第二次回车透传，
  同时检查英文直输、修饰快捷键、Shift 英文组合、空输入联想和引擎失败。
- 同一回归针对 `3b10add3` 旧源码成功编译运行，在两种 ASCII 场景及两个回车键上
  共 4 项消费断言失败；新源码为 50/50。失败来自运行行为而非接口或编译差异。
- Rust core 的两项 Enter 定向测试通过；本次没有修改 Rust 源码。
- 现有 Host 文本策略自检和 46 项普通物理输入优先级回归通过。
- 新 Host 回归已接入成套构建门槛。

- 独立只读审查未发现需要修改的正确性或安全问题。
- 完整成套构建及新增回归通过；安装前配对校验和更新后运行身份校验通过。
  首次构建因既有 `node_modules` 共享链接失效而缺少 Tauri CLI，随后在本工作树按
  `bun.lock` 执行 `bun install --frozen-lockfile` 后完成构建，未改变依赖版本。

## 本机安装与实际试用边界

已安装 **1.1.26/build114**，主程序与输入法代码来源均为
`340758b93652018449a2d6822c91f080e2a76985`，分支 `codex/inputia-return-commit`。
安装后新鲜状态中两组件均 ready，后台准入开启；当前输入源恢复为
`com.inputia.inputmethod.Inputia.UnifiedCandidate.Hans`，TCC 未变，未重放录音。

数据核对：10 条语音历史和原有 8467 条剪贴记录内容均保留，更新期间新增 7 条剪贴记录；
两份设置及 11 个 WAV 字节一致，5 个数据库完整性检查通过。词库原有 40 个文件均保留，
38 个字节一致，用户词典日志追加，`user.yaml` 仅更新双拼方案访问时间。

自动化在新建 TextEdit 文稿输入 `abc` 时未观察到待确认的组合状态，未执行目标回车验证，
因此不能据此声称完成真实输入框验收。测试文稿已保存在本轮私有验证目录并关闭，
未向聊天对象发送测试内容。

用户随后明确回复“仍然同时发送”，并提供 X Web App 的 Grok 截图，已上屏的
`nihk` 被当作消息发送。因此 **1.1.26 没有通过该原场景验收**，之前的 Host 分支
回归及安装成功只能证明已修掉显式透传例外，不能证明网页侧不会再次处理回车。

只读核实 `/Users/lzl/Applications/X.app` 使用 Safari Web App 模板，标识为
`com.apple.Safari.WebApp.7DAA0D15-0997-433C-9820-ED5C74751C09`，manifest 为
`https://x.com/manifest.json`。X 的扩展设置当前为空，未安装脚本扩展、授予站点权限
或通过自动化向 Grok 发送测试消息。后续浏览器边界调查见
[X Web App 回车保护候选](2026-10-21-x-webapp-return.md)。

制品目录：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/release-hflTpfXU`。
旧程序备份：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission-update-8z9hmebm`。
构建、旧版失败对照、安装日志与数据备份：
`/Users/lzl/Library/Application Support/InputiaBuilds/return-commit-20261021`。
本次仅本机更新，未推送或公开发布。
