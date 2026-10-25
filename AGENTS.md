# Inputia 开发规范（AGENTS.md）

## 工作语言与产品名称

**本项目工作语言为中文，当前产品统一称为 Inputia。** 对话、文档、提交描述和 PR 说明使用中文；代码标识保持英文，已有英文注释风格可延续。不得用历史产品名称称呼当前产品。

本目录维护当前产品源码；产品主分支为 `codex/unified-input-system`。本机主工作树位于 `/Users/lzl/FILE/github/Handy-unified-input-system`，隔离修复工作树会随任务更换（2026-10-23 起为 `/Users/lzl/FILE/github/Handy-prediction-ngram`，分支 `codex/prediction-ngram`，基于已安装 1.1.30 的来源分支 `codex/inputia-return-commit`，1.1.31 起的联想版本都从这里构建），以 `git worktree list` 和已安装制品记录的来源提交为准，不要沿用文档里的旧路径。先读 [README.md](README.md)，并核对实际 Git 状态和安装制品来源。`/Users/lzl/FILE/github/Handy` 是历史基线，不能用其旧版本配置覆盖当前产品。

真实远程 URL、版权和第三方许可证、历史证据、兼容 bundle ID、协议字段及已有数据路径保留原值；产品显示名称清理不授权破坏这些身份或用户数据。需要迁移时单独设计并验证，不能宣称仓库已无所有历史名称。

以下保留本仓库有效的开发、国际化和协作规范。

## 普通语音兼容性

普通全局口述默认在实际插入时使用当前前台应用的光标，讲话期间切换应用不使会话失效。设置可选择“开始录音时的应用”：固定开始时的进程实例，允许中途切走再回来，但插入前必须回到该实例；不得自动切换应用，也不声称固定原窗口或字段。模式在每次开始录音时冻结，不把完整 AX 字段、Host 租约、历史保存或投影成功设为准入前提。显式输入法会话和历史插入仍遵循精确目标合同，两者不得在失败后自动互相降级。开始与派发时的安全输入检查、Inputia 自身排除、取消、系统权限、剪贴板新复制优先及可能已派发不重试必须保留。自动 Enter 需要独立证明；正常 `dispatched` 不能显示成故障。详见[普通口述边界](docs/architecture/ordinary-dictation.md)。

## 输入控制恢复

权限检查与原生组件恢复按 worker、权限代数和关闭代数归属发布结果；旧检查、迟到错误或旧快捷键句柄不得关闭新一轮恢复。公开输入准入只使用新鲜的真实权限证明，原生构造许可不能冒充对外可输入状态。恢复必须等旧原生工作确实退出及退休回执完成，不因超时释放单飞位置；仅对白名单暂时故障有界重试，维护、明确停用、真实拒权和未知失败保持关闭，不重放旧按键、录音或输出。健康记录须区分恢复中、退休中、已停用与需处理，保留固定原因，界面不得在终态继续声称正在重建。

## 基础候选显示

基础 Rime 候选显示不依赖语音或增强内容服务租约。增强缓存到期或权限代数变化时，必须撤销旧增强结果及选择映射，再按当前活动输入会话恢复本次成功清理得到的基础候选；不得复用旧增强列表。安全输入、维护和失去原输入会话的边界仍须关闭显示。共享候选窗只能由当前活动控制器更新，旧控制器的同步或异步清理不得隐藏新控制器的窗口。

空输入联想仅由无修饰空格完成选词上屏触发，在原候选窗内连续交接，最多展示 5 秒；Esc、新输入或输入会话变化结束该轮。后台续租不得延长展示时间或重新弹出已关闭推荐，展示计时也不得延长内容授权租约。普通拼音候选不受联想计时限制。

等待联想时使用空白原框，不绘制省略号。后续推荐最多 3 项、不显示编号（2026-10-23 用户要求）：无修饰空格选择当前高亮项；左右方向键与 Tab（Shift+Tab 反向）在推荐间循环移动高亮，不移动光标、不结束本轮，且先于前文与字段边界处理；数字键不选择推荐，直接结束本轮并照常输入；只有一项时方向键与 Tab 照常交给宿主。按键边界与实际选择必须使用同一分类，复用原准入与一次插入流程。不要拦截 Return、修饰快捷键或把尚未显示的旧内容当作可选项。普通拼音候选的数字选择不变。

