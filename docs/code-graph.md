# Code graph — maintainer notes

How the code graph inside wara-bridge works, and what to touch when something has to change. For how AI uses it, see `skills/code-graph.md`.

## Pieces

| File (`server/codegraph/`) | Job |
|---|---|
| `files.mjs` | Which files belong to the graph (`git ls-files` incl. untracked, respecting every `.gitignore`; plain walk outside git), language by extension, and `ensureGitignored()` |
| `db.mjs` | SQLite schema (`node:sqlite`, no native deps). `SCHEMA_VERSION` bump = rebuild from source; notes are carried over |
| `parse-ts.mjs` | TS/TSX/JS via `web-tree-sitter` + `tree-sitter-typescript` (WASM). Symbols, props, imports/re-exports, references (`jsx` / `ref` / `type`) |
| `resolve-ts.mjs` | Import → file: relative, tsconfig/jsconfig `paths` + `baseUrl` (follows relative `extends`), workspace packages, index files, `.js`→`.ts` |
| `routes-next.mjs` | Next.js App Router (`app/`, `src/app/`: groups, slots, intercepting, private folders), Pages Router, proxy/middleware |
| `parse-dart.mjs` + `dart/` | Dart via the official analyzer (see below) |
| `styles.mjs` | Token definitions/usages in web code: CSS custom properties, Tailwind utilities (mapped to `--color-*`, `--radius-*`, …), raw hex colors |
| `build.mjs` | `updateGraph()`: list → diff by mtime/size → parse changed files → write → `relink()` (import targets, routes, Flutter screens) |
| `query.mjs` | Text answers: overview, find, file, used_by, uses, impact, route, design, tokens, notes. `graph(db)` loads everything into maps per query |
| `design.mjs` | Figma snapshot (`code_design_sync`), component links (`wara.components.json` + name matching; class mappings `{element, className, variants}` are checked against the stylesheets on demand), token table, `confirmLink()` |
| `knowledge.mjs` | `project_knowledge`: detected stack (package.json / go.mod, one level of subfolders) and researched notes per topic in `.wara/knowledge/<topic>/<tool>@<major>.md`; stale after 180 days or a major version change |
| `data/sql-ddl.mjs` | Tolerant SQL DDL reader (Postgres/MySQL): splits statements (quotes, `$$` bodies, comments), applies CREATE/ALTER/DROP TABLE, indexes, RLS policies, views, functions in order; counts what it can not read |
| `data/model.mjs` | One data model for every store: Supabase migrations (filename order); plain SQL = `.sql` files + DDL strings in code, ordered schema files → other creating code → migrations → ALTER-only code; dumps (phpMyAdmin/mysqldump/pg_dump) skipped, cached in meta `data_model`, rebuilt when a source changes (`MODEL_VERSION` bump forces it). Flags tables created outside migrations and indexes on unknown columns |
| `data/usage.mjs`, `data/sql-usage.mjs` (also Dart: string literals incl. `'''…'''`, sqflite `query/insert/update/delete`; pubspec decides `sqflite`; test files are never schema sources) | Code → data refs (`data_refs` table): supabase-js `.from().select/insert/update/upsert/delete`, `.rpc()`; raw SQL string literals (mysql2/pg/sqlite drivers) parsed for tables (aliases), columns and WHERE/ON filters. A file whose strings hold CREATE/ALTER TABLE is recorded (op `ddl`) and becomes a schema source. `DATA_SCAN_VERSION` bump rescans every file once |
| `data/prisma.mjs` | schema.prisma (models, `@map`/`@@map`, `@id`/`@unique`/`@default`, `@relation` FKs, `@@index`) and client calls `x.<model>.<op>({ select, where, data, include })`; generated copies skipped, `prisma/migrations/*.sql` left to the Prisma store |
| `memory.mjs`, `memory-cli.mjs` | `project_memory`: project decisions in `docs/decisions.md` (committed; indexed into `.wara/memory.db` when its mtime/size changes), tasks/handoffs and working notes only in `.wara/memory.db`. `node wara.mjs memory brief --stdin` is the Claude Code SessionStart hook (startup/clear/compact) that injects open tasks + decision titles |
| `notes.mjs` | AI notes, staleness hashes, "worth a note" suggestions |
| `export.mjs` | Whole graph as JSON for the `/code` page |
| `projects.mjs` | Projects added in the `/code` page (`server/code-projects.json`, git-ignored). The page may only read graphs of listed folders |
| `cli.mjs` | `node wara.mjs code-graph …` — works without the server |

