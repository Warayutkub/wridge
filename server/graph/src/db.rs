//! SQLite storage for the code graph: `<project>/.wara/code-graph.db` (port of `db.mjs`).
//!
//! Same tables, columns, schema version, meta keys and pragmas as the Node server, so either
//! can open the other's file. Notes (written by AI) survive a schema change; everything else is
//! rebuilt from source.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

/// Bump when the tables change (must match `SCHEMA_VERSION` in `db.mjs`).
pub const SCHEMA_VERSION: i64 = 4;

/// The DDL, verbatim from `db.mjs` (comments included: SQLite stores the text in `sqlite_master`).
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);

CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,          -- relative to the project root, forward slashes
  lang TEXT NOT NULL,                 -- ts | dart | config | <extension> (file-level only)
  mtime REAL, size INTEGER,
  directive TEXT,                     -- "use client" / "use server"
  parse_error TEXT
);

CREATE TABLE IF NOT EXISTS symbols (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,                 -- component | hook | function | class | const | type | interface | enum | widget | alias
  exported INTEGER NOT NULL DEFAULT 0,
  is_default INTEGER NOT NULL DEFAULT 0,
  line INTEGER, end_line INTEGER,
  props TEXT,                         -- JSON: { names: [...], type: "..." } for components/widgets
  hash TEXT                           -- hash of the symbol's source; notes go stale when it changes
);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS symbols_file ON symbols(file_id);

CREATE TABLE IF NOT EXISTS imports (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                 -- import | reexport | reexport_all | side_effect | dynamic | part
  source TEXT NOT NULL,               -- as written: "@/components/ui/button"
  imported TEXT,                      -- exported name taken from the target ("default", "*", "Button")
  local TEXT,                         -- name bound in this file (or the re-exported name)
  type_only INTEGER NOT NULL DEFAULT 0,
  line INTEGER,
  target_file_id INTEGER REFERENCES files(id) ON DELETE SET NULL,
  external TEXT                       -- package name when it resolves outside the project
);
CREATE INDEX IF NOT EXISTS imports_file ON imports(file_id);
CREATE INDEX IF NOT EXISTS imports_target ON imports(target_file_id);

