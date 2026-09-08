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

## 真实快捷键对照实验（未通过）

因系统菜单工具不能取得点击目标，给同一候选入口增加可见`Control+Option+Shift+V`，仅IMK按键路径消费，不注册第二个菜单keyEquivalent，不改Handy原Option+Space。已构建、更新仅候选Inputia并刷新签名配对；原包保留于持久构建目录`Inputia-before-shortcut.app`，首个物理键码版本另存`Inputia-physical-shortcut.app`。

第一次真实工具按键在TextEdit产生Control-V控制字符，语音会话数0、无入口日志。已撤销该字符。再用单字母n检查基础路径：实际出现选中的带下划线拼音组合，Escape清除，说明基础输入不是完全失效。

第二次只增加V/Control-V语义字符兼容，保持修饰键不变；重新构建安装后结果仍为控制字符，已撤销。没有开始麦克风录音，没有播放测试音频，没有生成转写。此修改尚未被证明有效，当前源码与已安装候选为该实验状态，不能报快捷键已修复。

按用户规则停止继续换键/改判定。当前假设是组合键未进入IMK或自动化修饰键传递差异，需要用户在同一空白测试文稿用实体键盘按一次该组合键进行区分。普通n/Escape对照与两次失败不替代真实语音入口证据。TextEdit把新测试文稿自动保存为自己的未命名7.rtf，正文已恢复为空；它是本次新建测试文件，不是原有用户文稿，后续测试应转入本地专用目录。

## 2026-09-08 11:34 北京时间：菜单失败后的路径实验

- 用户实体组合键和实际菜单均报告无录音浮窗；Option+Space能触发的是Handy全局入口，不算Inputia会话通过。
- 候选outbox.db已经存在，schema=1、shared_policy=0/0、learning_outbox=0；Handy会话计数0。两端安装签名与配对清单一致，监听socket属于当前Handy进程且目录700/socket600。
- 可重复阻塞：Foundation对实际`/private/tmp/...sock`调用`resolvingSymlinksInPath()`返回`/tmp/...sock`，严格字符串比较失败。Darwin.realpath返回原始规范路径。新增命名socket测试在旧实现失败endpoint，换为realpath严格比较后通过；没有移除符号链接、属主、模式或签名检查。
- 证据根：`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/loop-20260908.r0xugn`；`framed-path-before.log`为失败，`framed-path-after.log`为12检查/100帧及主线程拒绝通过，`inputia-path-fix-build.log`为完整候选构建与严格签名校验。此测试只证明传输路径，不是实际菜单/录音验收。
- 已更新用户级候选Inputia，CDHash `705179631388e82c290ae564f6f61672257419fb`；`pair-path-fix.json`已签名安装。原包与清单保存在`Inputia-before-path-fix.app`、`Inputia-replaced-path-fix.app`、`pair-before-path-fix.json`，未动日常安装/数据。Handy重启后03:32:52 UTC监听ready；候选输入源selectStatus=0且selected=true。
- 下一实验：在专用空白TextEdit从实际输入法菜单点击语音，核对认证peer、会话及录音浮窗。当前尚无修复后菜单运行、真实录音、正常插入或焦点变化证据，闭环未通过。
- 路径修复提交`984ef71a`。独立原生子代理review_socket_path仅审查路径修复及命名socket测试，重新编译执行通过，无阻塞发现；未审查既有未提交快捷键实验，安装包包含这些既有实验改动，不是干净最终提交的发布包。

## 2026-09-08 用户实体快捷键录音后的核验

- 用户确认Control+Option+Shift+V能唤起浮窗但没有文字。实际认证peer计数1、owned会话2、结果关联2，两条会话均start_claimed=1且pending_target，关联history记录2/3。
- 当日04:51:50、04:52:05 UTC日志分别显示24480/35520采样，完成本地识别耗时0.25/0.13秒，正文日志为REDACTED。不是未录音或ASR未完成。
- 实际候选Handy界面点击历史后出现这两条语音记录，单击最新条目可看到非空转写预览。只查看，没有点击插入、复制或手工注入结果。
- 当前最短阻塞是未实现Host结果接收与原输入框身份校验/唯一输出，不是权限或快捷键；现有Launcher终态只更新菜单状态并关闭连接。因此正常插入与焦点变化仍未验收，不能要求用户反复录音来替代这段实现。

