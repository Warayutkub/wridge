// Parity check for wave 2 of the Rust code graph: brain, memory, notes, knowledge, flows, design,
// query and export (server/graph/src/*.rs) against legacy-node/codegraph/*.mjs.
//
// For each project: two temp copies (node/<name>, rust/<name>; node_modules, .git, build… skipped;
// .wara/memory.db and .wara/knowledge/ are copied so real memory and knowledge are read; a git repo
// gets a fresh `git init`). The code graph is built with Node in both copies (wave 1 has its own
// parity check), then the same list of calls runs in Node (parity/node-wave2.mjs) on one copy and in
// Rust (examples/wave2.rs) on the other, and every answer is compared as raw JSON text (roots and
// ISO timestamps normalised):
//   1. read:  every exported function on the project as it is — brain (parse, find, read, graph,
//      code links, trash), memory, knowledge, flows (parse/format, overview, drafts, trace, checks),
//      code-graph notes, design, export, the code_graph queries (find, file, used_by, uses, impact,
//      route, design, tokens, notes, data, table, flows, trace, with brain signposts) and the HTTP
//      read paths (/api/brain, /code-links, /note, trash, /api/flows, /api/flows/trace)
//   2. write: the same writes on both copies (brain write/append/replace/trash/restore/empty,
//      memory decide/note/handoff, knowledge facts/notes, flow files, code-graph notes, design
//      sync + confirm link), then every file and the brain/memory/notes tables are compared
//   3. cross: Rust reads the Node-written brain.db/memory.db and Node the Rust-written ones (same
//      answers, and the index is not rewritten: dumps with rowids identical before/after), then an
//      identical edit is synced by Rust in the Node copy and by Node in the Rust copy → equal dumps.
//
// usage: node server/graph/parity/wave2.mjs [--project DIR …] [--only NAME] [--keep] [--no-build] [--verbose]
// Exit code 0 = every comparison matched.
import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { PROJECTS } from "../../parity-projects.mjs";

process.removeAllListeners("warning");
const HERE = dirname(fileURLToPath(import.meta.url));
const CRATE = join(HERE, "..");
const DEFAULT_PROJECTS = [
  ...PROJECTS,
  join(HERE, "fixtures", "edge-cases"),
  join(HERE, "fixtures", "wave2"), // synthetic: brain links/tags/trash, flows, decisions, design links, stack
];
const SKIP = new Set(["node_modules", "vendor", ".git", "build", "dist", ".wara", ".next", ".dart_tool", ".turbo", ".vercel", "coverage", "target", "Pods", ".gradle", ".idea", "__pycache__", ".venv"]);

const args = process.argv.slice(2);
const projects = [];
for (let i = 0; i < args.length; i++) if (args[i] === "--project") projects.push(args[++i]);
const only = args.includes("--only") ? args[args.indexOf("--only") + 1] : null;
const keep = args.includes("--keep");
const verbose = args.includes("--verbose");
const EXE = join(CRATE, "target", "release", "examples", process.platform === "win32" ? "wave2.exe" : "wave2");
if (!args.includes("--no-build")) {
  console.log("building the Rust example (release)…");
  execFileSync(process.env.CARGO ?? join(process.env.USERPROFILE ?? process.env.HOME, ".cargo", "bin", "cargo"), ["build", "--release", "--example", "wave2"], { cwd: CRATE, stdio: "inherit" });
}

const flowsMod = await import("../../../legacy-node/codegraph/flows.mjs");
const knowledgeMod = await import("../../../legacy-node/codegraph/knowledge.mjs");

// ---------- running ----------

const base = join(tmpdir(), "wridge-wave2");
const callsFile = join(base, "calls.json");

function run(side, calls, root) {
  const text = JSON.stringify(calls).split("$ROOT").join(root.replaceAll("\\", "/"));
  writeFileSync(callsFile, text);
  const r = side === "node"
    ? spawnSync(process.execPath, [join(HERE, "node-wave2.mjs"), callsFile], { encoding: "utf8", maxBuffer: 512 * 1024 * 1024 })
    : spawnSync(EXE, [callsFile], { encoding: "utf8", maxBuffer: 512 * 1024 * 1024 });
  if (r.status !== 0) throw new Error(`${side} dispatcher failed: ${r.stderr}`);
  if (process.env.WAVE2_TIMES) {
    const slow = r.stderr.split(/\r?\n/).filter((l) => / ms /.test(l)).map((l) => [Number(l.trim().split(" ")[0]), l.trim()]).sort((a, b) => b[0] - a[0]);
    const total = slow.reduce((n, x) => n + x[0], 0);
    console.log(`    ${side}: ${total} ms in calls; slowest: ${slow.slice(0, 6).map((x) => x[1]).join(" | ")}`);
  }
  const lines = r.stdout.split("\n").filter(Boolean);
  if (lines.length !== calls.length) throw new Error(`${side}: ${lines.length} answers for ${calls.length} calls\n${r.stderr}`);
  return lines;
}

function normText(s, roots) {
  let out = s;
  for (const r of roots) {
    for (const form of [r.replaceAll("\\", "/"), r.replaceAll("/", "\\"), JSON.stringify(r.replaceAll("/", "\\")).slice(1, -1)]) out = out.split(form).join("<root>");
  }
  return out
    .replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z/g, "<ts>")
    .replace(/\(updated \d{4}-\d{2}-\d{2} \d{2}:\d{2}\)/g, "(updated <ts>)") // handoff task blocks: the time to the minute
    .replace(/flow-[0-9a-z]{6,}/g, "flow-<t>")
    .replace(/"ageDays":-?\d+/g, '"ageDays":<n>');
}

