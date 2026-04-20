#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};
use tokio::time::timeout;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};
use tauri_utils::config::Color;

const PIPE_NAME: &str = r"\\.\pipe\Navi";
const MAX_MESSAGE_BYTES: usize = 10 * 1024; // 10 KB
const SOCKET_TIMEOUT_SECS: u64 = 5;

const HOOK_MARKER: &str = "navi";
const HOOK_FLAG: &str = "--hook";
const ENSURE_RUNNING_FLAG: &str = "--ensure-running";

const ALLOWED_KEYS: &[&str] = &[
    "event",
    "title",
    "message",
    "project",
    "type",
    "duration",
    "cwd",
    "session_id",
    "timestamp",
];

fn emit_notification(app: &AppHandle, event: &str, title: &str, message: &str) {
    let mut payload = serde_json::Map::new();
    payload.insert("event".into(), event.into());
    payload.insert("title".into(), title.into());
    payload.insert("message".into(), message.into());
    payload.insert("project".into(), "navi".into());
    if let Err(e) = app.emit("show-notification", payload) {
        eprintln!("[Navi] Failed to emit notification: {e}");
    }
}

// ── Named Pipe server ────────────────────────────────────────────────────────

async fn run_pipe_server(app: AppHandle) {
    loop {
        let server = match ServerOptions::new()
            .pipe_mode(PipeMode::Byte)
            .in_buffer_size(MAX_MESSAGE_BYTES as u32)
            .max_instances(254)
            .create(PIPE_NAME)
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[Navi] Failed to create pipe server: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        if let Err(e) = server.connect().await {
            eprintln!("[Navi] Pipe connect error: {e}");
            continue;
        }

        let app_clone = app.clone();
        tokio::spawn(handle_client(server, app_clone));
    }
}

async fn handle_client(
    mut pipe: tokio::net::windows::named_pipe::NamedPipeServer,
    app: AppHandle,
) {
    let mut buf = Vec::new();

    let read_fut = async {
        let mut chunk = [0u8; 1024];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() > MAX_MESSAGE_BYTES {
                        eprintln!("[Navi] Message too large, dropping");
                        return;
                    }
                }
                Err(_) => break,
            }
        }
    };

    if timeout(Duration::from_secs(SOCKET_TIMEOUT_SECS), read_fut)
        .await
        .is_err()
    {
        eprintln!("[Navi] Socket timeout");
        return;
    }

    if buf.is_empty() {
        return;
    }

    let raw = match std::str::from_utf8(&buf) {
        Ok(s) => s,
        Err(_) => return,
    };

    // "ping" keepalive — no notification
    if raw.trim() == "ping" {
        return;
    }

    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return,
    };

    let sanitized: serde_json::Map<String, serde_json::Value> = match &parsed {
        serde_json::Value::Object(map) => map
            .iter()
            .filter(|(k, _)| ALLOWED_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => return,
    };

    if let Err(e) = app.emit("show-notification", sanitized) {
        eprintln!("[Navi] Failed to emit event: {e}");
    }
}

// ── Window ───────────────────────────────────────────────────────────────────

