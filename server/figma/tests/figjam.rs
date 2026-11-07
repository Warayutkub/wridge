//! Port of legacy-node/test/figjam.test.mjs (plus a timing check of the elkjs runtime).
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;
use wridge_figma::figjam::{board_text, layout_diagram};

fn booking() -> Value {
    json!({
        "title": "จองบูธ · System design · Draft",
        "lanes": ["ร้านค้า", "server"],
        "nodes": [
            { "id": "s", "kind": "start", "lane": "ร้านค้า" },
            { "id": "pick", "text": "เลือกบูธ", "lane": "ร้านค้า" },
            { "id": "hold", "text": "ล็อกบูธ 15 นาที", "lane": "server" },
            { "id": "ok", "text": "สลิปตรงไหม", "kind": "decision", "lane": "server" },
            { "id": "ticket", "text": "ออกตั๋ว", "lane": "server" },
            { "id": "e", "kind": "end", "lane": "ร้านค้า" },
        ],
        "edges": [
            { "from": "s", "to": "pick" }, { "from": "pick", "to": "hold" }, { "from": "hold", "to": "ok" },
            { "from": "ok", "to": "ticket", "label": "ตรง" }, { "from": "ok", "to": "pick", "label": "ไม่ตรง" }, { "from": "ticket", "to": "e" },
        ],
        "notes": [{ "text": "ร้านแก้สลิปได้กี่ครั้ง?", "kind": "question" }],
    })
}

