# 1.0.9 后台语音服务与窗口分离

用户要求关闭设置窗口后仍能使用语音及剪贴快捷键；只有输入法菜单的“退出语音服务”停止服务，停止后显示“打开语音服务”，可隐藏启动并持续后台工作。

## 当前证据

开始检查时只有输入法组件在运行，主程序已退出；日志最后有 RunEvent::Exit 的模型卸载。未单凭此记录认定红色关闭按钮就是退出原因。当前 CloseRequested 已 prevent_close + hide；通过 CUA 启动 1.0.8 后 Cmd+W，后台进程仍在，无法复现单纯关窗导致退出。

输入法 readiness 在明确退出及观察到主程序终止后 suspend，现有菜单没有恢复入口，是可确认的功能缺口。关闭窗口、非显式退出、显式退出和再次启动需要分别验证。

## 实施及验收

菜单通过异步可信 status 与签名配对进程核验区分服务运行/停止/不可确定；停止时显示“打开语音服务”，显式打开解除 readiness 暂停，保留固定安装路径、动态/静态签名、配对 manifest 与维护边界检查。使用 --start-hidden、activates=false、hides=true，不发录音命令。明确退出后不自动循环复活。

每个新菜单独立接收刷新结果，避免重复打开时新菜单一直disabled。已退出连接/会话归属退休，旧delivery回执单次保护并限定旧session，避免污染重启后的新会话；不重放未知结果。

macOS 后台权限监视器本来就在 setup 初始化原生快捷键与输入控制，200ms watchdog/2s健康发布不依赖前端。修正旧注释，原 CloseRequested prevent_close + hide 保持；补充 window_close、exit_requested、实际exit独立日志。不把 Tauri code=None 一概当隐式退出，因为它也包含明确 Cmd+Q。

配对及非配对 launcher selfcheck 均通过；配对额外覆盖 menuServiceRetirementClearsStaleOwnership、preFetchFailureReleasesWait 和单次回执。原生 build83 构建、签名与既有自检通过，独立审查未见阻断。系统 TextInputMenuAgent 自动化读取超时，因此未实际点击输入法菜单进行服务停止/打开循环；静态审查和自检不等同此实体菜单验收。待补充主程序最终构建、配对更新和窗口/服务实际检查证据。不以构建成功替代实体按键及语音插入验收。

## 空闲高 CPU 与正式安装命名

用户补充报告两个 Inputia 进程持续高占用。root 实测已安装1.0.8主服务207.4% CPU、IME23.7%，不是 rustc 编译占用。分别用 sample 采集3秒调用栈，主服务热点为 SecStaticCodeCheckValidity→validateResources→read/SHA256；IME personalization 与 typed-capture 队列均在 openAuthenticatedConnection 的静态资源核验上反复工作，主线程基本休眠。

每次个性化/输入记录请求重新连接并完整握手，且服务端 read_frame 的首字节等待计入2秒超时，使空闲连接被关闭。修复保留首次连接的完整配对/签名校验，复用已认证连接；权限epoch、服务器实例、退出、维护与错误都必须使旧连接失效。服务端已认证连接先等可读，再以原严格帧deadline读取，不能无限容忍半帧。

同一产品有输入法组件与语音后台两个必要进程，不是安装两个版本。正式主程序路径迁为 /Applications/Inputia.app；保留现有证书、bundleID与trial-20260905数据根。旧 /Applications/Inputia 设置.app 是版本0.0.50的独立启动器。已复制到专用备份，未删除用户数据；原bundle归root所有，普通移动被拒绝，sudo非交互也提示需要密码，因此它仍留在应用目录，需要用户在Finder以管理员权限移至废纸篓。它未运行，不是第二个语音后台。旧Candidate与正式主程序不应同时作为安装入口存在；更新器需支持唯一旧路径迁移和完整路径回滚。

服务端真实 UnixStream 回归3项通过：空闲超过2秒后完整帧仍可处理、半帧仍受2秒总期限约束、对端关闭立即失败。命令：`DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo test --manifest-path src-tauri/Cargo.toml --lib idle_connection_tests -- --nocapture`。增加macOS现有锁内libc依赖以使用poll，Cargo.lock未变化。

配对 launcher 性能自检通过：连接作用域在权限epoch、manifest字节、expectedServer变化时失效；readiness只进行一次准备审计，退出暂停直到显式恢复。每个队列只保留一条已认证socket，退出/错误/权限退休时关闭；不缓存未经验证身份，不降低首次签名校验。

正式路径迁移回归安装前13项、最终14项通过，命令 `python3 macos/InputiaInputMethod/Tools/test_candidate_update.py`。覆盖唯一旧路径迁移、双路径共存拒绝、不变证书/profile、安装或启动失败回滚旧原路径；仅成功验证运行身份及恢复输入源后清理旧 LaunchServices 登记，清理失败单独报告而不将已验证安装误判失败。

主程序与原生最终1.0.9/build83构建通过；成对安装的动态身份核验通过，releaseUpdate=true、tccChanged=false、previousRecordingsReplayed=false。实际主程序已在 /Applications/Inputia.app，旧Candidate路径不存在；输入源仍选中Inputia。安装后的Swift热词自检仍为65条/prefix1，模型、Silero/VAD与Ctrl+Shift+V设置保留。

安装后连续25.44秒CPU时间增量测量：主服务平均1.37%、输入法平均1.88%；同期ps报告主服务0.8%–1.0%、IME1.2%–2.5%。健康心跳持续更新且状态ready。基线分别为207.4%与23.7%，两端资源验签热点的反复执行已明显降低。此测量属于短时间空闲窗口，不代表所有打字/模型识别负载上限。

实际UI显示v1.0.9、输入控制已就绪。点击红色关闭按钮后日志记录 window_close label=main action=hide service=running；后台保持同一PID26201、ready，关窗后再次读取健康心跳。系统菜单AX仍超时，未实际点击服务退出/打开循环；实体按键与真实语音插入未由合成事件冒充验收。

迁移后旧URL不存在导致原清理命令报告cleanupPending；root用有效备份旧bundle路径control-before.app执行lsregister -u成功，没有注销正式新安装。脚本随后修正为使用已备份旧bundle的有效URL清理。

关窗后的第二段无交互20秒CPU时间增量测量：主服务平均1.8%、输入法1.95%，期末ps报告1.0%/1.1%；同一后台PID持续ready且健康更新时间推进。系统旧设置启动器的恢复副本位于专用legacy-inputia-settings备份目录；原安装因root所有权和缺少非交互sudo授权未移动，不将其误报为已删除。
