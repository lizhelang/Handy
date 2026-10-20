# 2026-10-20 录音预览时长修复

## 问题与原因

统一历史展开录音时将已经取得的附件 URL 传给 `AudioPlayer`，但播放器原先只渲染 `<audio preload="none" />`，未将该 URL 绑定到媒体元素。直到用户点击播放，才向媒体元素赋予来源。因此尚未播放的完整录音没有触发元数据读取，而界面将初始时长 `0` 显示为 `0:00`。

这是播放器加载与显示问题，不是录音长度为零的证据，也不依赖转写是否成功。本次未修改录音、转写或数据库。

## 修复

- 显式传入的预览来源直接绑定到媒体元素，使用 `preload="metadata"` 读取时长，不自动播放。
- 未知时长使用 `—:—`，只有媒体元数据确认后才显示数值；未知时长时禁用进度拖动。
- 元数据读取失败沿用播放器的持续错误提示，保留播放重试入口。
- 保留按播放取得附件租约的路径，不因展开播放器提前取得租约；暂停清空来源、释放租约时保留已经读取的时长。
- 保留单播放器切换与卸载停止读取的现有行为。

修改范围：

- `src/components/ui/AudioPlayer.tsx`
- `tests/unified-history.spec.ts`
- `tests/history-audio-lease.spec.ts`

## 验证

以下命令全部通过：

```bash
bunx playwright test tests/unified-history.spec.ts tests/history-audio-lease.spec.ts --reporter=line
bunx tsc --noEmit
bunx eslint src/components/ui/AudioPlayer.tsx
bunx prettier --check src/components/ui/AudioPlayer.tsx tests/unified-history.spec.ts tests/history-audio-lease.spec.ts
git diff --check
```

Playwright 共 **24/24 通过**。新增用例在浏览器中生成合成的 159.24 秒、16 kHz、单声道 PCM16 WAV，使用真实 React 组件和 Chromium 媒体元数据读取，验证空转写正文的历史条目展开即显示 `2:39`，同时媒体保持暂停且播放位置为零。没有读取用户录音，也没有将用户音频或转写正文纳入测试。

另一个新增用例验证附件读取失败时显示错误与未知总时长，不能伪报为零秒。租约回归额外确认展开不获取租约，暂停后即使媒体重置触发未知时长事件，已读取的时长仍保留。既有切换预览、迟到请求取消和租约释放用例继续通过。

## 本机安装与实际界面验证

用户明确授权后，已于 2026-10-20 将 **1.1.22/build110** 成套安装到本机。
从原安装对应的 `551a5bd4` 创建独立工作树，只纳入播放器修复、专项测试、本文和版本元数据；
未纳入另一工作树中尚未提交的热词功能。制品来源提交为
`cd279b91042e578e34d3940cb970b82524c31607`，分支为 `codex/inputia-player-duration`。

- 独立工作树的前端构建与 24 项专项回归通过，完整成套构建及其原生自检通过。
- 主应用与输入法均为 1.1.22/build110、同一来源提交和 release ID；签名验证、配对预检、
  更新事务及实际运行进程身份验证通过。
- 两组件健康状态均为 `ready`，后台输入准入开启，维护状态退出；未修改 TCC。
- 安装后通过实际 Inputia 窗口展开原有空转写录音：未点击播放，显示当前位置 `0:00`、
  总时长 `2:39`，按钮仍为“播放录音”。这是最终 WKWebView 与实际附件路径的验证。
- 安装前 9 条语音历史及 8339 条剪贴历史逐条保留；剪贴历史期间新增 2 条。
  两份设置文件字节一致，备份的 10 个 WAV 字节一致，三个数据库完整性检查通过。

制品目录：
`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/release-HpZMgvxB`。
构建、安装日志与只读数据备份：
`/Users/lzl/Library/Application Support/InputiaBuilds/player-duration-20261020`。
更新器保存的旧程序：
`/Users/lzl/Library/Application Support/HandyUnifiedBuilds/permission-update-bw7bfjdj`。

本次未修改模型生成预算或长录音转写逻辑，也未将恢复文字写回历史库；
本机安装验证不代表公开签名公证或长期语音输入验收。未推送或公开发布。
