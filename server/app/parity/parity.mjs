// Parity: the same read-only requests to the Node server and the Rust server, answers diffed.
//   node server/app/parity/parity.mjs [nodePort=3156] [rustPort=3256]
// Read-only for Node: GET routes, MCP initialize/list/get, and tool calls that only read (get_skill,
// describe_tool, unknown tools, validation errors). It never pauses, stops or restarts a server, and no
// plugin is involved (see plugin-roundtrip.mjs for that). pid, start time, ports, cliPath and the command
// history (other clients use the Node server too) are normalised before comparing.
import { createHash } from "node:crypto";
import { request } from "node:http";

const [nodePort = 3156, rustPort = 3256] = process.argv.slice(2).map(Number);
if (rustPort === 3155 || rustPort === 3156 || rustPort === 3055 || rustPort === 3056) throw new Error("the Rust server must not use 3155/3156/3055/3056");

function http(port, method, path, { headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const req = request({ host: "127.0.0.1", port, method, path, headers: { host: `127.0.0.1:${port}`, ...headers } }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) }));
    });
    req.on("error", reject);
    req.setTimeout(20_000, () => req.destroy(new Error(`timeout ${method} ${path}`)));
    if (body !== undefined) req.write(typeof body === "string" ? body : JSON.stringify(body));
    req.end();
  });
}

// GET /mcp keeps an SSE stream open: read the head only.
function head(port, method, path, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = request({ host: "127.0.0.1", port, method, path, headers: { host: `127.0.0.1:${port}`, ...headers } }, (res) => {
      resolve({ status: res.statusCode, headers: res.headers, body: Buffer.alloc(0) });
      req.destroy();
    });
    req.on("error", (e) => (e.code === "ECONNRESET" ? null : reject(e)));
    req.end();
  });
}

const port = (p) => (p === "node" ? nodePort : rustPort);

