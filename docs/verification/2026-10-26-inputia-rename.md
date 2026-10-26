# 产品与仓库彻底改名为 Inputia

日期：2026-10-26。分支 `codex/inputia-rename`（基于已安装 1.1.35 的 `codex/prediction-ngram`），工作树 `/Users/lzl/FILE/github/Inputia-rename`。最终安装 1.1.39/build127。

## 目标与边界

用户要求："按这三个阶段彻底改名"。三个阶段：

1. 代码内部改名，不碰数据与身份，装 1.1.36。
2. 控制中心程序标识与数据目录迁移（风险最大），装 1.1.37；控制中心权限需用户在系统设置中重新授予。
3. GitHub 仓库、远程地址与本机文件夹改名。

任何阶段都不删除、不修改历史原文；迁移只复制（APFS 克隆）与移动目录，旧数据目录整体保留为备份。

## 第一阶段：代码内部

提交 `885449be`，版本 `88cab959`（1.1.36/build124）。

- 运行时组件 `inputia-handy-runtime` 改名 `inputia-runtime`；主程序包、库与可执行文件由 `handy` 改为 `inputia`（进程名随之改变），日志文件改为 `inputia.log`，Qwen 辅助程序改为 `inputia-qwen-custom-words`。
- 环境变量统一为 `INPUTIA_*`，运行时仍兼容读取改名前的 `HANDY_*`（新增测试）。
- 代码里把控制中心称作 Handy 的类型、函数与字段改为 Control（如 `InputiaControlMemorySync`、`ControlDataPaths`、`control_root`）；配对角色在代码中改称 Control，线上协议值（数字 1 与字符串 `"handy"`）不变。
- 注释、日志、构建流程、Nix 打包与构建说明改为 Inputia；Linux 私有库目录随产品名改为 `/usr/lib/Inputia`。
- 测试：运行时组件、设置、核心、更新器、发布组件全部通过；主应用 705 项通过，另有 1 项原有失败（`hf_download_configuration` 检查的常量在改名前已被移除）；`inputia-capi` 12 项失败与改名前完全相同（需本机 Rime 词库数据）。
- 安装：用改名前的安装脚本装 1.1.36（程序标识与数据目录未变），预检与安装一次成功，控制中心进程名变为 `inputia`，两端就绪，历史原文指纹（语音 2043 条、剪贴 9009 条、键入 1473 段、录音 2044 个、剪贴图片 2982 个）与学习数据前后一致。证据目录 `~/Library/Application Support/InputiaBuilds/rename-phase1-20261026`。

## 第二阶段：程序标识与数据目录

提交 `47e9931b`（改名）、`877fcbf7`（改写数据中的旧绝对路径）、`e1bc1102`（验签工具）、`e97b0181`（迁移账本重定位）、`76df940a`（附件按内容重新绑定），版本 `62007841`（1.1.37/build125）、`4925826c`（1.1.38/build126）与 `3490abd9`（1.1.39/build127）。

### 改动

- 控制中心程序标识由 `com.pais.handy` 改为 `com.inputia.control`，本机候选安装为 `com.inputia.control.UnifiedCandidate`；Info.plist 候选标记改为 `InputiaDevelopmentCandidate` / `InputiaProfileRunID`。
- 候选数据目录由 `HandyUnifiedCandidate/<运行编号>/{Handy,Inputia}` 改为 `InputiaUnifiedCandidate/<运行编号>/{Control,Inputia}`；安装收据新增 `candidate` 数据类型，旧收据的 `legacy_candidate` 仍可读取。
- 更新器新增 `--migrate-rename`：先把旧目录用 APFS 克隆复制到临时位置并逐项核对（目录、文件、字节数），改写数据中记录的旧绝对路径（知识库来源表中的托管路径），再原子换入新位置；旧目录整体移入本次更新的备份目录，任何一步失败都撤回。程序标识变化时输出 `controlPermissionsNeedRegrant=true`。
- 输入法首次读取设置时，若存储的 Rime 用户目录与记忆库路径正好是同一运行编号的旧位置，就通过版本化设置文档的 CAS 补丁改为新位置；只用于校验的 `integration_*` 扩展键保留旧值并按等价旧位置接受。
- 语音启动器只按新程序标识查找控制中心，不再查找旧的 Handy.app。

### 1.1.37 安装与启动故障

