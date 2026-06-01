# AGENTS.md

## 工作语言 / Working Language

**强制要求：所有 Agent 在本项目中的工作语言必须使用中文。** 包括但不限于：与用户的对话、代码注释（除非项目已有英文注释风格）、commit message 的描述部分、PR 说明、文档撰写。代码本身（变量名、函数名、类型名）保持英文不变。

**Mandatory: All agents must use Chinese as the working language in this project.** This applies to: conversations with users, code comments (unless the existing codebase uses English), commit message descriptions, PR descriptions, and documentation. Code itself (variable names, function names, type names) remains in English.

---

This is a **fork** of [cjpais/Handy](https://github.com/cjpais/Handy) — a Tauri 2.x desktop speech-to-text app (Rust backend + React/TypeScript frontend). The fork's goal is to add a **clipboard manager** feature by integrating [ropy](https://github.com/StudentWeis/ropy) capabilities. Current phase: frontend-first design (UX/logic before backend implementation).

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

## GitHub Workflow

**PRs/Issues:** Read templates in `.github/` before opening. Every section is mandatory. AI-assisted PRs welcome — disclose tools used.

**Feature freeze:** Upstream Handy is frozen. This fork can add features freely, but upstream PRs need community discussion first.

**Commits:** Conventional prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`). Focus on _why_, not _what_.
