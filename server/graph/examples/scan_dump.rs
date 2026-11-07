//! `cargo run --release --example scan_dump -- <project dir>`
//! Read-only: runs every scanner on each graph file of a project and prints one JSON line per
//! file (the Rust side of `parity/scan.mjs`).
use serde_json::{json, Value};
use wridge_graph::data::{prisma, sql_ddl, sql_usage, usage};
use wridge_graph::js::{json_stringify, read_utf8};

fn lits(v: &[sql_usage::SqlLiteral]) -> Value {
    Value::Array(
        v.iter()
            .map(|l| json!({ "sql": l.sql.to_string(), "line": l.line, "ddl": l.ddl, "lead": l.lead }))
            .collect(),
    )
}

fn refs(v: &[wridge_graph::data::DataRef]) -> Value {
    Value::Array(v.iter().map(|r| r.to_json()).collect())
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: scan_dump <project dir>");
    let root = std::path::PathBuf::from(&dir);
    let all: Vec<String> = ["supabase", "sql", "prisma"].iter().map(|s| s.to_string()).collect();
    let sqflite = vec!["sqflite".to_string()];
    for f in wridge_graph::files::list_files(&root) {
        let text = match read_utf8(root.join(&f.path)) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut o = serde_json::Map::new();
        o.insert("path".into(), json!(f.path));
        if f.lang == "ts" || wridge_graph::re!(r"\.(css|scss|sass|less|html|vue|svelte|astro)$").test_str(&f.path) {
            o.insert("styles".into(), wridge_graph::styles::scan_styles(&text, &f.path).to_json());
        }
        if f.lang == "ts" {
            o.insert(
                "ts".into(),
                match wridge_graph::parse_ts::parse_ts(&text, &wridge_graph::paths::extname(&f.path)) {
                    Ok(p) => p.to_json(),
                    Err(e) => json!({ "error": e.to_string() }),
                },
            );
            o.insert(
                "data".into(),
                match usage::scan_data(&text, &all) {
                    Ok(r) => refs(&r),
                    Err(e) => json!({ "error": e.to_string() }),
                },
            );
            let l = sql_usage::sql_literals(&text);
            // parseDml's own objects have no form/line (the callers add them)
            let dml: Vec<Value> = l
                .iter()
                .filter(|x| !x.ddl)
                .map(|x| {
                    let mut v = refs(&sql_usage::parse_dml(&x.sql));
                    for r in v.as_array_mut().unwrap() {
                        let o = r.as_object_mut().unwrap();
                        o.shift_remove("form");
                        o.shift_remove("line");
                    }
                    v
                })
                .collect();
            o.insert("sqlLits".into(), lits(&l));
            o.insert("dml".into(), Value::Array(dml));
        }
        if f.lang == "dart" {
            o.insert("data".into(), refs(&usage::scan_dart_data(&text, &sqflite)));
            o.insert("dartLits".into(), lits(&sql_usage::dart_sql_literals(&text)));
        }
        if f.lang == "sql" {
            let mut m = sql_ddl::empty_model();
            sql_ddl::apply_sql_str(&mut m, &text, &f.path, 0);
            o.insert("model".into(), json!(json_stringify(&json!({ "entities": m.entities_json(), "unparsed": m.unparsed }))));
        }
        if f.lang == "prisma" {
            let mut m = sql_ddl::empty_model();
            prisma::apply_prisma(&mut m, &text, &f.path);
            o.insert("model".into(), json!(json_stringify(&json!({ "entities": m.entities_json(), "unparsed": m.unparsed }))));
        }
        println!("{}", json_stringify(&Value::Object(o)));
    }
}
