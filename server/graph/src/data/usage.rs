//! Where code touches data (port of `data/usage.mjs`): which table/view/function each file reads
//! or writes, and which columns. Per access style (detected from the project's packages):
//! supabase-js, Prisma, raw SQL strings, sqflite.

use super::prisma::scan_prisma;
use super::sql_usage::{dart_sql_literals, parse_dml, scan_sqflite, sql_literals};
use super::DataRef;
use crate::jsre::JsStr;
use crate::re;
use anyhow::Result;
use indexmap::IndexSet;
use serde_json::Value;

/// Bump when a scanner changes: existing graphs rescan every file for data use once.
pub const DATA_SCAN_VERSION: i64 = 5;

// Receivers whose .from() is not a table query.
const NOT_DATA: [&str; 11] = [
    "Array",
    "Buffer",
    "Uint8Array",
    "Object",
    "storage",
    "Promise",
    "Set",
    "Map",
    "Readable",
    "Observable",
    "rxjs",
];

/// `"id, title, author:profiles(username), items(*)"` → columns `["id","title"]`, embeds `["profiles","items"]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Select {
    pub columns: Vec<String>,
    pub embeds: Vec<String>,
}

/// `parseSelect(spec)`.
pub fn parse_select(spec: &str) -> Select {
    parse_select_js(&JsStr::new(spec))
}

fn parse_select_js(spec: &JsStr) -> Select {
    // Collapse nested "( … )" so top-level commas separate the items.
    let mut flat = spec.clone();
    let mut prev = JsStr::default();
    while prev != flat {
        prev = flat.clone();
        flat = re!(r"\([^()]*\)", "g").replace_all(&flat, "()");
    }
    let mut columns = Vec::new();
    let mut embeds = Vec::new();
    for raw in flat.split(",") {
        let s = raw.trim();
        if s.is_empty() {
            continue;
        }
        if let Some(e) = re!(r"^(?:\w+\s*:\s*)?(\w+)(?:!\w+)?\s*\(\)").exec(&s) {
            embeds.push(e.str(&s, 1).unwrap());
            continue;
        }
        let a = re!(r"::\w+$").replace_first(&s, "");
        let b = re!(r"^\w+\s*:\s*").replace_first(&a, "");
        let col = re!(r"->.*$").replace_first(&b, "").trim();
        if !col.is_empty() {
            columns.push(col.to_string());
        }
    }
    Select { columns, embeds }
}

const WRITE_OPS: [&str; 4] = ["insert", "update", "upsert", "delete"];

