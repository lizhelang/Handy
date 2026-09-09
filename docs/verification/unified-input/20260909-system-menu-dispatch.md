# 真正系统输入法菜单的接线修复

用户侧聊反馈两个 Inputia 项及“剪贴历史”无响应后，暂停离线音频对照的实际识别，优先处理本入口。A11 的100个音频hash已验证、隔离基准包已构建，但未启动识别，不算质量验收。

## 当前事实

系统级 /Library/Input Methods/InputiaInputMethod.app 是日常版，identifier=com.inputia.inputmethod.Inputia，版本50，PID1022；用户级 InputiaUnifiedCandidate.app 是候选，identifier 带 UnifiedCandidate，版本51，PID45507动态身份检查通过。因此不能仅凭名称断言两份都旧，也不能擅自删除日常版。

本次读取设置 clipboard_enabled=true，用户已开启采集，不能沿用此前关闭/空库状态。控制中心按钮打开浮窗的记录仍不替代系统菜单验收。

## 代码层错误与修复

本机 Apple SDK 的 IMKInputController.h 284–296 行明确：菜单 action 的 sender 是 commandDictionary，其中 kIMKCommandMenuItemName 对应所选 NSMenuItem。原 unifiedMenuAction 却把 sender 声明并直接使用为 NSMenuItem，读取 representedObject；这个接法不符合真正系统菜单的合同。

已将动作参数改为 Any?，在既有 InputiaHostTextPolicy 中解开 IMK 字典，再读取原 NSMenuItem 的固定动作元数据；同时兼容直接 AppKit 菜单 sender。空或不支持的 sender 拒绝；动作仅允许既有固定菜单类型，继续进入原来的异步认证队列，不直调输出或放宽焦点/取消/去重规则。诊断日志只记固定动作名，不含正文、路径或客户端字典。

字典/直接菜单/空/非法四类回归和既有 HostTextPolicy 自检通过。它们证明参数合同，不代替真正系统菜单点击。

## 命名与范围

已安装候选的 Hans 本地化 key 原本就存在，不能把 raw ID 简单归因为缺 key。候选包将 CFBundleName、CFBundleDisplayName、父与 Hans key 统一为明确的测试版名称，版本升至52以刷新元数据。只改 candidate 构建资源，保留现有 Hans 模式集合及所有bundle/mode ID；不增加Hant模式，不修改日常资源。

生产打包与本地化自检调用同一个资源函数，自检确认名称和模式未扩张。包内名称验证通过，真实系统菜单是否已刷新仍须单独观察。

Inputia候选构建及签名通过，日志位于 inputia-signing-20260909.XYwaC8/menu-sender-build.log；实际安装、动态身份、注册结果和菜单点击证据后续追加。CUA读取TextInputMenuAgent仍返回timeoutReached，未通过其他技术绕过或用控制中心成功结果冒充。