## 2026-09-08 本批：接通待插入后的实际交付路径（尚待原生运行）

- 复用原输出账本，在已认证socket增加fetch/receipt；服务端核对owned session、最新策略、当前revision、取消屏障与field token，唯一claim后才交付正文；再次fetch只返回状态。正文回复没有Debug，JSON帧在claim前检查上限。
- Inputia在原开始回调保留AX控件/client/controller/激活代数与选择范围；焦点、选择、值或销毁通知使快照永久失效。唯一正文送回主线程，重新核对隐私、组合、原控件、选择与本地2秒派发期限，再调用IMK insertText。无法确认时回pending_target；未知回执不重传，不调用平台paste。
- 权限不足/字段不可观察可保留语音历史；已知安全字段、Secure Input、App身份冲突、未知错误不得降级录音。系统设置实际列表中Handy Unified Candidate已启用，候选Inputia尚无条目；要做AX原目标核验需用户授权候选Inputia，不能通过移除校验绕过。
- 新鲜检查：客户端合成socketpair完成fetch→delivery→receipt，身份/epoch/空或超限正文拒绝；Host选区/期限与敏感失败拒绝自检通过；Rust voice_protocol 10、voice_result 9通过，主代理voice_dispatch 17通过。它们不替代原生插入证据。
- 独立审查提出并已修复：Swift选区局部变量遮蔽导致编译失败；output账本claim未收到Host回执时必须映射uncertain，不展示已派发。修复后定向编译/测试通过。明确Host派发回执仍仅dispatched_only，不冒充应用级confirmed。
- 构建/测试证据位于前述持久根：inputia-delivery-build.log、inputia-delivery-final-build.log、handy-delivery-build.log、voice-output-server-tests.log。Handy构建本节记录时仍运行。专用空白文稿为该目录native-output-check.rtf（本地，不是iCloud）。
- 正常原框插入及转写期间焦点变化仍未运行，不能标记A04/A05或完整闭环通过；待配套候选安装和必要权限后继续。

### 配套候选已安装，停在权限确认

实现提交`0d1f83ca`，两端构建均成功；主代理lib clippy -D warnings通过（上游block未来兼容警告另存日志，不是全测试门禁全绿）。独立审查复核未知回执映射及新增断言后，确认可进入原生验证。
已停止确切候选进程，备份完整profile到持久根`profile-before-delivery`（历史quick_check=ok、记录1/2/3保留），旧包保留`Handy-before-delivery.app`与`Inputia-before-delivery.app`。仅更新两份候选并安装新签名清单`pair-delivery.json`，配套持久包为`package-delivery/`。Handy签名hash `141922fa74a1ad2082b9ae95b2b7ef4311eaa074`、Inputia `dc2cabb811191391aade8f1e72c2e3839dcff9e8`。恢复与重建步骤见持久根`DELIVERY-INSTALL-RESTORE.md`。
新Handy实际启动05:22:21 UTC监听ready，但界面仍在权限引导（麦克风/辅助功能按钮），未冒称录音已恢复。系统权限列表无Inputia候选；添加文件窗口已选中`/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app`，最终“打开”/授权留给用户。没有修改TCC或开启任何权限。
权限窗口中Command+Shift+G在候选输入源下无响应；切回已记录微信输入源后同一工具按键打开“前往文件夹”。这是一条新增原生对照线索，需后续复核候选对系统组合键的处理，不归为已经修复，也不以更多按键尝试掩盖。等待权限期间保持微信输入源，不继续让候选影响日常输入。

续轮现场核验：系统列表中两个Handy均显示开启，但Inputia候选仍无条目，不能认为它已授权。已重新选中确切Inputia候选App，最终打开/授权仍由用户完成。本轮微信输入源下，紧接“添加”批量发送Command+Shift+G也未打开路径面板；在确认文件窗口显示后单独发送则成功。因此此前键盘对照不足以归因Inputia吞键，也可能是窗口就绪时序；不据此前线索盲改输入法。

