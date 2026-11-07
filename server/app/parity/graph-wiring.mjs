// The routes, tools and CLI commands wired to wridge-graph (code graph, brain, memory, knowledge, flows,
// projects list, the dir-based /motion routes), Rust vs a private Node instance, on temp copies of the test
// projects: the same steps on both servers (HTTP routes, /api/tool and MCP, error cases and every write
// path, a fake plugin for the design sync and the motion routes), the answers diffed, then every file in
// the copies (SQLite files as table dumps), the projects lists and the `code-graph` / `memory` CLI output.
//   node server/app/parity/graph-wiring.mjs [--keep] [--verbose]
//       starts both servers itself: the Rust debug build on 3255/3256 (WRIDGE_PROJECTS_FILE → a temp file) and
//       legacy-node/index.mjs from a temp copy (private-node.mjs) on 3355/3356, stops them through /api/stop
//   node server/app/parity/graph-wiring.mjs 3255 3256 3355 3356     (what run-all.mjs does)
//       servers already running; GW_RUST_PROJECTS / GW_NODE_PROJECTS name their projects lists (temp files,
//       reset to [] here) and GW_NODE_LEGACY the legacy-node copy whose wara.mjs runs the Node CLI.
// Never the user's ports (3155/3156, 3055/3056) and never the real legacy-node/code-projects.json: refused.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { fakeAnswers, startFakePlugin } from "./fake-plugin.mjs";
import { ROOT, assertNotRealProjects, makePrivateNode } from "./private-node.mjs";
import { PROJECTS } from "../../parity-projects.mjs";

process.removeAllListeners("warning"); // node:sqlite's ExperimentalWarning
const HERE = dirname(fileURLToPath(import.meta.url));
const EXE = process.platform === "win32" ? ".exe" : "";
const RUST = join(HERE, "..", "target", "debug", `wridge-server${EXE}`);
const WRIDGE = join(HERE, "..", "target", "debug", `wridge${EXE}`);

const argv = process.argv.slice(2);
const keep = argv.includes("--keep");
const verbose = argv.includes("--verbose");
const ports = argv.filter((a) => /^\d+$/.test(a)).map(Number);
if (ports.some((p) => [3155, 3156, 3055, 3056].includes(p))) throw new Error("refusing the user's server ports");
if (ports.length && ports.length !== 4) throw new Error("usage: graph-wiring.mjs [<rustWs> <rustHttp> <nodeWs> <nodeHttp>] [--keep] [--verbose]");
const own = ports.length === 0;
const [rustWs, rustHttp, nodeWs, nodeHttp] = own ? [3255, 3256, 3355, 3356] : ports;

const TMP = mkdtempSync(join(tmpdir(), "wridge-graph-wiring-"));

// ---------------------------------------------------------------- servers

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

let privateNode = null;
const procs = [];
let rustProjects = process.env.GW_RUST_PROJECTS;
let nodeProjects = process.env.GW_NODE_PROJECTS;
let nodeLegacy = process.env.GW_NODE_LEGACY;
if (own) {
  privateNode = makePrivateNode();
  rustProjects = join(TMP, "rust-projects.json");
  nodeProjects = privateNode.projectsFile;
  nodeLegacy = privateNode.dir;
}
if (!rustProjects || !nodeProjects || !nodeLegacy) throw new Error("GW_RUST_PROJECTS, GW_NODE_PROJECTS and GW_NODE_LEGACY are needed with ports");
for (const f of [rustProjects, nodeProjects]) assertNotRealProjects(f);
if (join(nodeLegacy, "code-projects.json").toLowerCase() !== nodeProjects.toLowerCase()) throw new Error("GW_NODE_PROJECTS must be the code-projects.json of GW_NODE_LEGACY");
for (const f of [rustProjects, nodeProjects]) writeFileSync(f, "[]\n");
if (own) {
  procs.push(await start(RUST, [], join(HERE, ".."), rustWs, rustHttp, { WRIDGE_PROJECTS_FILE: rustProjects }));
  procs.push(await start(process.execPath, ["index.mjs"], privateNode.dir, nodeWs, nodeHttp));
}

// ---------------------------------------------------------------- project copies

const SOURCES = [
  ...PROJECTS,
  join(ROOT, "server", "graph", "parity", "fixtures", "wave2"),
];
const SKIP = new Set(["node_modules", "vendor", ".git", "build", "dist", ".wara", ".next", ".dart_tool", ".turbo", ".vercel", "coverage", "target", "Pods", ".gradle", ".idea", "__pycache__", ".venv"]);
const [TR, ED, FK, MR, WB, W2] = SOURCES.map((s) => basename(s));
const MA = "motion-app";
const ALL = [TR, ED, FK, MR, WB, W2, MA];

function copyProject(src, dest) {
  mkdirSync(dest, { recursive: true });
  cpSync(src, dest, { recursive: true, preserveTimestamps: true, filter: (p) => !SKIP.has(basename(p)) });
  // real memory and knowledge are read, as the user has them
  for (const keepWara of ["memory.db", "knowledge"]) {
    const from = join(src, ".wara", keepWara);
    if (existsSync(from)) cpSync(from, join(dest, ".wara", keepWara), { recursive: true, preserveTimestamps: true });
  }
  if (existsSync(join(src, ".git"))) execFileSync("git", ["init", "-q", dest]);
}

const lottie = (extra = {}) => JSON.stringify({ v: "5.7.4", fr: 60, ip: 0, op: 60, w: 120, h: 80, nm: "", layers: [], ...extra });
// A synthetic project: Lottie files, motion tokens in CSS, a file name cmd.exe would interpret.
function motionApp(dest) {
  const w = (rel, text) => { mkdirSync(dirname(join(dest, rel)), { recursive: true }); writeFileSync(join(dest, rel), text); };
  w("package.json", JSON.stringify({ name: "motion-app", dependencies: { next: "15.1.0", react: "19.0.0", "framer-motion": "11.0.0" } }, null, 2));
  w("app/globals.css", ":root {\n  --duration-fast: 150ms;\n  --ease-out: cubic-bezier(0.2, 0, 0, 1);\n  --motion-slide: 24px;\n  --transition-card: all 200ms ease;\n  --color-brand: #2f6bff;\n}\n.card { transition: var(--transition-card); color: var(--color-brand); }\n");
  w("app/page.tsx", "import { Box } from \"../src/Box\";\nexport default function Page() {\n  return <Box />;\n}\n");
  w("src/Box.tsx", "export function Box() {\n  return <div className=\"card\" style={{ transitionDuration: \"var(--duration-fast)\", color: \"#ff0000\" }} />;\n}\n");
  w("public/lottie/success.json", lottie({ nm: "Success" }));
  w("public/lottie/linked.json", lottie({ meta: { g: "wara-bridge", wara: { source: "spec", spec: {}, figma: { componentId: "77:1", file: "utils", sentAt: "2026-11-01T00:00:00.000Z", hash: "abc" } } } }));
  w("public/lottie/from-art.json", lottie({ w: 64, h: 64, meta: { g: "wara-bridge", wara: { source: "figma-motion", spec: { nodeId: "1:2" } } } }));
  w("public/lottie/notlottie.json", JSON.stringify({ a: 1 }));
  w("a%b.txt", "unsafe name\n");
}

