# 语音快捷键与过期校验恢复

用户报告 Control + Shift + Space 无响应。现场控制中心健康状态为 retiring，日志每两秒停用快捷键，设置为 ctrl_left+shift_left+space。手动重新检测后恢复 ready。随后将当前绑定与重置值改成 ctrl+shift+space，重启控制中心，注册日志确认支持左右两侧。

实际空白 TextEdit 窗口发送 Ctrl+Shift+Space 后，控制中心记录 TranscribeAction 启动、停止、麦克风流启动/停止；零样本未持久化。此证据证明按键触发录音链路，不等于真实语音识别和文字插入验收。

代码调查发现：3.5 秒权限校验租约过期会让旧 epoch 失效，后台监控只退休而不重建。以前没有记录关闭原因，不能事后确认本次是休眠或其他故障触发。修复仅允许租约过期恢复；先被动校验 AX 并退休旧资源，再确认无维护及资格仍有效，才建立新 epoch。真实权限撤销、主动暂停、原生故障与工作线程超时清除恢复资格，仍需显式恢复。新增关闭原因和恢复来源日志。

相关生命周期回归 9 项通过，包含恢复期间真实关闭使凭据失效，以及 32 轮关闭/恢复并发检查。独立源码审阅未发现新增阻断。源码 macOS 默认值此前已改为 ctrl+shift+space，本次随正式版构建交付。当前配置已备份为 settings_store.before-generic-voice-shortcut.json。

初版 1.0.2 已配对安装。现场 SIGSTOP 6 秒再恢复验证暴露旧监听线程退出无条件关闭当前状态：lease_expired 后立即出现旧线程“原生监听器意外退出”，恢复资格被清除。已把监听故障关闭绑定线程 epoch，删除两处随后的无条件关闭；当前线程真实故障仍关闭，旧线程不能影响恢复或新代。新增旧线程与当前线程故障回归后，权限生命周期共 11 项通过，独立审阅通过。最终 1.0.2 / 76 已配对安装，运行身份和输入源恢复检查通过，tccChanged=false。重复现场 SIGSTOP 6 秒 / SIGCONT：epoch 1→2，日志 permission_lease_expired、stale_shortcut_fault ignored_epoch=1，随后 input_permission_ready recovery=lease；无需手动点击即恢复 ready，快捷键重新注册。恢复后实际 Ctrl+Shift+Space 再次触发录音启动和停止，零样本未持久化。
