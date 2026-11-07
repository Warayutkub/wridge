# Porting the wridge server from Node (legacy-node/) to Rust (server/)

**Status: done (2026-11-06).** The desktop app runs `server/app/target/release/wridge-server.exe` by default; Settings → Server → Engine switches back to Node (legacy-node) as a fallback. Parity: `node server/app/parity/run-all.mjs --slow`, `node server/graph/parity/run.mjs`, `node server/graph/parity/wave2.mjs`, `node server/figma/parity/parity.mjs`.

The wridge desktop app (`src-tauri/`, Tauri) runs `legacy-node/index.mjs` as a child process today. This folder holds its Rust replacement. When every part matches the Node server, the app runs the Rust server instead and Node is no longer needed.

## Ground rules (all crates)
1. **Same behaviour, same JSON.** For the same input, a Rust function or endpoint gives the same output as its Node original.
   - Same keys (camelCase where JS uses camelCase) and the same order where the order is meaningful (lists).
   - Same strings and error messages, and the same numbers. Beware: JS prints `3` for a float 3.0; serde prints `3.0`. Emit integers as integers wherever JS would.
   - String sorting: JS `a < b` compares UTF-16 code units, and `localeCompare` is locale-aware. Match whatever the JS does (write a helper).
   - Hashes: the same algorithm and encoding as `node:crypto` (check each call).
2. **On-disk formats are frozen.** The user's everyday server (wara-bridge, port 3056) reads the **same** project folders. These must stay byte-compatible, so either server can read what the other wrote:
   - `brain/*.md`, `brain/.trash/`, `docs/decisions.md`, `docs/flows/*.md`
   - `.wara/code-graph.db` and `.wara/brain.db`: same tables, columns, schema version, meta keys and pragmas
   - `code-projects.json`, `wara.components.json`
3. **Never write into the user's real projects while testing.** Copy a project to a temp folder (skip `node_modules`, `vendor`, `.git`, `build`, `dist`) and run both Node and Rust on copies. Read-only parity runs on the real folders are fine only for functions that never write; note that building or refreshing an index writes `.wara/`, so use copies for those.
4. **Parity tests are the definition of done.** For each module, write a script or test that runs the Node function (an ESM import of `legacy-node/...` from a small `node` script) and the Rust function on the same inputs, and diffs the JSON. Port the matching unit tests in `legacy-node/test/*.test.mjs` into Rust tests too.
5. **Don't touch** `legacy-node/`, `web/`, `src-tauri/` or `figma-plugin/` (read them freely). Don't stop or restart running servers; never kill node processes.
6. **Don't commit.** The lead commits.
7. Rust 2021+, stable toolchain (cargo is at `%USERPROFILE%\.cargo\bin\cargo.exe`; MSVC is installed). Each crate is a **standalone package** with its own `Cargo.lock` and `target/` (so parallel builds don't block each other). Put `target/` in the crate's `.gitignore`.
8. Prefer well-known crates:
   - core: `serde`/`serde_json`, `rusqlite` (feature `bundled`), `regex`, `walkdir`/`ignore`, `sha1`/`sha2`/`md5` as needed
   - parsing and files: `tree-sitter` + `tree-sitter-typescript`, `lopdf`, `rquickjs` (to run `elkjs` unchanged)
   - server: `axum`, `tokio`, `tokio-tungstenite`, `rmcp` (the official MCP SDK)

## Crates
| Folder | Package | Ports |
|---|---|---|
| `server/graph` | `wridge-graph` (lib) | `legacy-node/codegraph/**` — wave 1: db, files, styles, resolve-ts, routes-next, parse-ts, parse-dart, data/*, build, projects; wave 2: brain, memory, notes, knowledge, flows, query, design, export |
| `server/figma` | `wridge-figma` (lib) | `codegen.mjs`, `lottie.mjs`, `motion.mjs`, `motion-figma.mjs`, `motion-handoff.mjs`, `figjam.mjs` (elkjs inside rquickjs), `pdf.mjs` |
| `server/app` | `wridge-server` (lib + bin) | `index.mjs` (HTTP API, pages, WebSocket bridge to the Figma plugin, MCP over Streamable HTTP, restart/stop), `tools.mjs` (tool registry: names, descriptions, input schemas, profiles, handlers), `guards.mjs`, `graph.mjs`, `skills.mjs`, `wara.mjs` CLI |

Public API style: one Rust module per JS file, with function names in snake_case of the JS exports, e.g. `brain::brain_graph(project_dir) -> Result<serde_json::Value>` or typed structs that serialise to the exact JS shape. Errors are `anyhow::Error` carrying the same message text as the JS `throw`.

## Test projects (copy them first, see rule 3)
- a brain with 54 notes; Lua, 3 code files
- a Laravel + TypeScript app: 411 files; tables, flows
- a web app: 71 files
- a small web app: 32 files
- a Node project: 57 files

(The folders are listed in `server/parity-projects.json`, which stays on the developer's computer; see
`server/parity-projects.example.json`.)

## Report (short)
- what's ported and how parity was shown: the commands to re-run it
- what differs, and why
- what's left
