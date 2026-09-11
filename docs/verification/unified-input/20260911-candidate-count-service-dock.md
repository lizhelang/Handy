# 候选数量、服务准备与 Dock 修复

用户授权：2026-09-11「请你全修复了」，并追加 Dock 空白图标问题。范围为这次已定位缺陷；不清空用户词频、重置权限或替换日常安装。

## 机制与修复

- 部分选词：core原来直接保留Rime选择回执自带的5项，没按用户7/8设置补足。现在仅在需要时复用ensure_candidates_for_page(0)，保留剩余组合与原候选选择语义。单测先红（5 != 7），修复后通过。
- 词序：“箱”存在，截图第二行第二列；隔离默认词库首7为想/向/像/象/相/香/项。不得伪造“箱必定首7”的验收条件。真实静态Rime测试nihkxd→你好→xd，7/8数量正确，并可翻页原索引选择箱。
- 服务准备：启用输入法时异步检查已签名配对服务，限定安装位置、拒绝符号链接与身份不匹配，仅--start-hidden启动，不发送录音命令、不注册另一热键、不重放旧操作。每Host进程最多自动启动一次；明确退出或观察到结束后抑制，避免与用户退出意图对抗。
- Dock：独立托盘已被移除，隐藏服务却仍要求“有独立tray”才Accessory；移除此过时条件。IME提示框临时Regular后恢复原policy。主控制中心显式打开仍Regular，有正常Dock入口。

## 已跑验证

- core sqlite-memory完整61项通过。
- 实际bundled-static-rime定向回归通过，独立临时用户词库，不允许skip。7/8页尺寸及部分消费/翻页选择均有断言。
- Swift paired类型检查和原启动自检通过，覆盖一次启动、明确退出/观察结束抑制、限定安装路径。
- 控制中心Rust库487通过、2忽略。前端由候选构建执行tsc/vite验证。
- 独立审查未发现HIGH/CRITICAL；提出活跃语音连接白名单缺quit_service，已补上且自检断言退出允许、会话中改模型仍拒绝。该自检只证明分派准入，不替代实际菜单退出。

## 原生验收边界

构建、安装、运行身份和实际观察结果随后追加。以上测试不能替代原生快捷键、Dock截图或最终语音闭环；截图空白位具体进程尚未被Dock工具识别（读取超时），不声称已证明唯一根因。

## 配套63实际安装和冷启动

- 功能提交 `b40f2022`，IME版本63。签名构建 `/tmp/inputia-ime63-build.log` 与 `/tmp/inputia-control63-build.log` 成功；未公证，未正式发布。
- 备份根 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/repair63-20260911.IAgY00`：完整profile-before、control-before.app、ime-before.app、pair-new.json。备份history quick_check=ok；不覆盖日常安装。
- 原安装位置更新后，系统首次启动的PID42454实际仍是旧62 CDHash `fbbdadf1d95ad1389dbba8db2d9ede353a11f8ad`，不是新包。已核对其与备份身份匹配、切离测试源、停止该PID，并仅重新注册候选包；不能仅凭ps路径或磁盘版本判断升级成功。
- 正确新IME PID43479，运行身份验证通过，CDHash `9ab521306bc0b4c2e05dbfb4e354da6d0f62ce97`。
- 保持控制中心关闭后，输入法启用实际启动了PID43929，命令行为 `/Applications/Inputia Candidate.app/Contents/MacOS/handy --start-hidden`。新控制中心CDHash `8e9f41ea90b60a18e6b843161d96316aa38a7b3b`，运行身份通过。
- 2026-09-11 03:55:11 UTC后台日志明确Accessory隐藏启动，03:55:12 listener ready；IME本地13:55:13记录 `target_registered field_observable=true`。本次验证经过实际启用路径，没有直接调用启动函数或录音命令。
- 未触发录音；当前候选设置always_on_microphone=false、audio_feedback=false。音频快捷键的真实开始/停止仍未验收。
- CUA新建专用空文稿（自动保存为未命名7.rtf），按键测试留下合成nihkxd文本。当前工具按键直接上屏，单Shift返回keyPressIncludedNoNonModifierKeys；虽然后续坐标点击已成功，仍未取得中文组合候选画面。不把底层测试冒充原生显示验证。
- Dock读取超时，用户已被询问空白位悬停名称；后台Accessory日志证明采用了修正策略，不证明截图特定空白图标已消失。明确菜单退出不复活只经自检，未做真实菜单退出。

恢复方式：切离测试源，核对并停止候选两PID；把当前两包移到新保留目录后，将本备份的control-before.app与ime-before.app复制到原候选安装路径，同时恢复profile-before/pair-manifest.json并重新注册候选输入法。不要覆盖整份旧profile，以免丢失安装后新增数据。此步骤已准备，未将它冒称为本轮实际回滚验收。