用户在控制中心选择来源并确认回填后，语音与键入历史才按联想词表切成"上一个词→下一个词"计入个人层；不得自动读取或回填历史。历史词对只服务上屏后联想，不参与拼音候选重排，上屏后联想也不读取导入整句。展示含历史贡献的推荐前，须按次数分批核对至少一条现存来源（来源类型开放、应用未被排除、记录存在且修订一致，语音与剪贴板另核对源库身份与源记录修订）；途中发现的失效来源立即撤销其全部学习并推进学习世代使本轮作废。来源失效、重新回填、遗忘与清空都须同步删除对应词对与汇总。撤销只删除学习库中的派生数据，不删除、不修改任何历史原文、录音或剪贴记录。

上屏后联想的问号、感叹号只来自公共二元表中"词后紧跟标点"的统计与当前分句的线索（2026-10-26 用户要求加问号与表情）：以疑问语气词结尾（排除"在吃饭呢"式陈述和建议的"吧"）时问号可排第一；疑问词、正反问、选择问线索的问号最多排在第一个续写之后；转述与宾语从句、任指、条件、"都/也"照应、交给对方决定等用法不算问句；感叹号只在感叹线索下出现。公共表从不给句号、逗号；每轮最多展示一个标点；标点概率不随个人证据总量折减。标点跟随同一段文字里汉字后已用的全角或半角，没有时按输入法默认给半角。表情只在前文以表情词完整结尾时给一个，固定放在第二位，使三项展示与九项准入重查前缀一致；否定词之后不给，单字表情词须跟在程度词后，遗忘与拒绝照常生效。改动这些规则须先在 `manifest.json` 记录的开发集上调参，测试集只用于一次判定。

普通物理输入和宿主复制粘贴等命令不能被异步增强选词暂存或等待。新的打字或命令到来时取消尚未完成的增强选择，按当前原始 IMK 事件继续处理；原选择键松开不取消选择，迟到的准入回调不得插入。不得靠放宽字段或权限证明补投已失效输入。

推荐成功上屏后继续下一轮联想，重新计时五秒；未成功插入不能续轮，旧计时器、排队旧按键和按住连发不得选择后来的列表。较长前文仅保存在内存，限定本输入法已确认上屏、同服务与字段证明、光标连续，最多 512 个 Unicode 标量；查询与准入使用同一完整前文，学习反馈仍最多 64 个标量。真实隐私撤销清理自有前文；同完整版本的连接握手仍清共享缓存和目标许可，但不得冒充新的隐私撤销清掉本次输入。

输入期间完全本地处理。模型或词库资源可事先联网取得并验证，实时按键链不得调用远程模型或下载资源。模型质量须用保留反例的公开测试语料验证；单独模型快、编译通过或合成边界回归通过均不等于实际输入体验已改善。

本次已确认上屏的短上下文与增强缓存分开管理；目标延迟捕获后可继续本轮只读推荐，但不得补写之前缺少来源证明的学习或历史。缓存到期应撤销旧目标及推荐，不应取消仍有效的本轮中性窗口。`target_query_busy` 不等于策略撤销；仅当前有效请求可在原期限内有限重试读取，反馈与插入不得自动重放。

## Development Commands

**Prerequisites:**

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager

**Core Development:**

```bash
# Install dependencies
bun install

# Run in development mode
bun run tauri dev
# If cmake error on macOS:
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev

# Build for production
bun run tauri build

# Frontend only development
bun run dev        # Start Vite dev server
bun run build      # Build frontend (TypeScript + Vite)
bun run preview    # Preview built frontend
```

**Linting and Formatting (run before committing):**

```bash
bun run lint              # ESLint for frontend
bun run lint:fix          # ESLint with auto-fix
bun run format            # Prettier + cargo fmt
bun run format:check      # Check formatting without changes
bun run format:frontend   # Prettier only
bun run format:backend    # cargo fmt only
```

