//! The Figma plugin side of index.mjs: the WebSocket server, which plugin window is active, request ids,
//! timeouts, file names, status pushes (notifyPlugin) and use_file.
//!
//! The plugin can be open in several files at once (a Figma design file and a FigJam board). Commands go
//! to the active one: the first window to connect, or the one the AI picked with use_file. Writes still need a
//! get_document_info check of that file first (guards.rs), so switching never writes to the wrong file.
//! A window that connects (or reconnects after Figma throttled it in the background) never takes over an
//! open active one: only use_file switches, so the file in use never changes behind the AI's back.
//!
//! Sessions (session.rs): the active window above belongs to the shared session. Every other session is
//! pinned to the window it first used (or picked with use_file), so one AI switching files never moves
//! another. Each window has a queue: commands to one file run one at a time in arrival order (a run_batch
//! is one turn), while different files run side by side.

use std::collections::HashMap;

/// Printed on stdout when the plugin asks to bring the wridge app forward (the app watches for it).
pub const SHOW_APP_LINE: &str = "[wridge:show-window]";
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::future::BoxFuture;
use futures::{FutureExt, SinkExt, StreamExt};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};

use crate::guards::PluginSend;
use crate::jsjson::{number_to_string, stringify};
use crate::session::{self, SessionId, SHARED};
use crate::util::log;

pub const COMMAND_TIMEOUT_MS: u64 = 30_000;
/// How long a command waits for its turn in a busy file before giving up.
pub const QUEUE_WAIT_MS: u64 = 120_000;
/// A session that sent nothing for this long is forgotten (it follows the active window again).
const SESSION_IDLE_MS: f64 = 12.0 * 3600.0 * 1000.0;

enum Out {
    Text(String),
    Close,
}

struct Conn {
    tx: mpsc::UnboundedSender<Out>,
    file_name: Value,
    editor_type: Value,
    at: f64,
    open: bool,
    /// One command at a time in this file (FIFO: tokio's mutex is fair).
    queue: Arc<tokio::sync::Mutex<()>>,
    /// Commands waiting for their turn.
    waiting: Arc<AtomicUsize>,
    /// figma.fileKey from the plugin's hello (newer plugins), so a pick survives a rename
    file_key: Option<String>,
    /// the plugin's version from its hello (newer plugins)
    plugin_version: Option<String>,
}

/// The window a session works in, and when it last sent something.
struct Pin {
    socket: u64,
    at: f64,
}

struct Pending {
    tx: oneshot::Sender<Result<Value, String>>,
    socket: u64,
}

#[derive(Default)]
struct BridgeState {
    /// socket → info, in connection order (a JS Map)
    plugins: IndexMap<u64, Conn>,
    /// the active socket
    active: Option<u64>,
    /// state.fileName: the file the dashboard shows
    file_name: Value,
    pending: HashMap<String, Pending>,
    file_name_request: bool,
    /// session → its window (the shared session uses `active` instead)
    sessions: HashMap<SessionId, Pin>,
    /// when each session (the shared one too) last sent a command
    seen: HashMap<SessionId, f64>,
}

pub struct Bridge {
    http_port: u16,
    state: Mutex<BridgeState>,
    paused: AtomicBool,
    next_socket: AtomicU64,
    /// Bumped when a window closes or a session switches file (guards.rs re-reads the file name then).
    epoch: AtomicU64,
    /// QUEUE_WAIT_MS unless a test shortens it.
    queue_wait_ms: AtomicU64,
}