fn build_window(app: &tauri::App) -> tauri::Result<()> {
    let win_width = 600.0_f64;
    let win_height = 200.0_f64;

    let x = if let Some(monitor) = app.primary_monitor()? {
        let scale = monitor.scale_factor();
        let logical_width = monitor.size().width as f64 / scale;
        ((logical_width - win_width) / 2.0).max(0.0)
    } else {
        0.0
    };

    let win = tauri::WebviewWindowBuilder::new(
        app,
        "main",
        tauri::WebviewUrl::App("island.html".into()),
    )
    .title("Navi")
    .inner_size(win_width, win_height)
    .position(x, 0.0)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .resizable(false)
    .shadow(false)
    .visible(true)
    .devtools(cfg!(debug_assertions))
    .build()?;

    win.set_background_color(Some(Color(0, 0, 0, 0)))?;
    win.set_ignore_cursor_events(true)?;

    // Keep window above everything including fullscreen
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        if let Ok(hwnd) = win.hwnd() {
            let hwnd = hwnd.0 as windows_sys::Win32::Foundation::HWND;
            unsafe {
                SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    Ok(())
}

// ── Tray ─────────────────────────────────────────────────────────────────────

struct TrayState {
    toggle_item: MenuItem<tauri::Wry>,
}

fn toggle_label(enabled: bool) -> &'static str {
    if enabled {
        "Disable Claude Code Hooks"
    } else {
        "Enable Claude Code Hooks"
    }
}

fn refresh_toggle_label(app: &AppHandle, enabled: bool) {
    let state = app.state::<TrayState>();
    if let Err(e) = state.toggle_item.set_text(toggle_label(enabled)) {
        eprintln!("[Navi] Failed to update toggle label: {e}");
    }
}

fn current_hooks_enabled() -> bool {
    match hooks::is_enabled() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[Navi] Failed to read hook state: {e}");
            false
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    const TRAY_ICON_PNG: &[u8] = include_bytes!("tray_icon.png");

    let icon = Image::from_bytes(TRAY_ICON_PNG)
        .unwrap_or_else(|_| Image::new_owned(vec![0, 0, 0, 0], 1, 1));

    let toggle_item = MenuItem::with_id(
        app,
        "toggle_hooks",
        toggle_label(current_hooks_enabled()),
        true,
        None::<&str>,
    )?;
    let test_item = MenuItem::with_id(app, "test", "Test Notification", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle_item, &test_item, &sep, &quit_item])?;

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Navi – Claude Code Notifications")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle_hooks" => {
                let was_enabled = current_hooks_enabled();
                let result = if was_enabled {
                    hooks::disable()
                } else {
                    hooks::enable()
                };
                let (title, message, now_enabled) = match (was_enabled, &result) {
                    (false, Ok(())) => (
                        "Hooks Enabled",
                        "Claude Code will now notify Navi".to_string(),
                        true,
                    ),
                    (true, Ok(())) => (
                        "Hooks Disabled",
                        "Navi hook entries removed from settings".to_string(),
                        false,
                    ),
                    (_, Err(e)) => ("Hook Error", e.to_string(), was_enabled),
                };
                refresh_toggle_label(app, now_enabled);
                emit_notification(app, "info", title, &message);
            }
            "test" => {
                emit_notification(
                    app,
                    "stop",
                    "Task Complete",
                    "Test notification from Navi",
                );
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    app.manage(TrayState { toggle_item });

    Ok(())
}

// ── Hook management ──────────────────────────────────────────────────────────

mod hooks {
    use super::{ENSURE_RUNNING_FLAG, HOOK_FLAG, HOOK_MARKER, MAX_MESSAGE_BYTES, PIPE_NAME};
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::io::{Read, Write};
    use std::path::PathBuf;

    struct HookEvent {
        name: &'static str,
        matcher: Option<&'static str>,
        args: &'static [&'static str],
    }

    const EVENTS: &[HookEvent] = &[
        HookEvent {
            name: "SessionStart",
            matcher: Some("startup"),
            args: &[HOOK_FLAG, ENSURE_RUNNING_FLAG],
        },
        HookEvent {
            name: "Notification",
            matcher: Some("permission_prompt|idle_prompt"),
            args: &[HOOK_FLAG],
        },
        HookEvent {
            name: "Stop",
            matcher: None,
            args: &[HOOK_FLAG],
        },
    ];

    pub fn settings_path() -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or_else(|| "Could not locate home directory".to_string())?;
        Ok(home.join(".claude").join("settings.json"))
    }

    fn read_settings() -> Result<Value, String> {
        let path = settings_path()?;
        if !path.exists() {
            return Ok(Value::Object(Map::new()));
        }
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("Read {}: {e}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        serde_json::from_str(&raw).map_err(|e| format!("Parse {}: {e}", path.display()))
    }

    fn write_settings(value: &Value) -> Result<(), String> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Create {}: {e}", parent.display()))?;
        }
        let pretty = serde_json::to_string_pretty(value)
            .map_err(|e| format!("Serialize settings: {e}"))?;
        fs::write(&path, pretty).map_err(|e| format!("Write {}: {e}", path.display()))
    }

    fn current_exe() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        Ok(exe.to_string_lossy().into_owned())
    }

    fn build_command(exe: &str, extra_args: &[&str]) -> String {
        let mut parts = vec![format!("\"{}\"", exe)];
        for a in extra_args {
            parts.push(a.to_string());
        }
        parts.join(" ")
    }

    fn is_navi_entry(entry: &Value) -> bool {
        entry
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s == HOOK_MARKER)
            .unwrap_or(false)
    }

    pub fn is_enabled() -> Result<bool, String> {
        let settings = read_settings()?;
        let Some(hooks) = settings.get("hooks").and_then(|h| h.as_object()) else {
            return Ok(false);
        };
        for ev in EVENTS {
            let Some(arr) = hooks.get(ev.name).and_then(|v| v.as_array()) else {
                return Ok(false);
            };
            let found = arr.iter().any(|matcher| {
                matcher
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hs| hs.iter().any(is_navi_entry))
                    .unwrap_or(false)
            });
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn enable() -> Result<(), String> {
        let exe = current_exe()?;
        let mut settings = read_settings()?;
        if !settings.is_object() {
            settings = Value::Object(Map::new());
        }
        let root = settings.as_object_mut().unwrap();
        let hooks_val = root
            .entry("hooks".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !hooks_val.is_object() {
            *hooks_val = Value::Object(Map::new());
        }
        let hooks = hooks_val.as_object_mut().unwrap();

        for ev in EVENTS {
            let arr = hooks
                .entry(ev.name.to_string())
                .or_insert_with(|| Value::Array(vec![]));
            if !arr.is_array() {
                *arr = Value::Array(vec![]);
            }
            let list = arr.as_array_mut().unwrap();

            let target_idx = list.iter().position(|g| {
                let existing_matcher = g.get("matcher").and_then(|m| m.as_str());
                existing_matcher == ev.matcher
            });

            let command = build_command(&exe, ev.args);
            let entry = json!({
                "type": "command",
                "command": command,
                "async": true,
                "source": HOOK_MARKER
            });

            match target_idx {
                Some(i) => {
                    let group = &mut list[i];
                    let hooks_arr = group
                        .get_mut("hooks")
                        .and_then(|h| h.as_array_mut())
                        .ok_or_else(|| format!("Malformed hook group in {}", ev.name))?;
                    hooks_arr.retain(|e| !is_navi_entry(e));
                    hooks_arr.push(entry);
                }
                None => {
                    let mut group = Map::new();
                    if let Some(m) = ev.matcher {
                        group.insert("matcher".into(), Value::String(m.to_string()));
                    }
                    group.insert("hooks".into(), Value::Array(vec![entry]));
                    list.push(Value::Object(group));
                }
            }
        }

        write_settings(&settings)
    }

    pub fn disable() -> Result<(), String> {
        let mut settings = read_settings()?;
        let Some(root) = settings.as_object_mut() else {
            return Ok(());
        };
        let Some(hooks_val) = root.get_mut("hooks") else {
            return Ok(());
        };
        let Some(hooks) = hooks_val.as_object_mut() else {
            return Ok(());
        };

        for ev in EVENTS {
            if let Some(Value::Array(list)) = hooks.get_mut(ev.name) {
                for group in list.iter_mut() {
                    if let Some(hooks_arr) =
                        group.get_mut("hooks").and_then(|h| h.as_array_mut())
                    {
                        hooks_arr.retain(|e| !is_navi_entry(e));
                    }
                }
                list.retain(|g| {
                    g.get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false)
                });
                if list.is_empty() {
                    hooks.remove(ev.name);
                }
            }
        }

        write_settings(&settings)
    }

    // ── Hook client subcommand (navi.exe --hook) ──────────────────────────────

    pub fn run_hook_subcommand(args: &[String]) {
        let ensure_running = args.iter().any(|a| a == ENSURE_RUNNING_FLAG);

        let payload = if ensure_running {
            "ping".to_string()
        } else {
            let Some(input) = read_stdin_bounded() else {
                return;
            };
            match build_payload(&input) {
                Some(p) => p,
                None => return,
            }
        };

        if write_pipe_once(&payload).is_ok() {
            return;
        }

        launch_navi();

        // After cold-starting the GUI, retry briefly so the first notification
        // isn't silently lost. --ensure-running doesn't need this (ping is a no-op).
        if ensure_running {
            return;
        }
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if write_pipe_once(&payload).is_ok() {
                return;
            }
        }
    }

    fn read_stdin_bounded() -> Option<String> {
        let mut input = String::new();
        // take() caps at MAX+1 so we can detect overruns without unbounded allocation.
        let limit = (MAX_MESSAGE_BYTES as u64) + 1;
        let stdin = std::io::stdin();
        let mut handle = stdin.lock().take(limit);
        if handle.read_to_string(&mut input).is_err() {
            return None;
        }
        if input.len() > MAX_MESSAGE_BYTES {
            eprintln!("[Navi] stdin exceeds {} bytes, dropping", MAX_MESSAGE_BYTES);
            return None;
        }
        Some(input)
    }

    fn build_payload(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }
        let data: Value = serde_json::from_str(trimmed).ok()?;
        let hook_event = data.get("hook_event_name").and_then(|v| v.as_str())?;
        let not_type = data.get("notification_type").and_then(|v| v.as_str());
        let get_msg = |default: &str| {
            data.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_string()
        };

        let (event, title, message) = match (hook_event, not_type) {
            ("Stop", _) => ("stop", "Task Complete", get_msg("Claude has finished working")),
            ("Notification", Some("permission_prompt")) => (
                "permission",
                "Permission Needed",
                get_msg("Claude needs your permission"),
            ),
            ("Notification", Some("idle_prompt")) => (
                "idle",
                "Waiting for Input",
                get_msg("Claude is waiting for your input"),
            ),
            ("Notification", _) => ("info", "Notification", get_msg("Claude needs attention")),
            _ => return None,
        };

        let cwd = data.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
        let project = PathBuf::from(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .filter(|c| {
                c.is_alphanumeric()
                    || *c == ' '
                    || *c == '-'
                    || *c == '_'
                    || *c == '.'
                    || *c == '('
                    || *c == ')'
                    || *c == '['
                    || *c == ']'
            })
            .collect::<String>()
            .trim()
            .to_string();

        let session_id = data.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
        let timestamp = iso8601_utc_now();

        let out = json!({
            "event": event,
            "title": title,
            "message": message,
            "project": project,
            "cwd": cwd,
            "session_id": session_id,
            "timestamp": timestamp,
        });
        Some(out.to_string())
    }

    // Matches the ISO-8601 format the legacy Node bridge emitted
    // (`new Date().toISOString()`), so the renderer sees the same shape
    // regardless of which producer delivered the payload.
    fn iso8601_utc_now() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let dur = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = dur.as_secs() as i64;
        let millis = dur.subsec_millis();

        // Civil-from-days (Howard Hinnant): convert unix days to Y-M-D.
        let days = total_secs.div_euclid(86_400);
        let secs_of_day = total_secs.rem_euclid(86_400) as u32;
        let (y, m, d) = civil_from_days(days);
        let h = secs_of_day / 3600;
        let min = (secs_of_day % 3600) / 60;
        let s = secs_of_day % 60;

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            y, m, d, h, min, s, millis
        )
    }

    fn civil_from_days(days: i64) -> (i32, u32, u32) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u32;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = y + if m <= 2 { 1 } else { 0 };
        (y as i32, m, d)
    }

    fn write_pipe_once(msg: &str) -> std::io::Result<()> {
        use std::fs::OpenOptions;

        let mut f = OpenOptions::new().write(true).open(PIPE_NAME)?;
        f.write_all(msg.as_bytes())?;
        f.flush()
    }

    fn launch_navi() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let _ = Command::new(exe)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
            .spawn();
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == HOOK_FLAG) {
        if std::env::var("CLAUDE_SUBORDINATE").is_ok() {
            return;
        }
        hooks::run_hook_subcommand(&args);
        return;
    }

    run_app();
}

#[tokio::main]
async fn run_app() {
    tauri::Builder::default()
        .setup(|app| {
            build_window(app)?;
            build_tray(&app.handle())?;
            let handle = app.handle().clone();
            tokio::spawn(run_pipe_server(handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("[Navi] Application failed to start");
}
