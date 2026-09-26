# 单辅助功能授权：Inputia 主程序与无特权输入法

## 用户授权及验收

用户要求按建议把需要辅助功能权限的能力集中到主程序，让输入法不再需要单独授权。保留两个内部进程、现有固定签名及用户数据。先完成软件、协议与回归，再通过安全维护流程移除多余系统授权；不能靠同名或写TCC数据库合并。

## 结构

- 主程序通过既有签名/audit token认证socket提供ime_target_broker_v1。前台AX目标、角色、选区、窗口隐私策略及观察器均只由主程序持有；IPC仅传绑定owner/server/permission epoch的opaque token。
- Capture/Register/Start/共享词/Fetch/Dispatch分别验证目标，派发仅向同连接已提取的一次性operation发验证nonce。失败保留历史，不伪造目标身份。
- 输入法只留IMK本地client、activation、真实composition/selection generation、markedRange和operation去重。接收服务端短期验证后，在同一主线程任务内重验本地目标再insertText。
- 输入法无AX授权检查、AXUIElement/AXObserver、全局键盘鼠标monitor或CGEventSource.keyState。基本拼音不依赖主程序ready。语音/共享候选依赖经过认证的服务状态，不读取health文件来冒充授权。
- 新C ABI inputia_session_set_context_unverified允许基本拼音，关闭Inputia Memory读取/写入/候选重排。已有Rime原生用户词典行为不在本次重构范围；不声称新增了Rime私密字典模式。
- UI只显示主程序授权需求。维护和签名更新仍保留两组件屏障，不变更主程序已有授权。

## 回归与审查

实际bundled-static-Rime回归：未验证上下文仍提交中国；Inputia Memory读写拒绝；恢复后未验证期间文字没有进入Memory。

Rust第一轮整库506通过/2忽略；新增broker ownership/epoch/nonce/单任务与协议测试通过。独立审查发现并要求修复共享候选清理顺序、鼠标换选区后旧目标复用、prepared target释放遗漏；实现和最终验收后追加证据。

后续二进制审计要求IME不导入AXIsProcessTrusted、AXUIElement*、AXObserver*或事件tap全局监听入口；构建成功本身不能替代该检查。

最终Rust库508通过/2忽略，源码与IME二进制权限调用审计通过，实际静态Rime禁Inputia Memory而保持中文输入回归通过。维护路径同时清空IME代理与旧剪贴浮窗的AX注册表。

## 2026-09-13 授权清理与真实运行复核

已通过维护marker获得两组件本代maintenance ACK，切到ABC并临时停用候选输入源，精确验证并退出两个进程后，在系统设置UI只移除InputiaUnifiedCandidate.app授权。列表导航时先验证selected行，未更改其他应用授权；未直接编辑TCC。

只读TCC复核显示仅com.pais.handy.UnifiedCandidate auth_value=2，主程序授权整条记录与迁移前一致。恢复输入源与两进程后，输入法权限项未重新出现；后台和IME健康均ready。实际运行：IME PID25327/CDHash8d0739419874d25c76161780d9e73f3c2ab6f4fa；主程序PID25329/CDHash9a477708eee1167d1fbab575b3ee3b263a7a0b8c，均与安装包匹配。

源码和安装二进制审计 localAX=false/globalKeyMonitoring=false。主程序仍有唯一辅助功能授权，输入法通过认证内部协议工作。此轮没有真实麦克风录音/原框转写提交测试，不将健康状态冒称完整语音闭环验收。
