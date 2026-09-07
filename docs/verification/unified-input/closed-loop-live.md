# 真实闭环推进记录

本记录取代提交/测试数量作为当前批次主进展指标。完整P0–P6/A01–A12目标不变。

## 2026-09-08 第一批：启动与入口前置检查

实际工作树底座 `421b2a9f`；Inputia菜单仍调用旧 `triggerVoiceInput`，指向日常Handy并等待1.5秒后toggle；新客户端没有调用点。Handy无listener启动调用。语音→统一历史→IME结果准备已有接线，Host结果接收/原框校验/去重上屏尚无实现。

最短依赖链：候选Handy实际监听 → Inputia实际入口使用认证客户端及真实策略清理 → 真实目标登记/录音与已有本地模型 → 已接的统一历史 → Host唯一派发和最终焦点验证。正常与焦点变化演示均未完成。

本批已运行：

- 系统TIS查询候选bundle `com.inputia.inputmethod.Inputia.UnifiedCandidate`：已启用与所有已安装列表均matches=0。当前输入源为 `com.tencent.inputmethod.wetype.pinyin`；没有修改选择或注册。
- 实际启动现存Handy候选app（工作区release/bundle/macos），候选设置确认always_on_microphone=false、clipboard_enabled=false，不采集真实剪贴板或自动录音。原生窗口显示权限页；最新截图/AX证据见本任务的“记录候选程序停在权限页的原生证据”。麦克风显示已授予，辅助功能等待中；没有点击授权。
- 进程真实存活并初始化转写后端。日志早期出现web content process terminated，但后续真实截图已显示权限页，不能将该单条日志误判为应用不能启动。
- 候选还未选模型；测试用Qwen权重已在隔离模型目录，不需要下载/复制真实录音。
- 已停止本批启动的确切候选进程，候选数据及Handy/Inputia应用备份到 `/tmp/handy-loop-preflight.uYXmzl/{profile-before,Handy-before.app,Inputia-before.app}`。未备份/迁移/改写日常唯一数据。

第一项实际接线：`989e0bc6`将既有认证连接处理接到候选setup。缺公钥或签名manifest在bind前拒绝；私有端点信息写入候选数据域。正在重建实际候选，日志 `/tmp/handy-loop-preflight.uYXmzl/handy-build.log`。审查要求显式Exit停止信号，已做小修，最终候选需重建后再给精确提交/签名。

当前唯一最关键阻塞：尚未取得新候选listener的运行证据。下一实验：启动重建候选，先核对缺配对材料的明确拒绝，再签当前两份候选身份并重启，核对listener端点与实际进程。此步骤不当作Inputia菜单闭环通过。

## 已确定的系统操作边界

要跑Inputia真实入口，需要批准并存安装候选输入法至 `/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app`，并选择其独立输入源。日常Inputia/微信输入法安装均不替换。候选Handy辅助功能权限需要系统授权；麦克风当前已授予，但换签名后需重新核验。只使用专用TextEdit、浏览器、Electron测试窗口，不向真实聊天或文档写入。

授权前先提供当前两份候选包、签名配对清单和隔离配置；不能把仍指向日常Handy的旧菜单作为测试入口。首次安装/权限操作尚未执行。

恢复方法：结束专用测试后切回原微信输入源，退出候选应用；只移走新增的候选输入法app，保留候选数据和证据。需要回退本次候选时使用上述备份，不将候选备份覆盖日常数据。系统输入源注册清理在安装授权范围确认后执行，不使用会删除日常Inputia的旧卸载脚本。

## 用户请求重装后：Handy权限卡点解除

用户明确请求重新安装当前卡住的Handy。确认实际运行的是候选版、系统列表此前添加的是日常Handy；候选包中没有独立Utility App。先在系统设置选择器定位正确候选，未替用户点击最终权限授权。

随后按明确重装请求，将已验证候选复制到 `/Applications/Handy Unified Candidate.app`，未覆盖 `/Applications/Handy.app`，未修改候选或日常数据。停止原开发目录候选进程后从新位置启动，实际PID93907路径确认新安装。签名验证通过，主程序SHA仍为 `4ec40525f5dee7ce212ef7819976059fcf505f0f43a843bb282b9d5a60c7c3dc`，配对身份没有因移动位置改变。

原生UI实际已显示“通用”设置页、Qwen3-ASR 0.6B及v0.10.0，不再停在权限页；日志显示Enigo已初始化，listener再次ready。没有通过修改权限数据库或伪造已授权状态解决。Inputia候选仍未安装，真实菜单→录音→历史→上屏闭环仍未执行；当前下一阻塞仍是安全菜单接线及候选输入法并存安装。