用户表示授权完成后的实际核验：系统列表出现InputiaUnifiedCandidate且开关on；Handy回到通用设置页。通过TextEdit打开本地native-output-check.rtf，TIS确认候选selected=true。CUA发送ctrl+alt+shift+v仍在专用文稿产生Control-V字符，未新增会话（计数仍2）；已用Undo撤销并确认文稿为空。此证据只说明本次自动化组合键没有走到语音会话，不能替代用户此前实体键盘成功的证据，也不能声称这版插入失败或通过。下一实验需实体键盘从同一专用窗口启动，核对新会话field_id及最终output receipt。

用户再次实际录音后：新增会话3/4/5为零样本后interrupted，会话6取得29280样本，本地ASR耗时0.18秒（1.83秒音频），已准备历史结果，但field_id仍空，输出账本prepared，未claim或调用IMK。问题收敛到capture拒绝，不是结果插入后丢回执。
读取最新NSLog谓词仍无Inputia事件；确切Inputia进程stdout/stderr都指向/dev/null。仅补failCapture的候选后台诊断（16条reason/time/pid、无正文/窗口标题/按键、atomic文件及600权限），完整构建通过并更新候选Inputia，保留旧包Inputia-before-target-diagnostic.app及pair-before-target-diagnostic.json，新配对清单pair-target-diagnostic.json。Handy只重启加载清单，不替换二进制。
两次焦点固定/实际输入源对照未能让CUA进入新的语音会话；字符已撤销，会话数仍6，诊断文件未生成。停止继续微调自动化组合键；下一次实体入口仅需开始/结束即可采集具体capture错误码，不需要重复说测试句。诊断接线不是根因修复，正常插入仍未通过。

用户“已按”后取得首次明确诊断：候选PID46623在本次启停均返回accessibility_permission_required，会话7仍field_id空/pending_target。系统设置同时显示InputiaUnifiedCandidate开关on，说明列表on不能证明当前进程AX信任生效。只读codesign -d -r-核验：原候选designated requirement绑定cdhash dc2cabb811191391aade8f1e72c2e3839dcff9e8，新诊断候选绑定baa2aeb419dbfae191aa5d8b6b451b8912df48c9，身份确已改变；授权与签名失配是有证据支持的当前假设，不能说控件通知不受支持。已将Inputia开关滚动到可见区域（Ice下、iPhone镜像上），未代用户更改权限。下一步用户刷新该候选授权后，只重启同一二进制，不再次重建改变身份。

用户刷新授权后仅重启Inputia，PID68509，前后cdhash均baa2aeb419dbfae191aa5d8b6b451b8912df48c9。新会话8/两条本地诊断变成field_unobservable，说明AX信任检查已通过。进一步只读Start目标元数据发现会话7/8实际source_app都是com.openai.codex，会话6才是com.apple.TextEdit且在授权修复之前；因此不能用本次Codex字段失败冒充授权后TextEdit仍失败。专用native-output-check.rtf实际仍为空。下一实验要由用户把光标点到该TextEdit文稿，固定输入框类型；Codex/Electron字段观测保留为待验证边界，不改签名或盲目放宽焦点门禁。

品牌方向暂停核验：用户先明确批准唯一Inputia品牌/永久取消独立Handy托盘，随后自行改写的goal又使用Handy唯一控制中心表述，主代理已提出一次明确的归属确认，未替用户修改goal。暂停的菜单桥半成品仅涉及voice_protocol/voice_connection/voice_dispatch三份文件，已保存在git stash对象ffceb667af6b715ca3aeaa67c92d9f8bed37659f（名称paused-inputia-menu-bridge-awaiting-product-direction-20260908），没有丢弃代码。其余未提交文档保留；恢复原可编译代码后cargo check --lib通过，证据paused-menu-baseline-check.log。已安装候选未更换，没有新增原生插入通过证据；恢复菜单分支时先检查该stash，不重复重写。

## 2026-09-08 19:20 北京时间：唯一Inputia产品化候选收口

