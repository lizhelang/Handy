# X Web App 确认组合时误发送：浏览器边界调查

## 当前结论

用户明确确认 Inputia 1.1.26/build114 在 X Web App 的 Grok 输入框仍会把确认英文的
回车当作发送。当前版本未通过该场景验收，不继续以编译或 Host 单元回归冒充解决。

本机 X 来自 Safari 的 Web App 模板，页面位于 `x.com/i/grok`，并非独立 Chromium 客户端。
只读查看 X 设置后确认没有可用扩展，检查结束已关闭设置并回到原页面。

## 上游证据及限制

- [WebKit 324380](https://bugs.webkit.org/show_bug.cgi?id=324380) 直接记录 Safari
  “添加到程序坞”Web App 的组合事件排序问题：确认回车可在 compositionend 之后才交付
  keydown，使网页看到 `isComposing=false`。截至 2026-10-21 状态仍为 NEW。
- 该记录的 Messenger 现场评论提到 keydown 为 Enter/229/false，keyup 为 Enter/13，
  并报告完整物理按键序列的站点保护有效。这是相邻应用的证据，尚非本机 X 的事件日志。
- [WebCore EventHandler.cpp](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/page/EventHandler.cpp)
  在输入法已处理时仍可将 keyCode 改为 229 并向 DOM 派发 keydown；IMK 的 consumed=true
  不意味着网页脚本收不到事件。KeyUp 还存在先派发 DOM 的路径。
- [WebKit 311717](https://bugs.webkit.org/show_bug.cgi?id=311717) 的修正曾被回退，不能承诺
  仅升级或切换 Safari 功能开关即可解决 Web App。

Inputia 还存在需区分的按键序列：Return keyUp、按住重复 keyDown、宿主预先
commitComposition、漏收 Shift 松开后的补偿。当前没有 X 的实际事件证据证明命中哪条，
因此本轮不增加全局吞回车或基于时间窗口的原生补丁，不重建安装新的 Inputia。

## 待审阅的站点保护

已准备仅在 `https://x.com/i/grok` 及其子路径的可编辑输入中启用的
[userscript](../../scripts/browser-compat/x-ime-enter-guard.user.js)。
识别组合确认的 Enter 后，仅阻断其 keydown/keypress/keyup 向网页监听器传播，
不调用 preventDefault，不阻止原生上屏；下一次普通 Enter、其他字段和修饰快捷键应保持。
脚本不读取聊天正文，不使用网络、Cookie、剪贴板或持久存储。

该脚本需要单独的浏览器扩展。拟使用 [Userscripts 官方项目](https://github.com/quoid/userscripts)
及其 [App Store 页面](https://apps.apple.com/app/userscripts/id1463298887)。站点权限只申请 x.com，
不能将安装脚本候选视为已获授权新增浏览器访问权限。当前尚未安装或启用；已向用户
询问是否授权安装官方 Userscripts 并仅授予 x.com 访问权限，等待明确回复。

## 候选验证

`node --test scripts/tests/test-x-ime-enter-guard.mjs` 的 20 个场景全部通过，直接执行
真实脚本，DOM 与事件仅使用本地替身。覆盖旧/新组合事件顺序、229 标记、完整松键、
下一次独立回车、长按、漏收松键、修饰快捷键、焦点或目标改变、非编辑节点、
非 Grok 路径和其他域、重复注入。访问正文或调用 preventDefault 会使测试失败。
格式检查及补丁检查通过。

本次限定代码检查确认脚本没有网络、Cookie、持久存储或正文读取路径，仅保存节点身份
及当前组合/回车状态。它依靠 document-start 捕获监听先于网页处理，仍需确认扩展在
Safari Web App 中的实际加载与原输入场景；模拟回归不构成 X 已修复的证据。

## 原生修复候选（2026-10-22，1.1.29/build117）

用户脚本需要另装 Safari 扩展并授予站点权限，尚未获得安装授权。改为在输入法内处理，
范围限定为 bundle ID 以 `com.apple.Safari.WebApp.` 开头的宿主（Safari“添加到程序坞”
的网页应用），Safari 浏览器本身和其他应用不变。

- 有组合时的回车 keyDown 只吞下并保留组合，等同一回车的 keyUp 再上屏。宿主处理完
  keyDown 才会把 keyUp 交给输入法，网页收到 keydown 时组合仍在进行，按 WebKit 的
  行为应得到 `isComposing=true`；上屏发生在 keyUp 处理期间，随后 keyup 照旧交给网页。
- keyUp 之前到来的任何事件、`commitComposition` 与 `deactivateServer` 都先同步补完上屏，
  不丢字、不改变顺序。按住重复的回车不进入延后。不使用计时窗口。
- `return-key-commit-self-check.py` 增至 62 项（新增网页应用延后、补完幂等、Shift 英文、
  按住重复及补完路径结构断言）；宿主策略自检新增网页应用与 Safari/Chrome/无标识对照；
  46 项普通物理输入、75 项候选恢复及联想、候选面板回归通过。

这是依据 WebKit 324380 排序现象推断的修复，无法在本机自动化中用实体按键验证，
也不会代替用户向 Grok 发送测试消息；是否解决须由用户在原 X Web App 的 Grok 输入框
实际确认。上节用户脚本保留为备选。

## 1.1.29 实测与 1.1.30 调整

用户在原 X Web App 的 Grok 输入框实测 1.1.29：第一次回车没有反应、候选不消失，第二次
回车直接发送。说明该宿主不把回车 keyUp 交给输入法，挂起的上屏一直等到下一次按键；
第二次回车先补完上屏，随即作为无组合回车交给网页，于是被当作发送。用户也指出问题在于
“候选在第一次回车后不消失”。

1.1.30/build118 改为吞下 keyDown 后约 50 毫秒由输入法自行上屏（`deferredReturnCommitDelay`），
不再依赖 keyUp；期间的新事件、提交组合与停用仍先同步补完，代次确保旧延时只补完自己那一次。
延时只决定上屏时机，不决定回车是否被吞。回车自检增至 74 项，新增“无 keyUp 也在延时后只
上屏一次”“自动上屏后下一次回车交给宿主”“连续延后各上屏一次且顺序不变”。
