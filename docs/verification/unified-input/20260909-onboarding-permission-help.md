# 被权限页阻挡时也能定位输入法

接续 `317aa01d`。实际入口缺口：通用设置已有 InputiaPermissionHelp，但首次权限门槛未通过时 App 只显示 AccessibilityOnboarding，用户仍无法进入通用设置找组件。

现在 macOS 授权页直接复用同一个 InputiaPermissionHelp，不新增导航命令、不另建授权流程。长页面可垂直滚动，避免600px高窗口中帮助区不可达。

`tests/inputia-permissions.spec.ts` 新增800×600场景：两项权限均false时点击定位和设置入口，导航命令调用两次，没有 request_accessibility_permission / request_microphone_permission，onComplete仍false，重新检查按钮可用。与现有权限及组件测试共7项通过；日志 `/tmp/inputia-onboarding-help-tests.log`。前端build与lint通过。

这是组件接线与门槛回归证据，不是原生首次启动验收；没有关闭用户权限、重置TCC或迁移用户数据。当前安装候选仍为代码 `322df3b1`，本前端改动尚未打入候选。语音实测确认未收到，未启动麦克风；完整goal仍未完成。
