# 目标注册原生诊断

北京时间2026-09-10 00:00后，接续IME61原生测试。

## 已观察事实

- 旧IME61 PID65104曾记录 `focused_application_mismatch`；后续诊断未更新，不足以推断循环停止。进程采样 `/tmp/inputia-ime61-focus-sample.txt` 显示正常事件等待，没有被采样到阻塞调用栈。
- 只读审查确认reason/provider日志会去重；准备目标的复合条件返回nil时原日志只有 `target_not_eligible`，不能区别client/bundle/敏感应用等分支。
- CUA两次可见坐标点击TextEdit返回 `Computer Use server error -10005: noWindowsAvailable`；AX读取、Raise和发按键仍能工作。不能以这些工具行为证明系统前台焦点保持。
- 代码 `3f9f8bf4`仅补固定失败原因，不改认证、权限、目标合格规则或输出：missing_imk_client、secure_input_enabled、missing_client_bundle、sensitive_app，以及已有capture原因。不打印bundle、窗口标题或正文。

## 候选62已运行

- 完整签名构建 `/tmp/inputia-ime62-build.log`通过，paired Swift typecheck通过。输入法版本62，实际PID66937、CDHash `fbbdadf1d95ad1389dbba8db2d9ede353a11f8ad`，运行身份核验通过。控制中心包不变，重启PID66900加载新配对。
- 备份 `/Users/lzl/Library/Application Support/HandyUnifiedBuilds/target-diagnostics62-20260910.iHCGIL` 含完整 `profile-before`、`ime-before.app`（61）、`pair-new.json`。两库quick_check=ok，历史5项；无日常安装变动。
- 实际TextEdit按n/i/space后专用文稿从 `?n ?你ni你` 变为 `?n ?你ni你你`。基础输入成功，不等于owned目标注册。
- 新进程02:00:02本地时间记录listener_started/provider has_target=false；02:00:19记录deactivate。当前没有ready/target_registered证据，也没有新capture失败码。与控制器未保持激活一致，但尚不能据此确定是工具焦点行为还是原生生命周期问题。

## 下一区分实验与边界

需要真实保持专用TextEdit文稿为前台，然后只读检查ready/target_registered。工具坐标点击连续失败，不能用放宽目标检查绕过去；最小用户动作是手动点专用文稿正文并保持几秒，无需录音或说话。

审查另提出acceptStart漏completion可能令busy不释放，但本轮未触发语音，也未证实当前调用存在可达漏回调。保留风险，不把它当根因，不直接增加超时重试造成回执未知时并行输出。

恢复62到61：切离测试源、核对并停止候选进程，保留当前包到新目录，恢复此备份的ime-before.app与profile-before/pair-manifest.json，重启未改变的控制中心；不覆盖整份旧profile，防止丢失新增数据。尚未执行此次恢复，不能标为回滚验收。