impl Bridge {
    pub fn new(http_port: u16) -> Arc<Bridge> {
        Arc::new(Bridge {
            http_port,
            state: Mutex::new(BridgeState {
                file_name: Value::Null,
                ..Default::default()
            }),
            paused: AtomicBool::new(false),
            next_socket: AtomicU64::new(1),
            epoch: AtomicU64::new(1),
            queue_wait_ms: AtomicU64::new(QUEUE_WAIT_MS),
        })
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Ordering::SeqCst);
        self.notify();
    }

    /// state.fileName
    pub fn file_name(&self) -> Value {
        self.state.lock().unwrap().file_name.clone()
    }

    /// `pluginConnected()`: null when no window is active (as `plugin && …` is in JS), else whether it is open.
    pub fn plugin_connected(&self) -> Value {
        let s = self.state.lock().unwrap();
        match s.active.and_then(|id| s.plugins.get(&id)) {
            None if s.active.is_none() => Value::Null,
            None => Value::Bool(false),
            Some(c) => Value::Bool(c.open),
        }
    }

    pub fn connected(&self) -> bool {
        self.plugin_connected() == Value::Bool(true)
    }

    fn open_files_locked(s: &BridgeState) -> Value {
        Self::open_files_for(s, s.active)
    }

    fn open_files_for(s: &BridgeState, target: Option<u64>) -> Value {
        Value::Array(
            s.plugins
                .iter()
                .filter(|(_, c)| c.open)
                .map(|(id, c)| {
                    let editor = if c.editor_type.is_null() {
                        Value::String("figma".into())
                    } else {
                        c.editor_type.clone()
                    };
                    json!({"fileName": c.file_name, "editorType": editor, "active": Some(*id) == target})
                })
                .collect(),
        )
    }

    /// What is open where, for get_document_info, use_file and the dashboard; "active" = the calling
    /// session's file.
    pub fn open_files(&self) -> Value {
        let s = self.state.lock().unwrap();
        let target = Self::target_locked(&s, &session::current());
        Self::open_files_for(&s, target)
    }

    /// The window `session` works in: its pin while that window is open, else the active one.
    fn target_locked(s: &BridgeState, session: &str) -> Option<u64> {
        if session != SHARED {
            if let Some(p) = s.sessions.get(session) {
                if s.plugins.get(&p.socket).map(|c| c.open).unwrap_or(false) {
                    return Some(p.socket);
                }
            }
        }
        s.active
    }

    /// Keeps `session` on `socket` from now on (the shared session moves `active` instead).
    fn pin_locked(&self, s: &mut BridgeState, session: &SessionId, socket: u64) {
        if &**session == SHARED {
            return;
        }
        let now = crate::guards::now_ms();
        s.sessions.retain(|_, p| now - p.at < SESSION_IDLE_MS);
        let moved = s.sessions.get(session).map(|p| p.socket) != Some(socket);
        s.sessions.insert(session.clone(), Pin { socket, at: now });
        if moved {
            self.epoch.fetch_add(1, Ordering::SeqCst);
            self.notify_locked(s);
        }
    }

    /// A tool call from `session` (app.rs run_tool): what "AI here" counts. The server's own lookups and the
    /// window's pages are not tool calls, so they never count.
    pub fn note_tool_call(&self, session: &SessionId) {
        let mut s = self.state.lock().unwrap();
        let now = crate::guards::now_ms();
        s.seen.retain(|_, t| now - *t < SESSION_IDLE_MS);
        s.seen.insert(session.clone(), now);
    }

    /// MCP DELETE: the client ended its session.
    pub fn forget_session(&self, session: &str) {
        let mut s = self.state.lock().unwrap();
        if s.sessions.remove(session).is_some() {
            self.notify_locked(&s);
        }
    }

    /// GET /api/sessions: every open file, the sessions working in it and its queue. "showing" = the
    /// calling session's file; "ai" = sessions other than the window's own that sent a command to it in
    /// the last 30 minutes.
    pub fn sessions_info(&self) -> Value {
        let s = self.state.lock().unwrap();
        let me = Self::target_locked(&s, &session::current());
        let now = crate::guards::now_ms();
        let recent = |k: &str| s.seen.get(k).map(|t| now - t < 30.0 * 60.0 * 1000.0).unwrap_or(false);
        let ai_on = |id: u64| {
            let pinned = s.sessions.iter().filter(|(k, p)| p.socket == id && &***k != session::APP && recent(k)).count();
            pinned + usize::from(s.active == Some(id) && recent(SHARED))
        };
        let files: Vec<Value> = s
            .plugins
            .iter()
            .filter(|(_, c)| c.open)
            .map(|(id, c)| {
                let mut users: Vec<String> = s.sessions.iter().filter(|(_, p)| p.socket == *id).map(|(k, _)| k.to_string()).collect();
                users.sort();
                if s.active == Some(*id) {
                    users.insert(0, SHARED.to_string());
                }
                json!({
                    "fileName": c.file_name,
                    "editorType": if c.editor_type.is_null() { Value::String("figma".into()) } else { c.editor_type.clone() },
                    "sessions": users,
                    "busy": c.queue.try_lock().is_err(),
                    "waiting": c.waiting.load(Ordering::SeqCst),
                    "fileKey": c.file_key,
                    "pluginVersion": c.plugin_version,
                    "showing": me == Some(*id),
                    "ai": ai_on(*id),
                })
            })
            .collect();
        json!({"files": files, "sessions": s.sessions.len()})
    }

    pub fn plugin_count(&self) -> usize {
        self.state.lock().unwrap().plugins.len()
    }

    fn notify_locked(&self, s: &BridgeState) {
        let paused = self.paused();
        for (id, c) in &s.plugins {
            if c.open {
                // the window's own pick (session "app") is a person looking, not an AI using the file
                let in_use = Some(*id) == s.active || s.sessions.iter().any(|(k, p)| p.socket == *id && &**k != session::APP);
                let msg = json!({"type": "status", "paused": paused, "dashboard": format!("http://localhost:{}", self.http_port), "active": in_use, "files": s.plugins.len()});
                let _ = c.tx.send(Out::Text(stringify(&msg)));
            }
        }
    }

    /// Status for every plugin window (paused, dashboard link, and whether the AI is using that file).
    pub fn notify(&self) {
        let s = self.state.lock().unwrap();
        self.notify_locked(&s);
    }

    /// How long a command waits for a busy file (tests shorten it).
    pub fn set_queue_wait_ms(&self, ms: u64) {
        self.queue_wait_ms.store(ms, Ordering::SeqCst);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn set_active_locked(&self, s: &mut BridgeState, socket: Option<u64>) {
        if s.active != socket {
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
        s.active = socket;
        s.file_name = socket.and_then(|id| s.plugins.get(&id)).map(|c| c.file_name.clone()).unwrap_or(Value::Null);
        self.notify_locked(s);
    }

    fn on_connect(&self, tx: mpsc::UnboundedSender<Out>) -> u64 {
        let id = self.next_socket.fetch_add(1, Ordering::SeqCst);
        {
            let mut s = self.state.lock().unwrap();
            s.plugins.insert(
                id,
                Conn {
                    tx,
                    file_name: Value::Null,
                    editor_type: Value::Null,
                    at: crate::guards::now_ms(),
                    open: true,
                    queue: Arc::new(tokio::sync::Mutex::new(())),
                    waiting: Arc::new(AtomicUsize::new(0)),
                    file_key: None,
                    plugin_version: None,
                },
            );
            let active_open = s.active.and_then(|a| s.plugins.get(&a)).map(|c| c.open).unwrap_or(false);
            if active_open {
                self.notify_locked(&s);
            } else {
                self.set_active_locked(&mut s, Some(id));
            }
        }
        log("Figma plugin connected");
        id
    }

    fn on_message(&self, socket: u64, raw: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(raw) else { return };
        if msg.get("type") == Some(&Value::String("hello".into())) {
            let file_name = msg.get("fileName").cloned().unwrap_or(Value::Null);
            let editor_type = msg.get("editorType").cloned().unwrap_or(Value::String("figma".into()));
            let mut s = self.state.lock().unwrap();
            let text = |k: &str| msg.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
            if let Some(c) = s.plugins.get_mut(&socket) {
                c.file_name = file_name.clone();
                c.editor_type = if editor_type.is_null() { Value::String("figma".into()) } else { editor_type };
                c.file_key = text("fileKey");
                c.plugin_version = text("version");
            }
            if s.active == Some(socket) {
                s.file_name = file_name;
            }
            let kind = if msg.get("editorType") == Some(&Value::String("figjam".into())) {
                "FigJam"
            } else {
                "Figma"
            };
            log(&format!("  {kind} file: {}", crate::jsjson::js_string(msg.get("fileName"))));
            self.notify_locked(&s);
            return;
        }
        if msg.get("type") == Some(&Value::String("show-app".into())) {
            // the plugin's "Open wridge": the desktop app reads this line from the server's output and shows its window
            println!("{SHOW_APP_LINE}");
            return;
        }
        let Some(id) = msg.get("id") else { return };
        let key = match id {
            Value::String(s) => s.clone(),
            other => stringify(other),
        };
        let entry = self.state.lock().unwrap().pending.remove(&key);
        let Some(entry) = entry else { return };
        let error = msg.get("error");
        let outcome = if crate::jsjson::truthy(error) {
            Err(crate::jsjson::js_string(error))
        } else {
            // a reply without result is `undefined` in JS; null is the closest JSON value
            Ok(msg.get("result").cloned().unwrap_or(Value::Null))
        };
        let _ = entry.tx.send(outcome);
    }

    fn on_close(&self, socket: u64) {
        let mut s = self.state.lock().unwrap();
        let info = s.plugins.shift_remove(&socket);
        let name = info.as_ref().map(|c| c.file_name.clone()).unwrap_or(Value::Null);
        log(&format!(
            "Figma plugin disconnected{}",
            if crate::jsjson::truthy(Some(&name)) {
                format!(" ({})", crate::jsjson::js_string(Some(&name)))
            } else {
                String::new()
            }
        ));
        let ids: Vec<String> = s.pending.iter().filter(|(_, p)| p.socket == socket).map(|(k, _)| k.clone()).collect();
        for id in ids {
            if let Some(p) = s.pending.remove(&id) {
                let _ = p.tx.send(Err("Figma plugin disconnected".into()));
            }
        }
        // sessions in that window follow the active one again (their next write needs a new file check)
        s.sessions.retain(|_, p| p.socket != socket);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if s.active == Some(socket) {
            // fall back to the most recently connected window that is still open
            let mut open: Vec<(u64, f64)> = s.plugins.iter().filter(|(_, c)| c.open).map(|(id, c)| (*id, c.at)).collect();
            open.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let next = open.first().map(|(id, _)| *id);
            self.set_active_locked(&mut s, next);
        }
    }

    pub fn reject_all_pending(&self, reason: &str) {
        let mut s = self.state.lock().unwrap();
        for (_, p) in s.pending.drain() {
            let _ = p.tx.send(Err(reason.to_string()));
        }
    }

    /// Closes every plugin socket (shutdown).
    pub fn close_all(&self) {
        let mut s = self.state.lock().unwrap();
        for c in s.plugins.values_mut() {
            c.open = false;
            let _ = c.tx.send(Out::Close);
        }
    }

    /// use_file: point commands at another open file (by name, or "figma" / "figjam").
    pub fn use_file(&self, name: Option<&Value>) -> anyhow::Result<Value> {
        let shown = crate::jsjson::js_string(name.filter(|v| !v.is_null()).or(Some(&Value::String(String::new()))));
        let want = crate::jsjson::js_trim(&shown).to_lowercase();
        let mut s = self.state.lock().unwrap();
        let raw_want = crate::jsjson::js_trim(&shown).to_string();
        let by_key = s.plugins.iter().find(|(_, c)| c.open && c.file_key.as_deref() == Some(raw_want.as_str())).map(|(id, _)| *id);
        let open: Vec<(u64, String, String)> = s
            .plugins
            .iter()
            .filter(|(_, c)| c.open)
            .map(|(id, c)| {
                let fname = if c.file_name.is_null() {
                    String::new()
                } else {
                    crate::jsjson::js_string(Some(&c.file_name))
                };
                let editor = if c.editor_type.is_null() {
                    "figma".to_string()
                } else {
                    crate::jsjson::js_string(Some(&c.editor_type))
                };
                (*id, fname.to_lowercase(), editor)
            })
            .collect();
        let found = by_key.or_else(|| {
            open.iter()
                .find(|(_, f, _)| *f == want)
            .or_else(|| open.iter().find(|(_, _, e)| *e == want))
                .or_else(|| open.iter().find(|(_, f, _)| f.contains(&want)))
                .map(|(id, _, _)| *id)
        });
        let Some(id) = found else {
            let files = Self::open_files_locked(&s);
            let list = files
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|f| format!("{} ({})", crate::jsjson::js_string(f.get("fileName")), crate::jsjson::js_string(f.get("editorType"))))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let list = if list.is_empty() {
                "none — run the wridge plugin in Figma or FigJam".to_string()
            } else {
                list
            };
            return Err(anyhow!("No open file matches \"{}\". Open: {list}", crate::jsjson::js_string(name)));
        };
        let session = session::current();
        if &*session == SHARED {
            self.set_active_locked(&mut s, Some(id));
        } else {
            self.pin_locked(&mut s, &session, id);
        }
        let c = &s.plugins[&id];
        let editor = if c.editor_type.is_null() {
            Value::String("figma".into())
        } else {
            c.editor_type.clone()
        };
        Ok(json!({"active": c.file_name, "editorType": editor, "openFiles": Self::open_files_for(&s, Some(id)), "next": "Call get_document_info before changing anything in it."}))
    }

    /// The dashboard shows which Figma file the plugin is in; ask once per connection.
    pub fn refresh_file_name(self: &Arc<Self>) {
        {
            let mut s = self.state.lock().unwrap();
            if crate::jsjson::truthy(Some(&s.file_name)) || s.file_name_request {
                return;
            }
            let connected = s.active.and_then(|a| s.plugins.get(&a)).map(|c| c.open).unwrap_or(false);
            if !connected {
                return;
            }
            s.file_name_request = true;
        }
        let me = self.clone();
        tokio::spawn(async move {
            if let Ok(info) = me.send("get_document_info", json!({}), Some(10_000)).await {
                if !info.is_null() {
                    me.state.lock().unwrap().file_name = info.get("fileName").cloned().unwrap_or(Value::Null);
                }
            }
            me.state.lock().unwrap().file_name_request = false;
        });
    }

    /// Waits for this file's turn. None needed when the task's held run (run_batch) already has it; a held
    /// run keeps the turn it takes until it ends.
    async fn take_turn(&self, socket: u64, queue: Arc<tokio::sync::Mutex<()>>, waiting: Arc<AtomicUsize>, file: &str) -> anyhow::Result<Option<tokio::sync::OwnedMutexGuard<()>>> {
        let hold = session::current_hold();
        if let Some(h) = &hold {
            let mut slot = h.slot.lock().unwrap();
            if slot.as_ref().map(|(sock, _)| *sock) == Some(socket) {
                return Ok(None);
            }
            // a held run moving to another file lets go of the old one first
            slot.take();
        }
        let guard = match queue.clone().try_lock_owned() {
            Ok(g) => g,
            Err(_) => {
                let ahead = waiting.fetch_add(1, Ordering::SeqCst) + 1;
                let wait_ms = self.queue_wait_ms.load(Ordering::SeqCst);
                let got = tokio::time::timeout(Duration::from_millis(wait_ms), queue.lock_owned()).await;
                waiting.fetch_sub(1, Ordering::SeqCst);
                got.map_err(|_| {
                    anyhow!(
                        "Figma file \"{file}\" stayed busy for {}s: another AI session is working in it ({ahead} command(s) were waiting). Try again in a moment, or work in another file.",
                        number_to_string(wait_ms as f64 / 1000.0)
                    )
                })?
            }
        };
        match hold {
            Some(h) => {
                *h.slot.lock().unwrap() = Some((socket, guard));
                Ok(None)
            }
            None => Ok(Some(guard)),
        }
    }

    async fn send_inner(self: Arc<Self>, command: String, params: Value, timeout_ms: u64) -> anyhow::Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let session = session::current();
        let not_connected = || anyhow!("Figma plugin is not connected — open Figma and run Plugins → Development → wridge");
        // 1. the calling session's file and its queue
        let (socket, queue, waiting, file) = {
            let mut s = self.state.lock().unwrap();
            let Some(socket) = Self::target_locked(&s, &session) else { return Err(not_connected()) };
            let Some(conn) = s.plugins.get(&socket).filter(|c| c.open) else { return Err(not_connected()) };
            let found = (conn.queue.clone(), conn.waiting.clone(), crate::jsjson::js_string(Some(&conn.file_name)));
            self.pin_locked(&mut s, &session, socket);
            (socket, found.0, found.1, found.2)
        };
        // 2. its turn in that file, kept until the answer is back
        let _turn = self.take_turn(socket, queue, waiting, &file).await?;
        // 3. send
        let rx = {
            let mut s = self.state.lock().unwrap();
            let Some(conn) = s.plugins.get(&socket).filter(|c| c.open) else { return Err(not_connected()) };
            let msg = json!({"id": id, "command": command, "params": params});
            let _ = conn.tx.send(Out::Text(stringify(&msg)));
            let (tx, rx) = oneshot::channel();
            s.pending.insert(id.clone(), Pending { tx, socket });
            rx
        };
        let result = match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(Ok(v))) => v,
            Ok(Ok(Err(msg))) => return Err(anyhow!(msg)),
            Ok(Err(_)) => return Err(anyhow!("Figma plugin disconnected")),
            Err(_) => {
                self.state.lock().unwrap().pending.remove(&id);
                return Err(anyhow!("Figma did not respond to \"{command}\" within {}s", number_to_string(timeout_ms as f64 / 1000.0)));
            }
        };
        // With more than one file open, say which ones and which is in use.
        if command == "get_document_info" && self.plugin_count() > 1 {
            let spread: Option<Map<String, Value>> = match &result {
                Value::Object(o) => Some(o.clone()),
                Value::Array(a) => Some(a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect()),
                _ => None,
            };
            if let Some(mut o) = spread {
                let files = Self::open_files_for(&self.state.lock().unwrap(), Some(socket));
                o.insert("openFiles".into(), files);
                return Ok(Value::Object(o));
            }
        }
        Ok(result)
    }
}

