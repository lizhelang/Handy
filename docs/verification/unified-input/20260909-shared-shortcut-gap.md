# 单一语音快捷键的实际会话缺口

检查点：候选 56 已安装，独立 Shift 原生验收待确认。此处只记录当前代码，不推断默认快捷键已通过焦点变化安全验收。

已接线：`main.swift::toggleVoiceInput` 在配对构建调用 `startUnifiedVoice(client:)`，由真实 IMK client 捕获 `InputiaVoiceTargetSnapshot`，通过认证连接建立 owned voice session。

未接线：控制中心快捷键的 `transcription_coordinator::InputEvent` 仅带 binding、按下/松开和激活方式，没有 HostTargetToken；从 Idle 进入普通转写路径不会取得 Inputia 会话。

输出分岔：`actions.rs` 在 Stop 时若取得 owned_voice，会保存统一历史并准备 Inputia 派发，失败也不转另一输出方式；无 owned_voice 则调用原有 `utils::paste`。当前正常粘贴成功不能证明转写期间换输入框不会写错位置。

不能复用成虚假 token：`integration_output.rs` 的 TargetRegistry 是控制中心主线程 AX 目标，用于历史浮窗输出，不是 Inputia 的 IMK 会话或组合输入凭据。

下一接线必须把同一快捷键边沿通过可信、可取消的交接绑定真实 IMK 目标，并维持一个输出所有者。不得重复注册两套可执行触发器，不得用假 HostTargetToken，不得把原有 owned 路线改回普通粘贴。具体最小字段/消息顺序正在独立核查，尚无本批实现或原生通过证据。

## 当前代码集成（尚未原生验收）

- Rust broker 已接统一 handler，采用认证 Register/Poll/Retire/Reject 和最长1500ms目标租约，active session 冻结首次target/owner；已转发或断线未知不回落到legacy。非Inputia源保留原路径；当前Inputia无ready租约则Pending。
- Swift 启动入口已开启后台监听，在主线程提供真实预备snapshot，收到开始触发后复核原目标，回传HostShortcut；后续边沿使用首次会话，不再捕获另一个输入框。三个激活模式交由原coordinator处理。
- 开始失败时只撤销broker尚未consume的trigger；已consume的拒绝撤销不冒称未执行。恢复只查原session状态，不重放Start；终端owner清理、旧连接poll身份检查已修复独立审查发现。
- 常驻poll要求服务端并发受理另一语音连接。原串行accept循环已改为最多8个连接线程，每线程独立创建Swift认证handle，不能跨线程共享其指针。只有最后一条同client连接关闭才入队断开通知，通知与连接计数变更保持顺序。
- `cargo check --lib`、主机Swift typecheck、15项host_shortcut相关测试及Tauri库回归（466通过、2忽略）通过。Swift审查的owner复活阻塞已解决；无目标无owner时停止连接/重试，避免后台空转。
- 下一步构建候选57，静默验证认证注册/双连接，再在获准录音条件下完成实际快捷键→录音→历史→原框插入及换焦点待插入。这里的代码/测试不替代原生验收。当前已安装版本仍为56。
