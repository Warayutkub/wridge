// wridge desktop app: one window (sidebar, Overview, Settings and the pages) and a tray icon around the
// wridge server. Phase 1 runs the Node server from legacy-node/ as a child process; the Rust server replaces
// it piece by piece later. The app owns the process: it captures the log (no file lock), restarts it when the
// server asks to (exit code 75), and stops it when the app quits.
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

const WS_PORT: u16 = 3155;
const HTTP_PORT: u16 = 3156;
/// The server exits with this code when its own dashboard asks for a restart; the app starts it again.
const RESTART_CODE: i32 = 75;
const LOG_LINES: usize = 600;

#[derive(Default)]
struct Server {
    child: Option<Child>,
    started_at: Option<u64>,
    /// Bumped on every start and stop: a watcher only acts on the run it was started for.
    run: u64,
    /// Exit code of the last run when it ended on its own (None while running or after a stop from here).
    last_exit: Option<i32>,
    stopped_by_user: bool,
    start_error: Option<String>,
    /// Which server the current (or last) run is: "rust" or "node".
    engine: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Prefs {
    /// Start the server when wridge opens.
    auto_start_server: bool,
    /// "rust" (server/app, no Node.js needed) or "node" (legacy-node, kept as a fallback).
    server_engine: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { auto_start_server: true, server_engine: "rust".into() }
    }
}

