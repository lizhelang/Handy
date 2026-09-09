# 安装后的真实运行身份门禁

2026-09-09。用户在图书馆，本批只做静默诊断和隔离进程测试，不录音、不播放声音、不改输入源、不操作文稿。

## 修复的实际错误

现有 install-check.sh 原先按 ps argv 找到 PID 后，又用 app_cdhash(SYSTEM_APP) 读取磁盘文件，可能把仍运行备份目录旧映像的进程误报为最新版。这与实际 PID5738 的认证失败一致。

已复用此脚本增加 verified_running_cdhash：先验证预期磁盘代码，读取经过字符限制的 identifier/CDHash，再用 codesign 对实际 PID 验证同一显式 requirement。不存在、身份不匹配或无法验证均失败关闭；不使用 argv 作为身份决定。默认安装检查改为使用该结果，同一安装入口存在无法验证的旧进程时，不能被另一个新进程的通过结果掩盖。

提供只读的独立入口，避免候选检查误入日常版 TIS/安装流程：

```bash
bash macos/InputiaInputMethod/install-check.sh --running-identity '/Users/lzl/Library/Input Methods/InputiaUnifiedCandidate.app' PID
```

这里的 PID 是待核对的具体运行进程；检查不会终止它、重新安装、修改权限或发起业务。它只是安装版本一致性检查，不替代 socket audit-token 认证、TIS 当前归属或整个产品验收。

## 验证

先添加回归，旧代码因缺少动态检查入口明确失败，不执行旧脚本的完整系统检查。随后真实编译两个不同的无 UI 小程序并签名：原程序运行时检查通过；将其移走、在相同路径放置新程序后，旧 PID 检查失败；新程序启动后通过；PID0拒绝。

夹具 /private/tmp/inputia-running-identity.WyzmfS。只终止本测试直接创建的子进程，保留其小程序供复核；没有终止实际 Inputia。回归脚本为 Tools/check-running-identity.sh，夹具源码为 Tools/RunningIdentityFixture.c。

原有 INPUTIA_INSTALL_CHECK_SELF_CHECK=1、bash -n 与 git diff --check 通过。实际候选 PID45507（输入法）与 PID6019（控制中心）均通过新增只读检查，hash 分别为 8efedfc5ad3c830a78c8d0819798da5a12ccec72 和 9cb6b8b689eb5f0731c20c4e741a0733883f966e。

本批不构建或替换产品包，已安装 UI 不变；未声称修复唯一快捷键会话衔接、完成菜单录音/焦点变化或达到完整目标。

独立审查无阻塞，bash语法、C语法及既有SELF_CHECK均通过。唯一低优先级建议是自动清理夹具目录；本批选择保留这些不含用户数据/私钥的小型合成程序用于复核，源码已明确说明。两个测试子进程均已退出。