let failures = 0;
const summary = [];

function compare(name, step, calls, a, b, rootsA, rootsB) {
  const problems = [];
  for (let i = 0; i < calls.length; i++) {
    const x = normText(a[i], rootsA);
    const y = normText(b[i], rootsB);
    if (x !== y) problems.push({ i, call: calls[i], node: x, rust: y });
  }
  if (problems.length) failures++;
  const label = `${name.padEnd(12)} ${problems.length ? "FAIL" : "ok  "} ${step}: ${calls.length - problems.length}/${calls.length} calls match`;
  console.log(`  ${problems.length ? "✗" : "✓"} ${step}: ${calls.length - problems.length}/${calls.length} calls match`);
  for (const p of problems.slice(0, verbose ? 50 : 8)) {
    const at = [...p.node].findIndex((c, k) => c !== p.rust[k]);
    console.log(`    #${p.i} ${JSON.stringify(p.call).slice(0, 200)}`);
    console.log(`      node: …${p.node.slice(Math.max(0, at - 120), at + 280)}`);
    console.log(`      rust: …${p.rust.slice(Math.max(0, at - 120), at + 280)}`);
  }
  summary.push(label);
  return problems.length === 0;
}

function check(name, step, problems) {
  if (problems.length) failures++;
  console.log(`  ${problems.length ? "✗" : "✓"} ${step}`);
  for (const p of problems.slice(0, verbose ? 50 : 10)) console.log(`    ${p}`);
  summary.push(`${name.padEnd(12)} ${problems.length ? "FAIL" : "ok  "} ${step}`);
}

// ---------- copies ----------

function copyProject(src, dest, { git = false } = {}) {
  rmSync(dest, { recursive: true, force: true });
  mkdirSync(dest, { recursive: true });
  cpSync(src, dest, { recursive: true, preserveTimestamps: true, filter: (p) => !SKIP.has(basename(p)) });
  for (const keepWara of ["memory.db", "knowledge"]) {
    const from = join(src, ".wara", keepWara);
    if (existsSync(from)) cpSync(from, join(dest, ".wara", keepWara), { recursive: true, preserveTimestamps: true });
  }
  if (git || existsSync(join(src, ".git"))) execFileSync("git", ["init", "-q", dest]);
}

function nodeBuild(dir) {
  const r = spawnSync(process.execPath, [join(HERE, "node-build.mjs"), dir], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (r.status !== 0) throw new Error(`node build failed in ${dir}: ${r.stderr}`);
}

function walkFiles(dir, base = dir, out = [], { all = false } = {}) {
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (!all && e.name.startsWith(".")) continue;
    const full = join(dir, e.name);
    if (e.isDirectory()) walkFiles(full, base, out, { all });
    else out.push(full.slice(base.length + 1).replaceAll("\\", "/"));
  }
  return out;
}

// ---------- databases ----------

const ISO = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z/g;

/** Every table of a SQLite file → rows (rowid first) as JSON lines, in rowid order. */
function dumpDb(file, { tables = null, stamps = "raw" } = {}) {
  if (!existsSync(file)) return { missing: true };
  const db = new DatabaseSync(file, { readOnly: true });
  try {
    const out = {};
    const master = db.prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").all();
    out.sqlite_master = master.map((m) => JSON.stringify([m.type, m.name, m.sql]));
    out.pragma = [JSON.stringify(db.prepare("PRAGMA journal_mode").get()), JSON.stringify(db.prepare("PRAGMA user_version").get())];
    for (const { name } of master.filter((m) => m.type === "table" && (!tables || tables.includes(m.name)))) {
      out[name] = db.prepare(`SELECT rowid AS _rowid_, * FROM ${name} ORDER BY rowid`).all().map((r) => {
        let s = JSON.stringify(Object.values(r).map((v) => (typeof v === "bigint" ? Number(v) : v)));
        if (stamps === "norm") s = s.replace(/"(-?[\d.e+]+):(\d+)"/g, '"<mtime>:$2"');
        return s.replace(ISO, "<ts>");
      });
    }
    return out;
  } finally {
    db.close();
  }
}

function diffDumps(a, b, label) {
  const problems = [];
  if (a.missing || b.missing) return a.missing && b.missing ? [] : [`${label}: exists only on one side (node ${!a.missing}, rust ${!b.missing})`];
  for (const t of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const ra = a[t] ?? [];
    const rb = b[t] ?? [];
    if (JSON.stringify(ra) === JSON.stringify(rb)) continue;
    problems.push(`${label} table ${t}: node ${ra.length} rows, rust ${rb.length} rows`);
    for (let i = 0; i < Math.max(ra.length, rb.length) && problems.length < 12; i++) {
      if (ra[i] !== rb[i]) problems.push(`  row ${i}\n      node: ${String(ra[i]).slice(0, 300)}\n      rust: ${String(rb[i]).slice(0, 300)}`);
    }
  }
  return problems;
}

// ---------- call lists ----------