- 预检 `updatePreflight=true permissionRecordsUnchanged=true`；`--migrate-rename --apply` 一次成功：迁移 141 个目录、19549 个文件、33,417,016,597 字节，改写路径引用 1 处，`tccChanged=false`、`previousRecordingsReplayed=false`。旧数据目录整体保留在 `InputiaUnifiedBuilds/permission-update-y2ewtier/data-before-rename/trial-20260905`（迁移时位于 `HandyUnifiedBuilds` 下）。
- 历史原文指纹与学习数据前后一致：语音 2043 条 / 165883 字 / 最大编号 4172，剪贴 9009 条 / 1750971 字，键入 1473 段 / 5494 字，录音 2044 个，剪贴图片 2982 个，学习事件 3897 条，历史词对 71991 行，导入来源 2948 条；录音保留仍为"从不"，各库完整性 ok。
- 新控制中心启动后停在"暂时无法完成启动"恢复页，没有打开任何业务数据库。原因：设置启动账本（`control-settings-document-v2/active-startup.json`，v3）记录了迁移根绝对路径的摘要，迁移清单与完成标记也记录旧绝对路径；目录整体移动后，启动校验把它判为"复制的数据根，需要显式重定位"（`startup v3 identity mismatch`，RepairRequired）。在账本与设置文件的私有副本上复现了这一错误，副本不含任何历史数据库。

### 1.1.38 修复：显式重定位

- 候选配置声明同一运行编号改名前的位置（`HandyUnifiedCandidate/<运行编号>/{Handy,Inputia}`）。只有当前根正是候选配置的根、且旧位置已不存在（确属移动而非复制）时，迁移根才登记为重定位；此时账本里的根摘要、清单源根与完成标记里的旧路径视为当前根，其他复制或位置不符的情况仍按原规则拦截。
- 新增 3 项测试：重定位只在旧位置不存在且标签与当前位置相符时成立；移动后的完成标记能在新备份域内解析；v3 已完成账本在整体移动后不声明重定位会失败、声明后通过、旧位置重新出现时再次拒绝。主应用库测试 704 项通过，2 项需本机模型或真实数据而默认忽略。
- 修复后在同一份私有副本上重跑：不重定位仍为 `startup v3 identity mismatch`，重定位后为"无需迁移"。

### 1.1.38 安装与附件身份问题

- 第一次 `--apply` 在改动程序前停下（"维护屏障未确认"）：更新器要求每个运行中的组件确认维护，卡在恢复页的控制中心没有启动权限健康模块，无法确认。让该进程正常退出后重跑，安装一次成功（运行身份校验通过，`tccChanged=false`、`previousRecordingsReplayed=false`）。
- 新控制中心通过启动校验，正常打开语音与剪贴板数据库。新程序标识需要重新授予辅助功能等权限，授权前后台状态为 `paused/permission_required`，符合预期。
- 但控制中心持续满负荷，输入法每次语音握手都超时（服务端记录 `unified_voice_connection_rejected stage=Handshake`）。采样显示工作线程忙于附件对账：附件登记表按设备号与 inode 核对每个录音和剪贴图片，APFS 克隆迁移后内容不变但 inode 全部改变，5025 个附件（录音 2043、图片 2982）被标为 `identity_changed`，对账反复失败，握手排在它后面超过 2 秒时限。文件本身完好，登记表只是状态标记，没有删除或改动任何附件与历史。

### 1.1.39 修复：附件按内容重新绑定

- inode 不符时，只要大小与内容 SHA-256 都与登记一致，就更新登记的设备号与 inode；内容有任何差异仍按身份变化暂停，登记不被改写。新增测试覆盖"克隆后内容相同重新绑定"与"同大小不同内容仍暂停"。运行时组件全套 378 项通过，clippy（全目标、`-D warnings`）通过。
- 首次对账需要把 4.5 GB 附件（录音 1.3 GB、图片 3.2 GB）各读一遍算摘要，之后恢复为只核对 inode。

### 1.1.39 安装结果

- 预检 `updatePreflight=true permissionRecordsUnchanged=true`；`--apply` 一次成功，两个运行组件身份校验通过，`tccChanged=false`、`previousRecordingsReplayed=false`。
- 控制中心沿用用户在 1.1.38 时重新授予的权限：输入模拟、快捷键与目标均为 ready，后台与输入法健康状态都是 ready；启动后不再出现握手被拒，CPU 回落到约 2%。附件状态中 `identity_changed` 由 5025 条降为 0。
- 历史原文与学习数据核对（只比对条数、字数合计与最大编号，不读正文），与 1.1.37 迁移前完全一致：语音 2043 条 / 165883 字 / 最大编号 4172，剪贴 9009 条 / 1750971 字，键入 1473 段 / 5494 字，录音 2044 个，剪贴图片 2982 个，学习事件 3897 条，历史词对 71991 行，导入来源 2948 条，联想表登记不变；录音保留仍为"从不"，各库完整性 ok。
- 证据目录 `~/Library/Application Support/InputiaBuilds/` 下的 `rename-phase2-20261026`（1.1.37）、`rename-fix-1138-20261026`、`rename-fix-1139-20261026`，各含构建日志、安装前后快照、预检与安装日志、数据库在线备份。制品与旧程序备份在 `InputiaUnifiedBuilds` 下：`release-CJn1behJ`（1.1.37）、`release-vaMJyld9`（1.1.38）、`release-3EVOr6rZ`（1.1.39）；`permission-update-y2ewtier`（1.1.37，含迁移前完整数据 `data-before-rename`）、`permission-update-4a4gu6im`（1.1.38 第一次尝试，未改动程序）、`permission-update-4v067wkl`（1.1.38）、`permission-update-vr44vd8d`（1.1.39）。

