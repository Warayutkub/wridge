// Function-level parity (read-only, safe on the real project folders): runs every wave-1 scanner
// on each graph file — parseTs, scanStyles, scanData (all access styles), sqlLiterals + parseDml,
// scanDartData (sqflite), dartSqlLiterals, applySql (.sql), applyPrisma (.prisma) — in Node and in
// Rust (examples/scan_dump.rs), and diffs the results file by file.
//
// usage: node server/graph/parity/scan.mjs [--no-build] [project dir …]   (default: the five test projects + fixtures/edge-cases)
import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { applyPrisma } from "../../../legacy-node/codegraph/data/prisma.mjs";
import { applySql, emptyModel } from "../../../legacy-node/codegraph/data/sql-ddl.mjs";
import { dartSqlLiterals, parseDml, sqlLiterals } from "../../../legacy-node/codegraph/data/sql-usage.mjs";
import { scanDartData, scanData } from "../../../legacy-node/codegraph/data/usage.mjs";
import { listFiles } from "../../../legacy-node/codegraph/files.mjs";
import { parseTs } from "../../../legacy-node/codegraph/parse-ts.mjs";
import { scanStyles } from "../../../legacy-node/codegraph/styles.mjs";
import { PROJECTS } from "../../parity-projects.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CRATE = join(HERE, "..");
const args = process.argv.slice(2);
const dirs = args.filter((a) => !a.startsWith("--"));
const projects = dirs.length
  ? dirs
  : [...PROJECTS, join(HERE, "fixtures", "edge-cases")];

const EXE = join(CRATE, "target", "release", "examples", process.platform === "win32" ? "scan_dump.exe" : "scan_dump");
if (!args.includes("--no-build")) {
  execFileSync(process.env.CARGO ?? join(process.env.USERPROFILE ?? process.env.HOME, ".cargo", "bin", "cargo"), ["build", "--release", "--example", "scan_dump"], { cwd: CRATE, stdio: "inherit" });
}

// Canonical form: object keys sorted, null/undefined dropped (the DB stores both as NULL), and
// strings well-formed: a JS string cut inside a surrogate pair (.slice(0, 80) through an emoji) holds a
// lone surrogate, which a Rust String cannot; both sides store it in SQLite as U+FFFD (known difference:
// only JSON.stringify of such a string differs, "\ud83c" vs "�").
const canon = (v) => {
  if (Array.isArray(v)) return v.map(canon);
  if (v && typeof v === "object") {
    const o = {};
    for (const k of Object.keys(v).sort()) if (v[k] !== null && v[k] !== undefined) o[k] = canon(v[k]);
    return o;
  }
  return typeof v === "string" ? v.toWellFormed() : v;
};
const wellFormedJson = (s) => (typeof s === "string" ? JSON.stringify(JSON.parse(s, (k, v) => (typeof v === "string" ? v.toWellFormed() : v))) : s);
const lits = (l) => l.map(({ sql, line, ddl, lead }) => ({ sql, line, ddl, lead }));
const tryOr = async (fn) => {
  try {
    return await fn();
  } catch (err) {
    return { error: String(err?.message ?? err) };
  }
};

async function nodeDump(root) {
  const all = ["supabase", "sql", "prisma"];
  const out = [];
  for (const f of listFiles(root)) {
    let text;
    try {
      text = readFileSync(join(root, f.path), "utf8");
    } catch {
      continue;
    }
    const o = { path: f.path };
    if (f.lang === "ts" || /\.(css|scss|sass|less|html|vue|svelte|astro)$/.test(f.path)) o.styles = scanStyles(text, f.path);
    if (f.lang === "ts") {
      o.ts = await tryOr(() => parseTs(text, extname(f.path)));
      o.data = await tryOr(() => scanData(text, all));
      const l = sqlLiterals(text);
      o.sqlLits = lits(l);
      o.dml = l.filter((x) => !x.ddl).map((x) => parseDml(x.sql));
    }
    if (f.lang === "dart") {
      o.data = scanDartData(text, ["sqflite"]);
      o.dartLits = lits(dartSqlLiterals(text));
    }
    if (f.lang === "sql") {
      const m = emptyModel();
      applySql(m, text, f.path);
      o.model = JSON.stringify(m);
    }
    if (f.lang === "prisma") {
      const m = emptyModel();
      applyPrisma(m, text, f.path);
      o.model = JSON.stringify(m);
    }
    out.push(o);
  }
  return out;
}

let failures = 0;
for (const root of projects) {
  const node = await nodeDump(root);
  const r = spawnSync(EXE, [root], { encoding: "utf8", maxBuffer: 512 * 1024 * 1024 });
  if (r.status !== 0) throw new Error(`scan_dump failed: ${r.stderr}`);
  const rust = r.stdout.split("\n").filter(Boolean).map((l) => JSON.parse(l));
  const byPath = new Map(rust.map((x) => [x.path, x]));
  let differ = 0;
  let items = 0;
  for (const n of node) {
    const x = byPath.get(n.path);
    for (const k of Object.keys(n)) {
      if (k === "path") continue;
      items++;
      const a = JSON.stringify(k === "model" ? wellFormedJson(n[k]) : canon(n[k]));
      const b = JSON.stringify(k === "model" ? x?.[k] : canon(x?.[k]));
      if (a !== b) {
        differ++;
        if (differ <= 8) console.log(`  ${n.path} [${k}]\n    node: ${a.slice(0, 600)}\n    rust: ${b.slice(0, 600)}`);
      }
    }
  }
  if (rust.length !== node.length) {
    differ++;
    console.log(`  file count: node ${node.length}, rust ${rust.length}`);
  }
  failures += differ;
  console.log(`${differ ? "✗" : "✓"} ${root}: ${node.length} files, ${items} results compared, ${differ} differ`);
}
console.log(failures ? `\n${failures} difference(s)` : "\nall scanner results match");
process.exit(failures ? 1 : 0);