// made once and copied, so both sides see the same file times
motionApp(join(TMP, "_src", MA));
for (const side of ["rust", "node"]) {
  for (const src of [...SOURCES, join(TMP, "_src", MA)]) copyProject(src, join(TMP, side, basename(src)));
}

// ---------------------------------------------------------------- normalising

const ISO = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z/g;
function normaliser(base) {
  const fwd = base.replaceAll("\\", "/");
  const esc = base.replaceAll("\\", "\\\\");
  const forms = [...new Set([base, fwd, esc].flatMap((f) => [f, f.toUpperCase(), f.toLowerCase()]))].sort((a, b) => b.length - a.length);
  const text = (s) => {
    for (const f of forms) s = s.split(f).join("<tmp>");
    return s
      .replace(ISO, "<ts>")
      .replace(/\(updated \d{4}-\d{2}-\d{2} \d{2}:\d{2}\)/g, "(updated <ts>)")
      .replace(/flow-[0-9a-z]{6,}/g, "flow-<t>")
      .replace(/"ms":\d+/g, '"ms":"<ms>"')
      .replace(/\b\d+ ms\b/g, "<n> ms")
      .replace(/"ageDays":-?\d+/g, '"ageDays":"<n>"');
  };
  const value = (v) => JSON.parse(JSON.stringify(v ?? null, (k, x) => {
    if (["ms", "time", "totalMs", "ageDays"].includes(k) && typeof x === "number") return `<${k}>`;
    return typeof x === "string" ? text(x) : x;
  }));
  return { text, value };
}

// ---------------------------------------------------------------- HTTP

function http(port, method, path, body) {
  return new Promise((resolve, reject) => {
    const req = request({ host: "127.0.0.1", port, method, path, headers: { host: `127.0.0.1:${port}`, "content-type": "application/json", accept: "application/json, text/event-stream" } }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const buf = Buffer.concat(chunks);
        const type = String(res.headers["content-type"] ?? "");
        let out;
        if (type.startsWith("application/json")) out = JSON.parse(buf.toString("utf8"));
        else if (type.startsWith("text/event-stream")) out = buf.toString("utf8").split("\n").filter((l) => l.startsWith("data: ")).map((l) => JSON.parse(l.slice(6)));
        else out = buf.toString("utf8");
        resolve({ status: res.statusCode, body: out });
      });
    });
    req.on("error", reject);
    req.setTimeout(600_000, () => req.destroy(new Error(`timeout ${method} ${path}`)));
    if (body !== undefined) req.write(JSON.stringify(body));
    req.end();
  });
}

// ---------------------------------------------------------------- the fake plugin's answers

const key = (t, v, ease) => ({ t, v, ...(ease ? { ease } : {}) });
const field = (name, keys, op = "SET", base = 1) => ({ field: name, base, tracks: [{ op, keys }] });
const vec = { text: false, instances: 0, autoLayout: false, vectorOnly: true, image: false, component: null };
const READ_ART = { root: { id: "1:3", name: "Success" }, timelines: [{ duration: 1.2 }], animated: [
  { id: "4:1", name: "Ring", type: "GROUP", parentId: "1:3", nth: 1, path: [], hint: vec, styles: [], tracks: [field("SCALE_X", [key(0, 0, "BOUNCY"), key(0.5, 1)]), field("SCALE_Y", [key(0, 0, "BOUNCY"), key(0.5, 1)])] },
  { id: "4:2", name: "Check", type: "VECTOR", parentId: "1:3", nth: 1, path: [], hint: vec, styles: [], tracks: [field("PATH_TRIM_END", [key(0.3, 0, "cubic(0.2,0,0,1)"), key(0.7, 1)])] },
] };
const SUCCESS_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" viewBox="0 0 200 200">
  <g id="Ring"><circle cx="100" cy="100" r="80" fill="none" stroke="#22c55e" stroke-width="12"/></g>
  <path id="Check" d="M60 104 L88 132 L142 74" fill="none" stroke="#22c55e" stroke-width="14" stroke-linecap="round"/>
