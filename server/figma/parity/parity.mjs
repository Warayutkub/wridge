// Parity check: the same inputs through the legacy Node modules and through the Rust crate, results diffed.
//
//   node server/figma/parity/parity.mjs            all modules
//   node server/figma/parity/parity.mjs lottie pdf only these modules
//
// Writes parity/out/{cases,node,rust}.json; exit code 1 on any difference. JSON results must be identical
// text (JSON.stringify on the Node side, the crate's JS-exact stringify on the Rust side). Merged PDFs
// can't be byte-identical: both are loaded with pdf-lib and compared by page count, page sizes and title.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { lottieCases } from "./cases-lottie.mjs";
import { otherCases } from "./cases-other.mjs";
import { pathCases } from "./cases-path.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const crate = join(here, "..");
const repo = join(crate, "../..");
const out = join(here, "out");
mkdirSync(out, { recursive: true });
const only = process.argv.slice(2);

// mulberry32: the same "random" inputs on every run
let seed = 20261106;
const rng = () => {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const ctx = { here, repo, out, rng };
let cases = [...lottieCases(ctx), ...(await otherCases(ctx)), ...pathCases(ctx)];
if (only.length) cases = cases.filter((c) => only.includes(c.module));
cases.forEach((c, i) => (c.id = `${String(i + 1).padStart(3, "0")} ${c.module}.${c.fn}`));
const casesFile = join(out, "cases.json");
writeFileSync(casesFile, JSON.stringify(cases));

const nodeFile = join(out, "node.json"), rustFile = join(out, "rust.json");
execFileSync(process.execPath, [join(here, "node-run.mjs"), casesFile, nodeFile], { stdio: "inherit" });
const cargo = process.env.CARGO ?? join(process.env.USERPROFILE ?? process.env.HOME, ".cargo/bin/cargo");
execFileSync(cargo, ["run", "--quiet", "--release", "--manifest-path", join(crate, "Cargo.toml"), "--example", "parity", "--", casesFile, rustFile], { stdio: "inherit" });

// Every JSON number and template string goes through Number → string: 300k doubles of all magnitudes,
// written by Node, must come out the same from the crate (and parse back to the same bits).
let numbersOk = true;
if (!only.length) {
  const buf = new Float64Array(1), bits = new BigUint64Array(buf.buffer);
  const lines = [];
  for (let i = 0; i < 300_000; i++) {
    const k = i % 7;
    buf[0] = k === 0 ? rng() * 1000 : k === 1 ? (rng() - 0.5) * 10 ** Math.floor(rng() * 44 - 22) : k === 2 ? Math.round(rng() * 1e6) / 1000
      : k === 3 ? rng() * 1e-6 : k === 4 ? (rng() - 0.5) * 1e22 : k === 5 ? 1 / (1 + Math.floor(rng() * 1000)) : Math.round(rng() * 2 ** 53) * 2 ** Math.floor(rng() * 20);
    lines.push(`${bits[0]} ${String(buf[0])}`);
  }
  const numFile = join(out, "numbers.txt");
  writeFileSync(numFile, lines.join("\n"));
  try { execFileSync(cargo, ["run", "--quiet", "--release", "--manifest-path", join(crate, "Cargo.toml"), "--example", "parity", "--", "--numbers", numFile], { stdio: "inherit" }); } catch { numbersOk = false; }
}

const node = JSON.parse(readFileSync(nodeFile, "utf8"));
const rust = new Map(JSON.parse(readFileSync(rustFile, "utf8")).map((r) => [r.id, r]));
const { PDFDocument } = await import(pathToFileURL(join(repo, "legacy-node/node_modules/pdf-lib/cjs/index.js")).href);

async function pdfFacts(b64) {
  const d = await PDFDocument.load(Buffer.from(b64, "base64"), { updateMetadata: false });
  return { pages: d.getPageCount(), sizes: d.getPages().map((p) => [p.getWidth(), p.getHeight()]), title: d.getTitle() ?? null, producer: d.getProducer() ?? null };
}
// PDF results: replace the bytes by what can be compared
async function comparable(c, r) {
  // sentAt is the clock of the run that wrote it
  if (r?.ok && c.module === "motion" && c.fn === "listLottie" && c.args[0] === "<copy>") return { ...r, ok: r.ok.replace(/"sentAt":"[^"]*"/g, "\"sentAt\":\"<now>\"") };
  if (!r?.ok || c.module !== "pdf" || !["mergePdfs", "designPdf"].includes(c.fn)) return r;
  const v = JSON.parse(r.ok);
  const facts = await pdfFacts(v.base64);
  delete v.base64;
  return { ...r, ok: JSON.stringify({ ...v, pdf: facts }) };
}

let same = 0;
const diffs = [];
for (const n of node) {
  const c = cases.find((x) => x.id === n.id);
  const a = await comparable(c, n), b = await comparable(c, rust.get(n.id));
  if (b && a.ok === b.ok && a.error === b.error) { same++; continue; }
  diffs.push({ id: n.id, node: a, rust: b ?? "(missing)" });
}
const firstDiff = (x, y) => { let i = 0; while (i < x.length && x[i] === y[i]) i++; return i; };
for (const d of diffs) {
  console.log(`\n✗ ${d.id}`);
  if (d.node.error || d.rust.error) { console.log(`  node: ${d.node.error ?? "ok"}\n  rust: ${d.rust.error ?? "ok"}`); continue; }
  const x = d.node.ok ?? "", y = d.rust.ok ?? "";
  const i = firstDiff(x, y);
  console.log(`  first difference at char ${i}:\n  node: …${x.slice(Math.max(0, i - 80), i + 80)}\n  rust: …${y.slice(Math.max(0, i - 80), i + 80)}`);
}
const byModule = {};
for (const c of cases) byModule[c.module] = (byModule[c.module] ?? 0) + 1;
console.log(`\nparity: ${same}/${node.length} identical (${Object.entries(byModule).map(([m, n]) => `${m} ${n}`).join(", ")})${diffs.length ? ` — ${diffs.length} different` : ""}`);
process.exit(diffs.length || !numbersOk ? 1 : 0);
