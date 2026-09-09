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

## 后续原生验证：通过两个导航入口

基于 `c52fdc51` 的签名候选构建完成，产物 `src-tauri/target/release/bundle/macos/Inputia Candidate.app`。固定本地测试证书签名及 deep/strict 校验通过；未 notarize，不是正式发布版本。

已更新 `/Applications/Inputia Candidate.app`，实际运行 PID 668 经 `install-check.sh --running-identity` 验证匹配 CDHash `c445ac89b924bc710e639b4f3e9dc9cf2a5aeedd`。配对清单重新签署并替换。候选输入法 PID 88734 未替换；日常 Handy PID 836、日常输入法 PID 1022 保持运行。

真实 CUA 操作路径及结果（本任务工具记录可回查）：

1. 打开已安装候选「通用」，原生 AX 显示 Inputia 权限区及两个按钮。
2. 点击「定位输入法组件」：Finder 的 Input Methods 窗口选中 `file:///Users/lzl/Library/Input%20Methods/InputiaUnifiedCandidate.app/`，没有启动组件或选中日常版。
3. 点击「打开权限设置」：系统设置显示「设备控制和数据访问」，Inputia Candidate.app 与 InputiaUnifiedCandidate.app 均为 on；没有操作开关。

备份目录：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission-navigation-20260909.07N3U4`，保留旧控制中心（Inputia-control-before.app / Inputia-installed-before.app）及 pair-before.json、新 pair-new.json。恢复时停止且验证候选进程，恢复旧候选包与旧配对再启动；保留最新 profile 数据，不覆盖回旧数据快照。此次未迁移数据库。

仅权限导航子项原生通过；统一语音正常插入、转写中焦点变化，以及整体安装/恢复验收仍未因此完成。
