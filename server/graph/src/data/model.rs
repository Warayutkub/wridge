//! The project's data model in one shape, whatever stores it (port of `data/model.mjs`): entities
//! (tables/views/functions) with columns, keys, indexes, relations and access policies. Stores:
//! Supabase migrations, Prisma schema, plain SQL (.sql files + CREATE/ALTER strings in TS/Dart code).

use super::prisma::{apply_prisma, prisma_provider};
use super::sql_ddl::{apply_sql, empty_model, Model};
use super::sql_usage::{dart_sql_literals, sql_literals};
use super::{str_list, Obj};
use crate::js::{cmp_utf16, js_number, locale_compare};
use crate::jsre::JsStr;
use crate::re;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

/// Bump when the readers change, so cached models are rebuilt even if no migration changed.
pub const MODEL_VERSION: i64 = 9;

/// Schema source files per store kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    pub supabase: Vec<String>,
    pub prisma: Vec<String>,
    pub sql: Vec<String>,
}

impl Sources {
    pub fn to_json(&self) -> Value {
        json!({ "supabase": self.supabase, "prisma": self.prisma, "sql": self.sql })
    }
}

// Tests build old or fake schemas on purpose (migration tests, fixtures): never a schema source.
fn is_test(p: &str) -> bool {
    re!(r"(^|\/)(test|tests|__tests__|spec|specs|e2e|fixtures?|integration_test)\/").test_str(p) || re!(r"(_test\.dart|\.(test|spec)\.[cm]?[jt]sx?)$").test_str(p)
}

fn js_sort(v: &mut [String]) {
    v.sort_by(|a, b| cmp_utf16(a, b));
}

/// `dataSources(allPaths, ddlCodeFiles)`: `ddl_code_files` = code files whose strings hold
/// CREATE/ALTER TABLE (found while scanning, e.g. lib/schema/tenant.ts or a migrate route).
pub fn data_sources(all_paths: &[String], ddl_code_files: &[String]) -> Sources {
    let paths: Vec<&String> = all_paths.iter().filter(|p| !is_test(p)).collect();
    let ddl: Vec<&String> = ddl_code_files.iter().filter(|p| !is_test(p)).collect();
    let mut supabase: Vec<String> = paths
        .iter()
        .filter(|p| re!(r"(^|\/)supabase\/migrations\/[^/]+\.sql$").test_str(p))
        .map(|p| p.to_string())
        .collect();
    js_sort(&mut supabase);
    // Generated copies (prisma-client output) are not sources.
    let mut prisma: Vec<String> = paths
        .iter()
        .filter(|p| p.ends_with(".prisma") && !re!(r"(^|\/)(generated|node_modules|\.prisma)\/").test_str(p))
        .map(|p| p.to_string())
        .collect();
    js_sort(&mut prisma);
    let mut sql: Vec<String> = paths
        .iter()
        // prisma/migrations/*.sql are generated from schema.prisma: the Prisma store already covers them.
        .filter(|p| p.ends_with(".sql") && !p.split('/').any(|s| s == "supabase") && !(!prisma.is_empty() && re!(r"(^|\/)prisma\/migrations\/").test_str(p)))
        .map(|p| p.to_string())
        .chain(ddl.iter().filter(|p| !p.split('/').any(|s| s == "supabase")).map(|p| p.to_string()))
        .collect();
    js_sort(&mut sql);
    Sources { supabase, prisma, sql }
}

// Database dumps (phpMyAdmin / mysqldump / pg_dump) are snapshots, not the schema's history: skipped.
fn is_dump(text: &JsStr) -> bool {
    re!(r"^--\s*(phpMyAdmin SQL Dump|MySQL dump|MariaDB dump|PostgreSQL database dump)", "im").test(&text.slice(0, 2000)) || re!(r"^\s*insert\s+into\b", "gim").count(text) > 50
}

/// Cheap fingerprint of every schema source, to rebuild the model only when one changed.
pub fn sources_stamp(root: &Path, sources: &Sources) -> String {
    let mut parts = Vec::new();
    for (kind, files) in [("supabase", &sources.supabase), ("prisma", &sources.prisma), ("sql", &sources.sql)] {
        for f in files {
            let (mtime, size) = match std::fs::metadata(root.join(f)) {
                Ok(st) => (crate::js::mtime_ms(&st), st.len() as f64),
                Err(_) => (0.0, 0.0),
            };
            parts.push(format!("{kind}:{f}:{}:{}", js_number(mtime), js_number(size)));
        }
    }
    format!("v{MODEL_VERSION}|{}", parts.join("|"))
}