#[derive(Default)]
struct AppState {
    server: Mutex<Server>,
    logs: Mutex<VecDeque<String>>,
    prefs: Mutex<Prefs>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Status {
    running: bool,
    pid: Option<u32>,
    started_at: Option<u64>,
    last_exit: Option<i32>,
    stopped_by_user: bool,
    start_error: Option<String>,
    engine: String,
    rust_available: bool,
    node_available: bool,
    ws_port: u16,
    http_port: u16,
    url: String,
    mcp_url: String,
    plugin_dir: String,
    plugin_built: bool,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Where things are. Installed: the app's resources (bin/wridge-server.exe, web/, skills/, figma-plugin/,
/// server-app/vendor, dart/) and a projects list in the user's app-data folder; no Node fallback.
/// Development: the wridge project folder, with legacy-node/ as the Node fallback.
struct Layout {
    rust_exe: PathBuf,
    /// The wridge CLI (wara.mjs's port): the session-memory hook runs it.
    cli_exe: PathBuf,
    plugin_dir: PathBuf,
    /// legacy-node/ when it is there (development only): the Node server, kept as a fallback.
    legacy: Option<PathBuf>,
    /// Extra environment for the Rust server, and the folder it runs in.
    env: Vec<(String, PathBuf)>,
    work_dir: PathBuf,
}

static LAYOUT: OnceLock<Layout> = OnceLock::new();

/// The project folder (wridge/) the binary was built in.
fn dev_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().map(PathBuf::from).unwrap_or_default()
}

/// Windows hands resource paths over as `\\?\C:\…`; people (and some tools) want the plain `C:\…`.
fn plain(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

fn init_layout(app: &AppHandle) {
    let res = app.path().resource_dir().ok().map(plain).filter(|r| r.join("bin").join("wridge-server.exe").exists());
    let layout = match res {
        Some(r) => {
            let data = app.path().app_data_dir().map(plain).unwrap_or_else(|_| r.clone());
            let _ = fs::create_dir_all(&data);
            // the projects list lives with the user and starts empty: projects are added from the window
            let projects = data.join("code-projects.json");
            if !projects.exists() {
                let _ = fs::write(&projects, "[]\n");
            }
            Layout {
                rust_exe: r.join("bin").join("wridge-server.exe"),
                cli_exe: r.join("bin").join("wridge.exe"),
                plugin_dir: r.join("figma-plugin"),
                legacy: None,
                env: vec![
                    ("WRIDGE_ROOT".into(), r.clone()),
                    ("WRIDGE_APP_DIR".into(), r.join("server-app")),
                    ("WRIDGE_DART_DIR".into(), r.join("dart")),
                    ("WRIDGE_PROJECTS_FILE".into(), projects),
                ],
                work_dir: data,
            }
        }
        None => {
            let root = dev_root();
            let legacy = root.join("legacy-node");
            Layout {
                rust_exe: root.join("server").join("app").join("target").join("release").join("wridge-server.exe"),
                cli_exe: root.join("server").join("app").join("target").join("release").join("wridge.exe"),
                plugin_dir: root.join("figma-plugin"),
                legacy: legacy.join("index.mjs").exists().then(|| legacy.clone()),
                env: vec![],
                work_dir: legacy,
            }
        }
    };
    let _ = LAYOUT.set(layout);
}

fn layout() -> &'static Layout {
    LAYOUT.get().expect("layout is set up at start")
}

fn status_of(state: &AppState) -> Status {
    let server = state.server.lock().unwrap();
    let dir = &layout().plugin_dir;
    Status {
        running: server.child.is_some(),
        pid: server.child.as_ref().map(|c| c.id()),
        started_at: server.started_at,
        last_exit: server.last_exit,
        stopped_by_user: server.stopped_by_user,
        start_error: server.start_error.clone(),
        engine: if server.engine.is_empty() { state.prefs.lock().unwrap().server_engine.clone() } else { server.engine.clone() },
        rust_available: layout().rust_exe.exists(),
        node_available: layout().legacy.is_some(),
        ws_port: WS_PORT,
        http_port: HTTP_PORT,
        url: format!("http://127.0.0.1:{HTTP_PORT}"),
        mcp_url: format!("http://127.0.0.1:{HTTP_PORT}/mcp"),
        plugin_dir: dir.to_string_lossy().replace('\\', "/"),
        plugin_built: dir.join("code.js").exists(),
    }
}

fn push_log(app: &AppHandle, line: String) {
    let state = app.state::<AppState>();
    {
        let mut logs = state.logs.lock().unwrap();
        logs.push_back(line.clone());
        while logs.len() > LOG_LINES {
            logs.pop_front();
        }
    }
    let _ = app.emit("log", line);
}

fn emit_status(app: &AppHandle) {
    let status = status_of(&app.state::<AppState>());
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(if status.running { "wridge · running" } else { "wridge · stopped" }));
    }
    if let Some(items) = TRAY.get() {
        let engine = if status.engine == "node" { "Node" } else { "Rust" };
        let _ = items.head.set_text(if status.running { format!("wridge · server running ({engine})") } else { "wridge · server stopped".into() });
        let _ = items.power.set_text(if status.running { "Stop server" } else { "Start server" });
        let _ = items.restart.set_enabled(status.running);
        let _ = items.autostart.set_checked(app.autolaunch().is_enabled().unwrap_or(false));
    }
    let _ = app.emit("status", status);
}

fn pump(app: &AppHandle, pipe: Option<impl Read + Send + 'static>) {
    let Some(pipe) = pipe else { return };
    let app = app.clone();
    thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            // the Figma plugin's "Open wridge" (the server prints this marker)
            if line.trim() == "[wridge:show-window]" {
                show_window(&app, None);
                continue;
            }
            push_log(&app, line);
        }
    });
}

