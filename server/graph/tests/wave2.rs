//! Ports of the wave-2 Node tests: legacy-node/test/brain.test.mjs, flows.test.mjs,
//! data-export.test.mjs and the code-graph parts of bridge.test.mjs (project memory, stack
//! detection). Node/Rust equality on real projects is checked by `parity/wave2.mjs`.
use serde_json::{json, Value};
use std::path::Path;
use wridge_graph::{brain, export, flows, knowledge, memory, query};

fn project(prefix: &str) -> (tempfile::TempDir, String) {
    let d = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
    let s = d.path().to_string_lossy().into_owned();
    (d, s)
}

fn put(root: &str, rel: &str, text: &str) {
    let p = Path::new(root).join("brain").join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(root: &str, rel: &str) -> String {
    std::fs::read_to_string(Path::new(root).join(rel)).unwrap()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

fn err<T: std::fmt::Debug>(r: anyhow::Result<T>) -> String {
    r.unwrap_err().to_string()
}

// ---------- brain.test.mjs ----------

#[test]
fn links_notes_other_projects_and_things_outside_the_brain() {
    assert_eq!(brain::link_target("Nathee"), ("note".into(), "Nathee".into()));
    assert_eq!(brain::link_target("figma:12:34"), ("figma".into(), "12:34".into()));
    assert_eq!(brain::link_target("code:src/Boss.lua#attack"), ("code".into(), "src/Boss.lua#attack".into()));
    assert_eq!(brain::link_target("book-lock:สีหลัก"), ("project".into(), "book-lock:สีหลัก".into()));
}

#[test]
fn parse_title_thai_tags_links_with_lines_front_matter_relations() {
    let n = brain::parse_note(
        "---\ntags: [character, ch1]\naliases:\n  - ครุฑ\nboss_of: \"[[CH1]]\"\n---\n# นที\n\nไกด์ที่จริงคือ [[ครุฑ]] #สปอยล์\n## หัวข้อ #ไม่ใช่แท็ก\nดู [[figma:1:2|เฟรม]]",
        "characters/nathee.md",
    );
    assert_eq!(n.title, "นที");
    assert_eq!(n.name, "nathee");
    assert_eq!(n.aliases, ["ครุฑ"]);
    let mut tags = n.tags.clone();
    tags.sort();
    let mut want = vec!["ch1", "character", "สปอยล์"];
    want.sort();
    assert_eq!(tags, want);
    let links: Vec<(String, String, Option<String>)> = n.links.iter().map(|l| (l.kind.clone(), l.target.clone(), l.rel.clone())).collect();
    assert_eq!(
        links,
        [
            ("note".into(), "ครุฑ".into(), None),
            ("figma".into(), "1:2".into(), None),
            ("note".into(), "CH1".into(), Some("boss_of".into()))
        ]
    );
    assert_eq!(n.links[0].line, 9);
}

#[test]
fn find_returns_one_line_per_note_read_gives_links_and_backlinks() {
    let (_d, root) = project("wara-brain-");
    put(
        &root,
        "characters/nathee.md",
        "---\ntags: [character]\n---\n# นที\n\nไกด์ใน CH1\nความลับ: เป็นครุฑ จะแปลงร่างตอนบอส [[CH1]]\n[[ยังไม่เขียน]]",
    );
    put(&root, "CH1.md", "# CH1\n\nบทแรก [[roblox:ServerScriptService.Boss]]");
    let hits = brain::find_notes(&root, "ครุฑ", 8).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0]["line"].as_str().unwrap().contains("ครุฑ"));
    let r = brain::read_note(&root, "นที").unwrap();
    let links: Vec<(String, bool)> = r["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["target"].as_str().unwrap().into(), l.get("missing") == Some(&json!(true))))
        .collect();
    assert_eq!(links, [("CH1".into(), false), ("ยังไม่เขียน".into(), true)]);
    assert_eq!(strs(&brain::read_note(&root, "ch1").unwrap()["backlinks"]), ["นที (nathee)"]);
    assert_eq!(brain::brain_summary(&root).unwrap().unwrap()["broken"], 1);
    let g = brain::brain_graph(&root).unwrap();
    let nodes = g["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["kind"] == "roblox"));
    assert!(nodes.iter().any(|n| n["kind"] == "missing" && n["label"] == "ยังไม่เขียน"));
}

#[test]
fn index_follows_edits_made_outside_wara() {
    let (_d, root) = project("wara-brain-");
    put(&root, "a.md", "# A\nold");
    assert_eq!(brain::find_notes(&root, "new", 8).unwrap().len(), 0);
    std::fs::write(Path::new(&root).join("brain/a.md"), "# A\nnew text, longer than before").unwrap();
    assert_eq!(brain::find_notes(&root, "new", 8).unwrap().len(), 1);
}

#[test]
fn write_create_refuses_to_overwrite_append_adds_replace_keeps_old_text_in_trash() {
    let (_d, root) = project("wara-brain-");
    let w = |name: &str, text: &str, mode: &str, tags: Option<Vec<String>>| {
        brain::write_note(
            &root,
            name,
            &brain::WriteOptions {
                text: Some(text.into()),
                mode: mode.into(),
                tags,
                keep_old: true,
            },
        )
    };
    assert_eq!(
        w("rules/Crime", "ห้ามขโมย", "create", Some(vec!["rule".into()])).unwrap(),
        json!({ "saved": "brain/rules/Crime.md", "mode": "create" })
    );
    assert!(read(&root, "brain/rules/Crime.md").starts_with("---\ntags: [rule]\n---\n# Crime\n\nห้ามขโมย"));
    assert!(err(w("Crime", "x", "create", None)).contains("already exists"));
    w("Crime", "ห้ามฆ่า", "append", None).unwrap();
    assert!(read(&root, "brain/rules/Crime.md").contains("ห้ามขโมย\n\nห้ามฆ่า"));
    w("Crime", "# Crime\nใหม่ [[ไม่มี]]", "replace", None).unwrap();
    assert_eq!(brain::list_trash(&root).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(read(&root, "brain/rules/Crime.md"), "# Crime\nใหม่ [[ไม่มี]]\n");
}

#[test]
fn trash_and_restore_only_empty_trash_removes_for_good() {
    let (_d, root) = project("wara-brain-");
    put(&root, "a.md", "# A\n[[B]]");
    put(&root, "B.md", "# B");
    let t = brain::trash_note(&root, "B").unwrap();
    assert_eq!(strs(&t["brokenLinksFrom"]), ["A"]);
    assert!(!Path::new(&root).join("brain/B.md").exists());
    assert_eq!(brain::find_notes(&root, "B", 8).unwrap().len(), 1); // A still mentions [[B]] — but B itself is gone
    assert!(err(brain::read_note(&root, "B")).contains("No note"));
    put(&root, "B.md", "# B again");
    assert!(err(brain::restore_note(&root, t["restore"].as_str().unwrap())).contains("exists again"));
    brain::trash_note(&root, "B").unwrap();
    assert_eq!(brain::list_trash(&root).unwrap().as_array().unwrap().len(), 2);
    assert_eq!(brain::restore_note(&root, "B (2).md").unwrap(), json!({ "restored": "brain/B.md" }));
    assert_eq!(brain::empty_trash(&root, None).unwrap(), json!({ "removed": 1 }));
    assert_eq!(brain::list_trash(&root).unwrap().as_array().unwrap().len(), 0);
    assert!(err(brain::empty_trash(&root, Some("../a.md"))).contains("Not in the trash"));
}

#[test]
fn project_memory_overview_and_find_include_the_brain_for_a_project_without_code() {
    let (_d, root) = project("wara-brain-");
    put(&root, "Buki.md", "---\ntags: [mascot]\n---\n# Buki\nมาสคอตสีส้ม");
    assert!(memory::overview(&root, false).unwrap().contains("## Brain (1 notes in brain/)\nTags: mascot 1"));
    assert!(memory::overview(&root, true).unwrap().contains("Brain"));
    let f = memory::find(&root, "มาสคอต", false, 8).unwrap();
    let at = f.find("## Brain notes").expect("brain block");
    assert!(f[at..].contains("- Buki [mascot] — มาสคอตสีส้ม"));
}

#[test]
fn code_graph_answers_point_at_brain_notes_about_that_code() {
    let (_d, root) = project("wara-brain-");
    std::fs::create_dir_all(Path::new(&root).join("src")).unwrap();
    std::fs::write(Path::new(&root).join("src/boss.ts"), "export function attack() { return 1 }\nexport const hp = 10\n").unwrap();
    std::fs::write(Path::new(&root).join("src/arena.ts"), "import { attack } from './boss'\nexport const go = () => attack()\n").unwrap();
    put(
        &root,
        "นที.md",
        "# นที\n\nบอส CH1 ห้ามหนีออกจากฉาก [[code:src/boss.ts#attack]]\nเก่า [[code:src/gone.ts]] [[code:src/boss.ts#heal]]",
    );
    wridge_graph::update_graph(&root, Default::default()).unwrap();
    let notes = |p: Option<&str>, n: Option<&str>| -> Vec<String> { brain::notes_about(&root, p, n).unwrap().iter().map(|x| x["note"].as_str().unwrap().to_string()).collect() };
    assert_eq!(notes(Some("src/boss.ts"), None), ["นที"]);
    assert_eq!(notes(None, Some("attack")), ["นที"]);
    assert!(notes(Some("src/arena.ts"), None).is_empty());
    assert!(!brain::code_link_missing(&root, "src/boss.ts#attack").unwrap());
    assert!(brain::code_link_missing(&root, "src/gone.ts").unwrap());
    assert!(brain::code_link_missing(&root, "src/boss.ts#heal").unwrap());
    let r = brain::read_note(&root, "นที").unwrap();
    let missing: Vec<&str> = r["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l.get("missing").is_some())
        .map(|l| l["target"].as_str().unwrap())
        .collect();
    assert_eq!(missing, ["src/gone.ts", "src/boss.ts#heal"]);
    assert_eq!(brain::brain_summary(&root).unwrap().unwrap()["broken"], 2);
    let (db, _) = wridge_graph::open_graph(&root).unwrap();
    let ask = |q: &str, t: &str| {
        query::run_query(
            &db,
            q,
            Some(t),
            &query::QueryOptions {
                root: Some(root.clone()),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let file = ask("file", "src/boss.ts");
    let at = file.find("## Brain notes linking here (1)").expect("signpost");
    assert!(file[at..].contains("- นที — บอส CH1 ห้ามหนีออกจากฉาก code:src/boss.ts#attack"));
    assert!(ask("used_by", "attack").contains("Brain notes linking here (1)"));
    assert!(!ask("file", "src/arena.ts").contains("Brain notes"));
    assert!(ask("find", "ห้ามหนี").contains("## Brain notes (1)"));
}

#[test]
fn a_project_without_brain_gets_no_brain_index_from_find() {
    let (_d, root) = project("wara-brain-");
    assert!(brain::find_notes(&root, "x", 8).unwrap().is_empty());
    assert!(!Path::new(&root).join(".wara").join("brain.db").exists());
}

// ---------- flows.test.mjs ----------

const SAMPLE: &str = "# Flow: Checkout
entry: screen /checkout

## Steps
1. [user] presses \"Pay\" on `/checkout`
2. [web] POST `/api/checkout` → `createPayment`
3. [⏳ payment] waits for `/api/payments/webhook`
4. [server] `setPaymentStatus`
   - paid → step 5a
   - failed → end
5a. [db] writes `payments, orders`
5b. [email] sends the receipt
6. [user] sees \"Paid\"

## States
`payments.status`: pending → paid | failed · paid → refunded (admin only)

## Notes
Webhooks can arrive twice.
";

fn ids(f: &Value) -> Vec<String> {
    f["steps"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn parse_reads_steps_actors_waits_refs_branches_groups_states_and_notes() {
    let f = flows::parse_flow(SAMPLE);
    assert_eq!(f["title"], "Checkout");
    assert_eq!(f["entry"], "screen /checkout");
    assert_eq!(ids(&f), ["1", "2", "3", "4", "5a", "5b", "6"]);
    assert_eq!(f["steps"][2]["wait"], true);
    assert_eq!(f["steps"][2]["actor"], "payment");
    assert_eq!(f["steps"][1]["refs"], json!(["/api/checkout", "createPayment"]));
    assert_eq!(f["steps"][3]["branches"], json!([{ "label": "paid", "to": "5a" }, { "label": "failed", "to": "end" }]));
    assert_eq!(f["steps"][4]["group"], "5");
    assert_eq!(f["steps"][5]["group"], "5");
    assert_eq!(
        f["states"][0]["transitions"],
        json!([
            { "from": "pending", "to": "paid", "note": null },
            { "from": "pending", "to": "failed", "note": null },
            { "from": "paid", "to": "refunded", "note": "admin only" },
        ])
    );
    assert_eq!(f["notes"], "Webhooks can arrive twice.");
}

#[test]
fn format_then_parse_keeps_the_flow_reordering_renumbers_and_branches_follow() {
    let f = flows::parse_flow(SAMPLE);
    assert_eq!(flows::parse_flow(&flows::format_flow(&f).unwrap())["steps"], f["steps"]);
    // Move the email step before the db step: still one parallel group, letters swap.
    let s = f["steps"].as_array().unwrap();
    let mut moved = f.clone();
    moved["steps"] = json!([s[0], s[1], s[2], s[3], s[5], s[4], s[6]]);
    let again = flows::parse_flow(&flows::format_flow(&moved).unwrap());
    let pairs: Vec<(String, String)> = again["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["id"].as_str().unwrap().into(), x["actor"].as_str().unwrap().into()))
        .collect();
    let want: Vec<(String, String)> = [("1", "user"), ("2", "web"), ("3", "payment"), ("4", "server"), ("5a", "email"), ("5b", "db"), ("6", "user")]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    assert_eq!(pairs, want);
    assert_eq!(again["steps"][3]["branches"][0], json!({ "label": "paid", "to": "5b" })); // still points at the db step
                                                                                          // Drop a step: later numbers shift down.
    let mut dropped = f.clone();
    dropped["steps"] = Value::Array(s.iter().filter(|x| x["id"] != "2").cloned().collect());
    assert_eq!(ids(&flows::parse_flow(&flows::format_flow(&dropped).unwrap())), ["1", "2", "3", "4a", "4b", "5"]);
}

#[test]
fn saving_refuses_to_overwrite_a_changed_file_and_odd_names() {
    let (_d, root) = project("wara-flows-");
    let first = flows::save_flow_file(&root, "checkout.md", SAMPLE, &Value::Null).unwrap();
    assert_eq!(first["file"], "docs/flows/checkout.md");
    let e = flows::save_flow_file(&root, "checkout.md", SAMPLE, &Value::Null).unwrap_err();
    assert!(e.downcast_ref::<flows::FlowConflict>().is_some()); // exists already
    std::fs::write(Path::new(&root).join("docs/flows/checkout.md"), format!("{SAMPLE}\nedited elsewhere\n")).unwrap();
    let e = flows::save_flow_file(&root, "checkout.md", "mine", &first["hash"]).unwrap_err();
    assert!(e.downcast_ref::<flows::FlowConflict>().unwrap().current.as_deref().unwrap().contains("edited elsewhere"));
    let current = read(&root, "docs/flows/checkout.md");
    flows::save_flow_file(&root, "checkout.md", "mine", &json!(flows::hash_text(&current))).unwrap();
    assert_eq!(read(&root, "docs/flows/checkout.md"), "mine");
    assert!(err(flows::save_flow_file(&root, "../escape.md", "x", &Value::Null)).contains("Invalid flow file name"));
    assert_eq!(flows::slugify("Sign up / Verify!"), "sign-up-verify");
}

#[test]
fn refs_are_checked_against_routes_symbols_files_and_tables() {
    let db = wridge_graph::open_db(":memory:").unwrap();
    db.execute_batch(
        "INSERT INTO files (id, path, lang) VALUES (1, 'app/api/checkout/route.ts', 'ts'), (2, 'app/checkout/page.tsx', 'ts');
         INSERT INTO symbols (id, file_id, name, kind, exported, line) VALUES (1, 1, 'setPaymentStatus', 'function', 0, 4), (2, 1, 'POST', 'function', 1, 9);
         INSERT INTO routes (file_id, framework, path, kind) VALUES (1, 'next-app', '/api/checkout', 'route'), (2, 'next-app', '/checkout', 'page');",
    )
    .unwrap();
    let model = json!({ "stores": [{ "entities": { "payments": { "name": "payments", "columns": [{ "name": "status" }] }, "orders": { "name": "orders", "columns": [] } } }] });
    wridge_graph::set_meta(&db, "data_model", &model.to_string()).unwrap();
    let g = query::graph(&db).unwrap();
    let check = flows::check_flow(&g, &flows::parse_flow(SAMPLE));
    let stale: Vec<&str> = check["stale"].as_array().unwrap().iter().map(|s| s["ref"].as_str().unwrap()).collect();
    assert_eq!(stale, ["createPayment", "/api/payments/webhook"]);
}

// ---------- data-export.test.mjs ----------

fn data_fixture() -> (tempfile::TempDir, String, rusqlite::Connection) {
    let (d, dir) = project("wara-data-");
    let db = wridge_graph::open_db(Path::new(&dir).join("code-graph.db")).unwrap();
    db.execute_batch("INSERT INTO files (id, path, lang) VALUES (1, 'app/orders/page.tsx', 'ts'), (2, 'supabase/migrations/1.sql', 'sql')")
        .unwrap();
    db.execute(
        "INSERT INTO data_refs (file_id, form, entity, op, columns, line) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![1, "supabase", "orders", "read", r#"["id","status"]"#, 12],
    )
    .unwrap();
    db.execute(
        "INSERT INTO data_refs (file_id, form, entity, op, columns, line) VALUES (?, ?, ?, ?, ?, ?)",
        rusqlite::params![2, "sql", "orders", "ddl", Option::<String>::None, 1],
    )
    .unwrap();
    (d, dir, db)
}

#[test]
fn stores_become_entity_lists_ddl_refs_are_left_out_refs_carry_the_file_path() {
    let (_d, dir, db) = data_fixture();
    let model = json!({ "stores": [{ "kind": "supabase", "label": "Supabase (Postgres)", "sources": ["supabase/migrations/1.sql"], "unparsed": [{ "sql": "x" }],
        "entities": { "orders": { "name": "orders", "kind": "table", "columns": [{ "name": "id", "pk": true }], "fks": [], "indexes": [], "policies": [], "rls": true } } }] });
    wridge_graph::set_meta(&db, "data_model", &model.to_string()).unwrap();
    let out = export::export_data(&db, &dir).unwrap();
    assert_eq!(out["stores"].as_array().unwrap().len(), 1);
    assert_eq!(out["stores"][0]["unparsed"], 1);
    assert_eq!(
        out["stores"][0]["entities"].as_array().unwrap().iter().map(|e| e["name"].clone()).collect::<Vec<_>>(),
        [json!("orders")]
    );
    assert_eq!(
        out["refs"],
        json!([{ "entity": "orders", "op": "read", "form": "supabase", "columns": ["id", "status"], "filters": null, "line": 12, "path": "app/orders/page.tsx" }])
    );
}

#[test]
fn a_project_without_a_data_model_gives_no_stores() {
    let (_d, dir, db) = data_fixture();
    assert_eq!(export::export_data(&db, &dir).unwrap()["stores"], json!([]));
}

// ---------- bridge.test.mjs (project memory, stack detection) ----------

#[test]
fn project_memory_decisions_replaced_ones_hidden_notes_close_with_their_task() {
    let (_d, root) = project("wara-bridge-");
    std::process::Command::new("git").args(["init", "-q"]).current_dir(&root).status().unwrap();
    let first = memory::decide(
        &root,
        &memory::DecideArgs {
            title: "Redis not used yet".into(),
            decision: "Rate limits in memory.".into(),
            tags: Some(vec!["infra".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(first["id"], "D-1");
    assert!(first["gitignore"].as_str().unwrap_or("").contains("/.wara/"));
    memory::decide(
        &root,
        &memory::DecideArgs {
            title: "Use Redis for rate limits".into(),
            decision: "Two instances now.".into(),
            tags: Some(vec!["infra".into()]),
            supersedes: Some("D-1".into()),
            ..Default::default()
        },
    )
    .unwrap();
    memory::note(&root, Some("Foundations"), "Keep all 4 grays.", None).unwrap();
    memory::handoff(
        &root,
        &memory::HandoffArgs {
            task: "Foundations".into(),
            stage: Some("5/8".into()),
            next: Some("Input, Card".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let ov = memory::overview(&root, false).unwrap();
    assert!(ov.lines().any(|l| l.starts_with("D-2 · ") && l.contains(" Use Redis")));
    assert!(!ov.contains("D-1 ·"));
    assert!(ov.contains("Keep all 4 grays"));
    assert!(memory::find(&root, "redis", true, 8).unwrap().contains("[replaced]"));
    memory::handoff(
        &root,
        &memory::HandoffArgs {
            task: "Foundations".into(),
            close: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(memory::overview(&root, false).unwrap().contains("Open tasks: none"));
    let text = read(&root, "docs/decisions.md");
    assert!(wridge_graph::re!(r"## D-2 · \d{4}-\d{2}-\d{2} · Use Redis").test_str(&text));
}

#[test]
fn stack_detection_package_json_and_go_mod_one_level_down_notable_packages_only() {
    let (_d, root) = project("wara-bridge-");
    std::fs::create_dir_all(Path::new(&root).join("web")).unwrap();
    std::fs::create_dir_all(Path::new(&root).join("backend")).unwrap();
    std::fs::write(
        Path::new(&root).join("web/package.json"),
        r#"{"dependencies":{"next":"^16.1.0","zod":"^3.23.0","left-pad":"1.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(
        Path::new(&root).join("backend/go.mod"),
        "module x\n\ngo 1.22\n\nrequire (\n\tgithub.com/gofiber/fiber/v2 v2.52.5\n\tgithub.com/stretchr/testify v1.9.0\n)\n",
    )
    .unwrap();
    let mut stack: Vec<String> = knowledge::detect_stack(&root)
        .iter()
        .map(|s| format!("{}@{}", s["tool"].as_str().unwrap(), s["version"].as_str().unwrap()))
        .collect();
    stack.sort();
    assert_eq!(stack, ["github.com/gofiber/fiber@v2.52.5", "go@1.22", "next@16.1.0", "zod@3.23.0"]);
}
