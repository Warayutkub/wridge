// Lifecycle parity: start / stop / restart / port clashes / parent watch, for the Rust binary and for
// legacy-node/index.mjs, each on private ports; compares exit codes and the log lines (stdout + stderr).
//   node server/app/parity/lifecycle.mjs
// Every server started here stops on its own (its stop/restart route, a port clash, or the parent watch).
import { spawn } from "node:child_process";
import { request } from "node:http";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..", "..");
const RUST = join(HERE, "..", "target", "debug", process.platform === "win32" ? "wridge-server.exe" : "wridge-server");
const impls = {
  rust: { cmd: RUST, args: [], cwd: join(HERE, ".."), ws: 3265, http: 3266 },
  node: { cmd: process.execPath, args: ["index.mjs"], cwd: join(ROOT, "legacy-node"), ws: 3365, http: 3366 },
};

function start(impl, env = {}) {
  const p = spawn(impl.cmd, impl.args, { cwd: impl.cwd, env: { ...process.env, WARA_PORT: String(impl.ws), WARA_HTTP_PORT: String(impl.http), ...env }, windowsHide: true });
  const lines = [];
  const add = (stream) => (b) => lines.push(...b.toString().split(/\r?\n/).filter(Boolean).map((l) => `${stream} ${l}`));
  p.stdout.on("data", add("out"));
  p.stderr.on("data", add("err"));
  const exited = new Promise((resolve) => p.on("exit", (code) => resolve(code)));
  const ready = new Promise((resolve) => {
    const t = setInterval(() => { if (lines.some((l) => l.includes("plugin      ws://"))) { clearInterval(t); resolve(); } }, 50);
    exited.then(() => { clearInterval(t); resolve(); });
  });
  return { p, lines, exited, ready };
}

const post = (port, path, body = {}) => new Promise((resolve, reject) => {
  const req = request({ host: "127.0.0.1", port, method: "POST", path, headers: { host: `127.0.0.1:${port}`, "content-type": "application/json" } }, (res) => {
    let s = "";
    res.on("data", (c) => (s += c));
    res.on("end", () => resolve({ status: res.statusCode, body: s }));
  });
  req.on("error", reject);
  req.end(JSON.stringify(body));
});

const norm = (impl, lines) => lines.map((l) => l.replace(/\[\d\d:\d\d:\d\d\]/, "[T]").replace(/pid \d+/, "pid N").replaceAll(String(impl.http), "<http>").replaceAll(String(impl.ws), "<ws>"));
const timeout = (ms, v) => new Promise((r) => setTimeout(() => r(v), ms));

async function scenarios(name) {
  const impl = impls[name];
  const out = {};

  // stop from the dashboard
  let s = start(impl);
  await s.ready;
  out.stop = { reply: (await post(impl.http, "/api/stop")).body, code: await s.exited, log: norm(impl, s.lines) };

  // restart while run by the wridge app: exit code 75
  s = start(impl, { WRIDGE_MANAGED: "1" });
  await s.ready;
  out.managedRestart = { reply: (await post(impl.http, "/api/restart")).body, code: await s.exited, log: norm(impl, s.lines) };

  // a second server on busy ports gives up at once; with WARA_WAIT_FOR_PORTS it waits for them
  const first = start(impl);
  await first.ready;
  const clash = start(impl);
  out.clash = { code: await clash.exited, log: norm(impl, clash.lines) };
  const waiter = start(impl, { WARA_WAIT_FOR_PORTS: "1" });
  await timeout(1500);
  await post(impl.http, "/api/stop");
  await first.exited;
  await waiter.ready;
  out.waitForPorts = { log: norm(impl, waiter.lines) };
  await post(impl.http, "/api/stop");
  out.waitForPorts.code = await waiter.exited;

  // the wridge app goes away: the server stops itself
  const parent = spawn(process.execPath, ["-e", "setTimeout(() => {}, 1500)"]);
  s = start(impl, { WRIDGE_PARENT_PID: String(parent.pid) });
  await s.ready;
  out.parentGone = { code: await Promise.race([s.exited, timeout(15_000, "still running")]), log: norm(impl, s.lines) };
  return out;
}

const rust = await scenarios("rust");
const node = await scenarios("node");
let same = 0;
for (const k of Object.keys(rust)) {
  const a = JSON.stringify(rust[k]), b = JSON.stringify(node[k]);
  if (a === b) same++;
  else console.log(`DIFF ${k}\n  rust: ${a}\n  node: ${b}`);
}
console.log(JSON.stringify(rust, null, 1));
console.log(`lifecycle: ${same}/${Object.keys(rust).length} identical`);
process.exit(same === Object.keys(rust).length ? 0 : 1);
