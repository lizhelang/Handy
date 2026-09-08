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

## 实际菜单接线批次

候选 `toggleVoiceInput` 已调用后台认证客户端，不再用旧进程toggle；重复菜单操作停止同一session，状态查询只更新菜单/日志，不激活窗口抢焦点。已配对候选才进入新路径，未配对候选明确拒绝，日常分支保持原样。输入来源未知/敏感/SecureInput不产生新Start；字段身份目前为nil，因此尚不自动上屏，不能冒称正常插入闭环通过。

共享状态接口已接实际候选SQLite，事务清理共享词快照并保存双版本；当前新候选无待同步学习队列。非空队列或未知旧格式明确拒绝确认，不删除队列假称重核验；完整学习同步仍在剩余范围。

本批失败与实验：

1. 初次构建未启动，检查确认旧 `/tmp/handy-loop-preflight.uYXmzl` 和 `/private/tmp/handy-paired-build.BdoZjO` 已不存在；不能继续引用它们为当前可用备份或签名材料。现已迁到持久私有目录 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/loop-20260908.r0xugn`，生成新配对材料并备份已安装Handy为`Handy-before.app`。旧已安装应用不改动，运行中的旧配对不因构建新key被重置。
2. Inputia完整构建到实际main入口时失败：Swift中的`IsSecureEventInputEnabled()`是Bool，不能与整数0比较。只改为布尔取反后重建成功；原失败日志`inputia-menu-build.log`与成功日志`inputia-menu-build-retry.log`均在该持久目录。
3. 审查发现漏传配对构建参数的候选会走日常Handy分支，已加候选未配对拒绝。含日志阶段定位和该修复的完整包构建成功，日志`inputia-menu-final-build.log`。此时仍未安装或从菜单实际录音。
4. 新配对key要求Handy同步重建，当前构建日志`handy-menu-pair-build.log`。下一步只进行配套清单签名、包校验与用户级候选安装确认，然后从实际菜单运行；不继续扩未消费接口。

本批尚未新增菜单→录音的运行证据。当前唯一关键阻塞：候选Inputia尚未并存安装/选中；安装前先完成配套Handy构建和签名清单。原框正常插入及焦点变化演示均未完成。

配套材料现已齐备：持久目录`loop-20260908.r0xugn/package`含两份候选app和签名清单（SHA256 `00227c9ca7b5a1af15462569171b9b3b76ec3e475b78b4f03406de24ceb18009`）。候选Handy退出后，数据备份到同目录`profile-before`；旧已安装测试app为`Handy-before.app`。`INSTALL-AND-RESTORE.md`明确只并存安装候选、切回微信输入源、保留新测试数据，不调用会覆盖日常Inputia的安装脚本。等待一次性确认并存安装/临时切换及配套测试Handy更新；未声称真实菜单闭环通过。

## 明确授权后的真实安装与切换

用户明确允许并存安装候选Inputia、临时切换测试和更新测试Handy后，已执行：

- 旧测试Handy移到持久构建目录`Handy-replaced-at-install.app`，新包安装到`/Applications/Handy Unified Candidate.app`。Inputia新安装到`/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app`。原日常两种输入法及`/Applications/Handy.app`未替换。
- 旧配对清单另存`pair-before-install.json`，候选profile使用新清单。两份已安装包严格签名验证通过。新Handy实际进程64379，listener就绪，原生UI完成权限检测后显示正常通用设置页和Qwen3-ASR 0.6B。
- 原TIS工具把图标路径写成inputia.pdf，与实际inputia-menu.pdf不符，导致父/主项匹配失败。修正后注册、启用返回0，但选择返回-50；重新检查发现候选尚未加入系统设置的用户输入法列表。没有反复注销日常输入法或要求重启。
- 在系统设置“键盘→文字输入→编辑→添加”中按完整候选ID筛选并添加。再次选择返回0，`selectCurrentMatchesTarget=true`；候选ID为`com.inputia.inputmethod.Inputia.UnifiedCandidate.Hans`，路径已匹配用户级候选包。当前候选实际进程65183。
- TextEdit新建空白文稿“未命名7”，未打开/改写真实文件。切换到该窗口后再次TIS确认仍选中候选。

当前运行位置停在实际菜单入口前：CUA无法取得TextInputMenuAgent/SystemUIServer这类无普通窗口进程的点击目标，两次查询超时；系统Ctrl+F8后未得到可操作的菜单状态。没有绕过限制直接调用toggleVoiceInput或其他底层接口，也没有注入最终文字。最近原生日志无inputia_unified_voice事件。

下一步需要用户在当前空白文稿从实际输入法菜单选择“语音输入”、说测试句并再次点击停止；主代理随后核对实际记录和历史。尚无菜单录音、正常插入或焦点变化演示，里程碑未通过。测试结束应切回已记录的微信输入源。
