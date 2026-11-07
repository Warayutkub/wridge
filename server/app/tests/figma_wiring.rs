//! The tools wired to wridge-figma, in process: a fake plugin answers over a real WebSocket, files go to a
//! temp folder. The full comparison with Node is parity/figma-wiring.mjs; this guards the wiring itself.

use std::path::PathBuf;
use std::time::Duration;

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

const SVG: &str =
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect id="Box" width="20" height="20" fill="#000"/><circle id="Dot" cx="30" cy="10" r="5"/></svg>"##;

fn motion_read() -> Value {
    let track = |field: &str, a: f64, b: f64| json!({"field": field, "base": 1, "tracks": [{"op": "SET", "keys": [{"t": 0, "v": a, "ease": "EASE_OUT"}, {"t": 0.5, "v": b}]}]});
    json!({
        "root": {"id": "1:1", "name": "Card"}, "timelines": [{"duration": 1}],
        "animated": [
            {"id": "1:2", "name": "Box", "type": "RECTANGLE", "parentId": "1:1", "nth": 1, "path": [], "styles": [], "tracks": [track("OPACITY", 0.0, 1.0)],
             "hint": {"text": false, "instances": 0, "autoLayout": false, "vectorOnly": true, "image": false, "component": null}},
        ],
    })
}

fn answer(command: &str, params: &Value) -> Result<Value, String> {
    let b64 = |s: &str| wridge_server::util::base64_encode(s.as_bytes());
    Ok(match command {
        "get_document_info" => json!({"fileName": "utils", "pages": [], "topLevel": []}),
        "file_name" => json!({"fileName": "utils"}),
        "motion_read" => motion_read(),
        "export_assets" => Value::Array(
            params["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|it| {
                    let svg = it["format"] == "svg" || it["format"] == "auto";
                    let data = if svg { b64(SVG) } else { b64(&format!("PNG {}", it["scale"])) };
                    json!({"nodeId": it["nodeId"], "name": "Icon / Close", "format": if svg { "svg" } else { "png" }, "base64": data})
                })
                .collect(),
        ),
        "figjam_draw" => json!({"ids": {"$section": "20:0", "a": "20:1", "b": "20:2"}, "section": {"id": "20:0", "name": params["section"]["name"]}}),
        "board_map" => json!({"file": "Board", "editorType": "figjam", "sections": [], "loose": [{"id": "9:1", "kind": "sticky", "text": "hi", "x": 0, "y": 0, "w": 10, "h": 10}]}),
        "get_tokens" => json!({"sources": ["variables"], "file": "t", "collections": [{"name": "c", "defaultMode": "Light"}],
            "variables": [{"token": "color-tile", "type": "COLOR", "collection": "c", "values": {"Light": "#ffffff"}}], "paintStyles": [], "textStyles": [], "effectStyles": []}),
        "get_node" if params["nodeId"] == "boom" => return Err("Node boom not found".into()),
        _ => json!({"id": "9:1", "params": params}),
    })
}

async fn fake_plugin(ws_port: u16) {
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{ws_port}")).await.unwrap();
    let (mut tx, mut rx) = socket.split();
    tx.send(Message::Text(stringify(&json!({"type": "hello", "fileName": "utils", "editorType": "figma"})).into()))
        .await
        .unwrap();
    tokio::spawn(async move {
        while let Some(Ok(Message::Text(t))) = rx.next().await {
            let msg: Value = serde_json::from_str(t.as_str()).unwrap();
            if msg["type"] == "status" {
                continue;
            }
            let reply = match answer(msg["command"].as_str().unwrap(), &msg["params"]) {
                Ok(result) => json!({"id": msg["id"], "result": result}),
                Err(error) => json!({"id": msg["id"], "error": error}),
            };
            tx.send(Message::Text(stringify(&reply).into())).await.unwrap();
        }
    });
}

async fn server() -> Ctx {
    let ws = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app = App::new(paths(), ws.local_addr().unwrap().port(), http.local_addr().unwrap().port());
    let ws_app = wridge_server::server::ws_router(&app);
    tokio::spawn(async move { axum::serve(ws, ws_app).await });
    let http_app = wridge_server::server::router(app.clone());
    tokio::spawn(async move { axum::serve(http, http_app).await });
    fake_plugin(app.ws_port).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    app
}

async fn run(app: &Ctx, name: &str, args: Value) -> (bool, String) {
    let t = app.tool(name).unwrap();
    let parsed = wridge_server::zod::safe_parse(&t.shape, Some(&args)).unwrap();
    let r = app.run_tool(&t, parsed, "CLI").await;
    (r["isError"] == json!(true), r["content"][0]["text"].as_str().unwrap_or_default().to_string())
}

fn slash(p: &std::path::Path) -> String {
    p.display().to_string().replace('\\', "/")
}