fn start_server(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut server = state.server.lock().unwrap();
    if server.child.is_some() {
        return Ok(());
    }
    server.last_exit = None;
    server.stopped_by_user = false;
    server.start_error = None;
    // Rust by default; Node when chosen in Settings (development) or when the Rust build is missing
    let lay = layout();
    let wants_rust = state.prefs.lock().unwrap().server_engine != "node" || lay.legacy.is_none();
    let exe = &lay.rust_exe;
    let rust = exe.exists() && (wants_rust || lay.legacy.is_none());
    let mut cmd = if rust {
        let mut c = Command::new(exe);
        for (k, v) in &lay.env {
            c.env(k, v);
        }
        c.current_dir(&lay.work_dir);
        c
    } else if let Some(legacy) = &lay.legacy {
        let mut c = Command::new("node");
        c.arg("index.mjs").current_dir(legacy);
        c
    } else {
        let msg = format!("— the server program is missing: {}", exe.display());
        server.start_error = Some(msg.clone());
        drop(server);
        emit_status(app);
        return Err(msg);
    };
    server.engine = if rust { "rust".into() } else { "node".into() };
    if wants_rust && !rust {
        let app2 = app.clone();
        thread::spawn(move || push_log(&app2, "— the Rust server isn't built (server/app/target/release) — using Node".into()));
    }
    cmd
        .env("WARA_PORT", WS_PORT.to_string())
        .env("WARA_HTTP_PORT", HTTP_PORT.to_string())
        .env("WARA_WAIT_FOR_PORTS", "1")
        .env("WRIDGE_MANAGED", "1")
        .env("WRIDGE_PARENT_PID", std::process::id().to_string()) // the server stops itself if the app dies
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window flashing up
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = if rust { format!("— couldn't start the Rust server ({}): {e}", exe.display()) } else { format!("— couldn't start the server (Node.js is needed): {e}") };
            server.start_error = Some(msg.clone());
            drop(server);
            emit_status(app);
            return Err(msg);
        }
    };
    pump(app, child.stdout.take());
    pump(app, child.stderr.take());
    server.run += 1;
    let run = server.run;
    server.child = Some(child);
    server.started_at = Some(now());
    drop(server);
    push_log(app, if rust { "— starting the server (Rust)".into() } else { "— starting the server (Node)".into() });
    watch(app.clone(), run);
    emit_status(app);
    Ok(())
}

/// Polls the child for its exit (the state owns it, so nothing can block on wait()).
fn watch(app: AppHandle, run: u64) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(400));
        let state = app.state::<AppState>();
        let mut server = state.server.lock().unwrap();
        if server.run != run {
            return; // stopped or restarted meanwhile
        }
        let Some(child) = server.child.as_mut() else { return };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(exit)) => {
                let code = exit.code();
                server.child = None;
                server.started_at = None;
                server.last_exit = Some(code.unwrap_or(-1));
                drop(server);
                push_log(&app, format!("— server exited (code {})", code.map_or("?".into(), |c| c.to_string())));
                emit_status(&app);
                if code == Some(RESTART_CODE) {
                    push_log(&app, "— restarting, as the server asked".into());
                    if let Err(e) = start_server(&app) {
                        push_log(&app, e);
                    }
                }
                return;
            }
            Err(_) => return,
        }
    });
}

fn stop_server(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut server = state.server.lock().unwrap();
    server.run += 1; // the watcher of the old run stands down
    let had = server.child.is_some();
    if let Some(mut child) = server.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    server.started_at = None;
    if had {
        server.stopped_by_user = true;
        server.last_exit = None;
    }
    drop(server);
    if had {
        push_log(app, "— server stopped".into());
    }
    emit_status(app);
}

fn restart_server(app: &AppHandle) -> Result<(), String> {
    stop_server(app);
    thread::sleep(Duration::from_millis(300)); // let the ports close
    start_server(app)
}

// ---------- preferences (a small JSON file in the app's config folder) ----------

fn prefs_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("prefs.json"))
}

