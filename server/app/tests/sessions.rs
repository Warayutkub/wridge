//! Several AI sessions at once (session.rs, bridge.rs, guards.rs): each keeps its own file and file check,
//! commands to one file take turns (a run_batch is one turn), and different files run side by side.
//! Two fake plugin windows over real WebSockets; each answers after a delay and logs when work starts/ends.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use wridge_server::app::{App, Ctx};
use wridge_server::jsjson::stringify;
use wridge_server::paths::Paths;

fn paths() -> Paths {
    let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = app_dir.parent().unwrap().parent().unwrap().to_path_buf();
    Paths::new(root, app_dir)
}

/// (file, command, name param, "start"|"end", ms since the test began)
type Log = Arc<Mutex<Vec<(String, String, String, &'static str, u128)>>>;

/// A plugin window for `file` that works on every command for `delay`, several at once if sent at once
/// (like the real plugin, which has no queue of its own).
async fn window(ws_port: u16, file: &'static str, delay: Duration, log: Log, t0: Instant) -> tokio::task::JoinHandle<()> {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{ws_port}")).await.unwrap();
    let (mut tx, mut rx) = socket.split();
    tx.send(Message::Text(stringify(&json!({"type": "hello", "fileName": file, "editorType": "figma", "fileKey": format!("key-{file}"), "version": "9.9.9"})).into())).await.unwrap();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if tx.send(Message::Text(m.into())).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(Message::Text(t))) = rx.next().await {
            let msg: Value = serde_json::from_str(t.as_str()).unwrap();
            if msg["type"] == "status" {
                continue;
            }
            let (log, out) = (log.clone(), out_tx.clone());
            tokio::spawn(async move {
                let command = msg["command"].as_str().unwrap().to_string();
                let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
                log.lock().unwrap().push((file.into(), command.clone(), name.clone(), "start", t0.elapsed().as_millis()));
                if command != "file_name" {
                    tokio::time::sleep(delay).await;
                }
                log.lock().unwrap().push((file.into(), command.clone(), name, "end", t0.elapsed().as_millis()));
                let result = match command.as_str() {
                    "get_document_info" => json!({"fileName": file, "pages": []}),
                    "file_name" => json!({"fileName": file}),
                    _ => json!({"id": "9:1", "file": file}),
                };
                let _ = out.send(stringify(&json!({"id": msg["id"], "result": result})));
            });
        }
    })
}

async fn start() -> (Ctx, u16, u16) {
    let ws = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (ws_port, http_port) = (ws.local_addr().unwrap().port(), http.local_addr().unwrap().port());
    let app = App::new(paths(), ws_port, http_port);
    let ws_app = wridge_server::server::ws_router(&app);
    tokio::spawn(async move { axum::serve(ws, ws_app).await });
    let http_app = wridge_server::server::router(app.clone());
    tokio::spawn(async move { axum::serve(http, http_app).await });
    (app, ws_port, http_port)
}

/// POST /api/tool as `session` (None = no header = the shared session).
async fn tool(http_port: u16, session: Option<&str>, name: &str, args: Value) -> Value {
    let mut req = reqwest::Client::new().post(format!("http://127.0.0.1:{http_port}/api/tool")).body(stringify(&json!({"name": name, "args": args})));
    if let Some(s) = session {
        req = req.header("X-Wridge-Session", s);
    }
    serde_json::from_str(&req.send().await.unwrap().text().await.unwrap()).unwrap()
}

fn text(r: &Value) -> String {
    r["content"][0]["text"].as_str().unwrap_or("").to_string()
}

fn file_of(r: &Value) -> String {
    let v: Value = serde_json::from_str(&text(r)).unwrap_or(Value::Null);
    v["fileName"].as_str().or(v["file"].as_str()).unwrap_or("").to_string()
}