#[tokio::test]
async fn design_to_code_tools_write_into_the_project() {
    let app = server().await;
    let dir = tempfile::tempdir().unwrap();
    let root = slash(dir.path());

    // component map: written sorted, read back, used to annotate nothing on a context without instances
    let (err, text) = run(
        &app,
        "map_component",
        json!({"projectDir": root, "figmaName": "Button", "import": "@/ui/button", "component": "Button"}),
    )
    .await;
    assert!(!err, "{text}");
    assert_eq!(text, format!("{{\"file\":\"{root}/wara.components.json\",\"entries\":1}}"));
    run(&app, "map_component", json!({"projectDir": root, "figmaName": "10", "className": "ten"})).await;
    let (_, text) = run(&app, "get_component_map", json!({"projectDir": root})).await;
    assert_eq!(text, "{\"10\":{\"className\":\"ten\"},\"Button\":{\"import\":\"@/ui/button\",\"component\":\"Button\"}}");
    let (err, text) = run(&app, "map_component", json!({"projectDir": root, "figmaName": "X", "element": "div"})).await;
    assert!(err);
    assert_eq!(text, "Pass import + component, or className (+ element), unless remove is true");
    let (err, text) = run(&app, "get_component_map", json!({"projectDir": "relative"})).await;
    assert!(err);
    assert_eq!(text, "projectDir must be an absolute path, got \"relative\"");

    // export_assets: kebab names, a numbered duplicate, Flutter 2.0x / 3.0x variants for PNGs
    let (err, text) = run(&app, "export_assets", json!({"nodeIds": ["1:1", "1:2"], "outDir": format!("{root}/web")})).await;
    assert!(!err, "{text}");
    assert!(text.contains("/web/icon-close.svg\"") && text.contains("/web/icon-close_2.svg\""), "{text}");
    let (_, text) = run(
        &app,
        "export_assets",
        json!({"nodeIds": ["1:1"], "outDir": format!("{root}/flutter"), "platform": "flutter", "format": "png"}),
    )
    .await;
    let files: Vec<Value> = serde_json::from_str(&text).unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(std::fs::read(dir.path().join("flutter").join("3.0x").join("icon_close.png")).unwrap(), b"PNG 3");

    // get_tokens: content + map, written when outFile is given
    let (_, text) = run(&app, "get_tokens", json!({"format": "css", "outFile": format!("{root}/tokens.css")})).await;
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["map"], json!({"color-tile": "--color-tile"}));
    assert_eq!(std::fs::read_to_string(dir.path().join("tokens.css")).unwrap(), v["content"].as_str().unwrap());

    // create_lottie from the node's SVG: the recipe rides along in meta
    let (err, text) = run(
        &app,
        "create_lottie",
        json!({"projectDir": root, "out": "lottie/box.json", "nodeId": "1:1", "duration": 600, "tracks": [{"layer": "Box", "prop": "opacity", "from": 0, "to": 1}]}),
    )
    .await;
    assert!(!err, "{text}");
    assert!(text.starts_with("Wrote lottie/box.json ("), "{text}");
    let lottie: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("lottie").join("box.json")).unwrap()).unwrap();
    assert_eq!(lottie["meta"]["wara"]["source"], "spec");
    assert_eq!(lottie["meta"]["wara"]["spec"]["nodeId"], "1:1");
    let (err, text) = run(&app, "create_lottie", json!({"projectDir": root, "out": "../x.json", "svg": SVG, "duration": 600})).await;
    assert!(err);
    assert!(text.starts_with("out must be inside "), "{text}");

    // motion_to_lottie: the Motion timeline becomes the spec
    let (err, text) = run(&app, "motion_to_lottie", json!({"projectDir": root, "out": "lottie/card.json", "nodeId": "1:1"})).await;
    assert!(!err, "{text}");
    assert!(text.contains(" from the Figma Motion timeline\n"), "{text}");
}

#[tokio::test]
async fn motion_figjam_and_pdf() {
    let app = server().await;
    let (_, text) = run(&app, "motion_read", json!({"nodeId": "1:1"})).await;
    assert!(
        text.starts_with("Motion · Card (1:1) · timeline 1000ms\n\nBox (1:2, RECTANGLE)\n  OPACITY base 1 · 0ms 0 EASE_OUT → 500ms 1"),
        "{text}"
    );
    let (_, text) = run(&app, "motion_check", json!({"nodeId": "1:1", "context": "ui"})).await;
    assert!(text.starts_with("Motion handoff · Card (1:1) · 1 animated layer(s) · context ui"), "{text}");
    let (_, text) = run(&app, "motion_tokens", json!({"nodeIds": ["1:1"]})).await;
    assert!(text.starts_with("## CSS\n:root {"), "{text}");

    // design context: the motion summary rides along
    let (_, text) = run(&app, "get_design_context", json!({"nodeId": "1:1"})).await;
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["motion"]["timelineMs"], json!(1000));
    assert_eq!(v["motion"]["layers"][0]["name"], json!("Box"));

    // figjam_diagram: ELK lays it out (QuickJS), the plugin draws it
    run(&app, "get_document_info", json!({})).await;
    let (err, text) = run(
        &app,
        "figjam_diagram",
        json!({"title": "T", "nodes": [{"id": "a", "text": "one"}, {"id": "b", "text": "two"}], "edges": [{"from": "a", "to": "b"}]}),
    )
    .await;
    assert!(!err, "{text}");
    assert_eq!(
        text,
        "Drew \"T\" (20:0) · 2 items, 1 arrows.\nNode ids → FigJam ids: a=20:1, b=20:2\nRead edits back later with board_map."
    );
    let (err, text) = run(&app, "figjam_diagram", json!({"title": "T", "nodes": [{"id": "a"}], "edges": [{"from": "a", "to": "zz"}]})).await;
    assert!(err);
    assert_eq!(text, "Edge a → zz: unknown node (ids: a)");
    let (_, text) = run(&app, "board_map", json!({})).await;
    assert!(text.starts_with("Board \"Board\" (figjam)"), "{text}");

    // export_pdf: a non-page node is refused before anything is exported
    let dir = tempfile::tempdir().unwrap();
    let (err, text) = run(&app, "export_pdf", json!({"nodeId": "boom", "out": slash(dir.path())})).await;
    assert!(err);
    assert_eq!(text, "Node boom not found");
    let (err, text) = run(&app, "export_pdf", json!({"nodeId": "1:1", "out": "rel.pdf"})).await;
    assert!(err);
    assert_eq!(text, "out must be an absolute path, got \"rel.pdf\"");
}