fn load_prefs(app: &AppHandle) -> Prefs {
    prefs_file(app).and_then(|f| fs::read_to_string(f).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_prefs(app: &AppHandle, prefs: &Prefs) -> Result<(), String> {
    let file = prefs_file(app).ok_or("no config folder")?;
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(file, serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

// ---------- commands for the window ----------

#[tauri::command]
fn server_status(state: State<AppState>) -> Status {
    status_of(&state)
}

#[tauri::command]
fn server_start(app: AppHandle) -> Result<Status, String> {
    start_server(&app)?;
    Ok(status_of(&app.state::<AppState>()))
}

#[tauri::command]
fn server_stop(app: AppHandle) -> Status {
    stop_server(&app);
    status_of(&app.state::<AppState>())
}

#[tauri::command]
fn server_restart(app: AppHandle) -> Result<Status, String> {
    restart_server(&app)?;
    Ok(status_of(&app.state::<AppState>()))
}

#[tauri::command]
fn server_logs(state: State<AppState>) -> Vec<String> {
    state.logs.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
fn get_prefs(state: State<AppState>) -> Prefs {
    state.prefs.lock().unwrap().clone()
}

#[tauri::command]
fn set_prefs(app: AppHandle, prefs: Prefs) -> Result<Prefs, String> {
    save_prefs(&app, &prefs)?;
    *app.state::<AppState>().prefs.lock().unwrap() = prefs.clone();
    Ok(prefs)
}

// ---------- Save as… (a page's download: PDF etc.) ----------
// The page hands its file to the window; here Windows' own Save dialog asks where it goes.
// Body = the file's bytes, header x-name = the suggested name (URI-encoded). → the saved path, or null if cancelled.

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
async fn save_as(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("no file to save".into());
    };
    let name = request.headers().get("x-name").and_then(|v| v.to_str().ok()).map(pct_decode).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "download".into());
    let name: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let ext = std::path::Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut dialog = app.dialog().file().set_title("Save as").set_file_name(&name);
    if let Some(win) = app.get_webview_window("control") {
        dialog = dialog.set_parent(&win);
    }
    if !ext.is_empty() {
        let label = match ext.as_str() {
            "pdf" => "PDF document".to_string(),
            "json" => "JSON file".to_string(),
            "png" => "PNG image".to_string(),
            "svg" => "SVG image".to_string(),
            e => format!("{} file", e.to_uppercase()),
        };
        dialog = dialog.add_filter(label, &[ext.as_str()]);
    }
    // blocking is fine here: async commands run off the main thread
    let Some(picked) = dialog.blocking_save_file() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, bytes).map_err(|e| format!("couldn't save {}: {e}", path.display()))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

// ---------- Claude Code setup (Settings → Connect an AI) ----------
// Two entries in the user's Claude Code config, both user-wide:
//   ~/.claude.json            mcpServers.wridge = { type: http, url: http://127.0.0.1:3156/mcp }
//   ~/.claude/settings.json   hooks.SessionStart: "<wridge.exe>" memory brief --stdin  (project memory at session start)
// An existing memory-brief hook (e.g. the older wara-bridge one) is pointed at wridge instead of added twice.
// WRIDGE_CLAUDE_HOME points both files somewhere else (tests).

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeSetup {
    /// Claude Code has been used on this computer (~/.claude or ~/.claude.json exists).
    installed: bool,
    /// "on" (wridge at our URL), "other" (a wridge entry with another URL), "off"
    mcp: String,
    /// "on" (our CLI), "other" (a memory hook that runs something else, e.g. wara-bridge), "off"
    hook: String,
    hook_command: Option<String>,
    config_file: String,
    settings_file: String,
}

fn claude_home(app: &AppHandle) -> PathBuf {
    match std::env::var_os("WRIDGE_CLAUDE_HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => app.path().home_dir().unwrap_or_default(),
    }
}

fn read_json(path: &PathBuf) -> Result<serde_json::Value, String> {
    match fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).map_err(|e| format!("{} isn't valid JSON ({e}) — left it untouched", path.display())),
        Ok(_) => Ok(serde_json::json!({})),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// Write next to the file, then swap it in: a crash never leaves half a config.
fn write_json(path: &PathBuf, value: &serde_json::Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("wridge-tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value).map_err(|e| e.to_string())? + "\n").map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| format!("couldn't save {}: {e}", path.display()))
}

fn hook_command() -> String {
    format!("\"{}\" memory brief --stdin", layout().cli_exe.to_string_lossy().replace('\\', "/"))
}

fn is_memory_hook(cmd: &str) -> bool {
    cmd.contains("memory brief")
}

fn claude_state(app: &AppHandle) -> ClaudeSetup {
    claude_state_at(&claude_home(app), &hook_command())
}

/// `ours` = the hook command this wridge would write.
fn claude_state_at(home: &std::path::Path, ours: &str) -> ClaudeSetup {
    let config_file = home.join(".claude.json");
    let settings_file = home.join(".claude").join("settings.json");
    let installed = config_file.exists() || home.join(".claude").is_dir();
    let url = format!("http://127.0.0.1:{HTTP_PORT}/mcp");
    let config = read_json(&config_file).unwrap_or_default();
    let mcp = match config.pointer("/mcpServers/wridge/url").and_then(|u| u.as_str()) {
        Some(u) if u == url => "on",
        Some(_) => "other",
        None => "off",
    };
    let settings = read_json(&settings_file).unwrap_or_default();
    let found = settings
        .pointer("/hooks/SessionStart")
        .and_then(|g| g.as_array())
        .into_iter()
        .flatten()
        .filter_map(|g| g.get("hooks").and_then(|h| h.as_array()))
        .flatten()
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
        .find(|c| is_memory_hook(c))
        .map(String::from);
    let hook = match &found {
        Some(c) if *c == ours => "on",
        Some(_) => "other",
        None => "off",
    };
    ClaudeSetup {
        installed,
        mcp: mcp.into(),
        hook: hook.into(),
        hook_command: found,
        config_file: config_file.to_string_lossy().replace('\\', "/"),
        settings_file: settings_file.to_string_lossy().replace('\\', "/"),
    }
}

#[tauri::command]
fn claude_status(app: AppHandle) -> ClaudeSetup {
    claude_state(&app)
}

#[tauri::command]
fn claude_setup(app: AppHandle) -> Result<ClaudeSetup, String> {
    let (home, ours) = (claude_home(&app), hook_command());
    claude_setup_at(&home, &ours)?;
    Ok(claude_state_at(&home, &ours))
}

#[tauri::command]
fn claude_remove(app: AppHandle) -> Result<ClaudeSetup, String> {
    let (home, ours) = (claude_home(&app), hook_command());
    claude_remove_at(&home, &ours)?;
    Ok(claude_state_at(&home, &ours))
}

fn claude_setup_at(home: &std::path::Path, ours: &str) -> Result<(), String> {
    // read and check both files before writing either
    let config_file = home.join(".claude.json");
    let mut config = read_json(&config_file)?;
    let settings_file = home.join(".claude").join("settings.json");
    let mut settings = read_json(&settings_file)?;
    // 1. the MCP server, user-wide
    let obj = config.as_object_mut().ok_or("~/.claude.json isn't a JSON object — left it untouched")?;
    let servers = obj.entry("mcpServers").or_insert_with(|| serde_json::json!({}));
    let servers = servers.as_object_mut().ok_or("mcpServers in ~/.claude.json isn't an object — left it untouched")?;
    servers.insert("wridge".into(), serde_json::json!({ "type": "http", "url": format!("http://127.0.0.1:{HTTP_PORT}/mcp") }));
    // 2. the session-memory hook: repoint an existing one, or add ours
    let root = settings.as_object_mut().ok_or("~/.claude/settings.json isn't a JSON object — left it untouched")?;
    let hooks = root.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks = hooks.as_object_mut().ok_or("hooks in settings.json isn't an object — left it untouched")?;
    let start = hooks.entry("SessionStart").or_insert_with(|| serde_json::json!([]));
    let groups = start.as_array_mut().ok_or("hooks.SessionStart isn't a list — left it untouched")?;
    let mut repointed = false;
    for g in groups.iter_mut() {
        if let Some(list) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
            for h in list.iter_mut() {
                if h.get("command").and_then(|c| c.as_str()).is_some_and(is_memory_hook) {
                    h["command"] = serde_json::Value::String(ours.to_string());
                    repointed = true;
                }
            }
        }
    }
    if !repointed {
        groups.push(serde_json::json!({ "matcher": "startup|clear|compact", "hooks": [{ "type": "command", "command": ours, "timeout": 15 }] }));
    }
    write_json(&config_file, &config)?;
    write_json(&settings_file, &settings)
}

/// Takes out only what wridge added: the wridge MCP entry and hooks that run exactly `ours`.
fn claude_remove_at(home: &std::path::Path, ours: &str) -> Result<(), String> {
    let config_file = home.join(".claude.json");
    let mut config = read_json(&config_file)?;
    if let Some(servers) = config.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        if servers.shift_remove("wridge").is_some() {
            write_json(&config_file, &config)?;
        }
    }
    let settings_file = home.join(".claude").join("settings.json");
    let mut settings = read_json(&settings_file)?;
    let mut changed = false;
    if let Some(groups) = settings.pointer_mut("/hooks/SessionStart").and_then(|g| g.as_array_mut()) {
        for g in groups.iter_mut() {
            if let Some(list) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                let before = list.len();
                list.retain(|h| h.get("command").and_then(|c| c.as_str()) != Some(ours));
                changed |= list.len() != before;
            }
        }
        if changed {
            groups.retain(|g| g.get("hooks").and_then(|h| h.as_array()).is_none_or(|l| !l.is_empty()));
        }
    }
    if changed {
        let mut no_hooks_left = false;
        if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            if hooks.get("SessionStart").and_then(|s| s.as_array()).is_some_and(|a| a.is_empty()) {
                hooks.shift_remove("SessionStart");
            }
            no_hooks_left = hooks.is_empty();
        }
        if no_hooks_left {
            if let Some(o) = settings.as_object_mut() {
                o.shift_remove("hooks");
            }
        }
        write_json(&settings_file, &settings)?;
    }
    Ok(())
}

