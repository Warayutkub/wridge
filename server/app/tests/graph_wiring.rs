//! The routes, tools and projects list wired to wridge-graph, in process, on a temp copy of the graph crate's
//! wave2 fixture and a temp projects list. The full comparison with Node is parity/graph-wiring.mjs; this
//! guards the wiring itself (and that the real legacy-node/code-projects.json is never touched).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::net::TcpListener;
use wridge_server::app::App;
use wridge_server::jsjson::stringify;
use wridge_server::paths::Paths;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

struct Server {
    http: u16,
    client: reqwest::Client,
}

impl Server {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = self.client.request(method, format!("http://127.0.0.1:{}{path}", self.http));
        if let Some(b) = body {
            req = req.header("content-type", "application/json").body(stringify(&b));
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        (status, serde_json::from_str(&res.text().await.unwrap()).unwrap())
    }
    async fn get(&self, path: &str) -> (u16, Value) {
        self.call(reqwest::Method::GET, path, None).await
    }
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }
    async fn tool(&self, name: &str, args: Value) -> Value {
        self.post("/api/tool", json!({"name": name, "args": args})).await.1
    }
}

#[tokio::test]
async fn graph_routes_and_tools_use_their_own_projects_list() {
    let app_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = app_dir.parent().unwrap().parent().unwrap().to_path_buf();
    let real_list = root.join("legacy-node").join("code-projects.json");
    let real_before = std::fs::read(&real_list).ok();

    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("wave2");
    copy_dir(&app_dir.join("..").join("graph").join("parity").join("fixtures").join("wave2"), &project);
    let dir = project.to_string_lossy().replace('\\', "/");
    let q = |d: &str| d.replace(' ', "%20");
    let mut paths = Paths::new(root.clone(), app_dir.clone());
    paths.projects_file = tmp.path().join("code-projects.json");
    let list_file = paths.projects_file.clone();

    let ws = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (ws_port, http_port) = (ws.local_addr().unwrap().port(), http.local_addr().unwrap().port());
    let app = App::new(paths, ws_port, http_port);
    let http_app = wridge_server::server::router(app.clone());
    tokio::spawn(async move { axum::serve(http, http_app).await });
    let s = Server { http: http_port, client: reqwest::Client::new() };

    // the list: empty, then the project (its graph built first), then gone again
    assert_eq!(s.get("/api/code/projects").await, (200, json!([])));
    assert_eq!(s.get(&format!("/api/brain?dir={}", q(&dir))).await, (404, json!({"error": "Add this project first"})));
    let (status, info) = s.post("/api/code/projects", json!({"dir": dir})).await;
    assert_eq!(status, 200, "{info}");
    assert_eq!(info["files"], json!(17));
    let (_, list) = s.get("/api/code/projects").await;
    assert_eq!(list[0]["dir"], json!(dir));
    assert_eq!((list[0]["exists"].clone(), list[0]["hasGraph"].clone(), list[0]["codeFiles"].clone()), (json!(true), json!(true), json!(12)));
    let (status, err) = s.post("/api/code/projects", json!({"dir": 5})).await;
    assert_eq!((status, err), (500, json!({"error": "The \"path\" argument must be of type string. Received type number (5)"})));

    // pages
    let (status, graph) = s.get(&format!("/api/code/graph?dir={}", q(&dir))).await;
    assert_eq!(status, 200);
    assert_eq!(graph["name"], json!("wave2"));
    assert!(graph["update"]["files"].is_number());
    let (_, brain) = s.get(&format!("/api/brain?dir={}", q(&dir))).await;
    assert!(brain["nodes"].as_array().unwrap().iter().any(|n| n["label"] == "CH1"));
    let (_, flows) = s.get(&format!("/api/flows?dir={}", q(&dir))).await;
    assert_eq!(flows["name"], json!("wave2"));

    // writes: a flow file and its conflict, a brain note to the trash and back
    let content = "# Flow: Test\n\n## Steps\n1. [user] pays `createPayment`\n";
    let (status, saved) = s.call(reqwest::Method::PUT, "/api/flows", Some(json!({"dir": dir, "content": content}))).await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(saved["file"], json!("docs/flows/test.md"));
    let (status, conflict) = s.call(reqwest::Method::PUT, "/api/flows", Some(json!({"dir": dir, "content": content}))).await;
    assert_eq!(status, 409);
    assert_eq!(conflict["currentHash"], saved["hash"]);
    let (status, _) = s.post("/api/brain/note", json!({"dir": dir, "name": "Idea", "content": "See [[CH1]]"})).await;
    assert_eq!(status, 200);
    let (_, trashed) = s.post("/api/brain/trash", json!({"dir": dir, "name": "Idea"})).await;
    let (_, restored) = s.post("/api/brain/restore", json!({"dir": dir, "path": trashed["restore"]})).await;
    assert_eq!(restored, json!({"restored": "brain/Idea.md"}));

    // tools
    let r = s.tool("code_graph", json!({"projectDir": dir, "query": "overview"})).await;
    assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Files: 17"), "{r}");
    let r = s.tool("project_memory", json!({"projectDir": dir, "action": "decide", "title": "T", "decision": "D"})).await;
    assert_eq!(r["content"][0]["text"], json!("{\"id\":\"D-4\",\"file\":\"docs/decisions.md\"}"));
    let r = s.tool("project_memory", json!({"projectDir": dir, "action": "read", "name": "Idea"})).await;
    assert_eq!(r["content"][0]["text"], json!("# Idea — brain/Idea.md\n# Idea\n\nSee [[CH1]]\nLinks: CH1"));
    let r = s.tool("code_graph_note", json!({"projectDir": dir, "target": "lib/db.ts", "text": "One pool."})).await;
    assert_eq!(r["content"][0]["text"], json!("{\"saved\":\"file:lib/db.ts\",\"label\":\"lib/db.ts\"}"));
    let r = s.tool("project_knowledge", json!({"projectDir": dir, "action": "fact", "key": "ci", "value": "none"})).await;
    assert!(r["content"][0]["text"].as_str().unwrap().starts_with("{\"facts\":{\"ci\":{\"value\":\"none\""), "{r}");
    let r = s.tool("code_design_sync", json!({"projectDir": dir})).await;
    assert_eq!(r["isError"], json!(true)); // no plugin

    assert_eq!(s.post("/api/code/projects/remove", json!({"dir": dir.to_uppercase()})).await, (200, json!({"ok": true})));
    assert_eq!(std::fs::read_to_string(&list_file).unwrap(), "[]\n");
    assert_eq!(std::fs::read(&real_list).ok(), real_before, "the real projects list changed");
}