const R = "$ROOT";
const c = (fn, ...a) => ({ fn, args: a });
const uniq = (xs) => [...new Set(xs)];
const spread = (xs, n) => (xs.length <= n ? xs : Array.from({ length: n }, (_, i) => xs[Math.floor((i * xs.length) / n)]));
const LINK = /\[\[([^\]|#]+)(?:#([^\]|]*))?(?:\|[^\]]*)?\]\]/g;

function graphFacts(dir) {
  const db = new DatabaseSync(join(dir, ".wara", "code-graph.db"), { readOnly: true });
  try {
    const files = db.prepare("SELECT id, path, lang FROM files ORDER BY id").all();
    const syms = db.prepare("SELECT s.id, s.name, s.kind, s.exported, f.path FROM symbols s JOIN files f ON f.id = s.file_id WHERE s.kind != 'alias' ORDER BY s.id").all();
    const routes = db.prepare("SELECT r.path, r.kind, r.framework, r.file_id FROM routes r ORDER BY r.id").all();
    const handlers = db.prepare("SELECT s.name, r.path FROM routes r JOIN symbols s ON s.file_id = r.file_id WHERE r.kind = 'route' AND s.exported = 1 AND s.name IN ('GET','POST','PUT','PATCH','DELETE') ORDER BY r.id, s.id").all();
    const defs = db.prepare("SELECT DISTINCT name FROM style_defs ORDER BY name").all().map((r) => r.name);
    let model = null;
    try { model = JSON.parse(db.prepare("SELECT value FROM meta WHERE key = 'data_model'").get()?.value ?? "null"); } catch { model = null; }
    const ents = (model?.stores ?? []).flatMap((s) => Object.values(s.entities));
    return { files, syms, routes, handlers, defs, ents };
  } finally {
    db.close();
  }
}

function readCalls(dir) {
  const calls = [];
  const g = graphFacts(dir);
  const code = g.files.filter((f) => f.lang === "ts" || f.lang === "dart");
  const names = uniq(g.syms.map((s) => s.name));
  const symNames = spread(names, 40);
  const filePaths = spread(code.map((f) => f.path), 25);
  const pathSyms = spread(g.syms.filter((s) => s.exported), 15).map((s) => `${s.path}#${s.name}`);
  const routePaths = spread(uniq(g.routes.map((r) => r.path)), 30);
  const handlerTargets = spread(g.handlers.map((h) => `${h.name} ${h.path}`), 20);
  const tables = spread(g.ents.map((e) => e.name), 30);
  const tableCols = spread(g.ents.filter((e) => e.columns?.length).flatMap((e) => e.columns.slice(0, 2).map((col) => `${e.name}.${col.name}`)), 15);
  const dirs = spread(uniq(code.map((f) => f.path.split("/").slice(0, -1).join("/")).filter(Boolean)), 6);

  // ---- brain ----
  const brainDir = join(dir, "brain");
  const notes = walkFiles(brainDir).filter((p) => /\.md$/i.test(p));
  const titles = [];
  const raws = [];
  const codeTargets = [];
  for (const rel of notes) {
    const text = readFileSync(join(brainDir, rel), "utf8");
    calls.push(c("brain.parseNote", text, rel), c("brain.parseFrontMatter", text));
    const h = /^#\s+(.+)$/m.exec(text);
    if (h) titles.push(h[1].trim());
    for (const m of text.matchAll(LINK)) {
      raws.push(m[1]);
      if (/^code:/i.test(m[1].trim())) codeTargets.push(`${m[1].trim().slice(5).trim()}${m[2] ? `#${m[2].trim()}` : ""}`);
    }
  }
  for (const raw of spread(uniq(raws), 150)) calls.push(c("brain.linkTarget", raw));
  calls.push(c("brain.brainSummary", R), c("brain.codeLinks", R), c("brain.listTrash", R));
  for (const q of uniq(["", "   ", "zzzqqq", "TRAIPHUM", "the world", "ครุฑ", "บอส", "#lore", ...spread(titles, 15).map((t) => t.split(/\s+/)[0]), ...spread(titles, 5)])) calls.push(c("brain.findNotes", R, q), c("brain.findNotes", R, q, { limit: 3 }));
  for (const rel of notes) calls.push(c("brain.readNote", R, rel.replace(/\.md$/i, "")));
  for (const t of spread(titles, 15)) calls.push(c("brain.readNote", R, t), c("brain.readNote", R, ` ${t.toLowerCase()}.md `));
  calls.push(c("brain.readNote", R, "no-such-note"), c("brain.readNote", R, ""));
  calls.push(c("brain.brainGraph", R));
  for (const t of uniq([...codeTargets, "src/none.ts", "", "./lib/db.ts", "lib\\db.ts#query", "query", "Nope"])) {
    const [p, n] = t.split("#");
    calls.push(c("brain.codeLinkMissing", R, t), c("brain.notesAbout", R, { path: p }), c("brain.notesAbout", R, { name: n ?? p }), c("brain.notesAbout", R, { path: p, name: n }));
  }
  calls.push(c("brain.notesAbout", R, {}), c("brain.notesAbout", R, { path: "" }));

  // ---- memory ----
  const decisionsFile = join(dir, "docs", "decisions.md");
  const decText = existsSync(decisionsFile) ? readFileSync(decisionsFile, "utf8") : "";
  calls.push(c("memory.parseDecisions", decText), c("memory.parseDecisions", "## D-7 | Title only\n**Supersedes**: D-1, D-2\n\nbody\n## x"));
  const decWords = spread(uniq((decText.match(/^## .+$/gm) ?? []).flatMap((l) => l.replace(/^##\s+/, "").split(/[\s·|–-]+/)).filter((w) => w.length > 3)), 10);
  calls.push(c("memory.overview", R, { brief: true }), c("memory.overview", R));
  for (const q of uniq(["", "payments", "zzzqqq", "สี", ...decWords])) calls.push(c("memory.find", R, q));
  calls.push(c("memory.find", R, "payments", { includeOld: true, limit: 2 }));
  calls.push(c("memory.findProjectRoot", `${R}/lib`), c("memory.findProjectRoot", R), c("memory.findProjectRoot", `${R}/docs/x/y`));

  // ---- knowledge ----
  let stack = [];
  try { stack = knowledgeMod.detectStack(dir); } catch { stack = []; }
  calls.push(c("knowledge.detectStack", R), c("knowledge.listKnowledge", R), c("knowledge.listKnowledge", R, "security"), c("knowledge.listKnowledge", R, "Security Stuff"), c("knowledge.readFacts", R));
  for (const s of stack) calls.push(c("knowledge.getKnowledge", R, "security", s.tool));
  for (const k of walkFiles(join(dir, ".wara", "knowledge"))) {
    const [topic, file] = k.split("/");
    if (file) calls.push(c("knowledge.getKnowledge", R, topic, file.replace(/@.*$/, "")));
  }
  calls.push(c("knowledge.getKnowledge", R, "cache", "nope"));
  for (const v of ["^16.1.0", "v2.52.5", "", "beta", "1.2", null]) calls.push(c("knowledge.major", v));

  // ---- flows ----
  const flowFiles = existsSync(join(dir, "docs", "flows")) ? readdirSync(join(dir, "docs", "flows")).filter((f) => f.endsWith(".md")) : [];
  const flowObjs = [];
  for (const f of flowFiles) {
    const text = readFileSync(join(dir, "docs", "flows", f), "utf8");
    const parsed = flowsMod.parseFlow(text);
    flowObjs.push({ file: `docs/flows/${f}`, hash: flowsMod.hashText(text), ...parsed });
    calls.push(c("flows.parseFlow", text), c("flows.hashText", text), c("flows.formatFlow", parsed), c("flows.slugify", parsed.title), c("flows.checkFlow", R, parsed));
  }
  const webFlow = { title: "Web", entry: "", steps: [{ id: 1, group: null, actor: "", text: "a", wait: true, branches: [{ label: "x", to: 3 }, { label: "y", to: "end" }, { label: "z" }] }, { id: 3, group: 2, actor: "db", text: "b" }, { id: 4, group: 2, text: "c" }, { id: 5, group: "2", actor: "x", text: "d", branches: null }], states: [{ column: "t.c", values: ["a", "b", null], transitions: [{ from: "a", to: "b", note: "n" }] }], notes: "  note  ", extra: [{ heading: "More", lines: ["x", null, 3] }] };
  calls.push(c("flows.formatFlow", webFlow), c("flows.formatFlow", {}), c("flows.formatFlow", { steps: [] }), c("flows.parseFlow", "# T\r\n## Steps\n1. [a] x `y`\n   - go → STEP 2B\n2b. [b] z\n## States\n- `x.y` : a → b"));
  for (const s of ["Sign up / Verify!", "café ﬁle Ａ", "  Checkout  ", "a".repeat(80)]) calls.push(c("flows.slugify", s));
  calls.push(c("flows.readFlowFiles", R), c("flows.readIgnored", R), c("flows.flowsOverview", R), c("flows.draftFlows", R, R, flowObjs));
  calls.push(c("flows.statusValues", R, R, g.ents.map((e) => e.name)));
  calls.push(c("flows.flowsText", R, R, null), c("flows.flowsText", R, R, null, 1), c("flows.flowsText", R, R, "nope"));
  for (const f of flowObjs) calls.push(c("flows.flowsText", R, R, f.title.toLowerCase()), c("flows.flowsText", R, R, f.file.split("/").pop().replace(/\.md$/, "")));
  for (const h of handlerTargets) calls.push(c("flows.flowsText", R, R, h.toLowerCase()));
  const traceTargets = uniq([...handlerTargets, ...routePaths, ...spread(names, 15), ...spread(filePaths, 5), "nope", "/nope", "GET /nope", "", `${routePaths[0] ?? "/x"}:12`]);
  for (const t of traceTargets) calls.push(c("flows.trace", R, t), c("flows.traceText", R, t));
  calls.push(c("flows.trace", R, traceTargets[0] ?? "x", { depth: 1, max: 2 }));
  const refs = uniq([...tables, ...tableCols, ...routePaths, ...handlerTargets, ...filePaths.slice(0, 5), ...symNames.slice(0, 10), "a, b", "foo bar", "x:12", "lib/db.ts:3", "payments.status", "Payments", "webhook /api/x", "cron /api/cron/cleanup", "DELETE /api/checkout", "no_such_table.col", "app/nope.tsx"]);
  for (const r of refs) calls.push(c("flows.checkRef", R, r));

  // ---- query (code_graph) ----
  const q = (query, target, opts = { root: R }) => calls.push(c("query.runQuery", R, query, target ?? null, opts));
  q("overview"); q("overview", null, { limit: 5 });
  for (const n of uniq(["page", "zzzqqq", "use", "Button", ...spread(names, 12).map((x) => x.slice(0, 4).toLowerCase()), ...tables.slice(0, 5)])) q("find", n);
  q("find", "page", { limit: 5, root: R });
  for (const f of uniq([...filePaths, "nope.ts", `./${filePaths[0] ?? "x"}`, (filePaths[0] ?? "x").split("/").pop()])) q("file", f);
  for (const t of uniq([...symNames.slice(0, 25), ...pathSyms, ...filePaths.slice(0, 5), "nope", "/"])) { q("used_by", t); q("uses", t); q("impact", t); }
  for (const t of symNames.slice(0, 5)) { q("impact", t, { depth: 1, root: R }); q("used_by", t, { limit: 5 }); q("impact", t, { depth: 5, limit: 5, root: R }); }
  for (const t of uniq([...tables, ...tableCols, "table:" + (tables[0] ?? "x"), "nope.col"])) { q("impact", t); q("table", t); }
  for (const r of uniq([...routePaths, "nope", "/nope", ...g.routes.filter((x) => x.framework === "flutter").slice(0, 5).map((x) => x.path)])) { q("route", r); q("route", r, { depth: 1, root: R }); }
  q("design"); q("tokens"); for (const t of spread(g.defs, 8)) q("tokens", t.replace(/^--/, "")); q("tokens", "nope");
  q("notes"); q("notes", null, { limit: 5, root: R }); q("data"); q("data", null, { limit: 5 });
  q("flows"); q("flows", "checkout"); q("trace", handlerTargets[0] ?? "nope");
  q("bogus", "x"); q("find"); q("file", "   ");
  q("used_by", symNames[0] ?? "x", {}); q("find", "page", {});

  // ---- code-graph notes, design, export, graph ----
  calls.push(c("notes.allNotes", R), c("notes.missingNotes", R, 15), c("notes.missingNotes", R, 3));
  for (const f of filePaths.slice(0, 10)) calls.push(c("notes.notesFor", R, f));
  for (const t of uniq([...dirs.map((d) => `dir:${d}`), "dir:", "./", `${dirs[0] ?? "x"}/`, "dir:nope", ...filePaths.slice(0, 5), ...pathSyms.slice(0, 5), ...symNames.slice(0, 10), "nope", `${filePaths[0] ?? "x"}#nope`])) calls.push(c("notes.resolveNoteTarget", R, t));
  calls.push(c("design.designSnapshot", R), c("design.componentLinks", R, R), c("design.tokenTable", R));
  for (const f of filePaths.slice(0, 10)) calls.push(c("design.importPathFor", R, R, f));
  calls.push(c("export.exportData", R, R), c("export.exportGraph", R, R), c("query.graph", R));

  // ---- HTTP read paths ----
  calls.push(c("http.brain", R), c("http.codeLinks", R), c("http.trash", R), c("http.flows", R));
  for (const n of [...notes.slice(0, 5).map((x) => x.replace(/\.md$/i, "")), "nope"]) calls.push(c("http.note", R, n));
  for (const t of [handlerTargets[0] ?? "x", routePaths[0] ?? "/", ""]) calls.push(c("http.flowsTrace", R, t));
  return { calls, g, notes, titles };
}

// Figma answers for design.syncDesign (shapes of get_document_info / get_components / get_tokens).
const FIGMA = [
  { fileName: "Shop UI" },
  [
    { name: "Button", id: "1:1", type: "COMPONENT_SET" }, { name: "Card/Default", id: "1:2", type: "COMPONENT" }, { name: "Icon Button", id: "1:3", type: "COMPONENT" },
    { name: "Type=Primary/CardHeader", id: "1:4", type: "COMPONENT" }, { name: "Badge", id: "1:5", type: "COMPONENT" }, { name: "Pill", id: "1:6", type: "COMPONENT" },
    { name: "Frame", id: "1:7", type: "FRAME" }, { name: "Order Page", id: "1:8", type: "COMPONENT" }, { name: "Nothing/Matches", id: "1:9", type: "COMPONENT" },
  ],
  {
    collections: [{ name: "Colors", defaultMode: "Light" }],
    variables: [
      { token: "color-primary", type: "COLOR", collection: "Colors", values: { Light: "#2563eb", Dark: "#1d4ed8" } },
      { token: "color-base", type: "COLOR", collection: "Colors", values: { Light: "#FFFFFFFF" } },
      { token: "color-brand", type: "COLOR", collection: "Colors", values: { Light: { alias: "color-primary" } } },
      { token: "radius-card", type: "FLOAT", collection: "Colors", values: { Light: 8 } },
      { token: "flag", type: "BOOLEAN", collection: "Colors", values: { Light: true } },
      { token: "font-name", type: "STRING", collection: "Other", values: { X: "Kanit" } },
    ],
    paintStyles: [{ token: "surface", value: "#fafafa" }, { token: "gradient", value: { stops: [] } }],
    textStyles: [{ token: "text-body", family: "Kanit", style: "Regular", size: 16, lineHeight: "150%", letterSpacing: "0px" }],
    effectStyles: [{ token: "shadow-card", shadows: [{ x: 0, y: 2, blur: 4, spread: 0, color: "#0000001a" }] }, { token: "shadow-none", shadows: [] }],
  },
];

function writeCalls(facts) {
  const { g, notes } = facts;
  const code = g.files.filter((f) => f.lang === "ts" || f.lang === "dart");
  const file = code[Math.floor(code.length / 2)]?.path ?? "none.ts";
  const dir = code.find((f) => f.path.includes("/"))?.path.split("/").slice(0, -1).join("/") ?? "src";
  const counts = new Map();
  for (const s of g.syms) counts.set(s.name, (counts.get(s.name) ?? 0) + 1);
  const unique = g.syms.find((s) => counts.get(s.name) === 1 && s.exported);
  const comps = g.syms.filter((s) => s.exported);
  const calls = [];
  // code-graph notes
  calls.push(
    c("notes.writeNote", R, `dir:${dir}`, "Folder note: what lives here."),
    c("notes.writeNote", R, file, "  File note ใช้ร่วมกัน  "),
    c("notes.writeNote", R, unique ? `${unique.path}#${unique.name}` : file, "Symbol note."),
    c("notes.writeNote", R, unique?.name ?? file, "Name note replaces it."),
    c("notes.writeNote", R, "nope-target-zzz", "x"),
    c("notes.allNotes", R), c("query.runQuery", R, "notes", null, { root: R }), c("query.runQuery", R, "overview", null, { root: R }),
    c("query.runQuery", R, "file", file, { root: R }),
  );
  if (unique) calls.push(c("query.runQuery", R, "used_by", `${unique.path}#${unique.name}`, { root: R }), c("query.runQuery", R, "uses", unique.name, { root: R }));
  calls.push(c("notes.writeNote", R, file, ""), c("notes.writeNote", R, `dir:${dir}`, "   "), c("notes.allNotes", R), c("export.exportGraph", R, R));
  // design
  calls.push(
    c("design.syncDesign", R, ...FIGMA), c("design.designSnapshot", R), c("design.componentLinks", R, R), c("design.tokenTable", R),
    c("query.runQuery", R, "design", null, { root: R }), c("query.runQuery", R, "tokens", null, { root: R }), c("query.runQuery", R, "tokens", "primary", { root: R }),
    c("query.runQuery", R, "tokens", "color-brand", { root: R }), c("export.exportGraph", R, R),
    c("design.confirmLink", R, R, "Button", comps[0]?.id ?? 1), c("design.confirmLink", R, R, "New/Thing", comps[1]?.id ?? 2), c("design.confirmLink", R, R, "X", 999999),
    c("design.confirmLink", R, R, "Y", 1.5), c("design.componentLinks", R, R), c("query.runQuery", R, "design", null, { root: R }),
  );
  // brain
  calls.push(
    c("brain.writeNote", R, "rules/Speed", { text: "ห้ามวิ่ง [[CH1]] [[ไม่มีจริง]]", tags: ["rule", "Speed"] }),
    c("brain.writeNote", R, "Speed", { text: "dup" }),
    c("brain.writeNote", R, "Speed", { text: "เพิ่ม\n\n", mode: "append" }),
    c("brain.writeNote", R, "speed", { text: "# Speed\nใหม่ [[CH1]]", mode: "replace" }),
    c("brain.writeNote", R, "Speed", { text: "inplace", mode: "replace", keepOld: false }),
    c("brain.writeNote", R, "Odd<na:me>?|x", { text: "odd" }),
    c("brain.writeNote", R, "", { text: "x" }),
    c("brain.writeNote", R, "x", { text: "  " }),
    c("brain.writeNote", R, ".hidden/secret", { text: "x" }),
    c("brain.writeNote", R, "weird", { text: "---\ntitle: x\n---\nbody", tags: ["t"] }),
    c("brain.writeNote", R, "Speed", { text: "foo", mode: "foo" }),
    c("brain.writeNote", R, "a/../b/..x..md", { text: "dots" }),
    c("brain.readNote", R, "Speed"), c("brain.listTrash", R),
    c("brain.trashNote", R, "Speed"), c("brain.trashNote", R, "rules/Speed"),
    c("brain.listTrash", R), c("brain.restoreNote", R, "rules/Speed (2).md"), c("brain.restoreNote", R, "rules/Speed.md"), c("brain.restoreNote", R, "../../x.md"),
    c("brain.trashNote", R, "Speed"), c("brain.listTrash", R),
    c("brain.emptyTrash", R, "../x.md"), c("brain.emptyTrash", R, "nope.md"), c("brain.emptyTrash", R, "rules/Speed.md"), c("brain.listTrash", R),
  );
  if (notes.length) {
    const victim = notes.find((n) => /nathee|ch1/i.test(n)) ?? notes[0];
    calls.push(c("brain.trashNote", R, victim.replace(/\.md$/i, "")), c("brain.brainGraph", R), c("brain.restoreNote", R, victim), c("brain.brainSummary", R));
  }
  calls.push(c("brain.brainGraph", R), c("brain.findNotes", R, "speed"), c("brain.emptyTrash", R), c("brain.listTrash", R), c("brain.emptyTrash", R));
  // memory
  calls.push(
    c("memory.decide", R, { title: "  Redis not used yet ", decision: "Rate limits in memory.", tags: ["Infra"] }),
    c("memory.decide", R, { title: "Use Redis for rate limits", decision: "Two instances now.", why: " scale ", tags: ["infra", "Cache"], supersedes: "D-1" }),
    c("memory.decide", R, { title: "No tags", decision: "x" }),
    c("memory.note", R, { task: "Foundations", text: " Keep all 4 grays. ", tags: ["ui"] }),
    c("memory.note", R, { text: "no task note" }),
    c("memory.handoff", R, { task: "Foundations", stage: "5/8", next: "Input, Card" }),
    c("memory.handoff", R, { task: "Foundations", done: "Buttons" }),
    c("memory.handoff", R, { task: "Other", questions: "Which font?" }),
    c("memory.overview", R), c("memory.find", R, "redis", { includeOld: true }), c("memory.find", R, "grays"),
    c("memory.handoff", R, { task: "Foundations", close: true }), c("memory.handoff", R, { task: "Nope", close: true }),
    c("memory.overview", R), c("memory.overview", R, { brief: true }), c("memory.find", R, "grays", { includeOld: true }),
  );
  // knowledge
  calls.push(
    c("knowledge.setFact", R, "deploy", "VPS"), c("knowledge.setFact", R, "redis", "no"), c("knowledge.setFact", R, "deploy", "Docker"), c("knowledge.setFact", R, "redis", ""), c("knowledge.readFacts", R),
    c("knowledge.saveKnowledge", R, { topic: "Security", tool: "next", content: "  Use middleware.  ", sources: ["https://nextjs.org/docs"] }),
    c("knowledge.saveKnowledge", R, { topic: "cache", tool: "github.com/gofiber/fiber", version: "v2.1.0", content: "x" }),
    c("knowledge.saveKnowledge", R, { topic: "x", tool: "nope", content: "x" }),
    c("knowledge.listKnowledge", R, "security"), c("knowledge.listKnowledge", R),
    c("knowledge.getKnowledge", R, "security", "next"), c("knowledge.getKnowledge", R, "cache", "github.com/gofiber/fiber"),
  );
  // flows
  const sample = flowsMod.formatFlow(flowsMod.parseFlow("# Flow: New flow\nentry: web\n\n## Steps\n1. [user] opens `/checkout`\n2. [db] writes `payments`\n"));
  calls.push(
    c("flows.saveFlowFile", R, "new-flow.md", sample, null), c("flows.saveFlowFile", R, "new-flow.md", sample, null),
    c("flows.saveFlowFile", R, "docs/flows/new-flow.md", "changed\n", flowsMod.hashText(sample)), c("flows.saveFlowFile", R, "new-flow.md", "again", "deadbeef"),
    c("flows.saveFlowFile", R, "../escape.md", "x", null), c("flows.saveFlowFile", R, "absent.md", "x", "abc"),
    c("flows.ignoreDraft", R, "GET /api/checkout"), c("flows.ignoreDraft", R, "POST /api/x"), c("flows.ignoreDraft", R, "GET /api/checkout"), c("flows.readIgnored", R),
    c("flows.flowsOverview", R), c("flows.flowsText", R, R, null), c("http.flows", R),
  );
  return calls;
}

// Text files after the writes (indexes and the code graph excluded), normalised.
function textState(dir, roots) {
  const out = {};
  for (const rel of walkFiles(dir, dir, [], { all: true })) {
    if (/\.(db|db-wal|db-shm)$/.test(rel) || rel.startsWith(".git/")) continue;
    if (!/^(brain\/|docs\/|\.wara\/|wara\.components\.json$|\.gitignore$)/.test(rel)) continue;
    out[rel] = normText(readFileSync(join(dir, rel), "utf8"), roots);
  }
  return out;
}

function crossCalls(facts) {
  const n = facts.notes[0]?.replace(/\.md$/i, "") ?? "nope";
  return [
    c("brain.brainGraph", R), c("brain.findNotes", R, "the"), c("brain.readNote", R, n), c("brain.listTrash", R), c("brain.brainSummary", R),
    c("memory.overview", R), c("memory.find", R, "redis", { includeOld: true }), c("notes.allNotes", R), c("design.designSnapshot", R),
    c("knowledge.listKnowledge", R), c("flows.flowsOverview", R), c("query.runQuery", R, "overview", null, { root: R }),
  ];
}

function crossEdit(dir, facts) {
  const when = new Date("2026-01-02T03:04:05.678Z");
  const note = facts.notes[0] ? join(dir, "brain", facts.notes[0]) : null;
  if (note && existsSync(note)) {
    appendFileSync(note, "\ncross edit [[CH1]] #cross\n");
    utimesSync(note, when, when);
  }
  mkdirSync(join(dir, "brain", "x"), { recursive: true });
  writeFileSync(join(dir, "brain", "x", "Cross.md"), "# Cross\n[[Speed]] [[code:lib/db.ts]]\n");
  utimesSync(join(dir, "brain", "x", "Cross.md"), when, when);
  const dec = join(dir, "docs", "decisions.md");
  if (existsSync(dec)) {
    appendFileSync(dec, "\n## D-99 · 2026-01-02 · Cross edit\nTags: cross\n");
    utimesSync(dec, when, when);
  }
}

const crossWrites = () => [c("brain.brainGraph", R), c("memory.overview", R), c("memory.note", R, { task: "Cross", text: "cross" }), c("memory.handoff", R, { task: "Cross", stage: "1/2" }), c("brain.findNotes", R, "cross")];

// ---------- main ----------

mkdirSync(base, { recursive: true });
for (const project of projects.length ? projects : DEFAULT_PROJECTS) {
  const name = basename(project);
  if (only && name !== only) continue;
  const nodeDir = join(base, "node", name);
  const rustDir = join(base, "rust", name);
  console.log(`\n=== ${name} (${project})`);
  const synthetic = project.includes(join("parity", "fixtures"));
  copyProject(project, nodeDir, { git: synthetic && name === "wave2" });
  copyProject(project, rustDir, { git: synthetic && name === "wave2" });
  nodeBuild(nodeDir);
  nodeBuild(rustDir);
  const rootsN = [nodeDir];
  const rootsR = [rustDir];
  const brainDbBefore = existsSync(join(nodeDir, ".wara", "brain.db"));

  // 1. read
  const facts = readCalls(nodeDir);
  let t = Date.now();
  const n1 = run("node", facts.calls, nodeDir);
  const nodeMs = Date.now() - t;
  t = Date.now();
  const r1 = run("rust", facts.calls, rustDir);
  console.log(`  ${facts.calls.length} read calls; node ${nodeMs} ms, rust ${Date.now() - t} ms`);
  compare(name, "read: every function", facts.calls, n1, r1, rootsN, rootsR);
  check(name, "read: brain index created on both sides or neither", existsSync(join(nodeDir, ".wara", "brain.db")) === existsSync(join(rustDir, ".wara", "brain.db")) ? [] : ["brain.db differs"]);
  if (!brainDbBefore && !existsSync(join(nodeDir, "brain"))) {
    // findNotes/notesAbout/codeLinks/brainSummary never create an index for a project without brain/
    const calls = [c("brain.findNotes", R, "x"), c("brain.notesAbout", R, { path: "a.ts" }), c("brain.codeLinks", R), c("brain.brainSummary", R)];
    const probeN = join(base, "node", `${name}-nobrain`);
    const probeR = join(base, "rust", `${name}-nobrain`);
    for (const d of [probeN, probeR]) { rmSync(d, { recursive: true, force: true }); mkdirSync(d, { recursive: true }); }
    compare(name, "no brain/: answers", calls, run("node", calls, probeN), run("rust", calls, probeR), [probeN], [probeR]);
    check(name, "no brain/: no index made", [probeN, probeR].filter((d) => existsSync(join(d, ".wara", "brain.db"))).map((d) => `${d} has .wara/brain.db`));
  }

  // 2. write
  const wcalls = writeCalls(facts);
  compare(name, "write: answers", wcalls, run("node", wcalls, nodeDir), run("rust", wcalls, rustDir), rootsN, rootsR);
  const sn = textState(nodeDir, rootsN);
  const sr = textState(rustDir, rootsR);
  const fileProblems = [];
  for (const k of new Set([...Object.keys(sn), ...Object.keys(sr)])) {
    if (sn[k] === undefined || sr[k] === undefined) fileProblems.push(`${k}: only in ${sn[k] === undefined ? "rust" : "node"}`);
    else if (sn[k] !== sr[k]) fileProblems.push(`${k} differs\n      node: ${JSON.stringify(sn[k]).slice(0, 300)}\n      rust: ${JSON.stringify(sr[k]).slice(0, 300)}`);
  }
  check(name, `write: files (${Object.keys(sn).length}: brain/, docs/, .wara/ texts, wara.components.json, .gitignore)`, fileProblems);
  const dbProblems = [
    ...diffDumps(dumpDb(join(nodeDir, ".wara", "brain.db"), { stamps: "norm" }), dumpDb(join(rustDir, ".wara", "brain.db"), { stamps: "norm" }), "brain.db"),
    ...diffDumps(dumpDb(join(nodeDir, ".wara", "memory.db"), { stamps: "norm" }), dumpDb(join(rustDir, ".wara", "memory.db"), { stamps: "norm" }), "memory.db"),
    ...diffDumps(dumpDb(join(nodeDir, ".wara", "code-graph.db"), { tables: ["notes", "meta"], stamps: "norm" }), dumpDb(join(rustDir, ".wara", "code-graph.db"), { tables: ["notes", "meta"], stamps: "norm" }), "code-graph.db notes/meta"),
  ];
  check(name, "write: brain.db, memory.db, code-graph notes/meta (rows with rowids)", dbProblems);

  // 3. cross: each side reads the other's indexes, then syncs an identical edit
  const xcalls = crossCalls(facts);
  const dumps = (d) => ({ brain: dumpDb(join(d, ".wara", "brain.db")), memory: dumpDb(join(d, ".wara", "memory.db")) });
  const beforeN = dumps(nodeDir);
  const beforeR = dumps(rustDir);
  const rustOnNode = run("rust", xcalls, nodeDir);
  const nodeOnNode = run("node", xcalls, nodeDir);
  const nodeOnRust = run("node", xcalls, rustDir);
  const rustOnRust = run("rust", xcalls, rustDir);
  compare(name, "cross read: rust on node-written indexes = node on them", xcalls, nodeOnNode, rustOnNode, rootsN, rootsN);
  compare(name, "cross read: node on rust-written indexes = rust on them", xcalls, rustOnRust, nodeOnRust, rootsR, rootsR);
  const afterN = dumps(nodeDir);
  const afterR = dumps(rustDir);
  check(name, "cross read: indexes not rewritten (raw stamps and rowids unchanged)", [
    ...diffDumps(beforeN.brain, afterN.brain, "node copy brain.db"), ...diffDumps(beforeN.memory, afterN.memory, "node copy memory.db"),
    ...diffDumps(beforeR.brain, afterR.brain, "rust copy brain.db"), ...diffDumps(beforeR.memory, afterR.memory, "rust copy memory.db"),
  ]);
  crossEdit(nodeDir, facts);
  crossEdit(rustDir, facts);
  const cw = crossWrites();
  compare(name, "cross update: rust syncs the node copy, node syncs the rust copy", cw, run("node", cw, rustDir), run("rust", cw, nodeDir), rootsR, rootsN);
  check(name, "cross update: brain.db and memory.db equal after the cross syncs", [
    ...diffDumps(dumpDb(join(rustDir, ".wara", "brain.db"), { stamps: "norm" }), dumpDb(join(nodeDir, ".wara", "brain.db"), { stamps: "norm" }), "brain.db (node-synced vs rust-synced)"),
    ...diffDumps(dumpDb(join(rustDir, ".wara", "memory.db"), { stamps: "norm" }), dumpDb(join(nodeDir, ".wara", "memory.db"), { stamps: "norm" }), "memory.db (node-synced vs rust-synced)"),
  ]);

  if (!keep) {
    rmSync(join(base, "node", name), { recursive: true, force: true });
    rmSync(join(base, "rust", name), { recursive: true, force: true });
  }
}

console.log(`\n${summary.join("\n")}`);
console.log(failures ? `\n${failures} comparison(s) differ` : "\nall comparisons match");
if (keep) console.log(`copies kept in ${base}`);
process.exit(failures ? 1 : 0);
