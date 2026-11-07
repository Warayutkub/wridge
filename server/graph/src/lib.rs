//! # wridge-graph
//!
//! Rust port of `legacy-node/codegraph/**` (the code graph stored in `<project>/.wara/code-graph.db`).
//! One module per JS file; function names are the snake_case of the JS exports. Output (database
//! rows, JSON) is byte-compatible with the Node version, so either server can read and update a
//! graph the other built.
//!
//! Wave 1: [`db`], [`files`], [`styles`], [`resolve_ts`], [`routes_next`], [`parse_ts`],
//! [`parse_dart`], [`data`] (`model`, `prisma`, `sql_ddl`, `sql_usage`, `usage`), [`build`] and
//! [`projects`].
//!
//! Wave 2: [`query`] (`graph()` + the `code_graph` answers), [`notes`] (code-graph notes),
//! [`brain`] (`brain/*.md` + `.wara/brain.db`), [`memory`] (`docs/decisions.md` + `.wara/memory.db`),
//! [`knowledge`] (`.wara/knowledge/`), [`flows`] (`docs/flows/*.md`, drafts, trace), [`design`]
//! (Figma snapshot, component links, tokens; uses `wridge-figma`'s codegen), [`export`] (the web
//! views' JSON) and [`cli`] (`wara code-graph` / `wara memory`).
//! - Results are `serde_json::Value`s in the JS shape (or small structs with `to_json`); serialise
//!   them with [`js::json_stringify`], which also puts integer-like keys first as JS does (e.g. the
//!   `externals` object of [`export::export_graph`], tag counters).
//! - Async JS that only waits on the Figma plugin is split: [`design::sync_design_from`] takes the
//!   three plugin answers, [`design::sync_design`] takes a blocking `send` closure.
//! - [`flows::save_flow_file`] reports a conflict as an `anyhow::Error` wrapping
//!   [`flows::FlowConflict`] (`err.downcast_ref::<FlowConflict>()` → HTTP 409).
//!
//! Shared helpers for later modules:
//! - [`js`]: `JSON.stringify` (JS key order and number format), `Number#toString`, UTF-16 lengths,
//!   JS string comparison, `localeCompare`, `toISOString`, `mtimeMs`.
//! - [`jsre`]: JS strings ([`jsre::JsStr`], UTF-16) and ECMAScript regexes ([`jsre::JsRegex`],
//!   the [`re!`] macro), so regex-heavy scanners behave exactly like V8.
//! - [`paths`]: `node:path` helpers (`posix.join/normalize/dirname`, win32 `resolve/relative`).
//! - [`db`]: open/migrate the SQLite file, meta get/set, transactions.
//!
//! ## Conventions (keep them in wave 2)
//! - JSON that goes to the DB, to files or to clients: [`js::json_stringify`] /
//!   [`js::json_stringify_pretty`] (never `serde_json::to_string`: JS prints `3` for 3.0 and puts
//!   integer-like keys first). JSON written by Node: [`js::json_parse`] (accepts V8's lone-surrogate
//!   escapes). `serde_json` is built with `preserve_order`, so objects keep JS insertion order; use
//!   `shift_remove` (not `remove`) for `delete obj.key`.
//! - A JS regex literal `/src/flags` becomes `re!(r"src", "flags")`, run on a [`jsre::JsStr`]; match
//!   indices, `.slice()`, `.length` are then UTF-16 like in JS. `\s`, `\b`, `/i` follow V8.
//! - Sorting: [`js::cmp_utf16`] for `.sort()` / `a < b`, [`js::locale_compare`] for `localeCompare`.
//! - Values from JSON bound to SQLite: [`db::js_sql_value`] (numbers as REAL, like `node:sqlite`).
//! - Files are read with [`js::read_utf8`] (BOM kept, like `readFileSync(f, "utf8")`); file times
//!   with [`js::mtime_ms`] (`stat.mtimeMs`).
//! - The crate pins rusqlite 0.39 (SQLite 3.51.3, the one inside Node 24.15) and tree-sitter
//!   0.27.0 + tree-sitter-typescript 0.23.2 (the WASM versions in legacy-node): same query plans
//!   and same parse trees. A crate that links this one must use the same rusqlite version.
//! - Dart files go through the analyzer program in `legacy-node/codegraph/dart/`
//!   ([`parse_dart::dart_dir`]; override with `WRIDGE_DART_DIR` or [`parse_dart::set_dart_dir`]).
//!
//! Parity with Node is checked by `parity/run.mjs` (whole DB, fresh/no-op/cross/forced updates),
//! `parity/scan.mjs` (every scanner, file by file), `parity/projects.mjs` and, for wave 2,
//! `parity/wave2.mjs` (every exported function on copies of the test projects, writes included,
//! and Node/Rust reading and updating each other's `brain.db`/`memory.db`).

pub mod js;
pub mod jsre;
pub mod paths;
pub(crate) mod util;

pub mod build;
pub mod data;
pub mod db;
pub mod files;
pub mod parse_dart;
pub mod parse_ts;
pub mod projects;
pub mod resolve_ts;
pub mod routes_next;
pub mod styles;

pub mod brain;
pub mod cli;
pub mod design;
pub mod export;
pub mod flows;
pub mod knowledge;
pub mod memory;
pub mod notes;
pub mod query;

pub use build::{graph_file, open_graph, project_root, update_graph, UpdateOptions};
pub use db::{get_meta, open_db, set_meta, transaction, SCHEMA_VERSION};
