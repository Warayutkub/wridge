//! End-to-end: build a small Next.js + SQL project, then update it incrementally.
//! (The Node suite has no build test; Node/Rust equality is checked by `parity/run.mjs`.)
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;
use wridge_graph::{update_graph, UpdateOptions};

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn one<T: rusqlite::types::FromSql>(db: &Connection, sql: &str) -> T {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn builds_and_updates_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "package.json", r#"{ "name": "demo", "dependencies": { "next": "16", "pg": "8" } }"#);
    write(root, "tsconfig.json", "{ // aliases\n \"compilerOptions\": { \"paths\": { \"@/*\": [\"./*\"] } } }");
    write(
        root,
        "components/button.tsx",
        "type Props = { label: string };\nexport function Button({ label }: Props) {\n  return <button className=\"bg-primary px-2\">{label}</button>;\n}\n",
    );
    write(
        root,
        "app/page.tsx",
        "\"use client\";\nimport { Button } from \"@/components/button\";\nimport { query } from \"../lib/db\";\nexport default function Page() {\n  const rows = query(`SELECT id, email FROM users WHERE id = ${1}`);\n  return <Button label=\"hi\" />;\n}\n",
    );
    write(root, "lib/db.ts", "export const query = (sql: string) => sql;\n");
    write(root, "db/schema.sql", "create table users (id int primary key, email text not null);\n");
    write(root, "app/globals.css", ":root { --color-primary: #2563eb; }\n");
    let dir_s = root.to_string_lossy().to_string();

    let info = update_graph(&dir_s, UpdateOptions::default()).unwrap();
    assert_eq!(info["files"], 7);
    assert_eq!(info["changed"], 7);
    assert_eq!(info["unresolvedImports"], 0);
    assert_eq!(info["parseErrors"], Value::Array(vec![]));

    let db = Connection::open(root.join(".wara").join("code-graph.db")).unwrap();
    let (kind, props): (String, String) = db
        .query_row("SELECT kind, props FROM symbols WHERE name = 'Button'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(kind, "component");
    assert_eq!(props, r#"{"names":["label"],"type":"Props"}"#);
    assert_eq!(one::<String>(&db, "SELECT directive FROM files WHERE path = 'app/page.tsx'"), "use client");
    assert_eq!(
        one::<String>(
            &db,
            "SELECT t.path FROM imports i JOIN files f ON f.id = i.file_id JOIN files t ON t.id = i.target_file_id WHERE f.path = 'app/page.tsx' AND i.local = 'Button'"
        ),
        "components/button.tsx"
    );
    assert_eq!(one::<String>(&db, "SELECT kind FROM refs WHERE local = 'Button' AND kind = 'jsx'"), "jsx");
    assert_eq!(one::<String>(&db, "SELECT path || ' ' || kind FROM routes"), "/ page");
    assert_eq!(one::<String>(&db, "SELECT columns FROM data_refs WHERE entity = 'users'"), r#"["id","email"]"#);
    assert_eq!(one::<String>(&db, "SELECT name FROM style_defs"), "--color-primary");
    let model: Value = serde_json::from_str(&one::<String>(&db, "SELECT value FROM meta WHERE key = 'data_model'")).unwrap();
    assert_eq!(model["stores"][0]["label"], "Postgres (schema in code/SQL files)");
    assert_eq!(model["stores"][0]["entities"]["users"]["columns"][1]["notNull"], true);
    drop(db);

    // nothing changed
    let info = update_graph(&dir_s, UpdateOptions::default()).unwrap();
    assert_eq!((info["changed"].as_i64(), info["removed"].as_i64()), (Some(0), Some(0)));

    // one file changed, one removed
    write(root, "lib/db.ts", "export const query = (sql: string) => sql;\nexport const other = 1;\n");
    std::fs::remove_file(root.join("app/globals.css")).unwrap();
    let info = update_graph(&dir_s, UpdateOptions::default()).unwrap();
    assert_eq!((info["changed"].as_i64(), info["removed"].as_i64(), info["files"].as_i64()), (Some(1), Some(1), Some(6)));
    let db = Connection::open(root.join(".wara").join("code-graph.db")).unwrap();
    assert_eq!(
        one::<i64>(&db, "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.path = 'lib/db.ts'"),
        2
    );
    assert_eq!(one::<i64>(&db, "SELECT COUNT(*) FROM style_defs"), 0);
}

#[test]
fn project_root_errors_match_node() {
    assert_eq!(
        update_graph("relative/dir", UpdateOptions::default()).unwrap_err().to_string(),
        "projectDir must be an absolute path, got \"relative/dir\""
    );
    let missing = if cfg!(windows) { "C:\\definitely\\not\\here" } else { "/definitely/not/here" };
    assert_eq!(update_graph(missing, UpdateOptions::default()).unwrap_err().to_string(), format!("Not a folder: {missing}"));
}
