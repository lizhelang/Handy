# Inputia 测试版68：快捷键无法开始录音

## 现象

选中 Inputia 后 Option+Space 无法开始录音，但已在录音时可以结束；结束后文字进历史，不自动进入输入框。

## 根因

IME 侧目标预捕获持续 `field_unobservable`（Electron/Codex 等常见），host shortcut lease 建不起来。控制中心在「当前是 Inputia 且无 lease」时返回 `HostPending`，只允许 `legacy_continuation`（能停不能开），也不会走本地粘贴插入。

## 修复

- 无 ready lease、但策略服务可用时，回退 `Legacy`：可开始/结束录音，并用本地粘贴插入。
- 有 lease 时仍优先 host 路径。
- 权限探测：已 ready 时慢探测超时不再抬升 epoch；软过期放宽到 5s 且探测中不抬升。
- 升级脚本同时认 `marker_epoch` / `maintenance_marker_epoch`，避免维护屏障误失败。

## 安装

IME/控制中心 68 已安装。运行身份：control PID49165 / IME PID49169。两端 health=`ready`。回滚：`~/Library/Application Support/HandyUnifiedBuilds/permission68-manual-install`。
