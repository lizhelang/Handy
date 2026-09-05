# 候选隔离与 P3 接入口复核

北京时间 2026-09-05，本地输出安全检查点 `eef101e3` 后接续。这里是实施进度，不是可安装候选交付。

## 本轮实际结果

- 独立复核发现：旧 build.sh 允许安装目录作为输出，仍复制日常 bundle/TIS/连接身份，SettingsLauncher 会查找日常Host；ad-hoc签名失败仍可能返回成功。候选Profile只检查固定路径，漏掉内部Rime链接、硬链接及SQLite sidecar。
- Profile 已补递归候选树元数据审计（不读正文、不跟随链接）、普通文件硬链接及sidecar检查。启动和每次候选Rime session初始化均完整检查；普通设置路径验证仍为常数级，不扫描目录树。
- `UnifiedInputProfileSelfCheck`：90项通过；Profile/Bridge/SettingsWindow/HandyMemorySync 联合 warnings-as-errors typecheck 通过。全部使用独立Caches UUID合成目录，未运行输入法。
- build.sh 已加入明确 `INPUTIA_UNIFIED_CANDIDATE=1` 和安全 run-id，候选固定输出到本工作区 `macos/InputiaInputMethod/candidate-builds/<run-id>`。输出路径、祖先符号链接及app根链接均在删除旧构建前检查；候选独立cargo目录及锁目录，不操作安装位置。
- 只读路径预检6项实际执行：正常默认/候选通过；系统Input Methods、用户Input Methods、路径穿越及Applications覆盖请求均以退出码2拒绝。随后已在固定trial目录执行完整候选build，未操作安装目录。
- 候选plist生成已接线：Host `com.inputia.inputmethod.Inputia.UnifiedCandidate`，独立TIS parent/mode和IMK连接，Boolean标记和run-id；候选版本51/0.1.0，日常源码plist不变。SettingsLauncher仅查候选同级/用户安装路径，校验同run-id、版本和Host CDHash，不回退日常Host。SettingsLauncher联合typecheck与build脚本语法检查通过。
- 候选签名失败现在无论是否ad-hoc都失败退出；候选build不做LaunchServices注册或注销。

## 尚未验证，不能当完成

- Inputia完整Rust/Swift编译、plist生成与双包签名已通过；修复Cargo丢弃部署版本环境变量的问题后，462个静态对象及最终二进制/plist不超过macOS13.0。Handy初次候选bundle及配对路径诊断已通过，但随后身份/WebView/迁移加固还需重新构建验证。详见[当前检查点](./pairing-corpus-checkpoint.md)。
- 现有 `prepare-rime-data.sh` 仍依赖Squirrel共享资源和远程schema下载；最终可复现构建需要固定来源版本/校验和并处理librime依赖，不得把日常Squirrel存在当安装前提。
- Handy配对根与候选身份已实现，候选服务profile_id为 `unified-candidate:<run-id>`；额外加入编译identity/NSBundle双向一致门禁。WK存储macOS14+按run-id派生UUID，13使用显式非持久存储，不再误用macOS无效的data_directory。真实WK存储与旧UI未决操作重载仍须原生验收。短socket产品接线与双方完整认证尚未完成。
- 候选专用安装/禁用/恢复脚本、备份演练、必要系统安装授权及真实跨应用验证尚未进行。不会使用旧安装脚本替换日常版本50。

## P3 入口映射结论

- `main.swift` 普通输入在 `bridge.handle → apply → insertCommittedText`；召回和英文补全另有直接insertText。最终调用必须归一为带目标检查与操作身份的交付入口，不能把调用返回Void当控件确认。
- `InputiaVoiceInputLauncher` 仍固定日常Handy、等待1.5秒并同步waitUntilExit；应只承担异步寻找/隐藏启动，收到服务Recording状态才显示录音。不得增加与Option+Space争抢的第二套快捷键状态机。
- `inputia-capi::with_session` 当前生成commit就同步SQLite学习；候选排名也锁memory。设置mtime检查/重建和召回读板也在按键路径。仅新增socket不足以通过A09，必须把学习写入/共享候选/配置刷新改成后台工作器与主线程只读快照。
- Host尚无controller UUID/激活代数/输入框token；bundle ID或weak active controller不构成字段身份。普通组合、英文后缀补全和系统失活原有提交语义必须保留。
- Handy保留 `TranscriptionCoordinator` 为唯一录音生命周期所有者；需要明确Start/Stop/Cancel/Status接线，不能用toggle重试。`actions.rs` 当前转写完成仍自动平台paste，IME会话接入时必须路由到单一输出所有者，不补第二次paste。
- 新协议不传任意路径或任意命令，不把键盘全文写入队列，也不沿用旧debug正文日志。

下一步先完成并审查候选构建/profile配对，再冻结P3消息与输出所有权接口，实施Host后台通讯、提交回执驱动学习及故障测试。整体P0–P6/A01–A12范围保持不变。
