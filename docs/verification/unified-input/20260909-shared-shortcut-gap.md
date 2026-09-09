# 单一语音快捷键的实际会话缺口

检查点：候选 56 已安装，独立 Shift 原生验收待确认。此处只记录当前代码，不推断默认快捷键已通过焦点变化安全验收。

已接线：`main.swift::toggleVoiceInput` 在配对构建调用 `startUnifiedVoice(client:)`，由真实 IMK client 捕获 `InputiaVoiceTargetSnapshot`，通过认证连接建立 owned voice session。

未接线：控制中心快捷键的 `transcription_coordinator::InputEvent` 仅带 binding、按下/松开和激活方式，没有 HostTargetToken；从 Idle 进入普通转写路径不会取得 Inputia 会话。

输出分岔：`actions.rs` 在 Stop 时若取得 owned_voice，会保存统一历史并准备 Inputia 派发，失败也不转另一输出方式；无 owned_voice 则调用原有 `utils::paste`。当前正常粘贴成功不能证明转写期间换输入框不会写错位置。

不能复用成虚假 token：`integration_output.rs` 的 TargetRegistry 是控制中心主线程 AX 目标，用于历史浮窗输出，不是 Inputia 的 IMK 会话或组合输入凭据。

下一接线必须把同一快捷键边沿通过可信、可取消的交接绑定真实 IMK 目标，并维持一个输出所有者。不得重复注册两套可执行触发器，不得用假 HostTargetToken，不得把原有 owned 路线改回普通粘贴。具体最小字段/消息顺序正在独立核查，尚无本批实现或原生通过证据。