**Model Setup (Required for Development):**

```bash
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

For detailed platform-specific build setup, see [BUILD.md](BUILD.md).

## Architecture Overview

Inputia 使用 Tauri 2.x（Rust 后端与 React/TypeScript 前端），整合语音输入、剪贴板历史、知识检索及 macOS 原生输入法。完整模块边界与当前支持范围见 [README.md](README.md)。

### Backend Structure (src-tauri/src/)

- `lib.rs` - Main entry point, Tauri setup, manager initialization
- `managers/` - Core business logic:
  - `audio.rs` - Audio recording and device management
  - `model.rs` - Model downloading and management
  - `transcription.rs` - Speech-to-text processing pipeline
  - `history.rs` - Transcription history storage
- `audio_toolkit/` - Low-level audio processing:
  - `audio/` - Device enumeration, recording, resampling
  - `vad/` - Voice Activity Detection (Silero VAD)
- `commands/` - Tauri command handlers for frontend communication
- `cli.rs` - CLI argument definitions (clap derive)
- `shortcut.rs` - Global keyboard shortcut handling
- `settings.rs` - Application settings management
- `overlay.rs` - Recording overlay window (platform-specific)
- `signal_handle.rs` - `send_transcription_input()` reusable function
- `utils.rs` - Platform detection helpers

### Frontend Structure (src/)

- `App.tsx` - Main component with onboarding flow
- `components/` - React UI components:
  - `settings/` - Settings UI
  - `model-selector/` - Model management interface
  - `onboarding/` - First-run experience
  - `overlay/` - Recording overlay UI
  - `update-checker/` - App update notifications
  - `shared/`, `ui/`, `icons/`, `footer/` - Shared components
- `hooks/useSettings.ts` - Settings state management hook
- `stores/settingsStore.ts` - Zustand store for settings
- `bindings.ts` - Auto-generated Tauri type bindings (via tauri-specta)
- `overlay/` - Recording overlay window entry point
- `lib/types.ts` - Shared TypeScript type definitions

### Key Architecture Patterns

**Manager Pattern:** Core functionality organized into managers (Audio, Model, Transcription) initialized at startup and managed via Tauri state.

**Command-Event Architecture:** Frontend → Backend via Tauri commands; Backend → Frontend via events.

**Pipeline Processing:** Audio → VAD → Whisper/Parakeet → Text output → Clipboard/Paste

**State Flow:** Zustand → Tauri Command → Rust State → Persistence (tauri-plugin-store)

### Technology Stack

**Core Libraries:**

- `transcribe-cpp` - Local Whisper-family inference (GGML/GGUF) with GPU acceleration
- `transcribe-rs` - ONNX speech recognition (Parakeet, Moonshine, SenseVoice, etc.)
- `cpal` - Cross-platform audio I/O
- `vad-rs` - Voice Activity Detection
- `rdev` - Global keyboard shortcuts
- `rubato` - Audio resampling
- `rodio` - Audio playback for feedback sounds

### Application Flow

1. **Initialization:** App starts minimized to tray, loads settings, initializes managers
2. **Model Setup:** First-run downloads preferred Whisper model (Small/Medium/Turbo/Large)
3. **Recording:** Global shortcut triggers audio recording with VAD filtering
4. **Processing:** Audio sent to Whisper model for transcription
5. **Output:** Text pasted to active application via system clipboard

### Settings System

Settings are stored using Tauri's store plugin with reactive updates:

- Keyboard shortcuts (configurable, supports push-to-talk)
- Audio devices (microphone/output selection)
- Model preferences (Small/Medium/Turbo/Large Whisper variants)
- Audio feedback and translation options

### Single Instance Architecture

The app enforces single instance behavior — launching when already running brings the settings window to front rather than creating a new process. Remote control flags (`--toggle-transcription`, etc.) work by launching a second instance that sends args to the running instance via `tauri_plugin_single_instance`, then exits.

## Internationalization (i18n)

All user-facing strings must use i18next translations. ESLint enforces this (no hardcoded strings in JSX).

**Adding new text:**

1. Add key to `src/i18n/locales/en/translation.json`
2. Use in component: `const { t } = useTranslation(); t('key.path')`

**File structure:**

```
src/i18n/
├── index.ts           # i18n setup
├── languages.ts       # Language metadata
└── locales/
    ├── en/translation.json  # English (source)
    ├── de/, es/, fr/, ja/, ru/, zh/, ...
    └── ...