#[tokio::test]
async fn sessions_keep_their_own_file_and_check() {
    let (app, ws_port, http_port) = start().await;
    let log: Log = Default::default();
    let t0 = Instant::now();
    window(ws_port, "alpha", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    window(ws_port, "beta", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    // A starts in the active file (alpha) and stays there; B switches to beta
    assert_eq!(file_of(&tool(http_port, Some("A"), "get_document_info", json!({})).await), "alpha");
    let r = tool(http_port, Some("B"), "use_file", json!({"file": "beta"})).await;
    assert!(text(&r).contains("\"active\":\"beta\""), "{r}");
    assert_eq!(file_of(&tool(http_port, Some("B"), "get_document_info", json!({})).await), "beta");
    // B's switch moved neither A nor the shared session (nor the dashboard)
    assert_eq!(file_of(&tool(http_port, Some("A"), "get_document_info", json!({})).await), "alpha");
    assert_eq!(file_of(&tool(http_port, None, "get_document_info", json!({})).await), "alpha");
    assert_eq!(app.bridge.file_name(), json!("alpha"));
    // writes land in each session's own file
    assert_eq!(file_of(&tool(http_port, Some("A"), "create_frame", json!({"name": "a1"})).await), "alpha");
    assert_eq!(file_of(&tool(http_port, Some("B"), "create_frame", json!({"name": "b1"})).await), "beta");
    // get_document_info marks the caller's file as active in openFiles
    let v: Value = serde_json::from_str(&text(&tool(http_port, Some("B"), "get_document_info", json!({})).await)).unwrap();
    let active: Vec<&str> = v["openFiles"].as_array().unwrap().iter().filter(|f| f["active"] == json!(true)).map(|f| f["fileName"].as_str().unwrap()).collect();
    assert_eq!(active, vec!["beta"]);

    // the file check is per session: C never checked, so C cannot write even though A and B did
    let r = tool(http_port, Some("C"), "create_frame", json!({"name": "c1"})).await;
    assert!(text(&r).starts_with("Check which Figma file is open"), "{r}");

    // /api/sessions shows who works where
    let s: Value = serde_json::from_str(&reqwest::get(format!("http://127.0.0.1:{http_port}/api/sessions")).await.unwrap().text().await.unwrap()).unwrap();
    let alpha = s["files"].as_array().unwrap().iter().find(|f| f["fileName"] == json!("alpha")).unwrap().clone();
    let beta = s["files"].as_array().unwrap().iter().find(|f| f["fileName"] == json!("beta")).unwrap().clone();
    assert_eq!(alpha["sessions"], json!(["shared", "A"])); // C was refused before reaching Figma
    assert_eq!(beta["sessions"], json!(["B"]));
}

#[tokio::test]
async fn one_file_takes_turns_and_two_files_run_together() {
    let (_app, ws_port, http_port) = start().await;
    let log: Log = Default::default();
    let t0 = Instant::now();
    let delay = Duration::from_millis(300);
    window(ws_port, "alpha", delay, log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    window(ws_port, "beta", delay, log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    for s in ["A", "B", "C"] {
        tool(http_port, Some(s), "get_document_info", json!({})).await;
    }
    tool(http_port, Some("C"), "use_file", json!({"file": "beta"})).await;
    tool(http_port, Some("C"), "get_document_info", json!({})).await;
    log.lock().unwrap().clear();

    // A and B write to alpha at the same moment; C writes to beta at the same moment
    let started = Instant::now();
    let (a, b, c) = tokio::join!(
        tool(http_port, Some("A"), "create_frame", json!({"name": "a"})),
        tool(http_port, Some("B"), "create_frame", json!({"name": "b"})),
        tool(http_port, Some("C"), "create_frame", json!({"name": "c"})),
    );
    let took = started.elapsed();
    assert_eq!((file_of(&a), file_of(&b), file_of(&c)), ("alpha".into(), "alpha".into(), "beta".into()));
    let entries = log.lock().unwrap().clone();
    // in alpha, one create_frame ended before the other started
    let alpha: Vec<_> = entries.iter().filter(|e| e.0 == "alpha" && e.1 == "create_frame").collect();
    assert_eq!(alpha.len(), 4, "{entries:?}");
    assert_eq!(alpha.iter().map(|e| e.3).collect::<Vec<_>>(), vec!["start", "end", "start", "end"], "{entries:?}");
    // beta ran alongside alpha: the whole thing took about two turns (600 ms), not three (900 ms)
    assert!(took < Duration::from_millis(850), "took {took:?}");
    let beta_start = entries.iter().find(|e| e.0 == "beta" && e.1 == "create_frame" && e.3 == "start").unwrap().4;
    let alpha_end = alpha[1].4;
    assert!(beta_start < alpha_end, "beta waited for alpha: {entries:?}");
}

#[tokio::test]
async fn a_batch_is_one_turn() {
    let (_app, ws_port, http_port) = start().await;
    let log: Log = Default::default();
    let t0 = Instant::now();
    window(ws_port, "alpha", Duration::from_millis(120), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    tool(http_port, Some("A"), "get_document_info", json!({})).await;
    tool(http_port, Some("B"), "get_document_info", json!({})).await;
    log.lock().unwrap().clear();

    let batch = json!({"steps": [
        {"tool": "create_frame", "args": {"name": "a1"}},
        {"tool": "create_frame", "args": {"name": "a2"}},
        {"tool": "create_frame", "args": {"name": "a3"}},
    ]});
    let (a, b) = tokio::join!(tool(http_port, Some("A"), "run_batch", batch), async {
        tokio::time::sleep(Duration::from_millis(60)).await; // lands while a1 is running
        tool(http_port, Some("B"), "create_frame", json!({"name": "b1"})).await
    });
    assert_ne!(a["isError"], json!(true), "{a}");
    assert_ne!(b["isError"], json!(true), "{b}");
    let order: Vec<String> = log.lock().unwrap().iter().filter(|e| e.1 == "create_frame" && e.3 == "start").map(|e| e.2.clone()).collect();
    assert_eq!(order, vec!["a1", "a2", "a3", "b1"]);
}

#[tokio::test]
async fn a_file_busy_for_too_long_says_so() {
    let (app, ws_port, http_port) = start().await;
    app.bridge.set_queue_wait_ms(200);
    let log: Log = Default::default();
    window(ws_port, "alpha", Duration::from_millis(700), log.clone(), Instant::now()).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (a, b) = tokio::join!(tool(http_port, Some("A"), "get_selection", json!({})), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        tool(http_port, Some("B"), "get_selection", json!({})).await
    });
    assert_ne!(a["isError"], json!(true), "{a}");
    assert_eq!(b["isError"], json!(true), "{b}");
    assert!(text(&b).starts_with("Figma file \"alpha\" stayed busy for 0.2s"), "{b}");
    // the waiting count went back down
    let s: Value = serde_json::from_str(&reqwest::get(format!("http://127.0.0.1:{http_port}/api/sessions")).await.unwrap().text().await.unwrap()).unwrap();
    assert_eq!(s["files"][0]["waiting"], json!(0));
}

#[tokio::test]
async fn a_closed_file_sends_its_sessions_back_to_the_active_one() {
    let (_app, ws_port, http_port) = start().await;
    let log: Log = Default::default();
    let t0 = Instant::now();
    window(ws_port, "alpha", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let beta = window(ws_port, "beta", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    tool(http_port, Some("B"), "use_file", json!({"file": "beta"})).await;
    assert_eq!(file_of(&tool(http_port, Some("B"), "get_document_info", json!({})).await), "beta");
    beta.abort(); // the window goes away
    tokio::time::sleep(Duration::from_millis(150)).await;
    // B checked beta, which is gone: its next write is refused until it checks again
    let r = tool(http_port, Some("B"), "create_frame", json!({"name": "x"})).await;
    assert!(text(&r).contains("changed from \"beta\" to \"alpha\""), "{r}");
    assert_eq!(file_of(&tool(http_port, Some("B"), "get_document_info", json!({})).await), "alpha");
}

#[tokio::test]
async fn mcp_hands_out_a_session_and_keeps_it_apart() {
    let (app, ws_port, http_port) = start().await;
    let log: Log = Default::default();
    let t0 = Instant::now();
    window(ws_port, "alpha", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    window(ws_port, "beta", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let client = reqwest::Client::new();
    let post = |id: Option<String>, body: Value| {
        let mut r = client
            .post(format!("http://127.0.0.1:{http_port}/mcp?profile=core"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18");
        if let Some(id) = id {
            r = r.header("mcp-session-id", id);
        }
        r.body(stringify(&body)).send()
    };
    let init = post(None, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}})).await.unwrap();
    let sid = init.headers().get("mcp-session-id").expect("a session id on initialize").to_str().unwrap().to_string();
    let _ = init.text().await;
    let call = |name: &str, args: Value| json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let body = post(Some(sid.clone()), call("use_file", json!({"file": "beta"}))).await.unwrap().text().await.unwrap();
    assert!(body.contains("\\\"active\\\":\\\"beta\\\""), "{body}");
    let body = post(Some(sid.clone()), call("get_document_info", json!({}))).await.unwrap().text().await.unwrap();
    assert!(body.contains("\\\"fileName\\\":\\\"beta\\\""), "{body}");
    // the shared session (and the dashboard) never moved
    assert_eq!(app.bridge.file_name(), json!("alpha"));
    assert_eq!(file_of(&tool(http_port, None, "get_document_info", json!({})).await), "alpha");
    // DELETE ends the session: it follows the active file again
    client
        .delete(format!("http://127.0.0.1:{http_port}/mcp"))
        .header("mcp-session-id", sid.clone())
        .header("mcp-protocol-version", "2025-06-18")
        .send()
        .await
        .unwrap();
    let body = post(Some(sid), call("get_document_info", json!({}))).await.unwrap().text().await.unwrap();
    assert!(body.contains("\\\"fileName\\\":\\\"alpha\\\""), "{body}");
}

#[tokio::test]
async fn the_window_picks_its_file_by_key_without_moving_anyone() {
    let (app, ws_port, http_port) = start().await;
    let log: Log = Default::default();
    let t0 = Instant::now();
    window(ws_port, "alpha", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    window(ws_port, "beta", Duration::from_millis(5), log.clone(), t0).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let client = reqwest::Client::new();
    let get = |session: &'static str| {
        let c = client.clone();
        async move { serde_json::from_str::<Value>(&c.get(format!("http://127.0.0.1:{http_port}/api/sessions")).header("X-Wridge-Session", session).send().await.unwrap().text().await.unwrap()).unwrap() }
    };
    // an AI works in alpha
    tool(http_port, Some("A"), "get_document_info", json!({})).await;
    // the window picks beta by its file key
    let r: Value = serde_json::from_str(
        &client.post(format!("http://127.0.0.1:{http_port}/api/figma-file")).header("X-Wridge-Session", "app").body(stringify(&json!({"file": "key-beta"}))).send().await.unwrap().text().await.unwrap(),
    )
    .unwrap();
    let showing: Vec<&str> = r["files"].as_array().unwrap().iter().filter(|f| f["showing"] == json!(true)).map(|f| f["fileName"].as_str().unwrap()).collect();
    assert_eq!(showing, vec!["beta"], "{r}");
    // keys and plugin versions come through
    let beta = r["files"].as_array().unwrap().iter().find(|f| f["fileName"] == json!("beta")).unwrap();
    assert_eq!((beta["fileKey"].clone(), beta["pluginVersion"].clone()), (json!("key-beta"), json!("9.9.9")));
    // pages the window loads read beta; the AI still reads alpha; the window is not counted as an AI
    assert_eq!(file_of(&tool(http_port, Some("app"), "get_document_info", json!({})).await), "beta");
    assert_eq!(file_of(&tool(http_port, Some("A"), "get_document_info", json!({})).await), "alpha");
    let s = get("A").await;
    let ai: Vec<(String, u64)> = s["files"].as_array().unwrap().iter().map(|f| (f["fileName"].as_str().unwrap().to_string(), f["ai"].as_u64().unwrap())).collect();
    assert_eq!(ai, vec![("alpha".to_string(), 1), ("beta".to_string(), 0)]);
    assert_eq!(app.bridge.file_name(), json!("alpha"));
    // an unknown file is a 404, and the pick stays
    let res = client.post(format!("http://127.0.0.1:{http_port}/api/figma-file")).header("X-Wridge-Session", "app").body(stringify(&json!({"file": "nope"}))).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 404);
    let s = get("app").await;
    assert_eq!(s["files"].as_array().unwrap().iter().find(|f| f["showing"] == json!(true)).unwrap()["fileName"], json!("beta"));
    // CORS lets the window send its session header
    let pre = client.request(reqwest::Method::OPTIONS, format!("http://127.0.0.1:{http_port}/api/sessions")).header("Origin", "http://tauri.localhost").send().await.unwrap();
    assert!(pre.headers()["access-control-allow-headers"].to_str().unwrap().contains("X-Wridge-Session"));
}
