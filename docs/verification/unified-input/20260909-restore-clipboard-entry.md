# 恢复原剪贴历史入口

2026-09-09 用户用日常版和测试版截图指出：原“历史记录”被改名为“剪贴历史”，真正的原剪贴板工具却没有出现。本条取代前次把所有历史入口同名的处理；不能用重命名代替功能接入。

## 已确认原因

Sidebar 中原 ClipboardSettings 页面仍存在，却由 clipboard_enabled 控制整个入口可见性。候选设置 clipboard_enabled=false、clipboard_hotkey_enabled=false，候选 clipboard_history 表 count=0。关闭采集不是删除工具的理由，也不意味着可以自动开启采集或把日常数据迁过来。

## 本批实际修改

- “历史记录”恢复作为历史管理页名称，保留语音及共享来源筛选。
- 原“剪贴历史”页面始终可达，仍是原 ClipboardSettings，不拿共享历史管理页替代。保留列表/网格、搜索、预览、收藏、置顶、复制、清空和图片/文件展示。
- 原页面显式显示现有采集开关，关闭时提示只暂停采集，查看页面不启动采集、不删除已有记录。
- 添加打开实际快捷浮窗的按钮。后端只把 show_clipboard_overlay 接到既有 overlay 函数，不实现新输出路径；打开浮窗不等于原输入框有效，插入仍遵守既有目标校验。
- 浮窗保留“剪贴历史”名称，语音状态改回“已保存到历史记录”。用户日常数据库没有迁入候选。

前端回归先证明入口隐藏/浮窗按钮缺失，再验证恢复后采集关闭也有两个区分入口；挂载不调用 change_clipboard_enabled_setting，点击按钮才调用一次 show_clipboard_overlay。完整回归中的分页测试因历史列表可访问名称变化找不到元素，实际快照中的列表和数据仍在；只同步了定位名称，40/45条分页与去重断言未放宽。

## 验证记录

前端与原生构建日志在 /Users/lzl/Library/Application Support/HandyUnifiedBuilds/inputia-signing-20260909.XYwaC8。源码/浏览器测试不能替代实际候选的两个入口和浮窗证据；原生安装与运行结果在后续追加。

50 项完整前端回归通过，lint、翻译键一致性、Rust fmt 与相关前端格式检查通过。独立只读审查未发现本批入口/采集隐私/浮窗接线阻塞。控制中心与输入法候选均已使用专用本地身份构建、严格签名验证；控制中心包含 Qwen helper 的签名也通过检查，隔离 profile 自检未打开日常数据。原生操作验收仍未因构建通过而成立。

本批不宣称完整共享输出、跨应用语音、图片文件插入、富文本原格式、迁移回滚及 P0–P6/A01–A12 全部完成。

## 安装后的实际验证（北京时间 2026-09-09 05:03–05:06）

实现提交 5520bf7d。只更新 /Applications/Inputia Candidate.app 和用户级 InputiaUnifiedCandidate.app，旧包与候选数据保存于上述构建根的 control-center-before-fixed.app、inputia-before-fixed.app、profile-before-fixed；三个数据库 quick_check 均为 ok。日常包和数据未修改，未从备份覆盖现用数据库。

真实控制中心先显示新权限页，随后进入通用页。工具读取到两个实际 sidebar 入口：“历史记录”和“剪贴历史”。点击后者，原 ClipboardSettings 原生截图显示：0 条、0 B、搜索、来源筛选、排序、列表/网格切换、采集开关 off 与暂停提示；不是共享语音历史页换名。

第一次操作浮窗按钮时界面已变化，工具拒绝失效编号，未继续按旧编号操作；刷新完整原生状态后重新进入“剪贴历史”，点击实际“打开快捷浮窗”按钮。真实 NSPanel（Clipboard）出现，截图/AX 显示 Ropy 风格快捷召回、文本/图片/文件过滤、收藏、编辑标题及语音来源标识。此证据来自安装后的真实 UI 控件，不是 mock 页面、直接底层 invoke 或注入最终文字。日志 21:05:55 UTC 为 Clipboard overlay window shown。

验证后设置文件仍 clipboard_enabled=false、clipboard_hotkey_enabled=false。此次没有开启真实剪贴板监听，没有复制/插入用户正文或更改真实文稿。因为候选复制库为空，图片/文件实际采集与复制输出尚未验证；Inputia 系统菜单打开路径也不以控制中心按钮路径代替验收。浮窗保留打开供用户查看。
