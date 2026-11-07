//! In-process checks of the server: tool schemas and profiles against snapshots taken from the Node server
//! (GET /api/tools and MCP tools/list per profile, 2026-11-06), and a fake plugin over a real WebSocket.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use wridge_server::app::{tool_schema, App};
use wridge_server::jsjson::stringify;
use wridge_server::paths::Paths;

fn paths() -> Paths {
    let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = app_dir.parent().unwrap().parent().unwrap().to_path_buf();
    Paths::new(root, app_dir)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)).unwrap()
}

#[test]
fn tool_names_descriptions_and_schemas_match_node() {
    let app = App::new(paths(), 3255, 3256);
    let list: Vec<Value> = app
        .tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "params": tool_schema(t)}))
        .collect();
    let ours = stringify(&Value::Array(list));
    let node = fixture("node-api-tools.json");
    if ours != node {
        let i = ours.bytes().zip(node.bytes()).take_while(|(a, b)| a == b).count();
        panic!(
            "differs at byte {i}:\n ours: …{}\n node: …{}",
            &ours[i.saturating_sub(80)..(i + 120).min(ours.len())],
            &node[i.saturating_sub(80)..(i + 120).min(node.len())]
        );
    }
}

#[test]
fn profiles_list_the_same_tools_as_node() {
    let app = App::new(paths(), 3255, 3256);
    let node: serde_json::Map<String, Value> = serde_json::from_str(&fixture("node-profiles.json")).unwrap();
    for (profile, names) in node {
        let ours: Vec<Value> = app.profile_tools(&profile).iter().map(|t| json!(t.name)).collect();
        assert_eq!(Value::Array(ours), names, "profile {profile}");
    }
}

/// A fake plugin window: hello, then answers every command (and records it).
async fn fake_plugin(ws_port: u16, file: &'static str) -> Arc<std::sync::Mutex<Vec<Value>>> {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{ws_port}")).await.unwrap();
    let (mut tx, mut rx) = socket.split();
    tx.send(Message::Text(stringify(&json!({"type": "hello", "fileName": file, "editorType": "figma"})).into()))
        .await
        .unwrap();
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Some(Ok(Message::Text(t))) = rx.next().await {
            let msg: Value = serde_json::from_str(t.as_str()).unwrap();
            if msg["type"] == "status" {
                continue;
            }
            log.lock().unwrap().push(json!({"command": msg["command"], "params": msg["params"]}));
            let reply = match msg["command"].as_str().unwrap() {
                "get_document_info" => json!({"id": msg["id"], "result": {"fileName": file, "pages": []}}),
                "file_name" => json!({"id": msg["id"], "result": {"fileName": file}}),
                "delete_node" => json!({"id": msg["id"], "error": "Node 1:2 not found"}),
                _ => json!({"id": msg["id"], "result": {"id": "9:1", "params": msg["params"]}}),
            };
            tx.send(Message::Text(stringify(&reply).into())).await.unwrap();
        }
    });
    seen
}

#[tokio::test]
async fn tools_reach_a_plugin_over_the_websocket() {
    let ws = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (ws_port, http_port) = (ws.local_addr().unwrap().port(), http.local_addr().unwrap().port());
    let app = App::new(paths(), ws_port, http_port);
    let ws_app = wridge_server::server::ws_router(&app);
    tokio::spawn(async move { axum::serve(ws, ws_app).await });
    let http_app = wridge_server::server::router(app.clone());
    tokio::spawn(async move { axum::serve(http, http_app).await });

    let seen = fake_plugin(ws_port, "utils").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(app.bridge.plugin_connected(), json!(true));
    assert_eq!(app.bridge.file_name(), json!("utils"));

    let run = |name: &str, args: Value| {
        let app = app.clone();
        let name = name.to_string();
        async move {
            let t = app.find_tool(Some(&json!(name))).unwrap();
            let parsed = wridge_server::zod::safe_parse(&t.shape, Some(&args)).unwrap();
            app.run_tool(&t, parsed, "CLI").await
        }
    };
    // the guard: no write before a check
    let r = run("create_frame", json!({})).await;
    assert_eq!(r["isError"], json!(true));
    assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Check which Figma file is open"));
    run("get_document_info", json!({})).await;
    let r = run("mcp_wara-bridge_create_frame", json!({"name": "Card"})).await;
    let v: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(v["params"]["layoutMode"], json!("VERTICAL"));
    assert_eq!(v["params"]["width"], json!(375));
    let r = run("delete_node", json!({"nodeId": "1:2"})).await;
    assert_eq!(r, json!({"content": [{"type": "text", "text": "Node 1:2 not found"}], "isError": true}));
    let commands: Vec<Value> = seen.lock().unwrap().iter().map(|c| c["command"].clone()).collect();
    assert_eq!(commands, vec![json!("get_document_info"), json!("create_frame"), json!("delete_node")]);

    // the same through HTTP and MCP
    let client = reqwest::Client::new();
    let body: Value = serde_json::from_str(
        &client
            .post(format!("http://127.0.0.1:{http_port}/api/tool"))
            .body(stringify(&json!({"name": "get_selection", "args": {}})))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["content"][0]["text"], json!("{\"id\":\"9:1\",\"params\":{\"depth\":1}}"));
    let sse = client
        .post(format!("http://127.0.0.1:{http_port}/mcp?profile=core"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(stringify(
            &json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "get_node", "arguments": {"nodeId": "1:1"}}}),
        ))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(sse, "event: message\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"{\\\"id\\\":\\\"9:1\\\",\\\"params\\\":{\\\"nodeId\\\":\\\"1:1\\\",\\\"depth\\\":2}}\"}]},\"jsonrpc\":\"2.0\",\"id\":7}\n\n");

    let status: Value = serde_json::from_str(&client.get(format!("http://127.0.0.1:{http_port}/api/status")).send().await.unwrap().text().await.unwrap()).unwrap();
    assert_eq!(status["today"]["count"], json!(6));
    assert_eq!(status["today"]["errors"], json!(2));
    assert_eq!(status["history"][0]["source"], json!("MCP"));
}
