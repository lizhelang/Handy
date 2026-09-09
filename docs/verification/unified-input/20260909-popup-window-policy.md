# 剪贴浮窗与控制中心分开显示

用户已确认从真正的 Inputia (Test) 系统菜单点击“剪贴历史”能打开浮窗，并提供双窗口截图。前一项“菜单到浮窗”因此不再记作尚无结果；本次失败点是左侧控制中心也在显示。

用户明确行为：剪贴历史只显示快捷浮窗；Inputia设置显示控制中心。复用现有窗口，不关闭/销毁控制中心，不丢设置页面状态。

## 修改

macOS show_clipboard_overlay_on_main_thread 在原目标 capture_before_ui 之后、激活应用和展示浮窗之前隐藏 main。若隐藏失败，则不继续以双窗假称成功。显示后记录原生 main_window_visible 状态，不记录正文。Inputia MenuCommand::Settings 则先隐藏剪贴浮窗，再显示控制中心。

独立审查确认当前可达IMK菜单路径无阻塞。旧Tauri独立托盘分支被 independent_tray_enabled 恒false关闭，不为此扩展不可达兼容代码。认证/取消/目标/输出规则未改；菜单契约3项回归、Rust格式检查、原生候选构建通过。

本批日志 menu-20260909.wEdagq/popup-only-build.log。实际安装和窗口验证继续追加，不以构建成功替代原生显隐结果。

## 实际更新与验证

实现36d9fca5已更新到 /Applications/Inputia Candidate.app，输入法52未替换或重启。旧控制中心、数据副本和旧配对清单保存在 popup-only-20260909.AEwVep；没有覆盖日常安装或恢复旧数据库。新配对清单已签署，固定签名身份不变。

首次CUA启动观察返回“Running application not found”，但只读检查已见PID87379运行，因此未盲目重启；随后读取同一运行应用成功。新控制中心PID87379的动态身份检查通过（7372e76bd5990e5ab408c192c2d57af1c641c0d5），输入法PID77513仍通过。

本次由真实控制中心“打开快捷浮窗”按钮执行共享show函数，原生NSPanel正常出现；05:20:26 UTC日志明确 main_window_visible=Some(false)。这直接验证共享窗口显隐实现，不冒称本轮重新操作了系统输入法菜单。此前用户已确认系统菜单能到达同一浮窗；本次Settings菜单的反向显隐路径已改并经审查，尚无新的实际点击证据。

没有点击复制、插入或删除，没有开启录音或修改采集设置；现有记录保留。完整语音会话/焦点变化目标未因这次窗口修复完成。