/// supabase-js: `from('t').select('a,b')` / `.insert` / `.update` / `.upsert` / `.delete`, and `rpc('fn')`.
pub fn scan_supabase(text: &str) -> Vec<DataRef> {
    let text = JsStr::new(text);
    let mut refs = Vec::new();
    for m in re!(r#"([\w$]+)\s*\.from\(\s*(['"`])([A-Za-z_][\w.]*)\2\s*\)"#, "g").all(&text) {
        let recv = m.str(&text, 1).unwrap();
        if NOT_DATA.contains(&recv.as_str()) {
            continue;
        }
        let entity = m.str(&text, 3).unwrap();
        let start = m.end;
        // The chain that follows, up to the next statement or the next .from().
        let end = re!(r";|\.from\(|\n\s*\n", "g").exec_at(&text, start).map(|s| s.index).unwrap_or(text.len());
        let chain = text.slice(start, end.min(start + 1500));
        let op = re!(r"^\s*\.\s*(select|insert|update|upsert|delete)\s*\(")
            .exec(&chain)
            .and_then(|x| x.str(&chain, 1))
            .or_else(|| re!(r"\.\s*(select|insert|update|upsert|delete)\s*\(").exec(&chain).and_then(|x| x.str(&chain, 1)))
            .unwrap_or_else(|| "select".into());
        let from_at = m.whole(&text).index_of(".from", 0).map(|x| x as isize).unwrap_or(-1);
        let line = text.line_at((m.index as isize + from_at).max(0) as usize) as i64;
        let mut r = DataRef::new(entity.clone(), if WRITE_OPS.contains(&op.as_str()) { "write" } else { "read" }, Some(&op), line);
        let sel = re!(r#"\.\s*select\(\s*(['"`])([\s\S]*?)\1"#).exec(&chain);
        match &sel {
            Some(sel) if !re!(r"\$\{").test(&sel.get(&chain, 2).unwrap()) => {
                let spec = re!(r"\s+", "g").replace_all(&sel.get(&chain, 2).unwrap(), " ");
                let Select { columns, embeds } = parse_select_js(&spec);
                r.columns = Some(if columns.iter().any(|c| c == "*") { vec!["*".into()] } else { columns });
                for e in embeds {
                    let mut x = DataRef::new(e, "read", Some("embed"), line);
                    x.via = Some(entity.clone());
                    refs.push(x);
                }
            }
            None if op == "select" => r.columns = Some(vec!["*".into()]),
            _ => {}
        }
        // Writes with an object literal: its keys are the columns written.
        if let Some(lit) = re!(r"\.\s*(?:insert|update|upsert)\(\s*\[?\s*\{").exec(&chain) {
            let open = lit.end - 1;
            let mut depth = 0i64;
            let mut body: Vec<u16> = Vec::new();
            for i in open..chain.len() {
                let ch = chain.at(i).unwrap();
                if b"{([".iter().any(|c| *c as u16 == ch) {
                    depth += 1;
                }
                if b"})]".iter().any(|c| *c as u16 == ch) {
                    depth -= 1;
                }
                if depth == 1 && i > open {
                    body.push(ch);
                } else if depth > 1 {
                    body.push(b' ' as u16);
                }
                if depth == 0 {
                    break;
                }
            }
            let body = JsStr(body);
            let keys: Vec<String> = re!(r#"(?:^|,)\s*(?:["']?([A-Za-z_]\w*)["']?\s*:|([A-Za-z_]\w*)\s*(?=,|$))"#, "g")
                .all(&body)
                .iter()
                .map(|k| k.str(&body, 1).or_else(|| k.str(&body, 2)).unwrap_or_else(|| "undefined".into()))
                .collect();
            if !keys.is_empty() {
                r.columns = Some(keys.into_iter().collect::<IndexSet<_>>().into_iter().collect());
            }
        }
        // Filters show which columns a query depends on (and should be indexed).
        let filters: Vec<String> = re!(r#"\.\s*(eq|neq|gt|gte|lt|lte|like|ilike|is|in|contains|match|order)\(\s*(['"`])([\w.]+)\2"#, "g")
            .all(&chain)
            .iter()
            .map(|f| f.str(&chain, 3).unwrap())
            .collect();
        if !filters.is_empty() {
            r.filters = Some(filters.into_iter().collect::<IndexSet<_>>().into_iter().collect());
        }
        refs.push(r);
    }
    for m in re!(r#"\.rpc\(\s*(['"`])([A-Za-z_][\w.]*)\1"#, "g").all(&text) {
        refs.push(DataRef::new(m.str(&text, 2).unwrap(), "call", Some("rpc"), text.line_at(m.index) as i64));
    }
    refs
}

const SQL_DRIVERS: [(&str, &str); 10] = [
    ("mysql2", "MySQL"),
    ("mysql", "MySQL"),
    ("mariadb", "MySQL"),
    ("pg", "Postgres"),
    ("postgres", "Postgres"),
    ("@neondatabase/serverless", "Postgres"),
    ("@vercel/postgres", "Postgres"),
    ("better-sqlite3", "SQLite"),
    ("sqlite3", "SQLite"),
    ("@libsql/client", "SQLite"),
];

fn sqflite(pubspec: &str) -> bool {
    re!(r"^\s+(sqflite|sqflite_common_ffi|sqflite_common)\s*:", "m").test_str(pubspec)
}

/// `{ ...pkg?.dependencies, ...pkg?.devDependencies }[name]` is truthy.
fn has_dep(pkg: Option<&Value>, name: &str) -> bool {
    let Some(pkg) = pkg else { return false };
    let dev = pkg.get("devDependencies").filter(|d| d.is_object()).and_then(|d| d.get(name));
    let dep = pkg.get("dependencies").filter(|d| d.is_object()).and_then(|d| d.get(name));
    // devDependencies win when both name the package
    crate::js::truthy(dev.or(dep))
}

/// Access styles from package.json (parsed, `None` when missing/invalid) and pubspec.yaml text.
pub fn data_access_styles(pkg: Option<&Value>, pubspec: &str) -> Vec<String> {
    let mut styles = Vec::new();
    if has_dep(pkg, "@supabase/supabase-js") || has_dep(pkg, "@supabase/ssr") || has_dep(pkg, "@supabase/auth-helpers-nextjs") {
        styles.push("supabase".to_string());
    }
    if SQL_DRIVERS.iter().any(|(d, _)| has_dep(pkg, d)) {
        styles.push("sql".to_string());
    }
    if has_dep(pkg, "@prisma/client") || has_dep(pkg, "prisma") {
        styles.push("prisma".to_string());
    }
    if sqflite(pubspec) {
        styles.push("sqflite".to_string());
    }
    styles
}

/// "MySQL" / "Postgres" / "SQLite" from the driver the project uses (labels the plain-SQL store).
pub fn sql_dialect(pkg: Option<&Value>, pubspec: &str) -> String {
    if sqflite(pubspec) {
        return "SQLite".into();
    }
    SQL_DRIVERS
        .iter()
        .find(|(d, _)| has_dep(pkg, d))
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| "SQL".into())
}

fn ddl_ref(line: i64) -> DataRef {
    DataRef {
        form: "sql".into(),
        entity: "*".into(),
        op: "ddl".into(),
        line,
        ..Default::default()
    }
}

/// Dart files: SQL strings (DDL + raw queries) and sqflite helper calls.
pub fn scan_dart_data(text: &str, styles: &[String]) -> Vec<DataRef> {
    let mut refs = Vec::new();
    let lits = dart_sql_literals(text);
    if let Some(l) = lits.iter().find(|l| l.ddl) {
        refs.push(ddl_ref(l.line));
    }
    if styles.iter().any(|s| s == "sqflite") {
        for l in &lits {
            if !l.ddl {
                for mut r in parse_dml(&l.sql) {
                    r.form = "sql".into();
                    r.line = l.line + l.lead;
                    refs.push(r);
                }
            }
        }
        for mut r in scan_sqflite(text) {
            r.form = "sqflite".into();
            refs.push(r);
        }
    }
    refs
}

/// TS/JS files: supabase-js, Prisma, DDL strings and raw SQL.
pub fn scan_data(text: &str, styles: &[String]) -> Result<Vec<DataRef>> {
    let has = |s: &str| styles.iter().any(|x| x == s);
    let mut refs = Vec::new();
    if has("supabase") {
        for mut r in scan_supabase(text) {
            r.form = "supabase".into();
            refs.push(r);
        }
    }
    if has("prisma") {
        for mut r in scan_prisma(text)? {
            r.form = "prisma".into();
            refs.push(r);
        }
    }
    let lits = sql_literals(text);
    // Schema DDL inside code (CREATE/ALTER TABLE strings) makes the file a schema source, whatever the driver.
    if let Some(l) = lits.iter().find(|l| l.ddl) {
        refs.push(ddl_ref(l.line));
    }
    // Raw SQL strings: SQL drivers, and Prisma's $queryRaw`…` / $executeRaw`…`.
    if has("sql") || has("prisma") {
        for l in &lits {
            if l.ddl {
                continue;
            }
            for mut r in parse_dml(&l.sql) {
                r.form = "sql".into();
                r.line = l.line + l.lead;
                refs.push(r);
            }
        }
    }
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|x| x.to_string()).collect())
    }

    // ports of legacy-node/test/data-usage.test.mjs (supabase-js and access styles)

    #[test]
    fn select_list() {
        assert_eq!(
            parse_select("id, total::int, author:profiles!fk(username, avatar_url), items(*)"),
            Select {
                columns: vec!["id".into(), "total".into()],
                embeds: vec!["profiles".into(), "items".into()]
            }
        );
    }

    #[test]
    fn supabase_calls() {
        let refs = scan_supabase("const { data } = await supabase\n  .from(\"orders\")\n  .update({ status: \"paid\", paid_at: new Date(), meta: { a: 1 } })\n  .eq(\"id\", id);\nawait supabase.from(\"cart_items\").insert([{ order_id, product_id: p.id, qty }]);\nconst b = Buffer.from(\"abc\"); supabase.storage.from(\"avatars\").upload(x);\nawait sb.from(\"products\").select().eq(\"slug\", s);\nawait supabase.rpc(\"redeem_code\", { c });");
        let got: Vec<_> = refs.iter().map(|r| (r.entity.clone(), r.op.clone(), r.columns.clone(), r.filters.clone())).collect();
        assert_eq!(
            got,
            vec![
                ("orders".into(), "write".into(), s(&["status", "paid_at", "meta"]), s(&["id"])),
                ("cart_items".into(), "write".into(), s(&["order_id", "product_id", "qty"]), None),
                ("products".into(), "read".into(), s(&["*"]), s(&["slug"])),
                ("redeem_code".into(), "call".into(), None, None),
            ]
        );
    }

    #[test]
    fn access_styles() {
        let pkg = json!({ "dependencies": { "@supabase/supabase-js": "2", "mysql2": "3", "@prisma/client": "6" } });
        assert_eq!(data_access_styles(Some(&pkg), ""), ["supabase", "sql", "prisma"]);
        assert_eq!(data_access_styles(None, "dependencies:\n  flutter:\n    sdk: flutter\n  sqflite: ^2.4.2\n"), ["sqflite"]);
        assert!(data_access_styles(Some(&json!({ "dependencies": { "next": "16" } })), "").is_empty());
    }
}
