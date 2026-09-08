# Inputia 青绿色与标识布局修复

核验时间：2026-09-08 20:04 +0800（北京时间）。实现提交：4da332b8。

## 用户要求与实现

移除侧栏标识被挤成细长粉色胶囊的问题；使用原生候选框的青绿色，而非Handy粉色。
主色直接对应InputiaCandidatePalette.firstHighlight的#2F6F73，悬停/按下色同源；深色强调文字使用#9DCCCE，浅色强调文字使用#24575A。
Wordmark固定图形尺寸、禁止flex收缩，去除装饰外框；复用原Inputia SVG，以正确转义的CSS mask在明暗模式中保持对比度。录音和剪贴板样式同步去掉旧粉色。未修改录音、IPC、输入法或数据合同。

## 实际证据

- 旧组件在等宽侧栏测试中图形宽度仅6.203125px；新组件明暗主题均通过方形、宽度至少30px、文字颜色与mask非none检查。
- 前端build/lint通过；36项Playwright通过。其中两项品牌测试与截图只证明前端布局，不替代原生插入验收。
- 原生候选构建成功，签名验证、包含配对信任的profile自检通过。增量构建仅跳过未改动的Qwen辅助程序重编译，仍从现有构建资源打包该程序；前后SHA256均为14a7581814a312fc1d49727095d79f6ad7b7e7b644bdd5816ffe6a3444bd16a6。
- 已更新/Applications/Inputia Candidate.app，主程序cdhash为653c775aba98f5eebed92f802c8da64936a6f60e。输入法组件未替换，cdhash仍为2f60451b3e1ddbe3fdf7d0f69b1a8a3ba690ab4e。
- 实际启动日志12:00:49 UTC显示unified_voice_listener_ready。CUA实际窗口先短暂出现权限初始化页，随后回到正常控制中心；已查看“关于”和“通用”页截图，标识比例正常、无胶囊外框、选中项与开关为青绿色。没有代用户授权。
- 只读独立审查没有发现阻塞。旧Button注释中的pink accent为非运行时遗留，未扩大本批改动。

## 构建与恢复

构建目录仍为/Users/lzl/FILE/github/Handy-unified-input-system。
在已存在且未改动Qwen辅助程序资源的情况下，本批使用标准候选config，并追加build.beforeBuildCommand为bun run build的增量覆盖；HANDY_UNIFIED_PAIR_BUILD指向已保留的public-build.json，Rust1.96.0，CMAKE_POLICY_VERSION_MINIMUM=3.5。全新环境仍需运行完整标准构建以生成辅助程序。
日志/旧安装位于/Users/lzl/Library/Application Support/HandyUnifiedBuilds/loop-20260908.r0xugn：inputia-teal-product-build.log、brand-geometry-before.log、brand-geometry-after.log、Inputia-before-teal.app、profile-before-teal、pair-teal.json。
恢复只需在停止候选后恢复旧App及profile-before-teal里的配对清单，不覆盖当前数据；本次没有数据迁移或清空。日常安装未改。

本报告只验收此次视觉修复。真实原输入框输出、统一快捷键衔接及完整融合验收仍未完成，不因视觉通过而标记goal完成。