- 用户确认最新方向为“Handy只作为内部底座，用户只看到Inputia”。提交`9193dc0b`已将控制中心标题、侧栏、引导、图标、更新提示和系统输入法菜单统一到Inputia；删除旧Handy手势/文字Logo组件和用户可见托盘开关。独立托盘路径保持不可启用，旧bundle/data/API命名仅作兼容与上游许可用途。
- 系统输入法菜单已成为日常入口：菜单项覆盖语音输入、同步语音/剪贴板记忆、召回剪贴板、模型、设置、检查更新、退出语音服务；菜单动作走已认证socket `menu` 帧，不新增未认证CLI旁路。前端新增`navigate-to`事件接线，使Inputia菜单可打开统一历史与设置页。
- 最新主候选包已重建并签名验证：`src-tauri/target/release/bundle/macos/Inputia Candidate.app`，`CFBundleDisplayName=Inputia Candidate`，`CFBundleExecutable=handy`，`codesign --verify --deep --strict`通过。可执行文件名仍为内部兼容名，Inputia启动器已按实际产物优先识别`handy`，同时兼容未来`Inputia`。
- 最新输入法候选已重建并签名验证：`macos/InputiaInputMethod/candidate-builds/trial-20260905/InputiaUnifiedCandidate.app`与`Inputia 候选设置.app`，build脚本输出签名、最低系统版本和Rime数据检查通过。
- 本轮验证通过：`bun run build`、`bun run lint`、`bun run check:translations`、`bun run format:check`、`macos/InputiaInputMethod/validation-policy-self-check.sh`、`InputiaVoiceInputLauncherSelfCheck`、`cargo +1.96.0 test independent_tray_cannot_be_enabled_by_legacy_settings_or_cli --manifest-path src-tauri/Cargo.toml`。额外手动单编译`InputiaHandyMemorySyncSelfCheck`因未链接Rust C API失败，属调用方式无效，未作为产品失败证据。
- 独立复核结果：未发现`handle_inputia_menu_cli`、`recreate_tray_icon`、`Handy v`、旧Logo组件或用户可见托盘设置入口残留。剩余`Handy`字符串集中在兼容路径、内部函数名、旧数据导入、测试路径、上游许可致谢和历史数据根；不作为独立产品入口。
- 尚未完成：最新唯一Inputia候选尚未安装到并存测试位置并从真实系统输入法菜单重跑“录音→本地转写→统一历史→原框插入/焦点变化待插入”。因此A04/A05和端到端闭环仍未通过，完整goal保持进行中。

## 2026-09-08 19:26 北京时间：最新候选安装态与自动化入口边界

- `/Applications/Inputia Candidate.app`已是最新主候选，运行PID 16619，cdhash `75a30e494006613486b24a895207c5b7adb90ad0`；`~/Library/Input Methods/InputiaUnifiedCandidate.app`已是最新用户级候选，cdhash `2f60451b3e1ddbe3fdf7d0f69b1a8a3ba690ab4e`。日常`/Applications/Handy.app`和系统正式`/Library/Input Methods/InputiaInputMethod.app`未替换。
- 已用持久根`signing-private.x963`离线签署新清单`pair-inputia-product.json`，并安装到`~/Library/Application Support/HandyUnifiedCandidate/trial-20260905/pair-manifest.json`；旧清单备份为`pair-before-inputia-product.json`。清单只列入上述两端最新cdhash。
- 候选主服务重启后写出`Handy/integration-endpoint.json`，mode 600，profile为`unified-candidate:trial-20260905`，socket在私有`/private/tmp/handy-unified-501-*`目录。输入源通过候选TIS工具选中，当前ID为`com.inputia.inputmethod.Inputia.UnifiedCandidate.Hans`，`selectCurrentMatchesTarget=true`。
- CUA在专用TextEdit `native-output-check.rtf` 中发送`Control+Option+Shift+V`仍只产生Control-V控制字符；已立即Undo，文稿恢复为空，历史计数仍为5。此自动化失败不等于实体键盘失败，不能替代真实菜单/实体入口验收。
- CUA只暴露TextEdit窗口，不暴露系统输入法菜单栏进程或全屏菜单栏坐标；本轮未能自动点击真实输入法菜单。下一步需要用户在当前专用TextEdit中用实体键盘`Control+Option+Shift+V`或系统输入法菜单“语音输入”触发并停止一次，随后继续核对会话、历史、原框插入/待插入结果。
