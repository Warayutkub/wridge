//! Lifecycle from index.mjs: ports, listening (with WARA_WAIT_FOR_PORTS retries), the startup lines,
//! stop / restart (exit code 75 under WRIDGE_MANAGED), and stopping when the wridge app (WRIDGE_PARENT_PID)
//! goes away.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;

use crate::app::{App, Ctx};
use crate::paths::Paths;
use crate::util::log;

/// The server's exit code when its dashboard asks for a restart and the wridge app owns the process.
pub const RESTART_CODE: i32 = 75;

fn env_truthy(name: &str) -> bool {
    std::env::var(name).map(|v| !v.is_empty()).unwrap_or(false)
}

fn port(name: &str, default: u16) -> u16 {
    match std::env::var(name) {
        Ok(v) => crate::util::js_number(&v) as u16,
        Err(_) => default,
    }
}

async fn listen(port: u16, label: &str, http_port: u16) -> TcpListener {
    let mut retries = if env_truthy("WARA_WAIT_FOR_PORTS") { 40 } else { 0 };
    loop {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => return l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if retries > 0 {
                    retries -= 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                eprintln!("Port {port} ({label}) is already in use — wridge is probably already running: http://localhost:{http_port}");
                std::process::exit(1);
            }
            Err(e) => {
                let code = match e.kind() {
                    std::io::ErrorKind::PermissionDenied => "EACCES: permission denied".to_string(),
                    std::io::ErrorKind::AddrNotAvailable => "EADDRNOTAVAIL: address not available".to_string(),
                    _ => e.to_string(),
                };
                log(&format!("{label} error: listen {code} 127.0.0.1:{port}"));
                // Node never resolves listen() here: the server just sits there.
                futures::future::pending::<()>().await;
            }
        }
    }
}

/// index.mjs shutdown(): reject waiting commands, close the plugin sockets and the servers, exit in 300 ms.
pub fn shutdown(app: Ctx, code: i32) {
    app.bridge.reject_all_pending("wridge server stopped");
    app.bridge.close_all();
    let _ = app.shutdown.send(true);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::process::exit(code);
    });
}

pub fn restart(app: Ctx) {
    log("Restarting…");
    // Run by the wridge app: it owns the process and the log, so hand the restart to it (exit code 75).
    if env_truthy("WRIDGE_MANAGED") {
        shutdown(app, 0);
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(250)).await; // before shutdown()'s own exit(0) at 300 ms
            std::process::exit(RESTART_CODE);
        });
        return;
    }
    // Started on its own: a new server writes to wara.log and waits for these ports to free up.
    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(std::env::args().skip(1))
            .current_dir(&app.paths.app_dir)
            .env("WARA_WAIT_FOR_PORTS", "1")
            .stdin(std::process::Stdio::null());
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&app.paths.log_file) {
            if let Ok(f2) = f.try_clone() {
                cmd.stdout(f).stderr(f2);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | crate::util::CREATE_NO_WINDOW);
        }
        let _ = cmd.spawn();
    }
    shutdown(app, 0);
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Run by the wridge app: if the app goes away without stopping us (killed, crashed), stop too, so no
/// orphan keeps the ports.
fn watch_parent(app: Ctx) {
    let Ok(raw) = std::env::var("WRIDGE_PARENT_PID") else { return };
    if raw.is_empty() {
        return;
    }
    let pid = crate::util::js_number(&raw);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(2), Duration::from_secs(2));
        loop {
            tick.tick().await;
            let alive = pid.is_finite() && pid.fract() == 0.0 && pid > 0.0 && process_alive(pid as u32);
            if !alive {
                log("wridge app is gone — stopping");
                shutdown(app.clone(), 0);
                return;
            }
        }
    });
}

pub fn router(app: Ctx) -> Router {
    Router::new().fallback(crate::http::entry).with_state(app).layer(axum::extract::DefaultBodyLimit::disable())
}

pub fn ws_router(app: &Ctx) -> Router {
    Router::new().fallback(crate::bridge::ws_entry).with_state(app.bridge.clone())
}

/// Starts the server like `node index.mjs`.
pub async fn run() {
    let ws_port = port("WARA_PORT", 3155); // wridge: 3155/3156, so wara-bridge (3055/3056) can run at the same time
    let http_port = port("WARA_HTTP_PORT", 3156);
    let paths = Paths::discover();
    paths.use_dart_dir();
    let app: Arc<App> = App::new(paths, ws_port, http_port);
    watch_parent(app.clone());

    let ws_listener = listen(ws_port, "Figma plugin", http_port).await;
    let http_listener = listen(http_port, "dashboard/MCP", http_port).await;
    log(&format!("wridge running (pid {})", std::process::id()));
    log(&format!("  dashboard   http://localhost:{http_port}"));
    log(&format!("  MCP         http://127.0.0.1:{http_port}/mcp"));
    log(&format!("  plugin      ws://localhost:{ws_port}"));
    // elkjs (figjam_diagram's layout, in QuickJS) loads once, off the async workers, so the first diagram
    // doesn't wait for it
    tokio::task::spawn_blocking(|| {
        if let Err(e) = wridge_figma::elk::warm_up() {
            log(&format!("ELK warm-up failed: {e}"));
        }
    });

    let stop_ws = app.clone();
    let stop_http = app.clone();
    let ws = axum::serve(ws_listener, ws_router(&app)).with_graceful_shutdown(async move {
        let mut rx = stop_ws.shutdown.subscribe();
        let _ = rx.wait_for(|v| *v).await;
    });
    let http = axum::serve(http_listener, router(app.clone())).with_graceful_shutdown(async move {
        let mut rx = stop_http.shutdown.subscribe();
        let _ = rx.wait_for(|v| *v).await;
    });
    let _ = tokio::join!(async { ws.await }, async { http.await });
    // shutdown() exits the process; if the servers end on their own, wait for that exit.
    futures::future::pending::<()>().await;
}
