# handy-upstream-first-reintegration

- Plan ID: `P20260902-073628`
- 执行时间(北京时间): `2026-09-02 07:36:28 +0800`
- 项目根目录: `/Users/lzl/FILE/github/Handy`
- 来源需求: 以上游最新结构为底座完整保留全部能力和数据并自动备份恢复

## Plan

# Handy 上游优先重接实施计划

基线设计：`docs/plans/2026-09-02-upstream-first-reintegration-design.md`

## 执行原则

- 固定 `upstream/main@fbd4e15fa14a721c66c57006ae110428b9e255b3`，不追逐执行期间的新提交。
- 使用 `codex/pre-upstream-reintegration-snapshot@3407a51740e8d9cd3e3017b96008b9bb5c5c7de5` 恢复全部已提交与未提交能力。
- 在独立 worktree/分支实现，保留原工作树。
- 数据迁移先备份后写入，任何验证失败自动恢复。
- 上游核心状态机保留；定制能力按扩展边界重新接入。

## 步骤

1. 建立独立 worktree `Handy-upstream-reintegration` 和分支 `codex/upstream-first-reintegration`，验证 HEAD 固定为上游基线。
2. 在未加入定制能力前运行上游前端构建、翻译检查与 Rust 目标测试，记录真实基线问题。
3. 先写数据备份/恢复的回归测试，再实现迁移锁、路径探测、SQLite 一致性备份、附件快照、manifest、SHA-256、校验与幂等恢复。
4. 按数据合同迁移剪贴板 manager、commands、store、设置和 overlay，并接入上游 paste transaction；增加收藏、命名、置顶、图片和事件隔离测试。
5. 按上游 catalog、download manager、model capabilities 与 transcription backend 重接 FunASR/Sherpa、custom words 与 native hotwords；重新生成 catalog、bindings 和翻译。
6. 移植 `crates/inputia-*`、macOS InputMethod Host、Rime 资源、安装与验证工具；保持 Handy 数据只读导入和 Inputia 独立写入边界。
7. 运行每个 Inputia crate 的测试、Rime schema/core flow、C API、settings、Handy runtime、macOS 非 GUI 自检和可执行的 readiness 层。
8. 运行项目完整门禁：lint、format check、translation parity、前端 build、Rust tests、Playwright smoke 和定制功能回归测试。
9. 对当前真实数据生成只读统计和正式备份；仅在备份副本上执行迁移演练，核对数据库计数、hash、收藏、标题、附件、录音和 Inputia memory。
10. 构建可安装应用，在隔离数据副本上 smoke；全部通过后备份现有应用、替换本机 Handy，并验证真实数据只读打开与核心行为。
11. 整理提交、运行代码审查与最终验证；成功后本地更新 `main`，保留快照分支、数据备份和旧应用恢复点，不自动推送远端。

## 完成条件

- 上游 `fbd4e15f` 的功能和修复全部处于新底座。
- 剪贴板、Inputia、FunASR/Sherpa、热词和纠错验收矩阵全部通过。
- 用户数据和附件统计不下降，备份与恢复演练成功。
- 项目门禁无已知错误；无法运行的系统权限测试有明确证据和替代验证。
- 新应用完成隔离 smoke；切换后仍保留代码、数据和应用三层恢复点。