Server side: tools `code_graph`, `code_graph_update`, `code_graph_note`, `code_design_sync` (`tools.mjs`, profile `code`); routes `/code`, `/api/code/*` (`index.mjs`). Page: `server/code.html` + `server/mindmap.js` (generic mind-map renderer) + `ui.css`/`ui.js`.

## The `.gitignore` rule
`ensureGitignored()` runs **before** `.wara/` is created. In a git repo it asks `git check-ignore` whether `.wara/code-graph.db` is ignored; if not, it appends `/<subdir>/.wara/` (or `/.wara/`) to the repo-root `.gitignore`, creating the file if needed. Outside git it does nothing. Callers surface `gitignore.added` to the user.

## Data model (short)
- `files` — path (relative, `/`), lang (`ts`, `dart`, `config`, or an extension for file-level-only languages), mtime/size, directive, parse_error.
- `symbols` — top-level declarations. `props` JSON: component props (`names`, `type`), Dart `extends`/`of` (State→widget)/`statics` (token classes). `hash` = hash of the declaration's source (notes staleness).
- `imports` — `kind` import | reexport | reexport_all | side_effect | dynamic | part | part_of; resolved `target_file_id` or `external` package.
- `refs` — uses of an import binding or same-file symbol, owned by the enclosing top-level symbol (NULL = module code). Dart has no named imports, so `query.bindingTarget()` resolves a Dart name through the file's `import '…'` list.
- `routes` (+ `declared_routes` for GoRouter/MaterialApp), `style_defs`, `style_refs`, `notes`, `meta` (`updated_at`, `design` snapshot JSON).

Edges are resolved at **query** time from names, so changing one file never requires re-parsing its users. `relink()` recomputes import targets and routes for the whole project after any change (cheap: no disk reads except tsconfig/package.json/pubspec).

## Dart parser
`codegraph/dart/bin/parse.dart` uses `package:analyzer` (syntax only, `parseString`, `throwIfDiagnostics: false`). It reads `{"files": [...]}` on stdin and prints JSON per file. `parse-dart.mjs` compiles it to `dart/build/wara-dart-parse(.exe)` on first use (`dart pub get` + `dart compile exe`, ~7 s) and rebuilds when `parse.dart`, `pubspec.yaml` or `pubspec.lock` is newer than the exe. Needs the Dart SDK (ships with Flutter) on PATH.

- **The analyzer is pinned** (`pubspec.yaml`: `analyzer: 14.4.0`) because its AST API changes between majors (e.g. `NamedExpression`→`NamedArgument`, class names moving to `namePart`). Version-sensitive reads go through `nameOf()`, `membersOfDecl()`, `namedArg()` and `paramName()` which try several shapes. To upgrade: bump the version, `dart pub get`, `dart analyze bin/parse.dart`, fix what breaks, test on a Flutter project.
- Why not tree-sitter for Dart: the npm `tree-sitter-dart` grammar is too old for Dart 3 (sealed classes, switch expressions, patterns) and its WASM doesn't load in current `web-tree-sitter`.
- Widgets: classes extending a known base (`widgetBases`) or (at query time) a project widget. `State<X>` classes are merged into `X`. Constructions `Foo(…)`/`const Foo(…)` are `jsx` refs; inside route builders they are `nav`, inside `showDialog`-like calls `overlay`.

## Adding a language
1. `files.mjs`: give its extension a lang (`CODE`), or leave it in `OTHER` for file level only.
2. Write `parse-<lang>.mjs` returning `{ directive, symbols, imports, refs, routes? }` in the same shape as `parse-ts.mjs` (refs carry `owner` = index into `symbols`).
3. `build.mjs`: call it for that lang (batch if the parser is an external process).
4. Resolution: add a resolver in `relink()` keyed by file extension.
5. If the language has no named imports, extend `bindingTarget()` like the Dart branch.
6. Test with a real project: `node server/wara.mjs code-graph update --force --dir <project>`, then `overview` (look for parse errors and unresolved imports) and a few `used_by`/`route` queries.

## Testing quickly
- **Automated:** `cd server && npm test` (node:test, `server/test/*.test.mjs`, < 1 s). Every reader bug found becomes a test case first, then the fix. Run it before every commit.
- Other people's projects: test read-only, on a copy (`git archive HEAD | tar -x -C <scratch>`), never in their folder.
- CLI against a project: `node server/wara.mjs code-graph overview --dir <project>`.
- A throwaway project for staleness/notes: a temp folder with two `.tsx` files (not a git repo, so `.gitignore` is untouched).
- Web page without touching the user's server: run the server on spare ports (`WARA_PORT=4055 WARA_HTTP_PORT=4056`) — the Figma plugin only talks to 3055, so design sync needs the real server.
