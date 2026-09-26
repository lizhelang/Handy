# Inputia 1.0.0 本机正式版升级

本次将中文输入法、语音输入及剪切板控制中心作为同一套 Inputia 本机正式版交付。展示版本为 1.0.0，原生组件构建号为 74。这里的正式版指当前机器的日常使用版本，不表示已经通过 Apple Developer ID 公证或可向其他机器公开分发。

## 身份与数据兼容

保留控制中心的 `com.pais.handy.UnifiedCandidate`、输入法的 `com.inputia.inputmethod.Inputia.UnifiedCandidate` 以及 `trial-20260905` 配对 profile。输入源的三种语言展示名称统一为 Inputia；设置展示名称为 Inputia设置。包物理路径仍由现有更新器控制，名称中的 Candidate 不再代表用户界面的发布状态。

保留 `~/Library/Application Support/HandyUnifiedCandidate/trial-20260905/{Handy,Inputia}` 中的语音历史、剪切板、知识库、设置与 Rime 学习数据。不搬移、不合并或覆盖旧日常版数据，不通过删除现有标记回退另一数据域。不修改系统隐私授权记录。

## 构建合同

输入法使用 `INPUTIA_RELEASE=1`，同时必须指定 `INPUTIA_UNIFIED_CANDIDATE=1`、`INPUTIA_PROFILE_RUN_ID=trial-20260905`、`INPUTIA_PAIR_BUILD_METADATA` 和非临时 `INPUTIA_CODESIGN_IDENTITY`。正式构建继续采用静态 Rime、严格运行时权限与已有配对信任；缺少这些参数时失败退出。

控制中心使用 `src-tauri/tauri.inputia-release.conf.json` 覆盖，版本为 1.0.0，产品展示名 Inputia。覆盖显式保留完整 `resources/**/*`，避免遗漏语音模型辅助程序或其他已有资源。`InputiaReleaseInfo.plist` 保留严格布尔 profile 标记，同时记录 stable 发布渠道。构建时还必须通过配置覆盖指定现有稳定签名证书，并提供 `HANDY_UNIFIED_PAIR_BUILD` 公共构建元数据；配置文件中的默认临时签名不能用于正式安装。

两端代码变化后，必须用实际最终签名包重新生成配对清单。公开元数据可以作为构建输入，私钥不能进入仓库、安装包或日志。不得只替换输入法而沿用不匹配的旧 CDHash 清单。

## 更新与验收

使用 `macos/InputiaInputMethod/update-candidate.py` 的预检和事务更新，保留稳定证书身份、维护屏障、包备份和失败回滚。更新器固定的安装路径保持兼容。若当前安装曾被临时签名覆盖，稳定身份预检可能拒绝更新；应据实际安装身份修复，不放宽验证或隐瞒失败。

应验证两个进程的运行身份匹配安装包和最终清单，配对服务可用，以及输入源展示名、Shift 双向切换、语音转写和剪切板召回。源码语法、自检和构建成功不能替代这些实际业务验收。旧 `release/full-check.sh` 检查系统级独立输入法公证安装包，不等于本次融合版本的验收。

本文件记录发布配置、安装和已观察到的验证结果；实体键盘与真实麦克风交付仍需用户实际验收。

## 2026-09-27 构建与回归证据

- 运行时测试 205 项、Tauri 后台测试 511 项通过；原生快捷键依赖 56 项通过、3 项实际系统权限测试按项目规则跳过。
- 知识库与统一历史界面 30 项通过；前端构建、Lint、翻译一致性检查通过。
- 双组件更新事务 5 项、公开配对信任 6 项通过；配对 IME 完整原生自检及固定证书签名构建通过。
- 正式包数据域自检确认仍使用 trial-20260905；剪切板原生自检使用独立 pasteboard，验证格式保留、撤销写入和较新复制保护通过。
- 包内 Laya MLX worker 实际推理返回 answers、无 error，首次请求约 1,415 ms。修复 Rust 请求缺少 kind 字段及响应等待无有效超时；首次等待另有 3 秒冷启动预算，业务期限外结果回退但保留预热进程，后续按业务期限处理。专项 4 项回归通过。
- 上述结果不等于真实麦克风转写、实体键盘 Shift/Tab 或长期输入质量验收。

离线语音引擎冒烟检查使用合成的 16 kHz 中文 WAV，已安装 Qwen3-ASR 0.6B Q8 模型通过 Metal 后端正确返回“今天下午三点开会，请保存这段测试文字。”。音频长 4.53 秒，模型加载 520 ms，转写 666 ms；未使用麦克风、未新建语音历史。此检查验证批量转写引擎，不替代真实输入框的录音交付验收。

## 实际安装结果

- 固定证书配对预检通过；最终发布目录：`~/Library/Application Support/HandyUnifiedBuilds/release-1.0.0-u6hk2exg`。
- 已恢复此前临时签名覆盖的 IME 到与旧清单完全匹配的签名备份，再执行双组件事务更新，没有放宽签名验证或改动 TCC。
- 第一次更新在替换程序前因维护回执等待退出组件而停止。修复更新器只等待仍运行 PID 的新鲜回执，仍拒绝任一运行组件缺失/过期/错误 epoch，新增回归通过后重试成功。
- 两端安装版本均为 1.0.0／74，运行映像身份验证通过；控制中心 CDHash `1fdea4986f405285b8e0aa28324146f583d62e84`，IME CDHash `b41790060addd299b13f1c44a66c0acb29855440`。
- 安装前程序及清单备份：`~/Library/Application Support/HandyUnifiedBuilds/permission-update-f9weqdyg`；临时签名版本另保存在 `release-recovery-20260927-080231`。
- 输入源恢复确认，系统展示名为 Inputia；控制中心窗口实际显示 Inputia、v1.0.0、Qwen3-ASR 0.6B、输入控制已就绪。后台与 IME permission-health 均为 ready，维护标记 active=false。
- 剪贴历史页面实际显示 6,592 条、25 收藏，采集开关已开启；升级前后 SQLite 计数均为 1,896 条语音历史、6,592 条剪贴记录。
- 已打开统一历史页面。未实施权限重授权、未开启外部 AI 连接、未执行云端发布或 Git push。
