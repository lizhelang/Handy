# AGENTS.md

## 工作语言 / Working Language

**强制要求：所有 Agent 在本项目中的工作语言必须使用中文。** 包括但不限于：与用户的对话、代码注释（除非项目已有英文注释风格）、commit message 的描述部分、PR 说明、文档撰写。代码本身（变量名、函数名、类型名）保持英文不变。

**Mandatory: All agents must use Chinese as the working language in this project.** This applies to: conversations with users, code comments (unless the existing codebase uses English), commit message descriptions, PR descriptions, and documentation. Code itself (variable names, function names, type names) remains in English.

---

This is a **fork** of [cjpais/Handy](https://github.com/cjpais/Handy) — a Tauri 2.x desktop speech-to-text app (Rust backend + React/TypeScript frontend). The fork's goal is to add a **clipboard manager** feature by integrating [ropy](https://github.com/StudentWeis/ropy) capabilities. Current phase: frontend-first design (UX/logic before backend implementation).

## Inputia macOS 验证分层

当工作范围在 `macos/InputiaInputMethod` 或 `crates/inputia-*` 时，默认开发验证只运行：

```bash
./macos/InputiaInputMethod/dev-fast.sh
```

`dev-fast.sh` 是候选词、双拼、快捷键、设置 UI、Core/Rime/CAPI 日常迭代的默认入口。它不得打开菜单栏，不得打开 GUI，不得改系统输入源，不得检查公证；只覆盖 build、Rust tests、Swift self-check、Rime probe、router/shortcut self-check。

只有重装、安装脚本、系统目录、TIS enabled/selectable、running host 或设置启动器版本链路发生变化时，才运行：

```bash
./macos/InputiaInputMethod/install-check.sh
```

只有发布前或安装脚本变更后，才运行：

```bash
./macos/InputiaInputMethod/release/full-check.sh
```

`release/full-check.sh` 才允许 pkg/postinstall、公证 readiness、菜单栏 AXPress、TextEdit/Safari/Clipboard GUI smoke。`menu-readiness.sh`、`gui-smoke-readiness.sh` 和真实 GUI smoke 必须显式 opt-in；一次验证周期里菜单栏 AXPress 结果必须通过 `INPUTIA_MENU_READINESS_CACHE_FILE` 缓存，不能反复触碰 `TextInputMenuAgent`。

## Inputia 安装-测试固定流程（macOS）

**为什么以前重装像撞大运——三个必须记住的系统事实：**

1. **macOS 只在用户登录时扫描 `/Library/Input Methods`**（由 `imklaunchagent` 完成）。`TISRegisterInputSource` 在现代 macOS 上返回 `noErr` 但对 System Settings 的输入法列表基本是 no-op。所以"装完立刻能在系统设置里加"是不成立的；**首次安装或 `Info.plist` 输入模式结构变化后，必须注销并重新登录**。（参考：Apple DevForums thread 775526、Squirrel 发布说明"安装完毕请退出当前用户再重新登录"。）
2. **反复调用 register/enable/修复脚本会在当前登录会话的 TIS 缓存里堆出重复的 `Inputia.Hans` 条目**，重复条目会让 `TISSelectInputSource` 返回 `-50 paramErr`。重复条目的正确清理方式是**注销重登录**（会话缓存按磁盘内容重建），不是循环跑 `repair-tis-duplicates.sh` 或 `defaults write` HIToolbox。
3. **判断"装没装对"只能看 `status.sh` 的 CDHash 对比**（`systemMatchesBuild` / `runningMatchesBuild` / `runningVersion`），不能看菜单栏截图、`TISEnableInputSource` 返回码或"所有输入法"列表可见性。

### 场景 A：首次安装，或 bundle id / `Info.plist` 输入模式结构变化

```bash
cd macos/InputiaInputMethod
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./dev-fast.sh          # 1. 本地验证必须先通过
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./install-system.sh    # 2. 安装（会弹管理员授权）
```

3. **注销 macOS 并重新登录**（必须，无 API 可替代）。
4. System Settings > 键盘 > 文字输入 > 输入法：先删掉所有重复的 Inputia，再通过 `+` > 简体中文 > Inputia 添加一次。
5. 验证（见下方"验收命令"）。

### 场景 B：日常代码更新（bundle id 与输入模式结构不变）

```bash
cd macos/InputiaInputMethod
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./dev-fast.sh
INPUTIA_RUST_TOOLCHAIN=1.96.0 ./install-system.sh    # 会 killall 旧 Host 并热替换 bundle
```

然后**切到其他输入法再切回 Inputia**（触发系统用新二进制重启 Host），不需要注销。若切回后无响应，说明系统仍握着旧进程记录，此时才需要注销重登录。

### 验收命令（每次安装后都要跑，输出为准）

```bash
./status.sh                                # systemMatchesBuild=true 且 runningMatchesBuild=true
INPUTIA_APP='/Library/Input Methods/InputiaInputMethod.app' \
INPUTIA_TIS_REQUIRE_APP_MATCH=1 \
./build/inputia-tis-tool --dump            # 精确一条 Inputia.Hans，enabled=true selectable=true
./build/inputia-tis-tool --dump-current-input-source   # 选中后应显示 Inputia.Hans
```

不能宣布"修好了"的完整验收标准见 `handoff.md`（唯一 Hans、可选择、系统启动的是新版本 Host、普通 App 能输入）。

### 禁止事项

- 禁止在一次安装周期里多次调用 `--register-input-source`（会制造重复条目，`install-system.sh` 已收敛为刷新后单次注册）。
- 禁止把 `defaults write com.apple.HIToolbox ...` 当正式修复；只读诊断可以。
- 禁止直接手工启动 `/Library/Input Methods/InputiaInputMethod.app/Contents/MacOS/InputiaInputMethod` 当作"运行验证"；Host 必须由系统在选择输入法时启动。
- 禁止为了测试反复开 TextEdit/Safari 抢用户焦点；GUI smoke 必须显式 opt-in。

## Quick Reference

```bash
bun install                                          # Install deps
bun run tauri dev                                    # Full app dev (macOS cmake fix: CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev)
bun run dev                                          # Frontend only (Vite, port 1420)
bun run lint                                         # ESLint (enforces i18n — no hardcoded JSX strings)
bun run format                                       # Prettier + cargo fmt
bun run format:check                                 # Check formatting (CI runs this)
bun run build                                        # tsc + vite build
bun run tauri build                                  # Production binary
bun run test:playwright                              # E2E tests (needs dev server on :1420)
bun run check:translations                           # Translation key consistency (CI runs this)
```

**Required model setup** (won't compile without it):

```bash
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

## CI Gate — Must Pass Before PR

| Check        | Command                        | Scope                                                     |
| ------------ | ------------------------------ | --------------------------------------------------------- |
| ESLint       | `bun run lint`                 | `src/**` — i18next/no-literal-string enforced             |
| Prettier     | `bun run format:check`         | All files, `endOfLine: lf`                                |
| Translations | `bun run check:translations`   | Key parity across all locale dirs                         |
| Rust tests   | `cargo test` (in `src-tauri/`) | Uses `transcription_mock.rs` in CI to skip whisper/Vulkan |
| Playwright   | `bun run test:playwright`      | Smoke tests against Vite dev server                       |

## Architecture

```
src-tauri/src/
├── lib.rs                    # Tauri setup, manager init, command registration
├── main.rs                   # Entry point, CLI parsing
├── managers/                 # Core business logic (Audio, Model, Transcription, History)
│   └── transcription_mock.rs # CI-only mock — CI copies this over transcription.rs
├── commands/                 # Tauri command handlers (audio, models, transcription, history)
├── audio_toolkit/            # Low-level audio: device enum, recording, resampling, VAD
├── cli.rs                    # clap derive CLI definitions
├── shortcut/                 # Global keyboard shortcuts (rdev)
├── settings.rs               # Settings management (tauri-plugin-store)
├── overlay.rs                # Recording overlay window (platform-specific)
├── signal_handle.rs          # send_transcription_input() — shared between CLI and signal handlers
└── utils.rs                  # Platform detection helpers

src/
├── main.tsx                  # App entry
├── App.tsx                   # Main component + onboarding flow
├── bindings.ts               # Auto-generated Tauri type bindings (tauri-specta) — DO NOT EDIT
├── overlay/                  # Recording overlay window entry (separate Vite entry point)
├── components/               # React UI (settings/, model-selector/, onboarding/, overlay/, shared/)
├── hooks/useSettings.ts      # Settings state hook
├── stores/settingsStore.ts   # Zustand store
├── i18n/                     # i18next setup + locale files
└── lib/types.ts              # Shared TypeScript types
```

**Vite has three entry points:** `index.html` (main app), `src/overlay/index.html` (recording overlay), and `src/overlay/clipboard/index.html` (clipboard overlay). Configured in `vite.config.ts` `build.rollupOptions.input`.

## Key Patterns

- **Manager pattern:** Audio, Model, Transcription, History managers initialized at startup via Tauri state
- **Command-Event:** Frontend → Backend via `#[tauri::command]`; Backend → Frontend via events
- **Pipeline:** Audio → VAD (Silero) → Whisper/Parakeet → Text → Clipboard/Paste
- **State flow:** Zustand → Tauri Command → Rust State → Persistence (tauri-plugin-store)
- **Bindings:** `src/bindings.ts` is auto-generated by tauri-specta. Never edit manually; regenerate by running the app.
- **Shared state:** Use `Arc<Mutex<T>>` for managers. Error handling: `anyhow::Error` with descriptive context.

## i18n Rules

All user-facing strings must use i18next. ESLint enforces this.

1. Add key to `src/i18n/locales/en/translation.json`
2. Use: `const { t } = useTranslation(); t('key.path')`
3. Run `bun run check:translations` to verify key parity across locales
4. For new languages: add folder + `translation.json`, register in `src/i18n/languages.ts`

## Code Style

**Rust:** `cargo fmt` (edition 2021), `cargo clippy`, explicit error handling (no `unwrap` in prod), doc comments on public APIs.

**TypeScript/React:** Strict TS (no `any`), functional components, Tailwind CSS, path alias `@/` → `./src/`, Zod for validation, `useCallback` for stable refs, named imports preferred.

**Formatting:** Prettier with `endOfLine: lf`. Rustfmt edition 2021.

## CLI Flags

| Flag                     | Description                                                  |
| ------------------------ | ------------------------------------------------------------ |
| `--toggle-transcription` | Toggle recording (remote control via single-instance plugin) |
| `--toggle-post-process`  | Toggle recording with post-processing                        |
| `--cancel`               | Cancel current operation                                     |
| `--start-hidden`         | Launch minimized to tray                                     |
| `--no-tray`              | No system tray (closing window quits)                        |
| `--debug`                | Verbose Trace logging                                        |

Flags are runtime overrides, not persisted. Remote control: second instance sends args via `tauri_plugin_single_instance`, then exits.

## Platform Quirks

- **macOS:** Metal acceleration, accessibility permissions required for shortcuts. `CMAKE_POLICY_VERSION_MINIMUM=3.5` may be needed for cmake errors.
- **Windows:** Vulkan acceleration, code signing via Azure trusted-signing-cli.
- **Linux:** OpenBLAS + Vulkan. Overlay uses GTK layer shell (disable with `HANDY_NO_GTK_LAYER_SHELL=1`). Wayland needs `wtype` or `dotool` for text input.

## Debug Mode

`Cmd+Shift+D` (macOS) / `Ctrl+Shift+D` (Windows/Linux) — opens debug menu with app data directory path, model info, etc.

See the [Troubleshooting](README.md#troubleshooting) section in README.md.

## GitHub Workflow

**PRs/Issues:** Read templates in `.github/` before opening. Every section is mandatory. AI-assisted PRs welcome — disclose tools used.

**Feature freeze:** Upstream Handy is frozen. This fork can add features freely, but upstream PRs need community discussion first.

**AI coding assistants:** Before opening any PR, issue, or discussion in this repo, read the relevant template file and follow it strictly. That includes sections that look "ceremonial" — checklists, AI Assistance disclosures, and "Human Written Description". A generic Summary/Test-plan layout is not acceptable.

- **Opening a PR:** Read [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md). Every section listed there is mandatory. If a section requires a human-written paragraph (e.g. "Human Written Description"), leave a clear TODO placeholder and ask the human contributor to fill it in — do not invent their voice.
- **Opening an issue:** Read [`.github/ISSUE_TEMPLATE/`](.github/ISSUE_TEMPLATE/). Blank issues are disabled; pick the right template (`bug_report.md` for bugs). Feature requests do not belong in issues — they go to [Discussions](https://github.com/cjpais/Handy/discussions) (see `.github/ISSUE_TEMPLATE/config.yml`).
- **Proposing a feature:** Handy is under a feature freeze. New features require community support gathered in [Discussions](https://github.com/cjpais/Handy/discussions) before any PR is opened — see the PR template's "Community Feedback" section.
- **Translations:** Follow [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).
- **Full contributor workflow:** [CONTRIBUTING.md](CONTRIBUTING.md).

**Commits:** Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`). Focus the message on _why_, not _what_.