## 第三阶段：仓库与本机文件夹

- GitHub 仓库 `lizhelang/Handy` 改名为 [`lizhelang/Inputia`](https://github.com/lizhelang/Inputia)，简介改为 Inputia 的中文介绍，清除了指向上游产品网站的主页链接。所有工作树共用的 `origin` 改为 `https://github.com/lizhelang/Inputia`，已验证可读取远程分支；`upstream` 仍为 `cjpais/Handy`。
- 本机源码目录：历史基线 `/Users/lzl/FILE/github/Handy` 改为 `/Users/lzl/FILE/github/Inputia`，原路径保留为指向它的链接，供仍记着旧路径的会话与工具使用。工作树用 `git worktree move` 改名：`Handy-inputia-v44` → `Inputia-v44`、`Handy-prediction-ngram` → `Inputia-prediction-ngram`、`Handy-unified-input-compat` → `Inputia-unified-input-compat`、`Handy-unified-input-system` → `Inputia-unified-input-system`、`Handy-upstream-reintegration` → `Inputia-upstream-reintegration`；交接备份 `Handy-handoff-backup-1783435040` 改为 `Inputia-handoff-backup-1783435040`。各工作树的 `.git` 指向改为新主目录，未提交改动原样保留（历史基线 49 个文件、`Inputia-unified-input-system` 17 个）。
- `~/Library/Application Support/HandyUnifiedBuilds` 整体改名为 `InputiaUnifiedBuilds`（制品、配对签名输入、各次更新备份与迁移前完整数据），其中的 `g0-delivery` 工作树已修复指向。改名前留下的未使用候选运行 `HandyUnifiedCandidate/a11-20260909`（1.6 GB）移入 `InputiaUnifiedBuilds/archive/HandyUnifiedCandidate-a11-20260909`，随后移除空的 `HandyUnifiedCandidate` 目录。
- 构建脚本、更新器与本机预览打包脚本改用 `InputiaUnifiedBuilds`；README、AGENTS.md、现状说明、发布说明与 0.10.0 版本说明中的仓库地址、数据目录与工作树路径改为新值。静态 Rime 产物路径随工作树改为 `/Users/lzl/FILE/github/Inputia-unified-input-system/native/static-rime/artifacts/output/arm64`。

## 未处理的旧名字残留

以下不属于当前应用的运行依赖，保持原样，是否清理由用户决定：

- `~/Library` 中按旧程序标识或旧进程名生成的系统数据：`Application Support/com.pais.handy`（37 MB，早期开发运行留下的日用数据目录，含剪贴库）、`Caches/handy`、`Preferences/com.pais.handy.UnifiedCandidate.plist`、`Preferences/handy.plist`，以及 `WebKit` 下的 `HandyRawCopy`、`com.pais.handy`、`com.pais.handy.UnifiedCandidate`、`handy`、`handy-sign-test`。
- 系统设置"隐私与安全性"中旧程序标识的授权条目；授权属于用户的安全设置，未读取也未修改。
- Codex 配置中旧路径 `/Users/lzl/FILE/github/Handy` 的项目信任与编辑器条目，以及 Codex 管理的工作树 `~/.codex/worktrees/inputia-player-duration/Handy`（只更新了指向，未改名）。
- Git 内部的工作树管理目录名（`.git/worktrees/Handy-*`），只是内部标签。
- 输入法设置中 4 个只用于核对的 `integration_*` 路径仍是改名前的值；运行时按当前数据域计算，输入法把同一运行编号的旧位置视为等价。

## 仍保留旧名字的地方及原因

- 第三方键盘库 handy-keys 及其适配代码（库名）。
- 上游模型下载地址 `blob.handy.computer`、上游仓库与讨论链接、许可证与上游贡献文档。
- 已写入用户数据或签名格式的标识：迁移账本标签 `"handy"` 与备份子目录、哈希与签名域字符串（如 `Handy-Inputia-Candidate-Pair-Manifest-v1`、`handy-owned-voice-result-v1`、`handy-unified-candidate-webview-v1`）、配对协议角色值 `"handy"`（代码中称 Control）。
- 读取改名前安装的兼容路径：旧收据 `legacy_candidate` 布局与旧控制中心程序标识的识别（只用于一次性迁移）。
- 固定基准测试集中作为识别词的 "Handy"。
- 历史验证记录与 Git 提交历史。
- 剪贴记录与条目快照中自带的来源应用标识 `com.pais.handy`（属于历史内容）。