/// `send(command, params, timeoutMs)` on the raw (unguarded) connection.
pub struct RawSend(pub Arc<Bridge>);

impl PluginSend for RawSend {
    fn send(&self, command: &str, params: Value, timeout_ms: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>> {
        self.0.clone().send_inner(command.to_string(), params, timeout_ms.unwrap_or(COMMAND_TIMEOUT_MS)).boxed()
    }
    fn epoch(&self) -> u64 {
        self.0.epoch()
    }
}

impl Bridge {
    pub fn send(self: &Arc<Self>, command: &str, params: Value, timeout_ms: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>> {
        self.clone().send_inner(command.to_string(), params, timeout_ms.unwrap_or(COMMAND_TIMEOUT_MS)).boxed()
    }
}

// ---------- WebSocket server ----------

/// Every request on the plugin port: a WebSocket upgrade from the plugin, or 426 for plain HTTP.
pub async fn ws_entry(State(bridge): State<Arc<Bridge>>, req: Request) -> Response {
    let is_upgrade = req
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    if !is_upgrade {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    // The plugin iframe sends Origin "null"; refusing real origins stops web pages from driving Figma.
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        if origin.as_bytes() != b"null" {
            return (StatusCode::UNAUTHORIZED, [(header::CONTENT_TYPE, "text/html")], "Unauthorized").into_response();
        }
    }
    let (mut parts, _body) = req.into_parts();
    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    ws.max_message_size(100 << 20)
        .max_frame_size(100 << 20)
        .on_upgrade(move |socket| run_socket(bridge, socket))
}

async fn run_socket(bridge: Arc<Bridge>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let id = bridge.on_connect(tx);
    let writer = tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            match out {
                Out::Text(t) => {
                    if sink.send(Message::Text(t.into())).await.is_err() {
                        break;
                    }
                }
                Out::Close => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(t)) => bridge.on_message(id, t.as_str()),
            Ok(Message::Binary(b)) => bridge.on_message(id, &String::from_utf8_lossy(&b)),
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    if let Some(c) = bridge.state.lock().unwrap().plugins.get_mut(&id) {
        c.open = false;
    }
    bridge.on_close(id);
    writer.abort();
}
