// SQLite storage for the code graph: <project>/.wara/code-graph.db (built-in node:sqlite, no native deps).
import { DatabaseSync } from "node:sqlite";

// Bump when the tables change. The graph is rebuilt from source, but notes (written by AI) are carried over.
export const SCHEMA_VERSION = 4;

const SCHEMA = `
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
`;

export function openDb(file) {
  const db = new DatabaseSync(file);
  db.exec("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;");
  db.exec("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);");
  const version = Number(getMeta(db, "schema_version") ?? 0);
  if (version !== SCHEMA_VERSION) {
    const notes = version ? safeAll(db, "SELECT * FROM notes") : [];
    for (const t of ["refs", "imports", "symbols", "routes", "declared_routes", "style_defs", "style_refs", "data_refs", "notes", "files"]) db.exec(`DROP TABLE IF EXISTS ${t}`);
    db.exec(SCHEMA);
    const put = db.prepare("INSERT OR REPLACE INTO notes (target, text, code_hash, updated_at) VALUES (?, ?, ?, ?)");
    for (const n of notes) put.run(n.target, n.text, n.code_hash ?? null, n.updated_at);
    setMeta(db, "schema_version", String(SCHEMA_VERSION));
  } else {
    db.exec(SCHEMA);
  }
  return db;
}

function safeAll(db, sql) {
  try {
    return db.prepare(sql).all();
  } catch {
    return [];
  }
}

export const getMeta = (db, key) => db.prepare("SELECT value FROM meta WHERE key = ?").get(key)?.value;
export const setMeta = (db, key, value) => db.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)").run(key, value);

export function transaction(db, fn) {
  db.exec("BEGIN");
  try {
    const out = fn();
    db.exec("COMMIT");
    return out;
  } catch (err) {
    db.exec("ROLLBACK");
    throw err;
  }
}