CREATE TABLE IF NOT EXISTS refs (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  symbol_id INTEGER REFERENCES symbols(id) ON DELETE CASCADE,  -- enclosing top-level symbol; NULL = module level
  local TEXT NOT NULL,                -- identifier used (an import binding or a symbol of the same file)
  member TEXT,                        -- X.member when used as an object
  kind TEXT NOT NULL,                 -- jsx | ref | type
  line INTEGER, count INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS refs_file_local ON refs(file_id, local);
CREATE INDEX IF NOT EXISTS refs_symbol ON refs(symbol_id);

CREATE TABLE IF NOT EXISTS routes (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  framework TEXT NOT NULL,            -- next-app | next-pages | flutter
  path TEXT NOT NULL,                 -- URL pattern: /jobs/[id]
  kind TEXT NOT NULL,                 -- page | layout | route | loading | error | not-found | template | default | proxy | screen
  root TEXT                           -- app/pages directory or router file
);
CREATE INDEX IF NOT EXISTS routes_path ON routes(path);

-- Routes declared in code (GoRoute(path:), MaterialApp(home:, routes:)); turned into routes by relink.
CREATE TABLE IF NOT EXISTS declared_routes (
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  path TEXT NOT NULL, widget TEXT, kind TEXT NOT NULL, line INTEGER
);

-- Design tokens defined in code: CSS custom properties (--color-primary) and Dart statics (AppColors.primary).
CREATE TABLE IF NOT EXISTS style_defs (
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  name TEXT NOT NULL, value TEXT, line INTEGER
);
CREATE INDEX IF NOT EXISTS style_defs_name ON style_defs(name);

-- Token usage: var(--x) | tw (Tailwind utility, e.g. bg-primary) | dart (AppColors.primary) | hex (raw color in code)
CREATE TABLE IF NOT EXISTS style_refs (
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  form TEXT NOT NULL, name TEXT NOT NULL, line INTEGER, count INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS style_refs_name ON style_refs(name);

-- Code that touches data: form = access style (supabase…), op = read | write | call,
-- columns/filters = JSON arrays when the query names them. The schema itself is meta "data_model".
CREATE TABLE IF NOT EXISTS data_refs (
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  form TEXT NOT NULL, entity TEXT NOT NULL, op TEXT NOT NULL, detail TEXT,
  columns TEXT, filters TEXT, via TEXT, line INTEGER
);
CREATE INDEX IF NOT EXISTS data_refs_entity ON data_refs(entity);

CREATE TABLE IF NOT EXISTS notes (
  target TEXT PRIMARY KEY,            -- dir:<path> | file:<path> | symbol:<path>#<name>
  text TEXT NOT NULL,
  code_hash TEXT,                     -- hash of the code when the note was written
  updated_at TEXT NOT NULL
);
"#;

/// Tables dropped (in this order) when the schema version changes.
const GRAPH_TABLES: [&str; 10] = [
    "refs",
    "imports",
    "symbols",
    "routes",
    "declared_routes",
    "style_defs",
    "style_refs",
    "data_refs",
    "notes",
    "files",
];

/// `Number(text)` for the schema_version meta value (NaN when not a number).
fn js_to_number(s: Option<&str>) -> f64 {
    match s {
        None => 0.0,
        Some(s) => {
            let t = crate::js::js_trim(s);
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
    }
}

struct Note {
    target: String,
    text: String,
    code_hash: Option<String>,
    updated_at: String,
}

fn safe_notes(db: &Connection) -> Vec<Note> {
    let run = || -> rusqlite::Result<Vec<Note>> {
        let mut st = db.prepare("SELECT * FROM notes")?;
        let names: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let has = |n: &str| names.iter().any(|x| x == n);
        let has_hash = has("code_hash");
        let rows = st.query_map([], |r| {
            Ok(Note {
                target: r.get("target")?,
                text: r.get("text")?,
                code_hash: if has_hash { r.get("code_hash")? } else { None },
                updated_at: r.get("updated_at")?,
            })
        })?;
        rows.collect()
    };
    run().unwrap_or_default()
}

/// Opens (creating if needed) a graph database, migrating it when the schema version differs:
/// every graph table is dropped and recreated, notes are carried over.
pub fn open_db(file: impl AsRef<Path>) -> Result<Connection> {
    let db = Connection::open(file.as_ref())?;
    db.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    db.execute_batch("PRAGMA foreign_keys = ON;")?;
    db.busy_timeout(Duration::from_millis(5000))?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);")?;
    let version = js_to_number(get_meta(&db, "schema_version")?.as_deref());
    if version != SCHEMA_VERSION as f64 {
        let truthy = version != 0.0 && !version.is_nan();
        let notes = if truthy { safe_notes(&db) } else { Vec::new() };
        for t in GRAPH_TABLES {
            db.execute_batch(&format!("DROP TABLE IF EXISTS {t}"))?;
        }
        db.execute_batch(SCHEMA)?;
        {
            let mut put = db.prepare("INSERT OR REPLACE INTO notes (target, text, code_hash, updated_at) VALUES (?, ?, ?, ?)")?;
            for n in &notes {
                put.execute(params![n.target, n.text, n.code_hash, n.updated_at])?;
            }
        }
        set_meta(&db, "schema_version", &SCHEMA_VERSION.to_string())?;
    } else {
        db.execute_batch(SCHEMA)?;
    }
    Ok(db)
}

/// `getMeta(db, key)`: the value, or `None` when the key is missing (or NULL).
pub fn get_meta(db: &Connection, key: &str) -> Result<Option<String>> {
    let v: Option<Option<String>> = db.query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get(0)).optional()?;
    Ok(v.flatten())
}

/// `setMeta(db, key, value)`.
pub fn set_meta(db: &Connection, key: &str, value: &str) -> Result<()> {
    db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", params![key, value])?;
    Ok(())
}

/// `transaction(db, fn)`: BEGIN, run, COMMIT — or ROLLBACK and return the error.
pub fn transaction<T>(db: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    db.execute_batch("BEGIN")?;
    match f() {
        Ok(v) => {
            db.execute_batch("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            let _ = db.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Binds a JSON value the way `node:sqlite` binds a JS value: numbers as REAL (column affinity
/// then stores integers as INTEGER), strings as TEXT, null as NULL. Booleans/objects are refused
/// like `node:sqlite` refuses them.
pub fn js_sql_value(v: &serde_json::Value, index: usize) -> Result<rusqlite::types::Value> {
    use rusqlite::types::Value as V;
    Ok(match v {
        serde_json::Value::Null => V::Null,
        serde_json::Value::Number(n) => V::Real(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => V::Text(s.clone()),
        _ => anyhow::bail!("Provided value cannot be bound to SQLite parameter {index}."),
    })
}