</svg>`;
const COMPONENTS = [
  { id: "1:9", name: "Button", type: "COMPONENT" },
  { id: "1:10", name: "Card", type: "COMPONENT_SET" },
  { id: "1:11", name: "Size=Small", type: "COMPONENT" },
  { id: "1:12", name: "Hero", type: "FRAME" },
  { id: "1:13", name: "Primary Button", type: "COMPONENT" },
];
const TOKENS = {
  sources: ["Foundations page", "variables", "styles"],
  file: "utils",
  collections: [{ name: "Primitives", defaultMode: "Value" }, { name: "Theme", defaultMode: "Light" }],
  variables: [
    { token: "blue-500", type: "COLOR", collection: "Primitives", values: { Value: "#2f6bff" } },
    { token: "colors-primary", type: "COLOR", collection: "Theme", values: { Light: { alias: "blue-500" }, Dark: "#ffffff" } },
    { token: "duration-fast", type: "FLOAT", collection: "Primitives", values: { Value: 150 } },
    { token: "radius-md", type: "FLOAT", collection: "Theme", values: { Light: 8.5 } },
    { token: "feature-x", type: "BOOLEAN", collection: "Theme", values: { Light: true } },
  ],
  paintStyles: [{ name: "Brand/Primary", token: "color-brand", value: "#2f6bff" }, { name: "Sunset", token: "gradient-sunset", value: { gradient: "linear", stops: [] } }],
  textStyles: [{ token: "text-display", family: "IBM Plex Sans Thai", style: "SemiBold", size: 48, lineHeight: "120%", letterSpacing: "-0.02em" }],
  effectStyles: [{ token: "shadow-sm", shadows: [{ x: 0, y: 1, blur: 2, spread: 0, color: "#0000001a" }] }],
};
// 1×1 GIF
const GIF = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

function answers(fileName) {
  const base = fakeAnswers(fileName);
  return async (command, params) => {
    switch (command) {
      case "get_components": return COMPONENTS;
      case "get_tokens": return TOKENS;
      case "get_node":
        if (params.nodeId === "77:1") return { id: "77:1", name: "Lottie/linked", type: "COMPONENT" };
        return base(command, params);
      case "motion_read":
        if (params.nodeId === "1:3") return READ_ART;
        throw new Error("Figma Motion is not available in this editor");
      case "export_assets": return params.items.map((it) => ({ nodeId: it.nodeId, name: "Success", format: "svg", base64: Buffer.from(SUCCESS_SVG).toString("base64") }));
      default: return base(command, params);
    }
  };
}

// ---------------------------------------------------------------- the session

async function session(wsPort, httpPort, base) {
  const out = [];
  const norm = normaliser(base);
  const P = (n) => `${base}/${n}`.replaceAll("\\", "/");
  const B = (n) => join(base, n);
  const Q = (n) => encodeURIComponent(P(n));
  const step = async (label, fn) => {
    try {
      out.push({ label, value: norm.value(await fn()) });
    } catch (err) {
      out.push({ label, value: `THREW ${err.message}` });
    }
  };
  const get = (path) => http(httpPort, "GET", path);
  const post = (path, body) => http(httpPort, "POST", path, body);
  const put = (path, body) => http(httpPort, "PUT", path, body);
  const tool = (name, args) => post("/api/tool", { name, args });
  let mcpId = 0;
  const mcp = (name, args) => post("/mcp?profile=all", { jsonrpc: "2.0", id: ++mcpId, method: "tools/call", params: { name, arguments: args } });

  // ---- the projects list
  await step("GET /api/code/projects (empty)", () => get("/api/code/projects"));
  await step("GET /api/code/graph before the project is added", () => get(`/api/code/graph?dir=${Q(W2)}`));
  await step("POST /api/brain/note dir 5, empty list", () => post("/api/brain/note", { dir: 5, name: "x", content: "y" }));
  await step("POST /api/code/projects/remove, empty list", () => post("/api/code/projects/remove", {}));
  for (const n of ALL) await step(`POST /api/code/projects ${n}`, () => post("/api/code/projects", { dir: P(n) }));
  await step("POST /api/code/projects no dir", () => post("/api/code/projects", {}));
  await step("POST /api/code/projects null body", () => post("/api/code/projects", null));
  await step("POST /api/code/projects relative", () => post("/api/code/projects", { dir: "relative/x" }));
  await step("POST /api/code/projects missing folder", () => post("/api/code/projects", { dir: P("missing") }));
  await step("POST /api/code/projects dir 5", () => post("/api/code/projects", { dir: 5 }));
  await step("POST /api/code/projects dir \"\"", () => post("/api/code/projects", { dir: "" }));
  await step("POST /api/code/projects a file", () => post("/api/code/projects", { dir: `${P(MA)}/package.json` }));
  await step("POST /api/code/projects again, backslashes (moves first)", () => post("/api/code/projects", { dir: B(MR) }));
  await step("GET /api/code/projects", () => get("/api/code/projects"));
  await step("POST /api/code/projects/remove other case + backslashes", () => post("/api/code/projects/remove", { dir: B(MR).toUpperCase() }));
  await step("POST /api/code/projects/remove no dir", () => post("/api/code/projects/remove", {}));
  await step("POST /api/code/projects/remove dir 5", () => post("/api/code/projects/remove", { dir: 5 }));
  await step("POST /api/code/projects/remove array body", () => post("/api/code/projects/remove", [1]));
  await step("POST /api/code/projects/remove not listed", () => post("/api/code/projects/remove", { dir: P("missing") }));
  await step("GET /api/code/projects after remove", () => get("/api/code/projects"));
  await step("POST /api/code/projects re-add", () => post("/api/code/projects", { dir: P(MR) }));

  // ---- code graph and data pages
  for (const n of ALL) await step(`GET /api/code/graph ${n}`, () => get(`/api/code/graph?dir=${Q(n)}`));
  await step("GET /api/code/graph no dir", () => get("/api/code/graph"));
  await step("GET /api/code/graph unknown dir", () => get(`/api/code/graph?dir=${Q("missing")}`));
  await step("GET /api/code/graph other case + backslashes", () => get(`/api/code/graph?dir=${encodeURIComponent(B(W2).toUpperCase())}`));
  for (const n of [ED, W2, FK, MA]) await step(`GET /api/code/data ${n}`, () => get(`/api/code/data?dir=${Q(n)}`));
  await step("GET /api/code/data unknown dir", () => get(`/api/code/data?dir=${Q("missing")}`));

  // ---- motion (no plugin yet)
  await step("GET /api/motion motion-app", () => get(`/api/motion?dir=${Q(MA)}`));
  await step("GET /api/motion edupro", () => get(`/api/motion?dir=${Q(ED)}`));
  await step("GET /api/motion no dir", () => get("/api/motion"));
  await step("GET /api/motion/file", () => get(`/api/motion/file?dir=${Q(MA)}&path=public/lottie/success.json`));
  await step("GET /api/motion/file no path", () => get(`/api/motion/file?dir=${Q(MA)}`));
  await step("GET /api/motion/file outside", () => get(`/api/motion/file?dir=${Q(MA)}&path=../x.json`));
  await step("GET /api/motion/file not a Lottie", () => get(`/api/motion/file?dir=${Q(MA)}&path=public/lottie/notlottie.json`));
  await step("GET /api/motion/file missing", () => get(`/api/motion/file?dir=${Q(MA)}&path=public/lottie/nope.json`));
  await step("POST /api/motion/figma-to-lottie no plugin", () => post("/api/motion/figma-to-lottie", { dir: P(MA), id: "1:3", out: "public/lottie/new.json" }));
  await step("POST /api/motion/figma-to-lottie unknown dir", () => post("/api/motion/figma-to-lottie", { dir: P("missing"), id: "1:3", out: "x.json" }));
  await step("POST /api/motion/figma no plugin", () => post("/api/motion/figma", { dir: P(MA), path: "public/lottie/success.json", gif: GIF, confirmFile: "utils" }));
  await step("POST /api/motion/figma dir 5", () => post("/api/motion/figma", { dir: 5 }));
  await step("POST /api/code/design no plugin", () => post("/api/code/design", { dir: P(W2) }));
  await step("code_design_sync no plugin", () => tool("code_design_sync", { projectDir: P(W2) }));

  // ---- flows
  for (const n of [ED, W2, FK]) await step(`GET /api/flows ${n}`, () => get(`/api/flows?dir=${Q(n)}`));
  for (const t of ["POST /api/checkout", "/api/checkout", "createPayment", "lib/payments.ts", "nope"]) {
    await step(`GET /api/flows/trace ${t}`, () => get(`/api/flows/trace?dir=${Q(W2)}&target=${encodeURIComponent(t)}`));
  }
  await step("GET /api/flows/trace no target", () => get(`/api/flows/trace?dir=${Q(W2)}`));
  await step("GET /api/flows unknown dir", () => get(`/api/flows?dir=${Q("missing")}`));
  const flow = {
    title: "Refund money", entry: "POST /api/payments/webhook",
    steps: [
      { id: "a", actor: "user", text: "asks for a refund on `/orders/[id]`" },
      { id: "b", text: "checks `setPaymentStatus`", group: "g" },
      { id: "c", actor: "email", text: "sends `sendReceipt`", group: "g", branches: [{ label: "fails", to: "a" }, { label: "done", to: "end" }] },
      { id: "d", wait: true, actor: "bank", text: "pays back `payments, orders`" },
    ],
    states: [{ column: "payments.status", values: ["paid", "refunded", "lost"], transitions: [{ from: "paid", to: "refunded", note: "refund" }] }],
    notes: "Ask the bank first.",
  };
  let saved = null;
  await step("PUT /api/flows new, from flow", async () => { const r = await put("/api/flows", { dir: P(W2), flow, baseHash: null }); saved = r.body; return r; });
  await step("PUT /api/flows same title again → 409", () => put("/api/flows", { dir: P(W2), flow }));
  await step("PUT /api/flows with the right baseHash", () => put("/api/flows", { dir: P(W2), file: saved?.file, content: "# Flow: Refund money\n\n## Steps\n1. [user] asks `createPayment`\n", baseHash: saved?.hash }));
  await step("PUT /api/flows stale baseHash → 409", () => put("/api/flows", { dir: P(W2), file: saved?.file, content: "x", baseHash: saved?.hash }));
  await step("PUT /api/flows new file with baseHash → 409", () => put("/api/flows", { dir: P(W2), file: "docs/flows/brand-new.md", content: "# Flow: New\n", baseHash: "abc" }));
  await step("PUT /api/flows content + file", () => put("/api/flows", { dir: P(W2), file: "custom.md", content: "# Flow: Custom\nentry: `GET /orders/[id]`\n\n## Steps\n1. [user] opens `app/orders/[id]/page.tsx`\n2. [server] `loadOrder`\n" }));
  await step("PUT /api/flows content, name from its title", () => put("/api/flows", { dir: P(W2), content: "# Flow: Ünïcode Títle — Ölçek!\n\n## Steps\n1. [user] x\n" }));
  await step("PUT /api/flows bad file name", () => put("/api/flows", { dir: P(W2), file: "../evil.md", content: "x" }));
  await step("PUT /api/flows flow without steps", () => put("/api/flows", { dir: P(W2), flow: { title: "x" } }));
  await step("PUT /api/flows nothing", () => put("/api/flows", { dir: P(W2) }));
  await step("PUT /api/flows content 5", () => put("/api/flows", { dir: P(W2), file: "five.md", content: 5 }));
  await step("PUT /api/flows unknown dir", () => put("/api/flows", { dir: P("missing"), flow }));
  let overview = null;
  await step("GET /api/flows wave2 after saves", async () => { const r = await get(`/api/flows?dir=${Q(W2)}`); overview = r.body; return r; });
  const draft = overview?.drafts?.[0]?.id ?? "draft-id";
  await step("POST /api/flows/ignore a draft", () => post("/api/flows/ignore", { dir: P(W2), id: draft }));
  await step("POST /api/flows/ignore number id", () => post("/api/flows/ignore", { dir: P(W2), id: 7 }));
  await step("POST /api/flows/ignore no id", () => post("/api/flows/ignore", { dir: P(W2) }));
  await step("POST /api/flows/ignore nothing", () => post("/api/flows/ignore", {}));
  await step("POST /api/flows/ignore dir 5", () => post("/api/flows/ignore", { dir: 5, id: "x" }));
  await step("GET /api/flows wave2 after ignore", () => get(`/api/flows?dir=${Q(W2)}`));

  // ---- design links, open in VS Code (error paths only: the success path would open VS Code)
  const graph = (await get(`/api/code/graph?dir=${Q(W2)}`)).body;
  const button = graph?.symbols?.find((s) => s.name === "Button");
  await step("POST /api/code/link", () => post("/api/code/link", { dir: P(W2), figmaName: "Button", symbolId: button?.id }));
  await step("POST /api/code/link string id", () => post("/api/code/link", { dir: P(W2), figmaName: "Primary Button", symbolId: String(button?.id) }));
  await step("POST /api/code/link unknown symbol", () => post("/api/code/link", { dir: P(W2), figmaName: "X", symbolId: 999999 }));
  await step("POST /api/code/link no symbol", () => post("/api/code/link", { dir: P(W2), figmaName: "X" }));
  await step("POST /api/code/link unknown dir", () => post("/api/code/link", { dir: P("missing"), figmaName: "X", symbolId: 1 }));
  await step("POST /api/code/open outside", () => post("/api/code/open", { dir: P(MA), path: "../outside.txt" }));
  await step("POST /api/code/open missing", () => post("/api/code/open", { dir: P(MA), path: "nope.txt" }));
  await step("POST /api/code/open unsafe name", () => post("/api/code/open", { dir: P(MA), path: "a%b.txt", line: 3 }));
  await step("POST /api/code/open unknown dir", () => post("/api/code/open", { dir: P("missing"), path: "x" }));
  await step("GET /api/code/graph wave2 after link", () => get(`/api/code/graph?dir=${Q(W2)}`));

  // ---- brain
  for (const n of [TR, W2, ED]) await step(`GET /api/brain ${n}`, () => get(`/api/brain?dir=${Q(n)}`));
  for (const n of [TR, W2]) await step(`GET /api/brain/code-links ${n}`, () => get(`/api/brain/code-links?dir=${Q(n)}`));
  const trNotes = (await get(`/api/brain?dir=${Q(TR)}`)).body?.nodes?.filter((n) => n.kind === "note").slice(0, 3) ?? [];
  for (const n of trNotes) await step(`GET /api/brain/note Traiphum ${n.name}`, () => get(`/api/brain/note?dir=${Q(TR)}&name=${encodeURIComponent(n.name)}`));
  for (const name of ["CH1", "นที", "characters/nathee", "rules/crime", "nope", ""]) {
    await step(`GET /api/brain/note wave2 "${name}"`, () => get(`/api/brain/note?dir=${Q(W2)}&name=${encodeURIComponent(name)}`));
  }
  await step("GET /api/brain/note no name", () => get(`/api/brain/note?dir=${Q(W2)}`));
  await step("GET /api/brain no dir", () => get("/api/brain"));
  await step("GET /api/brain unknown dir", () => get(`/api/brain?dir=${Q("missing")}`));
  const note = (body) => post("/api/brain/note", { dir: P(W2), ...body });
  await step("POST /api/brain/note create", () => note({ name: "ideas/New idea", content: "Links to [[CH1]], [[Nowhere]] and [[code:lib/db.ts]]" }));
  await step("POST /api/brain/note create again", () => note({ name: "ideas/New idea", content: "again" }));
  await step("POST /api/brain/note append", () => note({ name: "New idea", mode: "append", content: "More." }));
  await step("POST /api/brain/note replace (in place)", () => note({ name: "New idea", mode: "replace", content: "# New idea\n\nReplaced. [[Gone]]" }));
  await step("POST /api/brain/note mode null", () => note({ name: "New idea", mode: null, content: "Null mode." }));
  await step("POST /api/brain/note mode 5, new note", () => note({ name: "Mode five", mode: 5, content: "x" }));
  await step("POST /api/brain/note mode 5, existing", () => note({ name: "Mode five", mode: 5, content: "y" }));
  await step("POST /api/brain/note no content", () => note({ name: "Empty" }));
  await step("POST /api/brain/note content 5", () => note({ name: "Five", content: 5 }));
  await step("POST /api/brain/note no name", () => note({ content: "No name given" }));
  await step("POST /api/brain/note bad name", () => note({ name: "///", content: "x" }));
  await step("POST /api/brain/note front matter", () => note({ name: "Tagged", content: "---\ntags: [a, b]\n---\n# Tagged\n\nbody" }));
  let trashed = null;
  await step("POST /api/brain/trash", async () => { const r = await post("/api/brain/trash", { dir: P(W2), name: "ideas/New idea" }); trashed = r.body; return r; });
  await step("POST /api/brain/trash again", () => post("/api/brain/trash", { dir: P(W2), name: "ideas/New idea" }));
  await step("POST /api/brain/trash CH1 (has backlinks)", () => post("/api/brain/trash", { dir: P(W2), name: "CH1" }));
  await step("POST /api/brain/trash no name", () => post("/api/brain/trash", { dir: P(W2) }));
  await step("POST /api/brain/restore", () => post("/api/brain/restore", { dir: P(W2), path: trashed?.restore }));
  await step("POST /api/brain/restore again", () => post("/api/brain/restore", { dir: P(W2), path: trashed?.restore }));
  await step("POST /api/brain/restore no path", () => post("/api/brain/restore", { dir: P(W2) }));
  await step("POST /api/brain/restore outside the trash", () => post("/api/brain/restore", { dir: P(W2), path: "../Traiphum.md" }));
  await step("POST /api/brain/trash again → (2)", () => post("/api/brain/trash", { dir: P(W2), name: "ideas/New idea" }));
  await step("POST /api/brain/note create the same again", () => note({ name: "ideas/New idea", content: "third" }));
  await step("POST /api/brain/trash → (3)", () => post("/api/brain/trash", { dir: P(W2), name: "ideas/New idea" }));
  let afterWrites = null;
  await step("GET /api/brain wave2 after writes", async () => { const r = await get(`/api/brain?dir=${Q(W2)}`); afterWrites = r.body; return r; });
  await step("POST /api/brain/empty-trash one missing", () => post("/api/brain/empty-trash", { dir: P(W2), path: "nope.md" }));
  await step("POST /api/brain/empty-trash path 5", () => post("/api/brain/empty-trash", { dir: P(W2), path: 5 }));
  await step("POST /api/brain/empty-trash outside", () => post("/api/brain/empty-trash", { dir: P(W2), path: "../CH1.md" }));
  await step("POST /api/brain/empty-trash one", () => post("/api/brain/empty-trash", { dir: P(W2), path: afterWrites?.trash?.at(-1)?.path }));
  await step("POST /api/brain/empty-trash all", () => post("/api/brain/empty-trash", { dir: P(W2) }));
  await step("POST /api/brain/empty-trash all again", () => post("/api/brain/empty-trash", { dir: P(W2), path: "" }));
  await step("POST /api/brain/note unknown dir", () => post("/api/brain/note", { dir: P("missing"), name: "x", content: "y" }));
  await step("POST /api/brain/trash dir 5", () => post("/api/brain/trash", { dir: 5, name: "x" }));
  await step("POST /api/brain/note null body", () => post("/api/brain/note", null));
  await step("GET /api/brain wave2 at the end", () => get(`/api/brain?dir=${Q(W2)}`));

  // ---- with the plugin
  const plugin = await startFakePlugin(wsPort, { fileName: "utils", answer: answers("utils") });
  await step("GET /api/motion with the plugin", () => get(`/api/motion?dir=${Q(MA)}`));
  const f2l = (body) => post("/api/motion/figma-to-lottie", { dir: P(MA), ...body });
  await step("figma-to-lottie no id", () => f2l({ out: "public/lottie/new.json" }));
  await step("figma-to-lottie out not .json", () => f2l({ id: "1:3", out: "x.txt" }));
  await step("figma-to-lottie exists", () => f2l({ id: "1:3", out: "public/lottie/success.json" }));
  await step("figma-to-lottie converts", () => f2l({ id: "1:3", out: "public/lottie/new.json" }));
  await step("figma-to-lottie overwrite, backslashes", () => f2l({ id: "1:3", out: "public\\lottie\\new.JSON", overwrite: true }));
  await step("figma-to-lottie out array", () => f2l({ id: "1:3", out: ["a.json"] }));
  await step("figma-to-lottie no Motion", () => f2l({ id: "1:2", out: "public/lottie/none.json" }));
  await step("figma-to-lottie outside", () => f2l({ id: "1:3", out: "../outside.json" }));
  const fig = (body) => post("/api/motion/figma", { dir: P(MA), gif: GIF, ...body });
  await step("motion → Figma: confirm first", () => fig({ path: "public/lottie/success.json" }));
  await step("motion → Figma: another file", () => fig({ path: "public/lottie/success.json", confirmFile: "other" }));
  await step("motion → Figma: new component", () => fig({ path: "public/lottie/success.json", confirmFile: "utils" }));
  await step("motion → Figma: linked component again", () => fig({ path: "public/lottie/linked.json", confirmFile: "utils" }));
  await step("motion → Figma: next to its art", () => fig({ path: "public/lottie/from-art.json", confirmFile: "utils" }));
  await step("motion → Figma: not a Lottie", () => fig({ path: "public/lottie/notlottie.json", confirmFile: "utils" }));
  await step("motion → Figma: bad GIF", () => fig({ path: "public/lottie/success.json", confirmFile: "utils", gif: "" }));
  await step("motion → Figma: no path", () => fig({ confirmFile: "utils" }));
  await step("GET /api/motion after sending", () => get(`/api/motion?dir=${Q(MA)}`));
  await step("POST /api/code/design", () => post("/api/code/design", { dir: P(W2) }));
  await step("POST /api/code/design motion-app", () => post("/api/code/design", { dir: P(MA) }));
  await step("code_design_sync", () => tool("code_design_sync", { projectDir: P(ED) }));
  await step("code_design_sync via MCP", () => mcp("code_design_sync", { projectDir: P(FK) }));
  await step("code_design_sync relative dir", () => tool("code_design_sync", { projectDir: "x" }));
  await step("code_graph design after sync", () => tool("code_graph", { projectDir: P(W2), query: "design" }));
  await step("code_graph tokens after sync", () => tool("code_graph", { projectDir: P(MA), query: "tokens" }));
  await step("GET /api/code/graph wave2 after sync", () => get(`/api/code/graph?dir=${Q(W2)}`));
  await step("GET /api/motion motion-app after sync", () => get(`/api/motion?dir=${Q(MA)}`));
  await step("plugin received", () => plugin.received);
  await plugin.close();
  await new Promise((r) => setTimeout(r, 150));

  // ---- code_graph and friends
  const cg = (projectDir, query, target, extra = {}) => tool("code_graph", { projectDir, query, ...(target === undefined ? {} : { target }), ...extra });
  for (const n of ALL) await step(`code_graph overview ${n}`, () => cg(P(n), "overview"));
  const asks = [
    [W2, "find", "order"], [W2, "find", "payments"], [ED, "find", "user"], [FK, "find", "home"], [WB, "find", "tool"],
    [W2, "file", "lib/payments.ts"], [W2, "file", "nope.ts"], [W2, "used_by", "Button"], [W2, "uses", "app/checkout/page.tsx"],
    [W2, "impact", "lib/db.ts"], [W2, "impact", "orders.status"], [W2, "impact", "query", { depth: 1 }],
    [W2, "route", "/checkout"], [W2, "route", "/orders/[id]", { depth: 2 }], [FK, "route", "HomeScreen"],
    [W2, "design"], [W2, "tokens"], [MA, "tokens", "duration-fast"], [W2, "notes"], [W2, "data"], [ED, "data"],
    [W2, "table", "orders"], [W2, "table", "nope"], [ED, "table", "users"], [W2, "flows"], [W2, "flows", "checkout"],
    [W2, "trace", "POST /api/checkout"], [ED, "overview", undefined, { limit: 5 }], [TR, "find", "Boss"],
  ];
  for (const [n, q, t, extra] of asks) await step(`code_graph ${q} ${n} ${t ?? ""} ${JSON.stringify(extra ?? {})}`, () => cg(P(n), q, t, extra));
  await step("code_graph via MCP", () => mcp("code_graph", { projectDir: B(W2), query: "find", target: "Card" }));
  await step("code_graph relative dir", () => cg("relative", "overview"));
  await step("code_graph missing folder", () => cg(P("missing"), "overview"));
  await step("code_graph bad query", () => cg(P(W2), "everything"));
  await step("code_graph_update", () => tool("code_graph_update", { projectDir: P(MR) }));
  await step("code_graph_update force", () => tool("code_graph_update", { projectDir: P(MA), force: true }));
  await step("code_graph_update new project", () => tool("code_graph_update", { projectDir: P(MA) + "/src" }));
  await step("code_graph_update relative", () => tool("code_graph_update", { projectDir: "." }));
  const cgn = (target, text, n = W2) => tool("code_graph_note", { projectDir: P(n), target, text });
  await step("code_graph_note file", () => cgn("lib/payments.ts", "Talks to Stripe; amounts in satang."));
  await step("code_graph_note dir", () => cgn("dir:components", "Shared UI kit."));
  await step("code_graph_note symbol", () => cgn("createPayment", "Creates a pending payment first."));
  await step("code_graph_note path#Name", () => cgn("lib/payments.ts#loadOrder", "Reads one order."));
  await step("code_graph_note unknown", () => cgn("Nope", "x"));
  await step("code_graph_note delete", () => cgn("dir:components", ""));
  await step("code_graph_note via MCP", () => mcp("code_graph_note", { projectDir: P(ED), target: "routes/web.php", text: "Web routes." }));
  await step("code_graph notes after notes", () => cg(P(W2), "notes"));
  await step("code_graph file with notes", () => cg(P(W2), "file", "lib/payments.ts"));

  // ---- project_knowledge
  const pk = (args, n = W2) => tool("project_knowledge", { projectDir: P(n), ...args });
  await step("knowledge list", () => pk({ action: "list" }));
  await step("knowledge list topic", () => pk({ action: "list", topic: "security" }));
  await step("knowledge fact", () => pk({ action: "fact", key: "deploy", value: "VPS + Docker Compose, 1 server" }));
  await step("knowledge fact 2", () => pk({ action: "fact", key: "Redis", value: "none" }));
  await step("knowledge fact delete", () => pk({ action: "fact", key: "Redis", value: "" }));
  await step("knowledge fact no value", () => pk({ action: "fact", key: "ci" }));
  await step("knowledge get missing", () => pk({ action: "get", topic: "security", tool: "next" }));
  await step("knowledge get no tool", () => pk({ action: "get", topic: "security" }));
  await step("knowledge save detected version", () => pk({ action: "save", topic: "security", tool: "next", content: "Use the App Router's server actions with CSRF checks.", sources: ["https://nextjs.org/docs/app"] }));
  await step("knowledge save with version", () => pk({ action: "save", topic: "Cache", tool: "github.com/gofiber/fiber", version: "v2.52.0", content: "Use the cache middleware.", sources: ["https://docs.gofiber.io/"] }));
  await step("knowledge save unknown version", () => pk({ action: "save", topic: "x", tool: "left-pad-2", content: "x", sources: ["https://example.com/"] }));
  await step("knowledge save no sources", () => pk({ action: "save", topic: "x", tool: "next", content: "x" }));
  await step("knowledge save bad source", () => pk({ action: "save", topic: "x", tool: "next", content: "x", sources: ["not a url"] }));
  await step("knowledge get", () => pk({ action: "get", topic: "security", tool: "next" }));
  await step("knowledge list after", () => pk({ action: "list", topic: "security" }));
  await step("knowledge list motion-app", () => pk({ action: "list" }, MA));
  await step("knowledge list edupro", () => pk({ action: "list" }, ED));
  await step("knowledge via MCP", () => mcp("project_knowledge", { projectDir: P(FK), action: "list", topic: "state" }));
  await step("knowledge relative dir", () => tool("project_knowledge", { projectDir: "x", action: "list" }));

  // ---- project_memory
  const pm = (args, n = W2) => tool("project_memory", { projectDir: P(n), ...args });
  await step("memory overview", () => pm({ action: "overview" }));
  await step("memory overview Traiphum", () => pm({ action: "overview" }, TR));
  await step("memory overview edupro", () => pm({ action: "overview" }, ED));
  await step("memory find", () => pm({ action: "find", query: "checkout payment" }));
  await step("memory find Traiphum", () => pm({ action: "find", query: "boss" }, TR));
  await step("memory decide", () => pm({ action: "decide", title: "Pay with Stripe", decision: "Stripe only, THB.", why: "One provider.", tags: ["Pay", "money"] }));
  await step("memory decide supersedes", () => pm({ action: "decide", title: "Pay with Omise", decision: "Omise for PromptPay.", supersedes: "D-1" }));
  await step("memory decide math-random (new file)", () => pm({ action: "decide", title: "Seeded random", decision: "Use a seeded PRNG." }, MR));
  await step("memory note", () => pm({ action: "note", task: "Checkout", text: "Agreed: refunds go through the admin.", tags: ["refund"] }));
  await step("memory note no task", () => pm({ action: "note", text: "Loose note." }));
  await step("memory handoff", () => pm({ action: "handoff", task: "Checkout", stage: "2/5 Flow", done: "flow drafted", next: "screens", questions: "refund window?" }));
  await step("memory handoff update", () => pm({ action: "handoff", task: "Checkout", stage: "3/5 Screens" }));
  await step("memory overview after", () => pm({ action: "overview" }));
  await step("memory find includeOld", () => pm({ action: "find", query: "refunds admin", includeOld: true }));
  await step("memory handoff close", () => pm({ action: "handoff", task: "Checkout", close: true }));
  await step("memory handoff close unknown", () => pm({ action: "handoff", task: "Nope", close: true }));
  await step("memory find after close", () => pm({ action: "find", query: "refunds" }));
  await step("memory find after close, includeOld", () => pm({ action: "find", query: "refunds", includeOld: true }));
  await step("memory read", () => pm({ action: "read", name: "นที" }));
  await step("memory read Traiphum", () => pm({ action: "read", name: trNotes[0]?.name ?? "x" }, TR));
  await step("memory read missing", () => pm({ action: "read", name: "nope" }));
  await step("memory write", () => pm({ action: "write", name: "Memory note", content: "Agreed in chat. [[CH1]]", tags: ["memo", "chat"] }));
  await step("memory write exists", () => pm({ action: "write", name: "Memory note", content: "again" }));
  await step("memory write append", () => pm({ action: "write", name: "Memory note", mode: "append", content: "Appended." }));
  await step("memory write replace (old text to the trash)", () => pm({ action: "write", name: "Memory note", mode: "replace", content: "Replaced." }));
  await step("memory read after", () => pm({ action: "read", name: "Memory note" }));
  let mtrash = null;
  await step("memory trash", async () => { const r = await pm({ action: "trash", name: "Memory note" }); mtrash = JSON.parse(r.body?.content?.[0]?.text ?? "null"); return r; });
  await step("memory restore", () => pm({ action: "restore", path: mtrash?.restore ?? "x" }));
  await step("memory restore again", () => pm({ action: "restore", path: mtrash?.restore ?? "x" }));
  for (const [label, args] of [["read no name", { action: "read" }], ["write no name", { action: "write", content: "x" }], ["write no content", { action: "write", name: "x" }], ["trash no name", { action: "trash" }], ["restore no path", { action: "restore" }], ["find no query", { action: "find" }], ["decide no decision", { action: "decide", title: "x" }], ["note no text", { action: "note" }], ["handoff no task", { action: "handoff" }], ["bad mode", { action: "write", name: "x", content: "y", mode: "overwrite" }]]) {
    await step(`memory ${label}`, () => pm(args));
  }
  await step("memory via MCP", () => mcp("project_memory", { projectDir: P(TR), action: "find", query: "นาที" }));
  await step("memory overview math-random", () => pm({ action: "overview" }, MR));
  await step("memory overview relative", () => tool("project_memory", { projectDir: "x", action: "overview" }));
  await step("memory overview missing folder", () => pm({ action: "overview" }, "missing"));
  await step("GET /api/brain wave2 after memory", () => get(`/api/brain?dir=${Q(W2)}`));
  await step("GET /api/code/projects at the end", () => get("/api/code/projects"));
  return out;
}

// ---------------------------------------------------------------- files and databases each side wrote

function dumpDb(file, norm) {
  const db = new DatabaseSync(file, { readOnly: true });
  try {
    const out = {};
    const master = db.prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").all();
    out.sqlite_master = master.map((m) => JSON.stringify([m.type, m.name, m.sql]));
    out.pragma = [JSON.stringify(db.prepare("PRAGMA journal_mode").get()), JSON.stringify(db.prepare("PRAGMA user_version").get())];
    for (const { name } of master.filter((m) => m.type === "table")) {
      out[name] = db.prepare(`SELECT rowid AS _rowid_, * FROM "${name}" ORDER BY rowid`).all().map((r) => {
        const s = JSON.stringify(Object.values(r).map((v) => (typeof v === "bigint" ? Number(v) : v)));
        // brain.db stamps are "<mtimeMs>:<size>": the time a file was written differs per side
        return norm.text(s.replace(/"(-?[\d.e+]+):(\d+)"/g, '"<mtime>:$2"'));
      });
    }
    return out;
  } finally {
    db.close();
  }
}

function manifest(base) {
  const norm = normaliser(base);
  const files = {};
  const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      const p = join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== ".git") walk(p); continue; }
      const rel = p.slice(base.length + 1).replaceAll("\\", "/");
      if (/-(wal|shm|journal)$/.test(rel)) continue;
      if (/\.db$/.test(rel)) { files[rel] = dumpDb(p, norm); continue; }
      const buf = readFileSync(p);
      const text = buf.toString("utf8");
      const body = text.includes("\uFFFD") ? buf : Buffer.from(norm.text(text));
      files[rel] = `${body.length} bytes sha256:${createHash("sha256").update(body).digest("hex").slice(0, 16)}`;
      if (verbose && /\.(md|json)$/.test(rel) && !rel.includes("/")) files[`${rel} (text)`] = norm.text(text);
    }
  };
  walk(base);
  return files;
}

// ---------------------------------------------------------------- CLI: wara.mjs (Node) vs wridge (Rust)

function cliRuns(base) {
  const P = (n) => `${base}/${n}`.replaceAll("\\", "/");
  return [
    [W2, ["code-graph"]], [W2, ["code-graph", "help"]], [W2, ["code-graph", "overview"]], [W2, ["code-graph", "find", "order"]],
    [W2, ["code-graph", "impact", "orders.status"]], [W2, ["code-graph", "route", "/checkout", "--depth", "2"]],
    [W2, ["code-graph", "used_by", "C:/Program Files/Git/api/checkout"]], [W2, ["code-graph", "update"]], [MR, ["code-graph", "update", "--force"]],
    [W2, ["code-graph", "note", "lib/db.ts", "Wraps", "pg;", "one", "pool."]], [W2, ["code-graph", "notes", "--limit", "5"]],
    [MA, ["code-graph", "overview", "--dir", P(W2)]], [W2, ["code-graph", "overview", "--dir", "relative"]], [W2, ["code-graph", "table", "orders"]],
    [`${W2}/lib`, ["code-graph", "find", "query"]],
    [W2, ["memory"]], [W2, ["memory", "overview"]], [W2, ["memory", "find", "checkout"]], [TR, ["memory", "overview"]], [`${W2}/lib`, ["memory", "overview"]],
    [W2, ["memory", "find"]], [W2, ["memory", "brief"]], [W2, ["memory", "brief", "--stdin"], JSON.stringify({ cwd: P(TR) })],
    [W2, ["memory", "brief", "--stdin"], "not json"], [W2, ["memory", "brief", "--stdin"], ""], [MR, ["memory", "overview", "--dir", P(TR)]],
  ];
}

function runCli(side, base) {
  const norm = normaliser(base);
  return cliRuns(base).map(([cwd, args, input]) => {
    const opts = { cwd: join(base, cwd), encoding: "utf8", input: input ?? "", windowsHide: true };
    const r = side === "node" ? spawnSync(process.execPath, [join(nodeLegacy, "wara.mjs"), ...args], opts) : spawnSync(WRIDGE, args, opts);
    // an uncaught error: Node prints the source line and a stack, wridge "Error: <message>"
    const error = /^(?:\w*Error): .*$/m.exec(r.stderr ?? "")?.[0] ?? (r.stderr ?? "").trim();
    return { label: `${cwd}> wara ${args.join(" ")}`, value: { code: r.status, out: norm.text((r.stdout ?? "").replace(/\r\n/g, "\n")), error: norm.text(error) } };
  });
}

// ---------------------------------------------------------------- report

function report(a, b) {
  let same = 0;
  const diffs = [];
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = JSON.stringify(a[i]?.value), y = JSON.stringify(b[i]?.value);
    if (x === y) { same++; continue; }
    let j = 0;
    while (j < x?.length && x[j] === y?.[j]) j++;
    diffs.push(`DIFF ${a[i]?.label ?? b[i]?.label}\n    rust: …${String(x).slice(Math.max(0, j - 150), j + 250)}\n    node: …${String(y).slice(Math.max(0, j - 150), j + 250)}`);
  }
  return { same, total: Math.max(a.length, b.length), diffs };
}

const entries = (m) => Object.entries(m).map(([label, value]) => ({ label, value }));

let exitCode = 1;
try {
  const started = Date.now();
  const [rust, node] = await Promise.all([session(rustWs, rustHttp, join(TMP, "rust")), session(nodeWs, nodeHttp, join(TMP, "node"))]);
  const steps = report(rust, node);
  const files = report(entries(manifest(join(TMP, "rust"))), entries(manifest(join(TMP, "node"))));
  const lists = report(
    [{ label: "projects list", value: normaliser(join(TMP, "rust")).text(readFileSync(rustProjects, "utf8")) }],
    [{ label: "projects list", value: normaliser(join(TMP, "node")).text(readFileSync(nodeProjects, "utf8")) }],
  );
  const cliRust = runCli("rust", join(TMP, "rust"));
  const cli = report(cliRust, runCli("node", join(TMP, "node")));
  const filesAfterCli = report(entries(manifest(join(TMP, "rust"))), entries(manifest(join(TMP, "node"))));
  for (const d of [...steps.diffs, ...files.diffs, ...lists.diffs, ...cli.diffs, ...filesAfterCli.diffs]) console.log(d);
  if (verbose) for (const s of [...rust, ...cliRust]) console.log(`${s.label}: ${JSON.stringify(s.value).slice(0, 400)}`);
  console.log(
    `graph wiring: ${steps.same}/${steps.total} steps identical, ${files.same}/${files.total} files identical (db files as table dumps), ` +
      `projects list ${lists.diffs.length ? "differs" : "identical"}, CLI ${cli.same}/${cli.total} identical, ` +
      `${filesAfterCli.same}/${filesAfterCli.total} files identical after the CLI (:${rustHttp} vs :${nodeHttp}, ${Math.round((Date.now() - started) / 1000)} s)`,
  );
  exitCode = steps.diffs.length || files.diffs.length || lists.diffs.length || cli.diffs.length || filesAfterCli.diffs.length ? 1 : 0;
} finally {
  if (own) {
    for (const port of [rustHttp, nodeHttp]) await http(port, "POST", "/api/stop").catch(() => {});
    await Promise.all(procs.map((p) => new Promise((r) => (p.exitCode === null ? p.on("exit", r) : r()))));
  }
  if (keep) console.log(`kept ${TMP}${privateNode ? ` and ${privateNode.base}` : ""}`);
  else {
    rmSync(TMP, { recursive: true, force: true });
    privateNode?.cleanup();
  }
}
process.exit(exitCode);
