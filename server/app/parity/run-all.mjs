// Every parity check in one go:
//   cargo build (in server/app), then: node server/app/parity/run-all.mjs [--slow]
// Starts the Rust server on 3255/3256 and a private Node server (legacy-node/index.mjs, run from a temp copy:
// private-node.mjs) on 3355/3356, each with its own temp projects list (never the real
// legacy-node/code-projects.json), runs the plugin round trip, the wridge-figma wiring (tools and routes with
// real-looking plugin answers, files written into temp folders), the wridge-graph wiring (code graph, brain,
// memory, knowledge, flows, projects routes/tools/CLI on temp copies of the test projects), the read-only
// parity against the user's Node server (3156, if it is up) and against the private one, the CLI and
// WebSocket-guard checks, stops both through /api/stop, then runs the lifecycle checks (their own ports).
// Nothing touches 3155 or the user's Figma.
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { makePrivateNode } from "./private-node.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const RUST = join(HERE, "..", "target", "debug", process.platform === "win32" ? "wridge-server.exe" : "wridge-server");
const slow = process.argv.includes("--slow");

function start(cmd, args, cwd, ws, http, env = {}) {
  const p = spawn(cmd, args, { cwd, env: { ...process.env, WARA_PORT: String(ws), WARA_HTTP_PORT: String(http), ...env }, windowsHide: true });
  let out = "";
  p.stdout.on("data", (b) => (out += b));
  p.stderr.on("data", (b) => (out += b));
  return new Promise((resolve, reject) => {
    const t = setInterval(() => { if (out.includes("plugin      ws://")) { clearInterval(t); resolve(p); } }, 50);
    p.on("exit", (code) => { clearInterval(t); reject(new Error(`${cmd} exited (${code}): ${out}`)); });
  });
}

const up = (port) => new Promise((resolve) => {
  const req = request({ host: "127.0.0.1", port, path: "/api/status", headers: { host: `127.0.0.1:${port}` } }, (res) => { res.resume(); resolve(true); });
  req.on("error", () => resolve(false));
  req.end();
});
const stop = (port) => new Promise((resolve) => {
  const req = request({ host: "127.0.0.1", port, method: "POST", path: "/api/stop", headers: { host: `127.0.0.1:${port}` } }, (res) => { res.resume(); res.on("end", resolve); });
  req.on("error", resolve);
  req.end();
});

function run(label, script, args, env = {}) {
  const r = spawnSync(process.execPath, [join(HERE, script), ...args.map(String)], { encoding: "utf8", env: { ...process.env, ...env } });
  const lines = (r.stdout + r.stderr).trim().split("\n");
  console.log(`\n## ${label}\n${lines.filter((l) => /DIFF|identical|Error|THREW/.test(l) || lines.length < 4).join("\n")}`);
  return r.status === 0;
}

const TMP = mkdtempSync(join(tmpdir(), "wridge-run-all-"));
const rustProjects = join(TMP, "rust-projects.json");
writeFileSync(rustProjects, "[]\n");
const privateNode = makePrivateNode();
const rust = await start(RUST, [], join(HERE, ".."), 3255, 3256, { WRIDGE_PROJECTS_FILE: rustProjects });
const node = await start(process.execPath, ["index.mjs"], privateNode.dir, 3355, 3356);
const results = [];
results.push(run("plugin round trip (Rust vs private Node)", "plugin-roundtrip.mjs", [3255, 3256, 3355, 3356], slow ? { PARITY_SLOW: "1" } : {}));
results.push(run("wridge-figma wiring (Rust vs private Node)", "figma-wiring.mjs", [3255, 3256, 3355, 3356]));
results.push(run("wridge-graph wiring (Rust vs private Node)", "graph-wiring.mjs", [3255, 3256, 3355, 3356], {
  GW_RUST_PROJECTS: rustProjects,
  GW_NODE_PROJECTS: privateNode.projectsFile,
  GW_NODE_LEGACY: privateNode.dir,
}));
if (await up(3156)) results.push(run("read-only parity vs the running Node server :3156", "parity.mjs", [3156, 3256]));
else console.log("\n## :3156 is not running — skipped the parity run against it");
results.push(run("read-only parity vs the private Node server :3356", "parity.mjs", [3356, 3256]));
results.push(run("CLI (wara.mjs vs wridge)", "cli.mjs", [3356, 3256]));
results.push(run("plugin WebSocket origin guard", "ws-guard.mjs", [3255, 3355]));
await stop(3256);
await stop(3356);
await Promise.all([new Promise((r) => rust.on("exit", r)), new Promise((r) => node.on("exit", r))]);
privateNode.cleanup();
rmSync(TMP, { recursive: true, force: true });
results.push(run("lifecycle (stop, managed restart, port clash, wait for ports, parent gone)", "lifecycle.mjs", []));
console.log(`\n${results.filter(Boolean).length}/${results.length} checks passed`);
process.exit(results.every(Boolean) ? 0 : 1);