fn read(root: &Path, f: &str) -> Result<String> {
    crate::js::read_utf8(root.join(f)).map_err(|e| anyhow::anyhow!("{}: {e}", root.join(f).display()))
}

fn store(kind: &str, label: String, sources: &[String], skipped: Option<&[String]>, model: &Model) -> Value {
    let mut o = Obj::new();
    o.insert("kind".into(), json!(kind));
    o.insert("label".into(), json!(label));
    o.insert("sources".into(), str_list(sources));
    if let Some(s) = skipped {
        o.insert("skipped".into(), str_list(s));
    }
    o.insert("entities".into(), model.entities_json());
    o.insert("unparsed".into(), Value::Array(model.unparsed.clone()));
    Value::Object(o)
}

/// `buildDataModel(root, sources, { dialect })` → `{ stores, builtAt }`.
pub fn build_data_model(root: &Path, sources: &Sources, dialect: &str) -> Result<Value> {
    let mut stores = Vec::new();
    if !sources.supabase.is_empty() {
        let mut model = empty_model();
        for f in &sources.supabase {
            apply_sql(&mut model, &JsStr::new(&read(root, f)?), f, 0);
        }
        finish(&model);
        stores.push(store("supabase", "Supabase (Postgres)".into(), &sources.supabase, None, &model));
    }
    if !sources.prisma.is_empty() {
        let mut model = empty_model();
        let mut provider: Option<String> = None;
        for f in &sources.prisma {
            let text = read(root, f)?;
            if provider.is_none() {
                provider = prisma_provider(&text);
            }
            apply_prisma(&mut model, &text, f);
        }
        finish(&model);
        let label = match provider {
            Some(p) => format!("Prisma ({p})"),
            None => "Prisma".into(),
        };
        stores.push(store("prisma", label, &sources.prisma, None, &model));
    }
    if !sources.sql.is_empty() {
        let mut model = empty_model();
        let mut skipped = Vec::new();
        // Each source → its SQL chunks (whole .sql file, or each DDL string of a code file with its line).
        struct Chunk {
            f: String,
            sql: JsStr,
            offset: i64,
        }
        let mut chunks: Vec<Chunk> = Vec::new();
        for f in &sources.sql {
            let text = read(root, f)?;
            if f.ends_with(".sql") {
                let t = JsStr::new(&text);
                if is_dump(&t) {
                    skipped.push(f.clone());
                    continue;
                }
                chunks.push(Chunk { f: f.clone(), sql: t, offset: 0 });
            } else {
                let lits = if f.ends_with(".dart") { dart_sql_literals(&text) } else { sql_literals(&text) };
                for l in lits.into_iter().filter(|x| x.ddl) {
                    chunks.push(Chunk {
                        f: f.clone(),
                        sql: l.sql,
                        offset: l.line - 1,
                    });
                }
            }
        }
        // Order: schema files (*.sql, schema/, db…) → other code that creates tables (migrate/setup scripts,
        // whose CREATE TABLE IF NOT EXISTS then adds nothing) → numbered migrations → ALTER-only code.
        let is_create = |c: &Chunk| re!(r"\bcreate\s+table\b", "i").test(&c.sql);
        let creates: std::collections::HashSet<&String> = chunks.iter().filter(|c| is_create(c)).map(|c| &c.f).collect();
        let schema_like = |f: &str| f.ends_with(".sql") || re!(r"(^|\/)(schemas?|db|database|models?)(\/|[-_.])|schema", "i").test_str(f);
        let rank = |f: &String| -> i64 {
            if !creates.contains(f) {
                3
            } else if re!(r"(^|\/)migrations?\/").test_str(f) {
                2
            } else if schema_like(f) {
                0
            } else {
                1
            }
        };
        let mut files: Vec<String> = Vec::new();
        for c in &chunks {
            if !files.contains(&c.f) {
                files.push(c.f.clone());
            }
        }
        files.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| locale_compare(a, b)));
        // Within a file, CREATEs before ALTERs (sqflite's onUpgrade often sits above onCreate).
        for f in &files {
            let own: Vec<&Chunk> = chunks.iter().filter(|x| &x.f == f).collect();
            for c in own.iter().filter(|c| is_create(c)).chain(own.iter().filter(|c| !is_create(c))) {
                apply_sql(&mut model, &c.sql, f, c.offset);
            }
        }
        finish(&model);
        if !model.entities.is_empty() || !skipped.is_empty() {
            stores.push(store("sql", format!("{dialect} (schema in code/SQL files)"), &files, Some(&skipped), &model));
        }
    }
    Ok(json!({ "stores": stores, "builtAt": crate::js::iso_now() }))
}