```

For translation contribution guidelines, see [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).

## Code Style

**Rust:**

- Run `cargo fmt` and `cargo clippy` before committing
- Handle errors explicitly (avoid unwrap in production)
- Use descriptive names, add doc comments for public APIs

**TypeScript/React:**

- Strict TypeScript, avoid `any` types
- Functional components with hooks
- Tailwind CSS for styling
- Path aliases: `@/` → `./src/`

## CLI Parameters

Inputia 保留以下进程控制参数。实际可执行文件和成套安装路径以当前发布清单为准；支持范围及原生验收按平台分别记录。

**Implementation:** `cli.rs` (definitions), `main.rs` (parsing), `lib.rs` (applying), `signal_handle.rs` (shared logic)

| Flag                     | Description                                                |
| ------------------------ | ---------------------------------------------------------- |
| `--toggle-transcription` | Toggle recording on/off on a running instance              |
| `--toggle-post-process`  | Toggle recording with post-processing on/off               |
| `--cancel`               | Cancel the current operation on a running instance         |
| `--start-hidden`         | Launch without showing the main window (tray icon visible) |
| `--no-tray`              | Launch without system tray (closing window quits the app)  |
| `--debug`                | Enable debug mode with verbose (Trace) logging             |

**Key design decisions:**

- CLI flags are runtime-only overrides — they do NOT modify persisted settings
- Remote control flags work via `tauri_plugin_single_instance`: second instance sends args, then exits
- `send_transcription_input()` in `signal_handle.rs` is shared between signal handlers and CLI

## Debug Mode

Access debug features: `Cmd+Shift+D` (macOS) or `Ctrl+Shift+D` (Windows/Linux)

## Platform Notes

- **macOS**: Metal acceleration, accessibility permissions required for keyboard shortcuts
- **Windows**: Vulkan acceleration, code signing
- **Linux**: OpenBLAS + Vulkan, limited Wayland support, overlay uses GTK layer shell (disable with `HANDY_NO_GTK_LAYER_SHELL=1`)
- **Nix/NixOS**: the Nix package sets `HANDY_DISABLE_UPDATER=1` to force-disable the self-updater at runtime without touching the persisted setting (self-update can't work against an immutable `/nix/store`)

## Troubleshooting

See the [Troubleshooting](README.md#troubleshooting) section in README.md.

## GitHub workflow for AI coding assistants

**MANDATORY. Before opening any PR, issue, or discussion in this repo: you MUST read the relevant template file and follow it strictly.** That includes sections that look "ceremonial" — checklists, AI Assistance disclosures, "Human Written Description". A generic Summary/Test-plan layout is not acceptable.

- **Opening a PR:** Read [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md). Every section listed there is mandatory. If a section requires a human-written paragraph (e.g. "Human Written Description"), leave a clear TODO placeholder and ask the human contributor to fill it in — do not invent their voice.
- **Opening an issue:** Read [`.github/ISSUE_TEMPLATE/`](.github/ISSUE_TEMPLATE/). Blank issues are disabled; pick the right template (`bug_report.md` for bugs). Feature requests do not belong in issues — they go to [Discussions](https://github.com/cjpais/Handy/discussions) (see `.github/ISSUE_TEMPLATE/config.yml`).
- **功能提议：** Inputia 分支可以开发已授权的新功能。向上游提交时，遵循上游功能冻结要求，先在 [Discussions](https://github.com/cjpais/Handy/discussions) 收集支持，并填写 PR 模板的 “Community Feedback” 章节。
- **Translations:** Follow [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).
- **Full contributor workflow:** [CONTRIBUTING.md](CONTRIBUTING.md).

**Commits:** Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`). 描述部分使用中文，解释变更原因。提交授权不等于推送或发布授权。
