# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build, run, and validation commands

- Install Tauri CLI (once): `cargo install tauri-cli --version "^2"`
- Check Rust backend compiles: `cargo check --manifest-path src-tauri/Cargo.toml`
- Run app in dev mode: `cargo tauri dev`
- Build distributable (`navi.exe`): `cargo tauri build`
- Run backend binary directly: `cargo run --manifest-path src-tauri/Cargo.toml --bin navi`
- Run Rust tests: `cargo test --manifest-path src-tauri/Cargo.toml`
- Format/lint:
  - `cargo fmt --manifest-path src-tauri/Cargo.toml`
  - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`

## Architecture overview

Navi is a Windows desktop overlay that shows Claude Code notifications in a Dynamic-Island-style pill. It ships as a **single self-contained `navi.exe`** — a Tauri (Rust) runtime with an HTML/CSS/JS renderer. The same binary also serves as the Claude Code hook client via the `--hook` subcommand, so no Node runtime or external scripts are required.

### End-to-end event flow

1. Claude Code fires a configured hook (`SessionStart`, `Notification`, or `Stop`) and invokes `navi.exe --hook`, passing the hook JSON on stdin.
2. The `--hook` subprocess reads stdin, maps the event to a notification payload, and writes it to the Windows named pipe `\\.\pipe\Navi`. If the pipe is unreachable it launches the main `navi.exe` and exits.
3. The main `navi.exe` process hosts the named-pipe server (`src-tauri/src/main.rs`), enforces size/timeout/field allowlist, and emits the Tauri event `show-notification`.
4. `src/renderer/island.js` listens for `show-notification`, queues entries, drives the pill animation, and plays sounds.

### Key runtime pieces

- **Rust backend (`src-tauri/src/main.rs`)** — single binary with two entry modes:
  - **GUI mode** (default, no args): creates the transparent, always-on-top, click-through overlay window; creates the tray icon/menu; runs the async named-pipe server; sanitizes incoming JSON by `ALLOWED_KEYS` before emitting to the renderer.
  - **Hook mode** (`--hook` [`--ensure-running`]): synchronous, no GUI. Reads stdin (or sends `ping` for `--ensure-running`), writes to the named pipe, launches the GUI process if the pipe is not yet bound, then exits.
  - **Hook management** (`hooks` module): reads/writes `~/.claude/settings.json` to enable/disable Navi's entries under `SessionStart` / `Notification` / `Stop`. Every entry carries `"source": "navi"` so Disable only touches Navi-owned entries.

- **Renderer (`src/renderer/`)**
  - `island.html` — single overlay DOM.
  - `island.css` — pill animation, type-specific icon glow.
  - `island.js` — subscribes to `window.__TAURI__.event.listen('show-notification', …)`, maps events to UI states, manages the queue, and plays Web Audio cues.
  - `sounds/` — bundled audio assets.

- **Tray menu** (built in `main.rs`)
  - `Enable/Disable Claude Code Hooks` — toggles Navi's entries in the user-level `~/.claude/settings.json`. Label reflects current state; feedback is shown via a pill notification.
  - `Test Notification` — emits a sample "Task Complete" event to exercise the pipeline.
  - `Quit` — exits the app cleanly.

### Important config coupling

- `src-tauri/tauri.conf.json` sets `frontendDist` to `../src/renderer` and bundles `../src/renderer/sounds/*`.
- The named-pipe name and payload schema must stay aligned across:
  - `hooks` module in `src-tauri/src/main.rs` (writer side, `--hook`)
  - named-pipe server in `src-tauri/src/main.rs` (reader side, GUI)
  - `src/renderer/island.js` (event consumer)
- `ALLOWED_KEYS` in `main.rs` gates which payload fields reach the renderer; adding a new field requires updating both the hook writer and this list.
- Hook entries written to `~/.claude/settings.json` reference `std::env::current_exe()` at enable time. Moving `navi.exe` requires toggling Disable → Enable once to refresh the path.
- The tray icon PNG is embedded via `include_bytes!("tray_icon.png")` and must stay that way. Reading it at runtime via `env!("CARGO_MANIFEST_DIR")` or a relative path will silently fall back to a 1×1 transparent icon on any machine other than the one that built the exe — the tray entry is created but invisible, and the bug only manifests after distribution.

### Legacy reference

The repository previously used an Electron runtime with a Node hook bridge (`hooks/navi-hook.js`) writing to the same named pipe. That bridge has been removed — the tray menu's Enable Hooks writes `navi.exe --hook` directly into `settings.json`.
