# 剪贴浮窗与控制中心分开显示

用户已确认从真正的 Inputia (Test) 系统菜单点击“剪贴历史”能打开浮窗，并提供双窗口截图。前一项“菜单到浮窗”因此不再记作尚无结果；本次失败点是左侧控制中心也在显示。

用户明确行为：剪贴历史只显示快捷浮窗；Inputia设置显示控制中心。复用现有窗口，不关闭/销毁控制中心，不丢设置页面状态。

## 修改

macOS show_clipboard_overlay_on_main_thread 在原目标 capture_before_ui 之后、激活应用和展示浮窗之前隐藏 main。若隐藏失败，则不继续以双窗假称成功。显示后记录原生 main_window_visible 状态，不记录正文。Inputia MenuCommand::Settings 则先隐藏剪贴浮窗，再显示控制中心。

独立审查确认当前可达IMK菜单路径无阻塞。旧Tauri独立托盘分支被 independent_tray_enabled 恒false关闭，不为此扩展不可达兼容代码。认证/取消/目标/输出规则未改；菜单契约3项回归、Rust格式检查、原生候选构建通过。

本批日志 menu-20260909.wEdagq/popup-only-build.log。实际安装和窗口验证继续追加，不以构建成功替代原生显隐结果。