function normalise(value, label) {
  if (label.includes("/api/status") && value && typeof value === "object") {
    for (const k of ["pid", "startedAt", "cliPath", "ports", "history"]) value[k] = `<${k}>`;
    // the private Node server runs from a temp copy of legacy-node (private-node.mjs): compare the file name only
    if (typeof value.guidePath === "string") value.guidePath = value.guidePath.replace(/^.*\//, "<root>/");
    value.today = { ...value.today, count: "<n>", errors: "<n>", totalMs: "<n>" };
  }
  return value;
}

function decode(res, label) {
  const type = String(res.headers["content-type"] ?? "");
  const text = res.body.toString("utf8");
  let body;
  if (type.startsWith("application/json")) {
    try { body = JSON.parse(text); } catch { body = text; }
  } else if (type.startsWith("text/event-stream")) {
    body = text.split("\n").filter((l) => l.startsWith("data: ")).map((l) => JSON.parse(l.slice(6)));
  } else if (type.startsWith("image/") || /javascript/.test(type) && text.length > 100_000) {
    body = `sha256:${createHash("sha256").update(res.body).digest("hex")} (${res.body.length} bytes)`;
  } else {
    body = text;
  }
  const keep = ["content-type", "cache-control", "access-control-allow-origin", "vary", "access-control-allow-methods", "access-control-allow-headers", "access-control-max-age", "allow", "content-disposition"];
  const headers = Object.fromEntries(keep.filter((h) => res.headers[h] !== undefined).map((h) => [h, String(res.headers[h]).replace(/:\d{4}$/, ":<port>")]));
  return { status: res.status, headers, body: normalise(body, label) };
}

function firstDiff(a, b, path = "") {
  if (JSON.stringify(a) === JSON.stringify(b)) return null;
  if (a && b && typeof a === "object" && typeof b === "object") {
    const ka = Object.keys(a), kb = Object.keys(b);
    if (JSON.stringify(ka) !== JSON.stringify(kb)) return `${path || "/"}: keys ${JSON.stringify(ka).slice(0, 300)} vs ${JSON.stringify(kb).slice(0, 300)}`;
    for (const k of ka) {
      const d = firstDiff(a[k], b[k], `${path}/${k}`);
      if (d) return d;
    }
  }
  const s = (v) => (typeof v === "string" ? v : JSON.stringify(v));
  const x = s(a), y = s(b);
  let i = 0;
  while (i < x.length && x[i] === y[i]) i++;
  return `${path || "/"}: …${x.slice(Math.max(0, i - 60), i + 120)}…  vs  …${y.slice(Math.max(0, i - 60), i + 120)}…`;
}

const cases = [];
const add = (label, fn) => cases.push({ label, fn });
const get = (path, headers) => add(`GET ${path}${headers ? ` ${JSON.stringify(headers)}` : ""}`, (p) => http(port(p), "GET", path, { headers }));
const post = (path, body, headers) => add(`POST ${path} ${JSON.stringify(body).slice(0, 80)}`, (p) => http(port(p), "POST", path, { body, headers: { "content-type": "application/json", ...headers } }));
const MCP_HEADERS = { "content-type": "application/json", accept: "application/json, text/event-stream" };
const mcp = (profile, message, label = message.method, headers = {}) =>
  add(`MCP ${profile ?? "(none)"} ${label}`, (p) => http(port(p), "POST", profile ? `/mcp?profile=${profile}` : "/mcp", { body: message, headers: { ...MCP_HEADERS, ...headers } }));

// ---- pages, static files, guards ----
for (const page of ["/", "/graph", "/ui.css", "/ui.js", "/code", "/data", "/flows", "/skills", "/motion", "/pdf", "/brain", "/app", "/app.css", "/app.js", "/app-icons.js", "/mindmap.js", "/graph3d.js", "/search.js", "/guide"]) get(page);
for (const v of ["3d-force-graph.min.js", "force-graph.min.js", "three.module.js", "three.core.js", "lottie.min.js", "gifenc.esm.js", "nope.js"]) get(`/vendor/${v}`);
get("/lottie/loading.json");
get("/lottie/nope.json");
get("/lottie/UPPER.json");
get("/stickers/happy.png");
get("/stickers/nope.png");
get("/nope");
get("/nope", { accept: "text/html" });
get("/api/nope", { accept: "text/html" });
get("/a/../graph");
add("GET /api/status (forbidden host)", (p) => http(port(p), "GET", "/api/status", { headers: { host: "evil.example" } }));
add("GET /api/status (localhost host)", (p) => http(port(p), "GET", "/api/status", { headers: { host: `localhost:${port(p)}` } }));
get("/api/status", { origin: "http://evil.example" });
add("GET /api/status (tauri origin)", (p) => http(port(p), "GET", "/api/status", { headers: { origin: "http://tauri.localhost" } }));
add("GET /api/status (own origin)", (p) => http(port(p), "GET", "/api/status", { headers: { origin: `http://localhost:${port(p)}` } }));
add("OPTIONS /api/tool (preflight)", (p) => http(port(p), "OPTIONS", "/api/tool", { headers: { origin: "tauri://localhost" } }));
add("OPTIONS /anything", (p) => http(port(p), "OPTIONS", "/anything"));

// ---- API ----
get("/api/status");
get("/api/tools");
get("/api/skills");
for (const r of ["/api/code/graph", "/api/motion", "/api/motion/file", "/api/flows", "/api/flows/trace", "/api/code/data", "/api/brain", "/api/brain/code-links", "/api/brain/note"]) get(r);
// plugin not connected on either server: 503s and 400s
for (const r of ["/api/motion/figma-frames", "/api/motion/figma-preview", "/api/pdf/sections", "/api/pdf/download", "/api/thumb", "/api/outline", "/api/graph"]) get(r);
post("/api/flows/ignore", {});
post("/api/code/link", {});
post("/api/brain/note", {});
post("/api/motion/figma", {});
post("/api/code/projects/remove", null);
post("/api/tool", {});
post("/api/tool", { name: "nope" });
post("/api/tool", { name: "create_frame", args: { width: -1, fill: "red", layoutMode: "X" } });
post("/api/tool", { name: "get_node", args: {} });
post("/api/tool", { name: "create_text", args: { text: "a", lineHeight: true, letterSpacing: "x%" } });
post("/api/tool", { name: "set_effects", args: { nodeId: "1:1", shadows: [{ color: 5 }, {}, { blur: -1 }] } });
post("/api/tool", { name: "get_design_context", args: { breakpoints: { base: "1:1", huge: "2:2" } } });
post("/api/tool", { name: "add_interaction", args: { nodeId: "1:1", url: "not a url" } });
post("/api/tool", { name: "motion_animate", args: { nodeId: "1:1", tracks: [{ prop: "spin", ease: [1, 2] }] } });
post("/api/tool", { name: "get_skill", args: {} });
post("/api/tool", { name: "get_skill", args: { name: "motion" } });
post("/api/tool", { name: "get_skill", args: { name: "nope" } });
post("/api/tool", { name: "mcp_wara-bridge_get_skill", args: { name: "ui-visual" } });
post("/api/tool", { name: "wara_bridge:describe_tool", args: {} });
post("/api/tool", { name: "describe_tool", args: { name: "create_frame" } });
post("/api/tool", { name: "describe_tool", args: { name: "run_batch" } });
post("/api/tool", { name: "describe_tool", args: { name: "nope" } });
post("/api/tool", { name: "call_tool", args: { name: "nope" } });
post("/api/tool", { name: "call_tool", args: { name: "get_skill" } });
post("/api/tool", { name: "call_tool", args: { name: "get_node", args: { depth: 9 } } });
post("/api/tool", { name: "use_file", args: { file: "figjam" } });
post("/api/tool", { name: "run_batch", args: { steps: [{ tool: "nope" }, { tool: "get_skill" }] } });
post("/api/tool", { name: "run_batch", args: { steps: [{ tool: "get_node", args: { nodeId: "$x" } }], stopOnError: false } });
post("/api/tool", { name: "run_batch", args: { steps: [] } });
add("POST /api/tool (bad JSON)", (p) => http(port(p), "POST", "/api/tool", { body: "{nope", headers: { "content-type": "application/json" } }));
for (const bad of ["{\"a\":1", "[1,]", "{\"name\": nope, \"args\": {}}", "undefined", "1 2", "null"]) add(`POST /api/tool (bad JSON ${bad})`, (p) => http(port(p), "POST", "/api/tool", { body: bad, headers: { "content-type": "application/json" } }));
add("POST /api/tool (empty body)", (p) => http(port(p), "POST", "/api/tool", { body: "", headers: { "content-type": "application/json" } }));

// ---- MCP ----
const init = (v = "2025-06-18") => ({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: v, capabilities: {}, clientInfo: { name: "parity", version: "1" } } });
for (const profile of [null, "core", "code", "figjam", "prototype", "design", "all", "bogus"]) {
  mcp(profile, init());
  mcp(profile, { jsonrpc: "2.0", id: 2, method: "tools/list" });
}
mcp("core", init("1999-01-01"), "initialize unknown version");
mcp("core", { jsonrpc: "2.0", id: 3, method: "initialize", params: { protocolVersion: "2025-06-18" } }, "initialize missing fields");
mcp("core", { jsonrpc: "2.0", id: 4, method: "prompts/list" });
mcp("core", { jsonrpc: "2.0", id: 5, method: "prompts/get", params: { name: "motion", arguments: { task: "animate the hero" } } });
mcp("core", { jsonrpc: "2.0", id: 6, method: "prompts/get", params: { name: "motion", arguments: {} } }, "prompts/get no task");
mcp("core", { jsonrpc: "2.0", id: 7, method: "prompts/get", params: { name: "motion" } }, "prompts/get no arguments");
mcp("core", { jsonrpc: "2.0", id: 8, method: "prompts/get", params: { name: "zzz" } }, "prompts/get unknown");
mcp("core", { jsonrpc: "2.0", id: 9, method: "prompts/get", params: { name: "motion", arguments: { task: 5 } } }, "prompts/get bad arg");
mcp("core", { jsonrpc: "2.0", id: 10, method: "ping" });
mcp("core", { jsonrpc: "2.0", id: "s", method: "resources/list" });
mcp("core", { jsonrpc: "2.0", id: 11, method: "tools/call", params: { name: "nope", arguments: {} } }, "tools/call unknown");
mcp("core", { jsonrpc: "2.0", id: 12, method: "tools/call", params: { name: "create_frame", arguments: { width: -1, fill: "red", layoutMode: "X" } } }, "tools/call invalid");
mcp("core", { jsonrpc: "2.0", id: 13, method: "tools/call", params: { name: "get_document_info" } }, "tools/call no arguments");
mcp("core", { jsonrpc: "2.0", id: 14, method: "tools/call", params: {} }, "tools/call no name");
mcp("core", { jsonrpc: "2.0", id: 15, method: "tools/call", params: { name: "get_skill", arguments: { name: "nope" } } }, "tools/call get_skill unknown");
mcp("core", { jsonrpc: "2.0", id: 16, method: "tools/call", params: { name: "describe_tool", arguments: {} } }, "tools/call describe_tool");
mcp("core", { jsonrpc: "2.0", id: 17, method: "tools/call", params: { name: "create_ellipse", arguments: {} } }, "tools/call tool hidden in profile");
mcp("all", { jsonrpc: "2.0", id: 18, method: "tools/call", params: { name: "set_effects", arguments: { nodeId: "1", shadows: [{ x: "a" }] } } }, "tools/call nested invalid");
mcp("core", [{ jsonrpc: "2.0", id: 19, method: "ping" }, { jsonrpc: "2.0", method: "notifications/initialized" }], "batch");
mcp("core", { jsonrpc: "2.0", method: "notifications/initialized" }, "notification only");
mcp("core", { jsonrpc: "2.0", id: 20, method: "tools/list", extra: 1 }, "invalid message");
mcp("core", { jsonrpc: "2.0", id: 21, method: "ping" }, "bad protocol header", { "mcp-protocol-version": "1999" });
mcp("core", { jsonrpc: "2.0", id: 22, method: "ping" }, "no event-stream accept", { accept: "application/json" });
mcp("core", { jsonrpc: "2.0", id: 23, method: "ping" }, "text content type", { "content-type": "text/plain" });
add("MCP bad JSON", (p) => http(port(p), "POST", "/mcp", { body: "{x", headers: MCP_HEADERS }));
add("MCP GET without accept", (p) => http(port(p), "GET", "/mcp"));
add("MCP GET stream head", (p) => head(port(p), "GET", "/mcp", { accept: "text/event-stream" }));
add("MCP DELETE", (p) => http(port(p), "DELETE", "/mcp"));
add("MCP PUT", (p) => http(port(p), "PUT", "/mcp"));

let same = 0;
const diffs = [];
for (const c of cases) {
  let a, b;
  try {
    a = decode(await c.fn("node"), c.label);
    b = decode(await c.fn("rust"), c.label);
  } catch (err) {
    diffs.push(`${c.label}: ${err.message}`);
    continue;
  }
  const d = firstDiff(a, b);
  if (d) diffs.push(`${c.label}\n    ${d}`);
  else same++;
}
console.log(`parity: ${same}/${cases.length} identical (Node :${nodePort} vs Rust :${rustPort})`);
for (const d of diffs) console.log(`DIFF ${d}`);
process.exit(diffs.length ? 1 : 0);
