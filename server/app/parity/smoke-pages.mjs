// Smoke test of the release build: every page of the window loaded straight from the Rust server in headless
// Chrome, per project of the user's list, with console errors, page errors and failed /api requests collected.
//   cargo build --release (in server/app), then: node server/app/parity/smoke-pages.mjs [--keep]
// The projects list is a temp copy of legacy-node/code-projects.json (read only) whose folders point at temp
// copies of the projects (.wara included, node_modules/build/… skipped), so opening a page — which refreshes
// .wara/ — never writes into a real project. Server: 3255/3256 (a fake plugin on 3255 for /graph and /pdf),
// stopped through /api/stop. The window itself (/app) talks to :3156 (hard-coded), so its pages are loaded
// directly: /code?dir=…, /data, /flows, /motion, /brain, plus /, /skills, /graph, /pdf.
import { spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { request } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { startFakePlugin } from "./fake-plugin.mjs";
import { REAL_PROJECTS_FILE, assertNotRealProjects } from "./private-node.mjs";

const require = createRequire(import.meta.url);
// Playwright from PLAYWRIGHT_PATH (e.g. a global install), else a local one
const { chromium } = require(process.env.PLAYWRIGHT_PATH ?? "playwright");
const CHROME = "C:/Program Files/Google/Chrome/Application/chrome.exe";
const HERE = dirname(fileURLToPath(import.meta.url));
const RELEASE = join(HERE, "..", "target", "release", process.platform === "win32" ? "wridge-server.exe" : "wridge-server");
const [WS, HTTP] = [3255, 3256];
const keep = process.argv.includes("--keep");
const SKIP = new Set(["node_modules", "vendor", ".git", "build", "dist", ".next", ".dart_tool", ".turbo", ".vercel", "coverage", "target", "Pods", ".gradle", ".idea", "__pycache__", ".venv"]);

const TMP = mkdtempSync(join(tmpdir(), "wridge-smoke-"));
const real = JSON.parse(readFileSync(REAL_PROJECTS_FILE, "utf8"));
const list = real.map((p) => {
  const copy = join(TMP, "projects", basename(p.dir));
  cpSync(p.dir, copy, { recursive: true, preserveTimestamps: true, filter: (src) => !SKIP.has(basename(src)) });
  return { ...p, dir: copy.replaceAll("\\", "/") };
});
const projectsFile = join(TMP, "code-projects.json");
assertNotRealProjects(projectsFile);
writeFileSync(projectsFile, `${JSON.stringify(list, null, 2)}\n`);

const server = spawn(RELEASE, [], {
  cwd: join(HERE, "..", "..", "..", "legacy-node"),
  env: { ...process.env, WARA_PORT: String(WS), WARA_HTTP_PORT: String(HTTP), WRIDGE_PROJECTS_FILE: projectsFile },
  windowsHide: true,
});
let log = "";
server.stdout.on("data", (b) => (log += b));
server.stderr.on("data", (b) => (log += b));
await new Promise((resolve, reject) => {
  const t = setInterval(() => { if (log.includes("plugin      ws://")) { clearInterval(t); resolve(); } }, 50);
  server.on("exit", (code) => { clearInterval(t); reject(new Error(`server exited (${code}): ${log}`)); });
});
const plugin = await startFakePlugin(WS, { fileName: "utils" });

const pages = [["/", "dashboard"], ["/skills", "skills"], ["/graph", "design map (fake plugin)"], ["/pdf", "pdf (fake plugin)"]];
for (const p of list) for (const page of ["code", "data", "flows", "motion", "brain"]) pages.push([`/${page}?dir=${encodeURIComponent(p.dir)}`, `${page} · ${basename(p.dir)}`]);

const browser = await chromium.launch({ executablePath: CHROME, headless: true });
const rows = [];
let failed = 0;
try {
  for (const [path, label] of pages) {
    const page = await browser.newPage({ viewport: { width: 1280, height: 860 } });
    const errors = [];
    const api = [];
    page.on("console", (m) => {
      const at = m.location()?.url ?? "";
      if (m.type() === "error" && !at.endsWith("/favicon.ico")) errors.push(`console: ${m.text()} (${at})`);
    });
    page.on("pageerror", (e) => errors.push(`page: ${e.message}`));
    page.on("requestfailed", (r) => errors.push(`request failed: ${r.url()} ${r.failure()?.errorText}`));
    page.on("response", (r) => {
      const u = new URL(r.url());
      if (u.pathname.startsWith("/api/")) api.push(`${r.status()} ${u.pathname}`);
      // the browser asks for /favicon.ico on its own (the pages have no icon; Node answers the same 404)
      else if (r.status() >= 400 && u.pathname !== "/favicon.ico") errors.push(`${r.status()} ${u.pathname}`);
    });
    const started = Date.now();
    await page.goto(`http://127.0.0.1:${HTTP}${path}`, { waitUntil: "networkidle", timeout: 180_000 });
    await page.waitForTimeout(1500);
    const text = (await page.evaluate(() => document.body.innerText)).replace(/\s+/g, " ").trim();
    mkdirSync(join(TMP, "shots"), { recursive: true });
    await page.screenshot({ path: join(TMP, "shots", `${rows.length.toString().padStart(2, "0")}-${label.replace(/[^\w]+/g, "-")}.png`) });
    const bad = api.filter((a) => !a.startsWith("200 "));
    if (errors.length || bad.length) failed++;
    rows.push({ label, ms: Date.now() - started, api, errors, text: text.slice(0, 160) });
    await page.close();
  }
} finally {
  await browser.close();
  await plugin.close();
  await new Promise((resolve) => {
    const req = request({ host: "127.0.0.1", port: HTTP, method: "POST", path: "/api/stop", headers: { host: `127.0.0.1:${HTTP}` } }, (res) => { res.resume(); res.on("end", resolve); });
    req.on("error", resolve);
    req.end();
  });
  await new Promise((r) => (server.exitCode === null ? server.on("exit", r) : r()));
}

for (const r of rows) {
  console.log(`${r.errors.length || r.api.some((a) => !a.startsWith("200 ")) ? "✗" : "✓"} ${r.label} (${r.ms} ms) api: ${[...new Set(r.api)].join(", ") || "—"}`);
  for (const e of r.errors) console.log(`    ${e.slice(0, 300)}`);
  console.log(`    text: ${r.text}`);
}
console.log(`\nsmoke: ${rows.length - failed}/${rows.length} pages without console errors or failed API calls (release build :${HTTP}, ${list.length} projects from a temp copy of the list)`);
if (keep) console.log(`kept ${TMP} (screenshots in shots/)`);
else rmSync(TMP, { recursive: true, force: true });
if (!existsSync(REAL_PROJECTS_FILE)) console.log("the real projects list is missing!");
process.exit(failed ? 1 : 0);