/// Brings the window forward; `route` (overview, notes, settings…) tells it which page to show.
fn show_window(app: &AppHandle, route: Option<&str>) {
    if let Some(win) = app.get_webview_window("control") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        if let Some(r) = route {
            let _ = win.emit("navigate", r);
        }
    }
}

/// Tray menu items that change with the server's state.
struct TrayItems {
    head: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    power: MenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

static TRAY: OnceLock<TrayItems> = OnceLock::new();

/// The tray's right-click menu (Windows' own menu): a status line, the window, the server, the MCP address,
/// start with Windows, quit.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let head = MenuItem::with_id(app, "head", "wridge · starting…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open wridge", true, None::<&str>)?;
    let notes = MenuItem::with_id(app, "notes", "Brain notes", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "Restart server", true, None::<&str>)?;
    let power = MenuItem::with_id(app, "power", "Stop server", true, None::<&str>)?;
    let copy = MenuItem::with_id(app, "copy", format!("Copy MCP address  (:{HTTP_PORT}/mcp)"), true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, app.autolaunch().is_enabled().unwrap_or(false), None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit wridge (stops the server)", true, None::<&str>)?;
    let seps = [PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?, PredefinedMenuItem::separator(app)?];
    let menu = Menu::with_items(app, &[&head, &seps[0], &open, &notes, &seps[1], &restart, &power, &copy, &seps[2], &autostart, &seps[3], &quit])?;
    let _ = TRAY.set(TrayItems { head, restart, power, autostart });
    let mut tray = TrayIconBuilder::with_id("main").tooltip("wridge").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_window(app, None),
        "notes" => show_window(app, Some("notes")),
        "restart" => {
            let app = app.clone();
            thread::spawn(move || {
                if let Err(e) = restart_server(&app) {
                    push_log(&app, e);
                }
            });
        }
        "power" => {
            if app.state::<AppState>().server.lock().unwrap().child.is_some() {
                stop_server(app);
            } else {
                let app = app.clone();
                thread::spawn(move || {
                    if let Err(e) = start_server(&app) {
                        push_log(&app, e);
                    }
                });
            }
        }
        "copy" => {
            let _ = app.clipboard().write_text(format!("http://127.0.0.1:{HTTP_PORT}/mcp"));
        }
        "autostart" => {
            let auto = app.autolaunch();
            let on = !auto.is_enabled().unwrap_or(false);
            let _ = if on { auto.enable() } else { auto.disable() };
            if let Some(items) = TRAY.get() {
                let _ = items.autostart.set_checked(auto.is_enabled().unwrap_or(on));
            }
        }
        "quit" => {
            stop_server(app);
            app.exit(0);
        }
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        match event {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => show_window(tray.app_handle(), None),
            // the window's own Start-with-Windows switch may have changed it: refresh the check before the menu opens
            TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Down, .. } => {
                if let Some(items) = TRAY.get() {
                    let _ = items.autostart.set_checked(tray.app_handle().autolaunch().is_enabled().unwrap_or(false));
                }
            }
            _ => {}
        }
    })
    .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WRIDGE_TEST_INSTANCE=1: a throwaway copy for tests next to the real one — no hand-off to the running
    // wridge, no tray, never starts the server (the real one keeps 3155/3156), closing the window quits.
    let test_instance = std::env::var_os("WRIDGE_TEST_INSTANCE").is_some_and(|v| !v.is_empty());
    let mut builder = tauri::Builder::default();
    if !test_instance {
        // a second launch just brings the running one forward
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app, None)));
    }
    builder
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![server_status, server_start, server_stop, server_restart, server_logs, get_prefs, set_prefs, claude_status, claude_setup, claude_remove, save_as])
        .setup(move |app| {
            init_layout(app.handle());
            let prefs = load_prefs(app.handle());
            let auto = prefs.auto_start_server && !test_instance;
            *app.state::<AppState>().prefs.lock().unwrap() = prefs;
            if !test_instance {
                build_tray(app.handle())?;
            }
            emit_status(app.handle()); // the tray status line before any server start
            // started with Windows: stay in the tray
            if std::env::args().any(|a| a == "--background") {
                if let Some(win) = app.get_webview_window("control") {
                    let _ = win.hide();
                }
            }
            if auto {
                if let Err(e) = start_server(app.handle()) {
                    push_log(app.handle(), e);
                }
            }
            Ok(())
        })
        // closing the window keeps the server running in the tray; "Quit" in the tray quits
        .on_window_event(move |window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "control" && !test_instance {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building wridge")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                stop_server(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OURS: &str = r#""C:/x/wridge/bin/wridge.exe" memory brief --stdin"#;

    fn temp_home(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wridge-cc-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join(".claude")).unwrap();
        d
    }
    fn text(p: PathBuf) -> String {
        fs::read_to_string(p).unwrap()
    }
    fn read(p: PathBuf) -> serde_json::Value {
        serde_json::from_str(&text(p)).unwrap()
    }

    #[test]
    fn setup_repoints_the_old_hook_keeps_the_rest_and_remove_undoes_it() {
        let home = temp_home("repoint");
        let old_hook = r#"node "C:/p/wara-bridge/server/cli.mjs" memory brief --stdin"#;
        fs::write(home.join(".claude.json"), json!({ "numStartups": 5, "mcpServers": { "Roblox_Studio": { "type": "stdio", "command": "x" } }, "projects": { "a": {} } }).to_string()).unwrap();
        fs::write(
            home.join(".claude").join("settings.json"),
            json!({ "hooks": { "SessionStart": [{ "matcher": "startup|clear|compact", "hooks": [{ "type": "command", "command": old_hook, "timeout": 15 }] }], "Stop": [{ "hooks": [{ "type": "command", "command": "echo hi" }] }] }, "switchModelsOnFlag": true }).to_string(),
        )
        .unwrap();
        let s = claude_state_at(&home, OURS);
        assert_eq!((s.installed, s.mcp.as_str(), s.hook.as_str()), (true, "off", "other"));
        assert_eq!(s.hook_command.as_deref(), Some(old_hook));

        claude_setup_at(&home, OURS).unwrap();
        let s = claude_state_at(&home, OURS);
        assert_eq!((s.mcp.as_str(), s.hook.as_str()), ("on", "on"));
        let c = read(home.join(".claude.json"));
        assert_eq!(c.as_object().unwrap().keys().collect::<Vec<_>>(), ["numStartups", "mcpServers", "projects"]); // order kept
        assert_eq!(c["numStartups"], 5);
        assert_eq!(c["mcpServers"]["Roblox_Studio"]["command"], "x");
        assert_eq!(c["mcpServers"]["wridge"], json!({ "type": "http", "url": "http://127.0.0.1:3156/mcp" }));
        let st = read(home.join(".claude").join("settings.json"));
        let groups = st["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "the old memory hook is repointed, not doubled");
        assert_eq!(groups[0]["hooks"][0]["command"], OURS);
        assert_eq!(groups[0]["hooks"][0]["timeout"], 15);
        assert_eq!(st["hooks"]["Stop"][0]["hooks"][0]["command"], "echo hi");
        assert_eq!(st["switchModelsOnFlag"], true);

        // a second run changes nothing
        let before = (text(home.join(".claude.json")), text(home.join(".claude").join("settings.json")));
        claude_setup_at(&home, OURS).unwrap();
        assert_eq!(before, (text(home.join(".claude.json")), text(home.join(".claude").join("settings.json"))));

        // remove takes out only what wridge added
        claude_remove_at(&home, OURS).unwrap();
        let s = claude_state_at(&home, OURS);
        assert_eq!((s.mcp.as_str(), s.hook.as_str()), ("off", "off"));
        let c = read(home.join(".claude.json"));
        assert!(c["mcpServers"].get("wridge").is_none());
        assert_eq!(c["mcpServers"]["Roblox_Studio"]["command"], "x");
        assert_eq!(c["projects"], json!({ "a": {} }));
        let st = read(home.join(".claude").join("settings.json"));
        assert!(st["hooks"].get("SessionStart").is_none());
        assert_eq!(st["hooks"]["Stop"][0]["hooks"][0]["command"], "echo hi");
        assert_eq!(st["switchModelsOnFlag"], true);
        assert!(!home.join(".claude.wridge-tmp").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_computer_without_claude_code_gets_both_files() {
        let home = temp_home("fresh");
        fs::remove_dir_all(home.join(".claude")).unwrap();
        let s = claude_state_at(&home, OURS);
        assert_eq!((s.installed, s.mcp.as_str(), s.hook.as_str()), (false, "off", "off"));
        claude_setup_at(&home, OURS).unwrap();
        let st = read(home.join(".claude").join("settings.json"));
        assert_eq!(st, json!({ "hooks": { "SessionStart": [{ "matcher": "startup|clear|compact", "hooks": [{ "type": "command", "command": OURS, "timeout": 15 }] }] } }));
        claude_remove_at(&home, OURS).unwrap();
        assert_eq!(read(home.join(".claude").join("settings.json")), json!({}));
        assert_eq!(read(home.join(".claude.json")), json!({ "mcpServers": {} }));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_broken_config_is_left_untouched() {
        let home = temp_home("broken");
        fs::write(home.join(".claude.json"), "{ not json").unwrap();
        assert!(claude_setup_at(&home, OURS).is_err());
        assert_eq!(text(home.join(".claude.json")), "{ not json");
        assert!(!home.join(".claude").join("settings.json").exists(), "nothing written when one file can't be read");
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn file_names_decode() {
        assert_eq!(pct_decode("design%20%E0%B8%97.pdf"), "design ท.pdf");
        assert_eq!(pct_decode("100%"), "100%");
        assert_eq!(pct_decode("a%2"), "a%2");
    }
}
