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

## 安装与证据边界

本次仅修改源码和测试，未安装、提交或推送。读取 `/Applications/Inputia.app/Contents/Info.plist` 确认当前安装仍为 **1.1.21/build109**，未由本次修改替换。

浏览器回归验证了时长读取与显示行为，但没有验证最终打包应用的 WKWebView 和 Tauri 资源协议。安装新制品后的原生界面效果仍需单独验证；不能将本次通过的浏览器测试表述为已安装应用修复完成。