// Entities only referenced (policies on storage.objects, FKs to auth.users) are platform tables.
fn finish(model: &Model) {
    for ent in model.entities.values() {
        let mut e = ent.borrow_mut();
        // Never created here: either a platform table (only referenced) or one made outside migrations
        // (dashboard) that migrations later altered — its columns and RLS state are incomplete.
        if !crate::js::truthy(e.get("source")) {
            let has_cols = e.get("columns").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
            let changed = e.get("changedIn").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
            if has_cols || changed {
                e.insert("outsideMigrations".into(), json!(true));
            } else {
                e.insert("external".into(), json!(true));
            }
        }
        // Indexes on columns no migration defines = schema drift (changed in a dashboard, not in code).
        let names: Vec<String> = e
            .get("columns")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|c| c.get("name").and_then(Value::as_str).map(String::from)).collect())
            .unwrap_or_default();
        let flag = |k: &str| crate::js::truthy(e.get(k));
        if !flag("external") && !flag("outsideMigrations") && e.get("kind").and_then(Value::as_str) == Some("table") {
            let mut missing: Vec<String> = Vec::new();
            for i in e.get("indexes").and_then(Value::as_array).cloned().unwrap_or_default() {
                for c in i.get("columns").and_then(Value::as_array).cloned().unwrap_or_default() {
                    let Some(c) = c.as_str() else { continue };
                    if re!(r"^\w+$").test_str(c) && !names.iter().any(|n| n == c) && !missing.iter().any(|m| m == c) {
                        missing.push(c.to_string());
                    }
                }
            }
            if !missing.is_empty() {
                e.insert("unknownColumns".into(), str_list(&missing));
            }
        }
    }
}

