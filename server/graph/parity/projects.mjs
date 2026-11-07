// Parity of projects.mjs (the code-projects.json list) — on temp copies only.
// The Node module writes next to itself, so it is copied into a temp folder first.
// usage: node server/graph/parity/projects.mjs [--no-build]
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CRATE = join(HERE, "..");
const LEGACY = join(CRATE, "..", "..", "legacy-node");
if (!process.argv.includes("--no-build")) {
  execFileSync(process.env.CARGO ?? join(process.env.USERPROFILE ?? process.env.HOME, ".cargo", "bin", "cargo"), ["build", "--release", "--example", "projects"], { cwd: CRATE, stdio: "inherit" });
}
const EXE = join(CRATE, "target", "release", "examples", process.platform === "win32" ? "projects.exe" : "projects");

const base = join(tmpdir(), "wridge-graph-parity-projects");
rmSync(base, { recursive: true, force: true });
mkdirSync(join(base, "node", "codegraph"), { recursive: true });
mkdirSync(join(base, "rust"), { recursive: true });
copyFileSync(join(LEGACY, "codegraph", "projects.mjs"), join(base, "node", "codegraph", "projects.mjs"));
copyFileSync(join(LEGACY, "code-projects.json"), join(base, "node", "code-projects.json"));
copyFileSync(join(LEGACY, "code-projects.json"), join(base, "rust", "code-projects.json"));
const node = await import(pathToFileURL(join(base, "node", "codegraph", "projects.mjs")).href);
const rustFile = join(base, "rust", "code-projects.json");
const rust = (op, dir) => {
  const r = spawnSync(EXE, [rustFile, op, ...(dir === undefined ? [] : [dir])], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(r.stderr);
  return JSON.parse(r.stdout);
};

const steps = [
  ["known", "c:\\users\\someone\\desktop\\APP"],
  ["add", "C:/Temp/Some Project/"],
  ["add", "C:\\Temp\\some project"],
  ["add", "C:/Temp/a/../b/./c"],
  ["known", "C:/Temp/b/c/"],
  ["remove", "C:/USERS/someone/Desktop/WebApp"],
  ["known", "C:/Users/someone/Desktop/WebApp"],
  ["list"],
];
const norm = (s) => s.replace(/"addedAt":\s*"[^"]+"/g, '"addedAt":"<ts>"');
let fail = 0;
for (const [op, dir] of steps) {
  const fn = { add: node.addProject, remove: node.removeProject, known: node.isKnownProject, list: node.listProjects }[op];
  const n = norm(JSON.stringify(fn(dir) ?? null));
  const r = norm(JSON.stringify(rust(op, dir)));
  const ok = n === r;
  if (!ok) fail++;
  console.log(`${ok ? "✓" : "✗"} ${op} ${dir ?? ""}${ok ? "" : `\n  node: ${n}\n  rust: ${r}`}`);
}
const nf = norm(readFileSync(join(base, "node", "code-projects.json"), "utf8"));
const rf = norm(readFileSync(rustFile, "utf8"));
console.log(`${nf === rf ? "✓" : "✗"} file bytes (addedAt normalised)`);
if (nf !== rf) fail++;
rmSync(base, { recursive: true, force: true });
process.exit(fail ? 1 : 0);
