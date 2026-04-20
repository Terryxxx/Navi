# Navi

A Windows desktop overlay that shows **Claude Code** notifications in a Dynamic-Island-style pill at the top of your screen. Single self-contained `navi.exe` — no Node runtime, no installer, no background services beyond the app itself.

## Features

- **Floating pill** at the top-center of your primary monitor: always-on-top, transparent, click-through (won't block the window beneath it).
- **Task complete**, **permission needed**, and **idle prompt** notifications mapped from Claude Code hook events, with per-type icons, glow, and sound.
- **One-click hook setup** — right-click the tray icon and choose *Enable Claude Code Hooks*. Navi writes the required entries into `~/.claude/settings.json` for you. Disable cleanly removes only Navi's entries.
- **Zero dependencies at runtime** — the same `navi.exe` serves both the GUI and the hook client (`navi.exe --hook`).

## Install

1. Download or build `navi.exe` (see *Build from source* below).
2. Put it anywhere you like (e.g. `C:\Tools\Navi\navi.exe`).
3. Double-click to run. A tray icon appears; the overlay stays hidden until a notification arrives.

> Moving the `.exe` later? Open the tray menu and toggle *Disable → Enable* so the path in `~/.claude/settings.json` refreshes.

## Enable Claude Code hooks

1. Start `navi.exe`.
2. Right-click the Navi tray icon → **Enable Claude Code Hooks**.
3. A pill confirms the update. Claude Code will now notify Navi on `SessionStart`, `Notification`, and `Stop`.

To turn it off, right-click the tray icon → **Disable Claude Code Hooks**. This removes only the Navi-owned entries (those tagged with `"source": "navi"`); any other hooks you or other plugins configured are left untouched.

### What gets written

Enable adds the following (abbreviated) to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [{ "matcher": "startup", "hooks": [
      { "type": "command",
        "command": "\"C:/path/to/navi.exe\" --hook --ensure-running",
        "async": true, "source": "navi" }
    ]}],
    "Notification":  [{ "matcher": "permission_prompt|idle_prompt", "hooks": [
      { "type": "command",
        "command": "\"C:/path/to/navi.exe\" --hook",
        "async": true, "source": "navi" }
    ]}],
    "Stop":          [{ "hooks": [
      { "type": "command",
        "command": "\"C:/path/to/navi.exe\" --hook",
        "async": true, "source": "navi" }
    ]}]
  }
}
```

## Tray menu

| Item | Action |
|------|--------|
| **Enable / Disable Claude Code Hooks** | Toggles Navi's entries in `~/.claude/settings.json`. |
| **Test Notification** | Fires a sample "Task Complete" pill. |
| **Quit** | Exits the app. |

## How it works

```
Claude Code hook
      │ (stdin JSON)
      ▼
navi.exe --hook  ───►  \\.\pipe\Navi  ───►  navi.exe (GUI)  ───►  overlay pill
```

- The main `navi.exe` process hosts an async named-pipe server on `\\.\pipe\Navi`.
- When Claude Code triggers a hook, it launches `navi.exe --hook`. That short-lived subprocess reads the hook JSON, writes a compact notification payload to the pipe, and exits. If the GUI isn't running, it spawns it and exits.
- Payloads are limited to 10 KB and filtered through an allowlist before being forwarded to the renderer.

## Build from source

Prerequisites: Rust stable (`rustup`), Visual Studio 2022 Build Tools with C++ workload, WebView2 runtime (shipped with Windows 11).

```bash
cargo install tauri-cli --version "^2"       # once
cargo tauri build                            # produces src-tauri/target/release/navi.exe
```

Dev loop:

```bash
cargo tauri dev
```

## Repository layout

```
navi/
├── src-tauri/           # Rust backend (Tauri) — GUI + pipe server + hook client
│   └── src/main.rs
└── src/renderer/        # HTML/CSS/JS overlay
    ├── island.html
    ├── island.css
    ├── island.js
    └── sounds/
```

## Troubleshooting

**Tray icon missing after double-clicking `navi.exe`**

1. Expand the hidden-icons area (`^` arrow) on the taskbar — Windows 11 tucks new apps there by default.
2. Settings → Personalization → Taskbar → *Other system tray icons* → toggle **Navi** on.
3. Task Manager → Details: confirm `navi.exe` is actually running. If it isn't, see the next item.

**`navi.exe` exits immediately or nothing happens on double-click**

- Install the **WebView2 Runtime** (required by Tauri): https://developer.microsoft.com/microsoft-edge/webview2/. Windows 11 ships it; older Windows 10 installs may not.
- Windows **SmartScreen** may silently block unsigned binaries. Right-click the exe → Properties → *Unblock*, then re-run. First launch may show "Windows protected your PC" — click *More info → Run anyway*.
- Don't drop it into `Program Files` and run without admin — Navi writes to `~/.claude/settings.json` when you toggle hooks, and UAC-restricted contexts can interfere.

**Hooks enabled but no pill shows up**

- Confirm `~/.claude/settings.json` contains entries with `"source": "navi"` pointing to your current `navi.exe` path.
- If you moved `navi.exe` after enabling, re-toggle *Disable → Enable* so the stored path refreshes.
- Try tray → *Test Notification*; if that works but real Claude Code events don't, the issue is on the hook side — check Claude Code's hook logs.

## License

See `LICENSE` (if present) or the repository for terms.
