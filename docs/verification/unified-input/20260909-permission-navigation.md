# 统一权限入口接线

北京时间 2026-09-09 20:06 起，接续 `9bd456b6`，分支 `codex/unified-input-system`。

## 范围与顺序

按当前 Inputia 品牌 goal 的统一权限引导要求，解决用户删除权限条目后找不到隐藏组件的实际问题。先增加固定导航后端，再接通用设置入口、验证错误状态与候选隔离，最后构建候选；不替换日常安装、不改变授权、不读取历史正文。

## 实现

- `src-tauri/src/commands/permissions.rs`：只接受 settings/component 两种动作；固定系统隐私页地址，Finder 定位已存在的组件。候选目录缺失直接报错，不能回退日常安装。文件存在不是身份认证或权限状态证明。
- `src/components/settings/general/InputiaPermissionHelp.tsx`：定位输入法组件、打开权限设置；明确控制中心和输入法会分别列出，没有伪造的“已授权”标签。
- 复用现有 SettingsGroup/Button 与 Inputia 主题，不改输入法按键、录音和输出路径。
- 中文和英文文案完成；其他语言新增键暂以英文回退，不能称作本地化翻译完成。
- bindings 由 `--export-bindings` 自动生成并格式化；同时补出已注册但此前未导出的历史接口，不手改生成文件。

## 本轮证据

- 实际系统设置读取：Inputia Candidate.app / InputiaUnifiedCandidate.app 两项均 on；没有点击授权开关。
- Rust 路径隔离测试通过：`/tmp/inputia-permission-help-rust.log`。
- 前端交互测试通过：`/tmp/inputia-permission-help-ui.log`；包括组件缺失、错误恢复、只调用固定导航动作。初次失败原因是 i18n 的 get_app_settings 初始化调用漏列，核对源码后补入精确预期，未过滤掉未知调用。
- 浏览器组件截图：`test-results/inputia-permission-help.png`，检查文字无裁切、两个按钮清楚可辨。仅组件级显示证据，不冒充原生窗口。
- 前端 build、lint、翻译键一致性、format:check 通过。保留既有大 bundle、Rust dead_code / future-incompat 警告。
- 独立只读审查 `review_permission_navigation`：Approve，无阻塞发现；覆盖固定导航、不自动授权、候选不回退、跨平台入口保护。

## 未验收

新入口尚未在已安装候选中真实点击；构建日志 `/tmp/inputia-permission-help-candidate.log`。下一步检查构建签名，按现有候选备份/重新配对流程更新控制中心，原生点击两个入口。浏览器 mock 测试不证明系统导航成功。

本批没有推进录音→转写→原输入框闭环。仍不得把权限 on 当作该闭环通过；未经用户确认方便，不启动麦克风。