/// Entity by name across stores (`profiles`, `public.profiles`, `auth.users`); Prisma also by the
/// mapped table name (`users` for model User) or the client accessor (`orderItem`).
/// Returns `(store, entity)`.
pub fn find_entity<'a>(data_model: &'a Value, name: &str) -> Option<(&'a Value, &'a Value)> {
    let key = re!(r"^public\.").replace_first_str(name, "");
    let stores = data_model.get("stores").and_then(Value::as_array)?;
    for store in stores {
        if let Some(ent) = store.get("entities").and_then(|e| e.get(&key)) {
            if crate::js::truthy(Some(ent)) {
                return Some((store, ent));
            }
        }
    }
    let lower = key.to_lowercase();
    for store in stores {
        let Some(Value::Object(ents)) = store.get("entities") else { continue };
        for ent in crate::js::js_values(ents) {
            let table = ent.get("table").and_then(Value::as_str);
            let name = ent.get("name").and_then(Value::as_str).unwrap_or("");
            if table == Some(key.as_str()) || name.to_lowercase() == lower || table.is_some_and(|t| t.to_lowercase() == lower) {
                return Some((store, ent));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::Builder::new().prefix("wara-data-").tempdir().unwrap();
        for (path, text) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        (dir, files.iter().map(|f| f.0.to_string()).collect())
    }

    // ports of legacy-node/test/data-model.test.mjs

    #[test]
    fn sources_skip_tests_generated_prisma_and_prisma_migrations() {
        let s0 = data_sources(
            &s(&[
                "supabase/migrations/0001_init.sql",
                "supabase/tests/rls.sql",
                "prisma/schema.prisma",
                "lib/generated/prisma/schema.prisma",
                "prisma/migrations/2026_init/migration.sql",
                "db/schema.sql",
                "test/fixtures/old.sql",
            ]),
            &s(&["lib/schema/tenant.ts", "test/migration_test.dart", "lib/data/repo.test.ts"]),
        );
        assert_eq!(
            s0,
            Sources {
                supabase: s(&["supabase/migrations/0001_init.sql"]),
                prisma: s(&["prisma/schema.prisma"]),
                sql: s(&["db/schema.sql", "lib/schema/tenant.ts"])
            }
        );
    }

    #[test]
    fn dumps_are_skipped_and_reported() {
        let (root, paths) = project(&[
            ("database/backup.sql", "-- phpMyAdmin SQL Dump\nCREATE TABLE old_stuff (id int);"),
            ("db/schema.sql", "create table students (id int primary key);"),
        ]);
        let dm = build_data_model(root.path(), &data_sources(&paths, &[]), "MySQL").unwrap();
        let store = &dm["stores"][0];
        assert_eq!(store["skipped"], json!(["database/backup.sql"]));
        assert_eq!(store["entities"].as_object().unwrap().keys().collect::<Vec<_>>(), ["students"]);
        assert!(store["label"].as_str().unwrap().starts_with("MySQL"));
    }

    #[test]
    fn schema_file_wins_over_migrate_route() {
        let (root, paths) = project(&[
            (
                "app/api/migrate/route.ts",
                "await q(`CREATE TABLE IF NOT EXISTS settings (id INT)`);\nawait q(`ALTER TABLE settings ADD COLUMN IF NOT EXISTS note TEXT`);",
            ),
            (
                "lib/schema/tenant.ts",
                "export const t = { settings: `CREATE TABLE IF NOT EXISTS settings (id INT PRIMARY KEY, \\`key\\` VARCHAR(100))` };",
            ),
        ]);
        let dm = build_data_model(root.path(), &data_sources(&[], &paths), "SQL").unwrap();
        let settings = &dm["stores"][0]["entities"]["settings"];
        assert_eq!(settings["source"]["file"], "lib/schema/tenant.ts");
        let cols: Vec<_> = settings["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(cols, ["id", "key", "note"]);
    }

    #[test]
    fn create_before_alter_inside_one_file() {
        let (root, paths) = project(&[(
            "lib/data/repo.dart",
            "onUpgrade: (db, o, n) async { await db.execute('ALTER TABLE pockets ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0'); },\nstatic Future<void> create(Database db) async {\n  await db.execute('''\n    CREATE TABLE pockets(\n      id INTEGER PRIMARY KEY AUTOINCREMENT,\n      name TEXT NOT NULL\n    )\n  ''');\n}",
        )]);
        let dm = build_data_model(root.path(), &data_sources(&[], &paths), "SQL").unwrap();
        let pockets = &dm["stores"][0]["entities"]["pockets"];
        let cols: Vec<_> = pockets["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(cols, ["id", "name", "pinned"]);
        assert_eq!(pockets["source"]["line"], 4);
    }

    #[test]
    fn flags_altered_only_unknown_index_columns_platform_tables() {
        let (root, paths) = project(&[(
            "supabase/migrations/0001.sql",
            "create table profiles (id uuid primary key references auth.users(id));\n      create index profiles_expiry_idx on profiles (plan_expires_at);\n      alter table payments add column note text;\n      create policy p on storage.objects for select using (true);",
        )]);
        let dm = build_data_model(root.path(), &data_sources(&paths, &[]), "SQL").unwrap();
        let e = &dm["stores"][0]["entities"];
        assert_eq!(e["profiles"]["unknownColumns"], json!(["plan_expires_at"]));
        assert_eq!(e["payments"]["outsideMigrations"], true);
        assert_eq!(e["storage.objects"]["external"], true);
    }

    #[test]
    fn find_entity_by_name_table_and_accessor() {
        let dm = json!({ "stores": [{ "kind": "prisma", "entities": { "OrderItem": { "name": "OrderItem", "table": "order_items", "columns": [] } } }] });
        for q in ["OrderItem", "order_items", "orderItem", "public.order_items"] {
            assert_eq!(find_entity(&dm, q).map(|(_, e)| e["name"].clone()), Some(json!("OrderItem")), "{q}");
        }
        assert!(find_entity(&dm, "orders").is_none());
    }
}