fn by_id(d: &Value) -> HashMap<String, Value> {
    d["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap().to_string(), n.clone())).collect()
}
fn n(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn err(spec: Value) -> String {
    layout_diagram(&spec).unwrap_err().to_string()
}

#[test]
fn columns_follow_the_arrows_lanes_follow_the_actor_loops_are_dashed() {
    let d = layout_diagram(&booking()).unwrap();
    let at = by_id(&d);
    let x = |id: &str| n(&at[id]["x"]);
    assert!(x("s") < x("pick") && x("pick") < x("hold") && x("hold") < x("ok") && x("ok") < x("ticket") && x("ticket") < x("e"));
    let lanes = d["lanes"].as_array().unwrap();
    let lane = |id: &str| {
        let node = &at[id];
        lanes.iter().position(|l| n(&node["y"]) >= n(&l["y"]) && n(&node["y"]) + n(&node["height"]) <= n(&l["y"]) + n(&l["height"]))
    };
    assert_eq!(lane("pick"), Some(0));
    assert_eq!(lane("hold"), Some(1));
    assert_eq!(at["ok"]["shape"], "DIAMOND");
    let edges = d["edges"].as_array().unwrap();
    let back = edges.iter().find(|e| e["from"] == "ok" && e["to"] == "pick").unwrap();
    assert_eq!(back["dashed"], true, "going back is a loop");
    assert!(edges.iter().find(|e| e["to"] == "ticket").unwrap().get("dashed").is_none());
    let sticky = d["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "sticky").unwrap();
    assert!(n(&sticky["x"]) > n(&lanes[0]["width"]), "stickies sit beside the diagram");
    assert!(n(&d["section"]["width"]) >= n(&sticky["x"]) + 200.0);
}

#[test]
fn bad_input_says_what_is_wrong() {
    assert!(err(json!({ "nodes": [{ "id": "a", "text": "x" }], "edges": [{ "from": "a", "to": "b" }] })).contains("unknown node"));
    assert!(err(json!({ "lanes": ["A"], "nodes": [{ "id": "a", "text": "x", "lane": "B" }] })).contains("lane \"B\" is not in lanes"));
    assert!(err(json!({ "nodes": [{ "id": "a", "text": "x", "kind": "question" }] })).contains("sticky"));
}

#[test]
fn board_text_steps_with_lanes_arrows_by_diagram_id_stickies() {
    let map = json!({
        "file": "book-lock board", "editorType": "figjam", "loose": [],
        "sections": [{
            "id": "1:1", "name": "จองบูธ · System design · Draft", "lanes": [{ "name": "ร้านค้า", "x": 0, "y": 72, "w": 1100, "h": 160 }, { "name": "server", "x": 0, "y": 232, "w": 1100, "h": 160 }],
            "items": [
                { "id": "2:1", "kind": "shape", "role": "step", "waraId": "pick", "x": 300, "y": 100, "w": 220, "h": 96, "text": "เลือกบูธ" },
                { "id": "2:2", "kind": "shape", "role": "step", "waraId": "hold", "x": 600, "y": 260, "w": 220, "h": 96, "text": "ล็อกบูธ 15 นาที" },
                { "id": "2:3", "kind": "sticky", "role": "question", "x": 1200, "y": 72, "w": 200, "h": 200, "text": "แก้สลิปได้กี่ครั้ง?" },
                { "id": "3:1", "kind": "connector", "from": "2:1", "to": "2:2", "label": "ยืนยัน" },
            ],
        }],
    });
    let t = board_text(&map).unwrap();
    assert!(t.contains("pick · step @ร้านค้า · \"เลือกบูธ\""));
    assert!(t.contains("hold · step @server"));
    assert!(t.contains("pick → hold [ยืนยัน]"));
    assert!(t.contains("sticky question · \"แก้สลิป"), "a sticky beside the lanes has no lane");
}

#[test]
fn state_nodes_default_to_states_transitions_keep_their_labels() {
    let d = layout_diagram(&json!({ "type": "state", "title": "booking", "nodes": [{ "id": "hold", "text": "hold" }, { "id": "paid", "text": "paid" }], "edges": [{ "from": "hold", "to": "paid", "label": "slip ok" }] })).unwrap();
    assert!(d["nodes"].as_array().unwrap().iter().all(|n| n["role"] == "state" && n["shape"] == "ELLIPSE"));
    assert_eq!(d["edges"][0]["label"], "slip ok");
}

#[test]
fn er_referenced_tables_sit_left_fk_arrows_point_at_them() {
    let d = layout_diagram(&json!({ "type": "er", "tables": [
        { "name": "bookings", "columns": [{ "name": "id", "type": "uuid", "pk": true }, { "name": "booth_id", "type": "uuid", "fk": "booths.id" }] },
        { "name": "booths", "columns": [{ "name": "id", "type": "uuid", "pk": true }, { "name": "label", "type": "text" }] },
    ] }))
    .unwrap();
    let at = by_id(&d);
    assert!(n(&at["booths"]["x"]) < n(&at["bookings"]["x"]));
    let edges: Vec<Value> = d["edges"].as_array().unwrap().iter().map(|e| json!([e["from"], e["to"], e["label"]])).collect();
    assert_eq!(Value::Array(edges), json!([["bookings", "booths", "booth_id"]]));
    let text = at["bookings"]["text"].as_str().unwrap();
    assert!(text.contains("🔑 id  uuid"));
    assert!(text.contains("→ booth_id  uuid  (booths.id)"));
    assert!(err(json!({ "type": "er", "tables": [{ "name": "a", "columns": [{ "name": "x_id", "fk": "x.id" }] }] })).contains("table \"x\" is not in tables"));
}

#[test]
fn sequence_a_lifeline_per_participant_messages_top_to_bottom() {
    let d = layout_diagram(&json!({ "type": "sequence", "participants": ["web", "server", "Slip2Go"], "messages": [
        { "from": "web", "to": "server", "text": "upload slip" }, { "from": "server", "to": "Slip2Go", "text": "verify" }, { "from": "Slip2Go", "to": "server", "text": "result", "kind": "reply" },
    ] }))
    .unwrap();
    let edges = d["edges"].as_array().unwrap();
    let lifelines: Vec<&Value> = edges.iter().filter(|e| e["id"].as_str().unwrap().starts_with("life")).collect();
    let msgs: Vec<&Value> = edges.iter().filter(|e| e["id"].as_str().unwrap().starts_with('m')).collect();
    assert_eq!(lifelines.len(), 3);
    assert!(lifelines.iter().all(|l| l["noArrow"] == true && l["fromPoint"]["x"] == l["toPoint"]["x"]));
    let y = |i: usize| n(&msgs[i]["fromPoint"]["y"]);
    assert!(y(0) < y(1) && y(1) < y(2));
    assert_eq!(msgs[2]["dashed"], true);
    assert_eq!(msgs[0]["label"], "1. upload slip");
    assert!(err(json!({ "type": "sequence", "participants": ["a", "b"], "messages": [{ "from": "a", "to": "c", "text": "x" }] })).contains("c is not a participant"));
}

#[test]
fn elk_runtime_is_loaded_once_and_layouts_are_quick() {
    wridge_figma::elk::warm_up().unwrap();
    let t = Instant::now();
    for _ in 0..5 {
        layout_diagram(&booking()).unwrap();
    }
    let per = t.elapsed() / 5;
    eprintln!("figjam layout after warm-up: {per:?} per diagram");
    assert!(per.as_millis() < 2000);
}
