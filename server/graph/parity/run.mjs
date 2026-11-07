// Parity check: the Rust code graph (server/graph) against the Node one (legacy-node/codegraph).
//
// For each project: copy it twice to a temp folder (skipping node_modules, vendor, .git, build, dist,
// .wara and other generated folders; a copy of a git repo gets a fresh `git init` so git-based file
// listing and the .gitignore handling run), build the graph with Node in one copy and with Rust in
// the other, then dump every table of .wara/code-graph.db (rows sorted; timestamps and mtimes
// normalised) and diff. Then:
//   - no-op update on both (nothing may change),
//   - identical edits in both copies (modify / add / delete a file, add a SQL driver to
//     package.json so every file is rescanned for data use), then CROSS updates: Rust updates the
//     Node-built DB and Node updates the Rust-built DB — the two results must match,
//   - a forced rebuild on both.
//
// usage (from anywhere):
//   node server/graph/parity/run.mjs                 # the five test projects of server/PORTING.md + fixtures/edge-cases
//   node server/graph/parity/run.mjs --project C:/path/to/project [--project …] [--keep] [--no-build]
// Exit code 0 = every comparison matched.
import { execFileSync, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, unlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { PROJECTS } from "../../parity-projects.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CRATE = join(HERE, "..");
const DEFAULT_PROJECTS = [
  ...PROJECTS,
  join(HERE, "fixtures", "edge-cases"), // synthetic: BOMs, emoji at slice limits, Thai, localeCompare ties, TS errors
];
const SKIP = new Set(["node_modules", "vendor", ".git", "build", "dist", ".wara", ".next", ".dart_tool", ".turbo", ".vercel", "coverage", "target", "Pods", ".gradle", ".idea", "__pycache__", ".venv"]);

const args = process.argv.slice(2);
const projects = [];
for (let i = 0; i < args.length; i++) if (args[i] === "--project") projects.push(args[++i]);
const keep = args.includes("--keep");
const noBuild = args.includes("--no-build");

const EXE = join(CRATE, "target", "release", "examples", process.platform === "win32" ? "update_graph.exe" : "update_graph");
if (!noBuild) {
  console.log("building the Rust example (release)…");
  execFileSync(process.env.CARGO ?? join(process.env.USERPROFILE ?? process.env.HOME, ".cargo", "bin", "cargo"), ["build", "--release", "--example", "update_graph"], { cwd: CRATE, stdio: "inherit" });
}

// ---------- running ----------

function runNode(dir, force = false) {
  const r = spawnSync(process.execPath, [join(HERE, "node-build.mjs"), dir, ...(force ? ["--force"] : [])], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (r.status !== 0) throw new Error(`node build failed in ${dir}: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

function runRust(dir, force = false) {
  const r = spawnSync(EXE, [dir, ...(force ? ["--force"] : [])], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (r.status !== 0) throw new Error(`rust build failed in ${dir}: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

// ---------- dumping ----------

const TS = "<ts>";

function normMeta(key, value) {
  if (key === "updated_at") return TS;
  if (key === "data_model") {
    const m = JSON.parse(value);
    m.builtAt = TS;
    return JSON.stringify(m);
  }
  if (key === "data_stamp") return value.split("|").map((p, i) => (i === 0 ? p : p.replace(/:([^:]+):([^:]+)$/, ":<mtime>:$2"))).join("|");
  return value;
}

/** Every table → sorted rows (arrays in column order), with timestamps normalised. */
function dump(dbFile, { rawMtime = false } = {}) {
  const db = new DatabaseSync(dbFile, { readOnly: true });
  try {
    const out = {};
    const master = db.prepare("SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").all();
    out.sqlite_master = master.map((m) => JSON.stringify([m.type, m.name, m.tbl_name, m.sql]));
    out.pragma = [JSON.stringify(db.prepare("PRAGMA journal_mode").get()), JSON.stringify(db.prepare("PRAGMA user_version").get())];
    for (const { name } of master.filter((m) => m.type === "table")) {
      const cols = db.prepare(`PRAGMA table_info(${name})`).all().map((c) => c.name);
      const rows = db.prepare(`SELECT * FROM ${name}`).all().map((r) => {
        const row = cols.map((c) => {
          const v = r[c];
          return typeof v === "bigint" ? Number(v) : v instanceof Uint8Array ? `blob:${Buffer.from(v).toString("hex")}` : v;
        });
        if (name === "files" && !rawMtime) row[cols.indexOf("mtime")] = "<mtime>";
        if (name === "meta") row[1] = normMeta(row[0], row[1]);
        return JSON.stringify(row);
      });
      out[name] = rows.sort();
    }
    return out;
  } finally {
    db.close();
  }
}

function diffDumps(a, b, labelA, labelB) {
  const problems = [];
  for (const t of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const ra = a[t] ?? [];
    const rb = b[t] ?? [];
    const sa = new Set(ra);
    const sb = new Set(rb);
    const onlyA = ra.filter((x) => !sb.has(x));
    const onlyB = rb.filter((x) => !sa.has(x));
    if (onlyA.length || onlyB.length || ra.length !== rb.length) {
      problems.push(`  table ${t}: ${labelA} ${ra.length} rows, ${labelB} ${rb.length} rows; only in ${labelA}: ${onlyA.length}, only in ${labelB}: ${onlyB.length}`);
      for (const x of onlyA.slice(0, 5)) problems.push(`    - ${labelA}: ${x.slice(0, 400)}`);
      for (const x of onlyB.slice(0, 5)) problems.push(`    + ${labelB}: ${x.slice(0, 400)}`);
    }
  }
  return problems;
}

function normInfo(info, root) {
  const r = root.replaceAll("\\", "/");
  const s = JSON.stringify(info).split(r).join("<root>");
  const o = JSON.parse(s);
  delete o.ms;
  return o;
}

function diffInfo(a, rootA, b, rootB) {
  const x = JSON.stringify(normInfo(a, rootA));
  const y = JSON.stringify(normInfo(b, rootB));
  return x === y ? [] : [`  info differs:\n    node: ${x}\n    rust: ${y}`];
}

// ---------- copies and edits ----------

function copyProject(src, dest) {
  rmSync(dest, { recursive: true, force: true });
  mkdirSync(dest, { recursive: true });
  cpSync(src, dest, {
    recursive: true,
    preserveTimestamps: true,
    filter: (p) => !SKIP.has(basename(p)),
  });
  if (existsSync(join(src, ".git"))) execFileSync("git", ["init", "-q", dest]);
}

function graphPaths(dbFile) {
  const db = new DatabaseSync(dbFile, { readOnly: true });
  try {
    return db.prepare("SELECT path, lang FROM files ORDER BY path").all();
  } finally {
    db.close();
  }
}

// The same edits in both copies, with the same mtimes.
function applyEdits(dirs, files) {
  const when = new Date("2026-01-02T03:04:05.678Z");
  const pick = (lang) => files.filter((f) => f.lang === lang).map((f) => f.path);
  const ts = pick("ts");
  const dart = pick("dart");
  const pool = ts.length ? ts : dart.length ? dart : files.filter((f) => f.lang !== "config").map((f) => f.path);
  const edits = [];
  if (pool.length) {
    const target = pool[Math.floor(pool.length / 3)];
    const victim = pool.length > 1 ? pool[pool.length - 1] : null;
    const isDart = target.endsWith(".dart");
    const added = isDart ? `${dirname(target)}/parity_added.dart` : target.endsWith(".ts") || target.endsWith(".tsx") || ts.length ? `${dirname(target)}/parity-added.ts` : "parity_added.sql";
    const rel = `./${basename(target)}`;
    const addedText = isDart
      ? `import '${basename(target)}';\n\nclass ParityAdded {}\n`
      : added.endsWith(".sql")
        ? "create table parity_added (id int primary key, name text);\n"
        : `import * as target from "${rel.replace(/\.(tsx?|jsx?|mjs|cjs)$/, "")}";\nexport const parityAdded = target;\nexport function useParity() { return 1; }\n`;
    for (const d of dirs) {
      const t = join(d, target);
      writeFileSync(t, readFileSync(t, "utf8") + (isDart ? "\nconst parityEdit = 1;\n" : "\nexport const parityEdit = `SELECT id FROM parity_table WHERE id = ${1}`;\n"));
      utimesSync(t, when, when);
      writeFileSync(join(d, added), addedText);
      utimesSync(join(d, added), when, when);
      if (victim && victim !== target) unlinkSync(join(d, victim));
    }
    edits.push(`modified ${target}`, `added ${added}`, ...(victim && victim !== target ? [`deleted ${victim}`] : []));
  }
  // A SQL driver appears in package.json: data access styles change → every source is rescanned.
  for (const d of dirs) {
    const pkg = join(d, "package.json");
    if (!existsSync(pkg)) continue;
    try {
      const json = JSON.parse(readFileSync(pkg, "utf8"));
      json.dependencies = { ...json.dependencies, pg: "^8.0.0" };
      writeFileSync(pkg, `${JSON.stringify(json, null, 2)}\n`);
      utimesSync(pkg, when, when);
      if (d === dirs[0]) edits.push("package.json: + pg");
    } catch {
      // not JSON: leave it
    }
  }
  return edits;
}

// ---------- main ----------

const base = join(tmpdir(), "wridge-graph-parity");
let failures = 0;
const summary = [];

// .gitignore handling on synthetic repos (the test projects already ignore .wara/):
// no trailing newline + project in a subfolder, no .gitignore at all, trailing newline.
for (const [label, gitignore, sub] of [
  ["gitignore: append, subfolder project", "node_modules", "apps/web"],
  ["gitignore: created", null, ""],
  ["gitignore: append after newline", "dist/\n", ""],
]) {
  const infos = {};
  const texts = {};
  for (const side of ["node", "rust"]) {
    const repo = join(base, "_gitignore", side);
    rmSync(repo, { recursive: true, force: true });
    mkdirSync(join(repo, sub, "src"), { recursive: true });
    execFileSync("git", ["init", "-q", repo]);
    if (gitignore !== null) writeFileSync(join(repo, ".gitignore"), gitignore);
    writeFileSync(join(repo, sub, "src", "a.ts"), "export const a = 1;\n");
    const dir = join(repo, sub);
    const first = side === "node" ? runNode(dir) : runRust(dir);
    const second = side === "node" ? runNode(dir) : runRust(dir);
    infos[side] = [normInfo(first, repo), normInfo(second, repo)];
    texts[side] = readFileSync(join(repo, ".gitignore"), "utf8");
  }
  const problems = [];
  if (JSON.stringify(infos.node) !== JSON.stringify(infos.rust)) problems.push(`  node: ${JSON.stringify(infos.node)}\n  rust: ${JSON.stringify(infos.rust)}`);
  if (texts.node !== texts.rust) problems.push(`  .gitignore node: ${JSON.stringify(texts.node)}\n  .gitignore rust: ${JSON.stringify(texts.rust)}`);
  if (!infos.node[0].gitignore?.added) problems.push(`  expected the first build to add a pattern, got ${JSON.stringify(infos.node[0].gitignore)}`);
  if (problems.length) failures++;
  console.log(`${problems.length ? "✗" : "✓"} ${label}: ${JSON.stringify(infos.rust[0].gitignore)}`);
  for (const p of problems) console.log(p);
  summary.push(`${"(synthetic)".padEnd(12)} ${problems.length ? "FAIL" : "ok  "} ${label}`);
}
if (!keep) rmSync(join(base, "_gitignore"), { recursive: true, force: true });

// Schema migration: a graph from an older schema version is rebuilt, its notes carried over.
{
  const dumps = {};
  for (const side of ["node", "rust"]) {
    const dir = join(base, "_migrate", side);
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(join(dir, ".wara"), { recursive: true });
    writeFileSync(join(dir, "a.ts"), "export function useThing() { return 1; }\n");
    const old = new DatabaseSync(join(dir, ".wara", "code-graph.db"));
    old.exec(`CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('schema_version', '3'), ('data_stamp', 'v1|');
      CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT); INSERT INTO files VALUES (1, 'old.ts');
      CREATE TABLE notes (target TEXT PRIMARY KEY, text TEXT NOT NULL, code_hash TEXT, updated_at TEXT NOT NULL);
      INSERT INTO notes VALUES ('file:a.ts', 'keep me', 'abc', '2026-01-01T00:00:00.000Z'), ('dir:src', 'and me', NULL, '2026-01-02T00:00:00.000Z');`);
    old.close();
    if (side === "node") runNode(dir);
    else runRust(dir);
    dumps[side] = dump(join(dir, ".wara", "code-graph.db"));
  }
  const problems = diffDumps(dumps.node, dumps.rust, "node", "rust");
  if (!dumps.rust.notes?.some((r) => r.includes("keep me"))) problems.push("  notes were not carried over");
  if (problems.length) failures++;
  console.log(`${problems.length ? "✗" : "✓"} schema migration (v3 → v4, notes kept)`);
  for (const p of problems) console.log(p);
  summary.push(`${"(synthetic)".padEnd(12)} ${problems.length ? "FAIL" : "ok  "} schema migration`);
  if (!keep) rmSync(join(base, "_migrate"), { recursive: true, force: true });
}

for (const project of projects.length ? projects : DEFAULT_PROJECTS) {
  const name = basename(project);
  const nodeDir = join(base, name, "node");
  const rustDir = join(base, name, "rust");
  console.log(`\n=== ${name} (${project})`);
  copyProject(project, nodeDir);
  copyProject(project, rustDir);
  const dbOf = (d) => join(d, ".wara", "code-graph.db");
  const check = (step, problems) => {
    if (problems.length) {
      failures++;
      console.log(`  ✗ ${step}`);
      for (const p of problems) console.log(p);
    } else console.log(`  ✓ ${step}`);
    summary.push(`${name.padEnd(12)} ${problems.length ? "FAIL" : "ok  "} ${step}`);
  };

  // 1. fresh build
  let t = Date.now();
  const ni = runNode(nodeDir);
  const nodeMs = Date.now() - t;
  t = Date.now();
  const ri = runRust(rustDir);
  const rustMs = Date.now() - t;
  console.log(`  files ${ni.files}, parsed ${ni.parsed}, parse errors ${ni.parseErrors.length}, unresolved ${ni.unresolvedImports}; node ${nodeMs} ms, rust ${rustMs} ms`);
  check("fresh build: info", diffInfo(ni, nodeDir, ri, rustDir));
  check("fresh build: every table", diffDumps(dump(dbOf(nodeDir)), dump(dbOf(rustDir)), "node", "rust"));
  check("fresh build: raw mtimes", diffDumps({ files: dump(dbOf(nodeDir), { rawMtime: true }).files }, { files: dump(dbOf(rustDir), { rawMtime: true }).files }, "node", "rust"));
  const gi = (d) => (existsSync(join(d, ".gitignore")) ? readFileSync(join(d, ".gitignore"), "utf8") : null);
  check(".gitignore after build", gi(nodeDir) === gi(rustDir) ? [] : [`  node: ${JSON.stringify(gi(nodeDir))}\n  rust: ${JSON.stringify(gi(rustDir))}`]);

  // 2. no-op update
  const ni2 = runNode(nodeDir);
  const ri2 = runRust(rustDir);
  check("no-op update: info", diffInfo(ni2, nodeDir, ri2, rustDir).concat(ni2.changed || ni2.removed ? [`  node changed ${ni2.changed} removed ${ni2.removed}`] : []));
  check("no-op update: every table", diffDumps(dump(dbOf(nodeDir)), dump(dbOf(rustDir)), "node", "rust"));

  // 3. identical edits, then cross updates (Rust on the Node-built DB, Node on the Rust-built DB)
  const edits = applyEdits([nodeDir, rustDir], graphPaths(dbOf(nodeDir)));
  console.log(`  edits: ${edits.join("; ")}`);
  const rustOnNode = runRust(nodeDir);
  const nodeOnRust = runNode(rustDir);
  check("cross update: info (rust on node db vs node on rust db)", diffInfo(nodeOnRust, rustDir, rustOnNode, nodeDir));
  check("cross update: every table", diffDumps(dump(dbOf(rustDir)), dump(dbOf(nodeDir)), "node-updated", "rust-updated"));

  // 4. forced rebuild
  const nf = runNode(nodeDir, true);
  const rf = runRust(rustDir, true);
  check("forced rebuild: info", diffInfo(nf, nodeDir, rf, rustDir));
  check("forced rebuild: every table", diffDumps(dump(dbOf(nodeDir)), dump(dbOf(rustDir)), "node", "rust"));

  if (!keep) rmSync(join(base, name), { recursive: true, force: true });
}

console.log(`\n${summary.join("\n")}`);
console.log(failures ? `\n${failures} comparison(s) differ` : "\nall comparisons match");
if (keep) console.log(`copies kept in ${base}`);
process.exit(failures ? 1 : 0);
