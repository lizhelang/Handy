# Inputia 测试版使用与恢复说明

> 当前状态更新：完整的 Handy 历史迁移已经在 2026-09-24 完成。请先看 [当前开发状态](../../CURRENT_STATUS.md) 和 [迁移验证记录](../../verification/2026-09-24-handy-history-migration.md)。本页下面的旧验证边界保留为当时记录。

本说明适用于本机已并存安装的候选版本，不是正式发布或完整验收声明。

## 入口

- 控制中心：`/Applications/Inputia Candidate.app`，用于设置、语音和历史管理。
- 系统输入源：`Inputia (Test)`，输入法组件位于 `/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app`。
- 两者属于同一个 Inputia 产品。macOS 要求输入法独立注册，因此系统权限页会分别列出两个组件；无需寻找 Utility。
- 「历史记录」是语音/统一历史；「剪贴历史」保留原有快速召回浮窗，点击该菜单只应打开浮窗。

## 找回权限入口

打开控制中心 → 通用 → Inputia 权限：

1. 「定位输入法组件」会在 Finder 中选中对应候选组件，不会打开日常版。
2. 「打开权限设置」进入系统设置的隐私权限页。本机系统名称为「设备控制和数据访问」，其他版本可能称「辅助功能」。
3. 如系统要求，请由用户亲自开启对应条目；按钮不会自动授权。麦克风权限独立管理。

首次授权页也提供同组导航入口的代码已经完成；是否已在本机安装，请以 `20260909-onboarding-permission-help.md` 的最新安装记录为准。不要仅凭按钮点击成功判断语音插入就绪。

## 当前验证边界

- 已有真实证据：权限导航、原生剪贴历史菜单打开、控制中心退出后全拼 ni 候选及空格上屏。
- 仍需验证：最新修复版本的真实语音正常插入、焦点变化后待插入、完整跨应用/恢复/性能与热词接线。
- 已知缺陷：旧导入会重复增加学习次数；兼容回滚尚不能证明保留新增数据和最新遗忘屏障。
- 测试包使用专用本地签名，尚未 Apple notarization。不要作为已验收正式版本分发。

## 数据与恢复

候选数据独立保存在 `/Users/lzl/Library/Application Support/HandyUnifiedCandidate/trial-20260905`。不手动删除或把该目录覆盖到日常数据。

每次替换候选都会在 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds` 的专用批次目录保留旧包和配对清单；具体路径见对应安装记录。恢复必须匹配控制中心、输入法和签名配对，不能只复制任意旧应用包。优先保留最新数据，不能通过恢复过期数据库快照宣称兼容回滚成功。

旧 `/Applications/Handy.app` 和旧 `com.pais.handy` 数据域已从原位置移入迁移备份；当前使用 `/Applications/Inputia Candidate.app` 和候选输入法组件。源码目录不属于应用卸载范围。永久清除迁移备份仍未执行。
