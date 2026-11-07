import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync, openSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { DatabaseSync } from "node:sqlite";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { WebSocketServer } from "ws";
import { z } from "zod";
import { graphFile, openGraph, projectRoot, updateGraph } from "./codegraph/build.mjs";
import { confirmLink, syncDesign } from "./codegraph/design.mjs";
import { graph as codeGraph } from "./codegraph/query.mjs";
import { designPdf, orderPages, pdfFileName } from "./pdf.mjs";
import { brainGraph, codeLinks, emptyTrash, readNote, restoreNote, trashNote, writeNote as writeBrainNote } from "./codegraph/brain.mjs";
import { exportData, exportGraph } from "./codegraph/export.mjs";
import { FlowConflict, checkFlow, flowsOverview, formatFlow, hashText, ignoreDraft, parseFlow, saveFlowFile, slugify, trace as traceFlow } from "./codegraph/flows.mjs";
import { addProject, isKnownProject, listProjects, removeProject } from "./codegraph/projects.mjs";
import { toGraphData } from "./graph.mjs";
import { listLottie, markSentToFigma, readLottie } from "./motion.mjs";
import { motionToSpec } from "./motion-figma.mjs";
import { classify, lint, motionContext } from "./motion-handoff.mjs";
import { compileLottie } from "./lottie.mjs";
import { createFileGuard } from "./guards.mjs";
import { loadSkills, skillIndex } from "./skills.mjs";
import { defineTools } from "./tools.mjs";

const WS_PORT = Number(process.env.WARA_PORT ?? 3155); // wridge: 3155/3156, so wara-bridge (3055/3056) can run at the same time
const HTTP_PORT = Number(process.env.WARA_HTTP_PORT ?? 3156);
const COMMAND_TIMEOUT_MS = 30_000;
const HERE = dirname(fileURLToPath(import.meta.url));
// The pages live in ../web, shared with the Rust server that replaces this one.
const WEB = join(HERE, "..", "web");
const GUIDE_PATH = join(HERE, "..", "AI_GUIDE.md");
const SKILLS_DIR = join(HERE, "..", "skills");

// Browser bundles served to graph.html (allowlist — never map arbitrary paths).
const VENDOR_FILES = {
  "/vendor/3d-force-graph.min.js": join(HERE, "node_modules", "3d-force-graph", "dist", "3d-force-graph.min.js"),
  "/vendor/force-graph.min.js": join(HERE, "node_modules", "force-graph", "dist", "force-graph.min.js"),
  // three.js as an ES module (graph3d.js needs Fog); three.module.js imports ./three.core.js.
  "/vendor/three.module.js": join(HERE, "node_modules", "three", "build", "three.module.js"),
  "/vendor/three.core.js": join(HERE, "node_modules", "three", "build", "three.core.js"),
  // /motion: Lottie player and the GIF encoder for "send to Figma"
  "/vendor/lottie.min.js": join(HERE, "node_modules", "lottie-web", "build", "player", "lottie.min.js"),
  "/vendor/gifenc.esm.js": join(HERE, "node_modules", "gifenc", "dist", "gifenc.esm.js"),
};

// Web pages and their shared UI kit, read per request so edits apply without a restart.
const PAGES = {
  "/": ["dashboard.html", "text/html"],
  "/graph": ["graph.html", "text/html"],
  "/ui.css": ["ui.css", "text/css"],
  "/ui.js": ["ui.js", "text/javascript"],
  "/code": ["code.html", "text/html"],
  "/data": ["data.html", "text/html"],
  "/flows": ["flows.html", "text/html"],
  "/github": ["github.html", "text/html"],
  "/tasks": ["tasks.html", "text/html"],
  "/reports": ["reports.html", "text/html"],
  "/report-view": ["report-view.html", "text/html"],
  "/report-render.js": ["report-render.js", "text/javascript"],
  "/skills": ["skills.html", "text/html"],
  "/motion": ["motion.html", "text/html"],
  "/pdf": ["pdf.html", "text/html"],
  "/brain": ["brain.html", "text/html"],
  // the wridge window itself, also openable in a browser for a look (no Tauri there: start/stop are off)
  "/app": ["app.html", "text/html"],
  "/app.css": ["app.css", "text/css"],
  "/app.js": ["app.js", "text/javascript"],
  "/app-icons.js": ["app-icons.js", "text/javascript"],
  "/mindmap.js": ["mindmap.js", "text/javascript"],
  "/graph3d.js": ["graph3d.js", "text/javascript"],
  "/search.js": ["search.js", "text/javascript"],
};

// Read on every use so edits to the guide apply without a restart.
function readGuide() {
  try {
    return readFileSync(GUIDE_PATH, "utf8");
  } catch {
    return "AI_GUIDE.md is missing from the wridge folder.";
  }
}

function codeFileCount(dir) {
  if (!existsSync(graphFile(dir))) return 0;
  let db;
  try {
    db = new DatabaseSync(graphFile(dir), { readOnly: true });
    return db.prepare("SELECT COUNT(*) AS n FROM files WHERE lang IN ('ts', 'dart')").get().n;
  } catch {
    return 0;
  } finally {
    db?.close();
  }
}

const ALLOWED_HOSTS = new Set([`localhost:${HTTP_PORT}`, `127.0.0.1:${HTTP_PORT}`]);
// + the wridge app's control window (Tauri serves it from tauri.localhost on Windows)
const ALLOWED_ORIGINS = new Set([...[...ALLOWED_HOSTS].map((h) => `http://${h}`), "http://tauri.localhost", "tauri://localhost"]);

const log = (...args) => console.log(`[${new Date().toLocaleTimeString("en-GB")}]`, ...args);

const state = { startedAt: Date.now(), paused: false, history: [], fileName: null, today: newDay() };

function newDay() {
  return { day: new Date().toDateString(), count: 0, errors: 0, totalMs: 0 };
}

function record(entry) {
  state.history.unshift(entry);
  if (state.history.length > 50) state.history.length = 50;
  if (state.today.day !== new Date().toDateString()) state.today = newDay();
  state.today.count++;
  state.today.totalMs += entry.ms;
  if (!entry.ok) state.today.errors++;
}

// The dashboard shows which Figma file the plugin is in; ask once per connection.
let fileNameRequest = null;
function refreshFileName() {
  if (state.fileName || fileNameRequest || !pluginConnected()) return;
  fileNameRequest = send("get_document_info", {}, 10_000)
    .then((info) => (state.fileName = info.fileName ?? null))
    .catch(() => {})
    .finally(() => (fileNameRequest = null));
}

// ---- Figma plugin (WebSocket) ----
// The plugin can be open in several files at once (a Figma design file and a FigJam board). Commands go
// to the active one: the first window to connect, or the one the AI picked with use_file. Writes still need a
// get_document_info check of that file first (guards.mjs), so switching never writes to the wrong file.
// A window that connects (or reconnects after Figma throttled it in the background) never takes over an
// open active one: only use_file switches, so the file in use never changes behind the AI's back.

const plugins = new Map(); // socket → { fileName, editorType, at }
let plugin = null; // the active socket
const pending = new Map(); // command id → { resolve, reject, timer, socket }

const wsHttp = createServer((_req, res) => res.writeHead(426).end());
const wss = new WebSocketServer({
  server: wsHttp,
  // The plugin iframe sends Origin "null"; refusing real origins stops web pages from driving Figma.
  verifyClient: ({ origin }) => !origin || origin === "null",
});

// ws re-emits the underlying server's errors; listen() below already reports them.
wss.on("error", () => {});

const isOpen = (socket) => socket && socket.readyState === socket.OPEN;
// Bumped when a window closes or use_file switches: the file guard re-reads the open file name then.
let epoch = 1;
function setActive(socket) {
  if (plugin !== socket) epoch++;
  plugin = socket;
  state.fileName = socket ? plugins.get(socket)?.fileName ?? null : null;
  notifyPlugin();
}
// What is open where, for get_document_info, use_file and the dashboard.
function openFiles() {
  return [...plugins].filter(([s]) => isOpen(s)).map(([s, info]) => ({ fileName: info.fileName, editorType: info.editorType ?? "figma", active: s === plugin }));
}

wss.on("connection", (socket) => {
  plugins.set(socket, { fileName: null, editorType: null, at: Date.now() });
  if (isOpen(plugin)) notifyPlugin();
  else setActive(socket);
  log("Figma plugin connected");

  socket.on("message", (raw) => {
    let msg;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    if (msg.type === "hello") {
      // Sent by the plugin window on connect: which file, and whether it is Figma or FigJam.
      plugins.set(socket, { ...plugins.get(socket), fileName: msg.fileName ?? null, editorType: msg.editorType ?? "figma" });
      if (socket === plugin) state.fileName = msg.fileName ?? null;
      log(`  ${msg.editorType === "figjam" ? "FigJam" : "Figma"} file: ${msg.fileName}`);
      return notifyPlugin();
    }
    if (msg.type === "show-app") return console.log("[wridge:show-window]"); // the plugin's "Open wridge": the app shows its window
    const entry = pending.get(msg.id);
    if (!entry) return;
    pending.delete(msg.id);
    clearTimeout(entry.timer);
    if (msg.error) entry.reject(new Error(msg.error));
    else entry.resolve(msg.result);
  });

  socket.on("close", () => {
    const info = plugins.get(socket);
    plugins.delete(socket);
    epoch++;
    log(`Figma plugin disconnected${info?.fileName ? ` (${info.fileName})` : ""}`);
    for (const [id, entry] of pending) {
      if (entry.socket !== socket) continue;
      clearTimeout(entry.timer);
      entry.reject(new Error("Figma plugin disconnected"));
      pending.delete(id);
    }
    if (plugin === socket) {
      // fall back to the most recently connected window that is still open
      const next = [...plugins].filter(([s]) => isOpen(s)).sort((a, b) => b[1].at - a[1].at)[0]?.[0] ?? null;
      setActive(next);
    }
  });
});

function rejectAllPending(reason) {
  for (const [id, entry] of pending) {
    clearTimeout(entry.timer);
    entry.reject(new Error(reason));
    pending.delete(id);
  }
}

const pluginConnected = () => isOpen(plugin);

// Status for every plugin window (paused, dashboard link, and whether the AI is using that file).
function notifyPlugin() {
  for (const socket of plugins.keys()) {
    if (isOpen(socket)) socket.send(JSON.stringify({ type: "status", paused: state.paused, dashboard: `http://localhost:${HTTP_PORT}`, active: socket === plugin, files: plugins.size }));
  }
}

// use_file: point commands at another open file (by name, or "figma" / "figjam").
function useFile(name) {
  const want = String(name ?? "").trim().toLowerCase();
  const match = [...plugins].filter(([s]) => isOpen(s)).find(([, i]) => (i.fileName ?? "").toLowerCase() === want)
    ?? [...plugins].filter(([s]) => isOpen(s)).find(([, i]) => (i.editorType ?? "figma") === want)
    ?? [...plugins].filter(([s]) => isOpen(s)).find(([, i]) => (i.fileName ?? "").toLowerCase().includes(want));
  if (!match) throw new Error(`No open file matches "${name}". Open: ${openFiles().map((f) => `${f.fileName} (${f.editorType})`).join(", ") || "none — run the wridge plugin in Figma or FigJam"}`);
  setActive(match[0]);
  return { active: match[1].fileName, editorType: match[1].editorType ?? "figma", openFiles: openFiles(), next: "Call get_document_info before changing anything in it." };
}

function send(command, params = {}, timeoutMs = COMMAND_TIMEOUT_MS) {
  if (!pluginConnected()) {
    return Promise.reject(
      new Error("Figma plugin is not connected — open Figma and run Plugins → Development → wridge"),
    );
  }
  const id = randomUUID();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`Figma did not respond to "${command}" within ${timeoutMs / 1000}s`));
    }, timeoutMs);
    pending.set(id, { resolve, reject, timer, socket: plugin });
    plugin.send(JSON.stringify({ id, command, params }));
  }).then((result) => {
    // With more than one file open, say which ones and which is in use.
    if (command === "get_document_info" && result && typeof result === "object" && plugins.size > 1) return { ...result, openFiles: openFiles() };
    return result;
  });
}

// ---- tools ----

// Figma writes only reach the file the AI checked (guards.mjs); plugin-side guards live in code.ts.
const TOOLS = defineTools(createFileGuard(send, { epoch: () => epoch }));
const TOOL_MAP = new Map(TOOLS.map((t) => [t.name, t]));

// Some MCP clients hand the model prefixed names (e.g. "mcp_wara-bridge_create_frame").
function findTool(name) {
  return TOOL_MAP.get(name) ?? TOOL_MAP.get(String(name).replace(/^.*?wara[-_]bridge[_:.\-/]+/i, ""));
}

// "$hero" → the hero step's id; "$hero.width" → a property; "$0.id" → by step index.
const REF = /^\$(\w+)((?:\.\w+)*)$/;

function resolveRefs(value, results) {
  if (typeof value === "string") {
    const m = REF.exec(value);
    if (!m) return value;
    if (!(m[1] in results)) throw new Error(`Unknown reference ${value} — no earlier step named "${m[1]}"`);
    let out = results[m[1]];
    if (!m[2]) return out && typeof out === "object" && "id" in out ? out.id : out;
    for (const key of m[2].slice(1).split(".")) out = out?.[key];
    if (out === undefined) throw new Error(`Reference ${value} resolved to nothing`);
    return out;
  }
  if (Array.isArray(value)) return value.map((v) => resolveRefs(v, results));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolveRefs(v, results)]));
  }
  return value;
}

function resultValue(result) {
  const first = result.content?.[0];
  if (!first) return null;
  if (first.type === "image") return "[image]";
  try {
    return JSON.parse(first.text);
  } catch {
    return first.text;
  }
}

const batchTool = {
  name: "run_batch",
  description:
    'Run many tools in order in ONE call. Name a step with "as" and reference its result later: "$hero" (its id), "$hero.width", or by index "$0". Use this to build whole sections instead of writing scripts.',
  inputSchema: {
    steps: z
      .array(
        z.object({
          tool: z.string(),
          args: z.record(z.string(), z.unknown()).default({}),
          as: z.string().regex(/^\w+$/).optional(),
        }),
      )
      .min(1)
      .max(500),
    stopOnError: z.boolean().default(true),
  },
  async run({ steps, stopOnError }) {
    const results = {};
    const summary = [];
    let failed = 0;
    for (const [i, step] of steps.entries()) {
      try {
        const t = findTool(step.tool);
        if (!t || META_TOOLS.has(t.name)) throw new Error(`Unknown tool "${step.tool}" — see describe_tool or "wara list"`);
        const parsed = z.object(t.inputSchema).safeParse(resolveRefs(step.args, results));
        if (!parsed.success) throw new Error(z.prettifyError(parsed.error));
        const result = await t.run(parsed.data);
        const value = resultValue(result);
        if (result.isError) throw new Error(String(value));
        results[i] = value;
        if (step.as) results[step.as] = value;
        summary.push({ step: i, tool: step.tool, as: step.as, id: value?.id });
      } catch (err) {
        failed++;
        summary.push({ step: i, tool: step.tool, as: step.as, error: err.message });
        if (stopOnError) break;
      }
    }
    const refs = Object.fromEntries(steps.filter((s) => s.as && results[s.as]).map((s) => [s.as, results[s.as]?.id]));
    const payload = { completed: summary.length - failed, failed, total: steps.length, refs, steps: summary };
    return {
      content: [{ type: "text", text: JSON.stringify(payload) }],
      isError: failed > 0 && stopOnError,
    };
  },
};

const callTool = {
  name: "call_tool",
  description:
    "Call any wridge tool by name, including ones not listed in this session (see the instructions for their names). Use describe_tool first if unsure of its parameters.",
  inputSchema: { name: z.string(), args: z.record(z.string(), z.unknown()).default({}) },
  async run({ name, args }) {
    const t = findTool(name);
    if (!t || META_TOOLS.has(t.name)) throw new Error(`Unknown tool "${name}" — see describe_tool`);
    const parsed = z.object(t.inputSchema).safeParse(args);
    if (!parsed.success) throw new Error(`${z.prettifyError(parsed.error)}\nExpected: ${JSON.stringify(toolSchema(t))}`);
    return t.run(parsed.data);
  },
};

const describeTool = {
  name: "describe_tool",
  description: "Show a tool's description and parameters (JSON schema). Without a name, list every tool.",
  inputSchema: { name: z.string().optional() },
  async run({ name }) {
    if (!name) return textResult(TOOLS.filter((t) => !META_TOOLS.has(t.name)).map((t) => `${t.name}: ${t.description}`).join("\n"));
    const t = findTool(name);
    if (!t) throw new Error(`Unknown tool "${name}"`);
    return textResult(JSON.stringify({ name: t.name, description: t.description, parameters: toolSchema(t) }));
  },
};

const getSkill = {
  name: "get_skill",
  description: "Load a skill (design/coding method) by name before doing the matching work. Without a name, list skills.",
  inputSchema: { name: z.string().optional() },
  async run({ name }) {
    const skills = loadSkills(SKILLS_DIR);
    if (!name) return textResult(skillIndex(skills));
    const skill = skills.find((s) => s.name === name);
    if (!skill) throw new Error(`Unknown skill "${name}". Available: ${skills.map((s) => s.name).join(", ")}`);
    return textResult(skill.body);
  },
};

const useFileTool = {
  name: "use_file",
  description: "Switch which open file wridge talks to when the plugin is open in more than one (e.g. a Figma design file and a FigJam board). Pass the file name, or \"figma\" / \"figjam\". get_document_info lists the open files. Check the file (get_document_info) again before changing it.",
  inputSchema: { file: z.string().min(1).describe('File name, or "figma" / "figjam"') },
  run: async ({ file }) => textResult(JSON.stringify(useFile(file))),
};
TOOLS.push(useFileTool);
TOOL_MAP.set(useFileTool.name, useFileTool);

const META_TOOLS = new Set([batchTool.name, callTool.name, describeTool.name, getSkill.name]);
for (const t of [batchTool, callTool, describeTool, getSkill]) {
  TOOLS.push(t);
  TOOL_MAP.set(t.name, t);
}

function textResult(text) {
  return { content: [{ type: "text", text }] };
}

function toolSchema(t) {
  return z.toJSONSchema(z.object(t.inputSchema), { io: "input" });
}

// Tool schemas are the largest fixed cost per request, so each profile advertises only what
// its kind of work needs; every other tool stays reachable through call_tool and run_batch.
const DISCOVER = ["design_map", "get_document_info", "use_file", "get_selection", "get_node", "find_nodes", "export_node_image"];
const PROFILES = {
  core: [
    ...DISCOVER, "component_usage", "check_flows", "get_styles", "get_variables", "get_components",
    "create_frame", "create_text", "create_rectangle", "create_instance", "create_from_svg",
    "set_auto_layout", "set_sizing", "set_text", "set_text_props", "set_fill", "set_props", "set_image_fill",
    "move_to_parent", "delete_node", "select_nodes", "create_section", "project_memory", "tasks", "report",
    "get_design_context", "get_tokens", "export_assets", "motion_read", "motion_animate", "motion_to_lottie", "motion_check", "motion_tokens", "motion_to_code",
    // Listed here too: sessions on this profile fix code as well, and an unlisted tool loses to Grep.
    "code_graph",
  ],
  code: [...DISCOVER, "code_graph", "code_graph_update", "code_graph_note", "create_lottie", "motion_read", "motion_animate", "motion_to_lottie", "motion_check", "motion_tokens", "motion_to_code", "code_design_sync", "project_knowledge", "project_memory", "tasks", "report", "component_usage", "get_design_context", "get_tokens", "export_assets", "get_component_map", "map_component", "get_prototype", "get_components", "get_variables", "get_styles", "scan_design_values"],
  // FigJam boards: system design with the code and data graphs at hand.
  figjam: ["get_document_info", "use_file", "figjam_diagram", "board_map", "create_sticky", "create_shape", "create_connector", "create_text", "create_section", "move_resize", "set_text", "delete_node", "code_graph", "project_memory"],
  prototype: [...DISCOVER, "check_flows", "get_prototype", "add_interaction", "remove_interactions", "set_flow_start", "set_scroll", "select_nodes", "project_memory", "code_graph"],
};

function profileTools(profile) {
  if (profile === "all") return TOOLS;
  if (profile === "design") {
    const elsewhere = new Set([...PROFILES.code, ...PROFILES.prototype].filter((n) => !PROFILES.core.includes(n)));
    return TOOLS.filter((t) => !elsewhere.has(t.name));
  }
  const names = new Set([...(PROFILES[profile] ?? PROFILES.core), ...META_TOOLS]);
  return TOOLS.filter((t) => names.has(t.name));
}

// Design sessions list design skills, code sessions code skills (area front matter); the rest stay loadable.
const PROFILE_AREA = { core: "design", prototype: "design", code: "code" };

function instructions(tools, profile) {
  const shown = new Set(tools.map((t) => t.name));
  const hidden = TOOLS.filter((t) => !shown.has(t.name)).map((t) => t.name);
  const all = loadSkills(SKILLS_DIR);
  const area = PROFILE_AREA[profile];
  const skills = area ? all.filter((s) => !s.area || s.area === "both" || s.area === area) : all;
  const others = all.length - skills.length;
  return [
    readGuide(),
    hidden.length ? `## More tools\nNot listed here but callable with call_tool / run_batch (describe_tool shows parameters): ${hidden.join(", ")}` : "",
    skills.length ? `## Skills\nLoad the matching skill with get_skill(name) before starting that kind of work:\n${skillIndex(skills)}${others ? `

${others} more skills (${area === "design" ? "code" : "design"} work) — their names are in the guide above; get_skill() lists them all.` : ""}` : "",
  ]
    .filter(Boolean)
    .join("\n\n");
}

async function runTool(t, args, source) {
  const started = Date.now();
  try {
    if (state.paused) throw new Error(`wridge is paused — resume it at http://localhost:${HTTP_PORT}`);
    const result = await t.run(args);
    // Tools like run_batch report failure in the result instead of throwing.
    const error = result.isError ? String(result.content?.[0]?.text ?? "failed").slice(0, 500) : undefined;
    record({ time: started, source, tool: t.name, ok: !result.isError, ms: Date.now() - started, error });
    return result;
  } catch (err) {
    record({ time: started, source, tool: t.name, ok: false, ms: Date.now() - started, error: err.message });
    return { content: [{ type: "text", text: err.message }], isError: true };
  }
}

function createMcpServer(profile) {
  const tools = profileTools(profile);
  const server = new McpServer({ name: "wridge", version: "1.7.4" }, { instructions: instructions(tools, profile) });
  for (const t of tools) {
    server.registerTool(t.name, { description: t.description, inputSchema: t.inputSchema }, (args) =>
      runTool(t, args, "MCP"),
    );
  }
  // Each skill is also a prompt, so users can start it explicitly with /<skill> in their client.
  for (const skill of loadSkills(SKILLS_DIR)) {
    server.registerPrompt(
      skill.name,
      { description: skill.description, argsSchema: { task: z.string().optional() } },
      ({ task }) => ({
        messages: [
          {
            role: "user",
            content: { type: "text", text: `${skill.body}\n\n---\nTask: ${task || "Ask me what to work on, then follow this skill."}` },
          },
        ],
      }),
    );
  }
  return server;
}

// ---- HTTP: dashboard, CLI API, MCP ----

async function readJson(req, limit = 1_000_000) {
  let size = 0;
  const chunks = [];
  for await (const chunk of req) {
    size += chunk.length;
    if (size > limit) throw new Error("Request body too large");
    chunks.push(chunk);
  }
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString("utf8")) : {};
}

function json(res, status, body) {
  res.writeHead(status, { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store" });
  res.end(JSON.stringify(body));
}

async function handleMcp(req, res, profile) {
  const mcp = createMcpServer(profile);
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: undefined,
    enableDnsRebindingProtection: true,
    allowedHosts: [...ALLOWED_HOSTS],
  });
  res.on("close", () => {
    transport.close();
    mcp.close();
  });
  await mcp.connect(transport);
  await transport.handleRequest(req, res);
}

const http = createServer(async (req, res) => {
  // Host check blocks DNS rebinding; Origin check blocks other websites (Claude/CLI send no Origin).
  if (!ALLOWED_HOSTS.has(req.headers.host ?? "")) return json(res, 403, { error: "forbidden host" });
  const origin = req.headers.origin;
  if (origin && !ALLOWED_ORIGINS.has(origin)) return json(res, 403, { error: "forbidden origin" });
  // the wridge window (another origin) reads the API: answer allowed origins with CORS, and their preflights
  if (origin) {
    res.setHeader("Access-Control-Allow-Origin", origin);
    res.setHeader("Vary", "Origin");
  }
  if (req.method === "OPTIONS") {
    res.writeHead(204, { "Access-Control-Allow-Methods": "GET, POST, OPTIONS", "Access-Control-Allow-Headers": "Content-Type, X-Wridge-Session", "Access-Control-Max-Age": "600" });
    return res.end();
  }

  const { pathname, searchParams } = new URL(req.url, `http://${req.headers.host}`);
  const route = `${req.method} ${pathname}`;

  try {
    if (pathname === "/mcp") return await handleMcp(req, res, searchParams.get("profile") ?? "core");

    if (req.method === "GET" && VENDOR_FILES[pathname]) {
      res.writeHead(200, { "Content-Type": "text/javascript; charset=utf-8", "Cache-Control": "max-age=86400" });
      return res.end(readFileSync(VENDOR_FILES[pathname]));
    }
    // wara-bridge's own Lottie art (server/lottie/<name>.json) for the pages' loading / success / empty states.
    const lottieName = req.method === "GET" && /^\/lottie\/([a-z0-9-]+)\.json$/.exec(pathname)?.[1];
    if (lottieName) {
      const file = join(WEB, "lottie", `${lottieName}.json`);
      if (!existsSync(file)) return json(res, 404, { error: "No such animation" });
      res.writeHead(200, { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-cache" });
      return res.end(readFileSync(file));
    }
    // The user's sticker set (web/stickers/<mood>.png) for empty / error / waiting states.
    const sticker = req.method === "GET" && /^\/stickers\/([a-z0-9-]+)\.png$/.exec(pathname)?.[1];
    if (sticker) {
      const file = join(WEB, "stickers", `${sticker}.png`);
      if (!existsSync(file)) return json(res, 404, { error: "No such sticker" });
      res.writeHead(200, { "Content-Type": "image/png", "Cache-Control": "max-age=86400" });
      return res.end(readFileSync(file));
    }

    if (req.method === "GET" && PAGES[pathname]) {
      const [file, type] = PAGES[pathname];
      res.writeHead(200, { "Content-Type": `${type}; charset=utf-8`, "Cache-Control": "no-store" });
      return res.end(readFileSync(join(WEB, file), "utf8"));
    }

    // GitHub (docs/flows/github.md) lives in the Rust engine only
    if (pathname === "/api/tasks" || pathname === "/api/tasks/wait" || pathname === "/api/reports" || pathname.startsWith("/api/reports/")) return json(res, 501, { error: "Tasks and Reports need the Rust engine — switch to Rust in Settings → Server" });
    if (pathname === "/api/github" || pathname.startsWith("/api/github/")) return json(res, 501, { error: "GitHub is not available on this engine — switch to Rust in Settings → Server" });

    switch (route) {
      // ---- code graph page (works without Figma) ----
      case "GET /api/code/projects":
        // codeFiles: parsed source files (0 for a book or a game the graph cannot read) — the menu hides the code pages then.
        return json(res, 200, listProjects().map((p) => ({ ...p, exists: existsSync(p.dir), hasGraph: existsSync(graphFile(p.dir)), codeFiles: codeFileCount(p.dir) })));

      case "POST /api/code/projects": {
        const { dir } = await readJson(req);
        const info = await updateGraph(dir); // validates the path; adds .wara/ to .gitignore before creating it
        addProject(dir);
        return json(res, 200, info);
      }

      case "POST /api/code/projects/remove": {
        // Forget the project in this list only; its .wara/ folder stays.
        removeProject((await readJson(req)).dir);
        return json(res, 200, { ok: true });
      }

      case "GET /api/code/graph": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const { db, info } = await openGraph(dir);
        try {
          return json(res, 200, { ...exportGraph(db, dir), update: info });
        } finally {
          db.close();
        }
      }

      // ---- flows page: docs/flows/*.md + drafts + live trace ----
      case "GET /api/motion": {
        // Lottie files in the project (found by content) + the motion tokens the code already defines.
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const root = projectRoot(dir);
        let tokens = [];
        try {
          const { db } = await openGraph(dir);
          try {
            tokens = exportGraph(db, root).tokens
              .filter((t) => t.value && /duration|easing|ease|motion|curve|transition/i.test(t.name))
              .map((t) => ({ name: t.name, value: t.resolved ?? t.value, defined: t.defined, count: t.count }));
          } finally {
            db.close();
          }
        } catch { /* no graph yet: the page shows the defaults */ }
        return json(res, 200, { name: root.replaceAll("\\", "/").split("/").pop(), files: listLottie(root), tokens, figmaFile: state.fileName ?? null, plugin: pluginConnected() });
      }

      case "GET /api/motion/figma-frames": {
        // Frames on the open Figma page that have a Figma Motion timeline, with the code-or-Lottie verdict.
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        const doc = await send("get_document_info");
        const frames = [];
        // frames on the page, and frames inside sections (where storyboards and flows live)
        const candidates = [];
        for (const top of doc.topLevel ?? []) {
          if (top.type !== "SECTION") { candidates.push(top); continue; }
          try { candidates.push(...((await send("get_node", { nodeId: top.id, depth: 1 })).children ?? []).map((c) => ({ ...c, section: top.name }))); } catch { /* section went away */ }
        }
        for (const top of candidates) {
          if (!["FRAME", "COMPONENT", "INSTANCE", "GROUP"].includes(top.type)) continue;
          let r;
          try { r = await send("motion_read", { nodeId: top.id }); } catch { continue; } // no Motion API here
          if (!r.animated.some((n) => n.tracks.length)) continue;
          const context = motionContext(top.section);
          const rows = classify(r, [], { context });
          frames.push({
            id: top.id, name: top.name, w: Math.round(top.width), h: Math.round(top.height),
            durationMs: Math.round((r.timelines?.[0]?.duration ?? 0) * 1000),
            rows: rows.map(({ id, name, nth, to, why, props, span }) => ({ id, name, nth, to, why, props, span })),
            lint: lint(rows, r, { context }), context, section: top.section ?? null,
          });
        }
        return json(res, 200, { file: doc.fileName, page: doc.currentPage?.name, frames });
      }

      case "GET /api/motion/figma-preview": {
        // A Lottie built in memory from a frame's Motion timeline — to watch it here, nothing is written.
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        const id = searchParams.get("id");
        if (!id) return json(res, 400, { error: "id is required" });
        const r = await send("motion_read", { nodeId: id });
        const spec = motionToSpec(r);
        const [asset] = await send("export_assets", { items: [{ nodeId: id, format: "svg", scale: 1 }] });
        const { lottie, warnings } = compileLottie({ duration: spec.duration, tracks: spec.tracks, split: spec.split, name: r.root.name, svg: Buffer.from(asset.base64, "base64").toString("utf8") });
        lottie.meta = { g: "wara-bridge", wara: { source: "figma-motion", spec: { duration: spec.duration, tracks: spec.tracks, split: spec.split, nodeId: id } } };
        return json(res, 200, { lottie, warnings: [...spec.warnings, ...warnings] });
      }

      case "POST /api/pick-folder": {
        // Windows' own "Select Folder" dialog (File Explorer style, folder-picker.ps1): a browser can pick a
        // folder but never tells the real path.
        if (process.platform !== "win32") return json(res, 501, { error: "Folder dialog is only available on Windows" });
        const { start, title } = await readJson(req);
        const picked = await new Promise((done) => {
          const args = ["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File", join(HERE, "folder-picker.ps1"), "-Title", String(title ?? "Choose a folder"), "-Start", String(start ?? "")];
          const p = spawn("powershell.exe", args, { windowsHide: true });
          let out = "";
          p.stdout.on("data", (b) => (out += b));
          p.on("close", () => done(out.trim() || null));
          p.on("error", () => done(null));
        });
        return json(res, 200, { path: picked ? picked.replaceAll("\\", "/") : null });
      }

      case "POST /api/motion/figma-to-lottie": {
        // The page's "convert" button: the same motion_to_lottie the AI uses, written into the project.
        // The page asks before replacing a file that exists (overwrite: true).
        const { dir, id, out, overwrite } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        if (!id || !/\.json$/i.test(String(out ?? ""))) return json(res, 400, { error: "id and out (….json) are required" });
        const root = projectRoot(dir);
        if (!overwrite && existsSync(resolve(root, out))) return json(res, 409, { error: "exists", path: out });
        const result = await runTool(TOOL_MAP.get("motion_to_lottie"), { projectDir: root, out, nodeId: id }, "motion");
        const text = result.content?.[0]?.text ?? "";
        if (result.isError) return json(res, 400, { error: text });
        return json(res, 200, { path: out.replaceAll("\\", "/"), report: text });
      }

      case "GET /api/motion/file": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        return json(res, 200, readLottie(projectRoot(dir), searchParams.get("path") ?? ""));
      }

      case "POST /api/motion/figma": {
        // A GIF of the animation (made in the page) into a Lottie/<name> component, so it moves in Figma's
        // Present view. The page first asks which file is open (no confirmFile) and confirms it with the user.
        const { dir, path, gif, confirmFile } = await readJson(req, 30_000_000);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected — open the design file and run the wridge plugin" });
        const call = async (name, args) => {
          const out = await runTool(TOOL_MAP.get(name), args, "motion");
          const body = out.content?.[0]?.text ?? "";
          if (out.isError) throw new Error(body);
          try { return JSON.parse(body); } catch { return body; }
        };
        const doc = await call("get_document_info", {});
        if (!confirmFile || confirmFile !== doc.fileName) return json(res, 409, { file: doc.fileName, error: confirmFile ? "Another Figma file is open now" : "Confirm the Figma file first" });
        const root = projectRoot(dir);
        const anim = readLottie(root, path);
        const name = `Lottie/${path.split("/").pop().replace(/\.json$/i, "")}`;
        const gifPath = join(tmpdir(), `wara-${randomUUID()}.gif`);
        writeFileSync(gifPath, Buffer.from(String(gif ?? ""), "base64"));
        try {
          // Same component again (in this file), else a new one next to the art it was made from.
          let componentId = null;
          const known = anim.meta?.wara?.figma;
          if (known?.componentId && known.file === doc.fileName) {
            try { const n = await call("get_node", { nodeId: known.componentId, depth: 0 }); if (n.type === "COMPONENT") componentId = n.id; } catch { /* deleted in Figma */ }
          }
          let created = false;
          if (!componentId) {
            // Next to everything on the page unless the art it came from is known (then right of the art).
            let x = Math.max(0, ...(doc.topLevel ?? []).map((n) => n.x + n.width)) + 200, y = 0;
            const source = anim.meta?.wara?.spec?.nodeId;
            // get_node gives x/y in the parent's space: only a top-level art frame says where it is on the page.
            const top = (doc.topLevel ?? []).find((n) => n.id === source);
            if (top) { x = top.x + top.width + 80; y = top.y; }
            const frame = await call("create_frame", { name, width: anim.w, height: anim.h, layoutMode: "NONE", x, y });
            await call("set_image_fill", { nodeId: frame.id, source: gifPath, scaleMode: "FIT" });
            const comp = await call("create_component", { nodeId: frame.id, name, description: `lottie: ${path}\nGIF preview for Figma (moves in Present). The app plays the .json file.` });
            componentId = comp.id;
            created = true;
          } else {
            await call("set_image_fill", { nodeId: componentId, source: gifPath, scaleMode: "FIT" });
          }
          const file = markSentToFigma(root, path, { componentId, file: doc.fileName });
          return json(res, 200, { ok: true, created, componentId, figmaFile: doc.fileName, file });
        } finally {
          try { unlinkSync(gifPath); } catch { /* already gone */ }
        }
      }

      case "GET /api/flows": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const { db, info } = await openGraph(dir);
        try {
          return json(res, 200, { name: projectRoot(dir).replaceAll("\\", "/").split("/").pop(), ...flowsOverview(codeGraph(db), projectRoot(dir)), update: info });
        } finally {
          db.close();
        }
      }

      case "GET /api/flows/trace": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const { db } = await openGraph(dir);
        try {
          return json(res, 200, traceFlow(codeGraph(db), searchParams.get("target") ?? ""));
        } finally {
          db.close();
        }
      }

      case "PUT /api/flows": {
        // Save one flow file. baseHash = the file's hash when it was opened (null for a new file): if the
        // file changed meanwhile (AI, VS Code, a teammate) nothing is overwritten — 409 with the current text.
        const { dir, file, flow, content, baseHash } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const root = projectRoot(dir);
        const text = content ?? formatFlow(flow ?? {});
        const name = file ?? `${slugify(flow?.title ?? parseFlow(text).title)}.md`;
        try {
          const saved = saveFlowFile(root, name, text, baseHash ?? null);
          const { db } = await openGraph(dir);
          try {
            const parsed = parseFlow(text);
            return json(res, 200, { ...saved, ...parsed, check: checkFlow(codeGraph(db), parsed) });
          } finally {
            db.close();
          }
        } catch (err) {
          if (err instanceof FlowConflict) return json(res, 409, { error: err.message, current: err.current, currentHash: err.current == null ? null : hashText(err.current) });
          throw err;
        }
      }

      case "POST /api/flows/ignore": {
        const { dir, id } = await readJson(req);
        if (!dir || !isKnownProject(dir) || !id) return json(res, 400, { error: "dir and id are required" });
        ignoreDraft(projectRoot(dir), String(id));
        return json(res, 200, { ok: true });
      }

      case "GET /api/code/data": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const { db, info } = await openGraph(dir);
        try {
          return json(res, 200, { ...exportData(db, dir), update: info });
        } finally {
          db.close();
        }
      }

      case "POST /api/code/design": {
        // Copy the open Figma file's components + tokens into this project's graph.
        const { dir } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected — open the design file and run the wridge plugin" });
        const { db } = await openGraph(dir);
        try {
          return json(res, 200, await syncDesign(db, send));
        } finally {
          db.close();
        }
      }

      case "POST /api/code/open": {
        // Open a project file in VS Code from the server (a vscode:// link makes the browser ask for permission every time).
        const { dir, path, line } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const root = projectRoot(dir);
        const file = resolve(root, String(path ?? ""));
        if (!file.startsWith(root) || !existsSync(file)) return json(res, 400, { error: "File is not inside this project" });
        // Windows needs a shell to run code.cmd: quote the path and refuse characters cmd.exe would interpret.
        if (/["%^&|<>]/.test(file)) return json(res, 400, { error: "This file name can't be opened safely from here" });
        const target = `${file}${Number(line) > 0 ? `:${Number(line)}` : ""}`;
        const win = process.platform === "win32";
        const child = spawn("code", ["-g", win ? `"${target}"` : target], { shell: win, windowsHide: true, stdio: "ignore" });
        // The launcher hands the file to VS Code and exits; a non-zero exit means "code" isn't installed.
        const ok = await new Promise((done) => {
          const timer = setTimeout(() => done(true), 5000);
          child.once("error", () => { clearTimeout(timer); done(false); });
          child.once("exit", (code) => { clearTimeout(timer); done(code === 0); });
        });
        return ok ? json(res, 200, { opened: target.replaceAll("\\", "/") }) : json(res, 503, { error: "VS Code's \"code\" command was not found — in VS Code run: Shell Command: Install 'code' command in PATH" });
      }

      case "POST /api/code/link": {
        // Confirm a Figma component ↔ code component pair (writes wara.components.json in the project).
        const { dir, figmaName, symbolId } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project in the code graph page first" });
        const { db } = await openGraph(dir);
        try {
          return json(res, 200, await confirmLink(codeGraph(db), projectRoot(dir), String(figmaName), Number(symbolId)));
        } finally {
          db.close();
        }
      }

      // ---- brain: a project's Markdown notes (brain/), their links, and the trash ----
      case "GET /api/brain": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, brainGraph(projectRoot(dir)));
      }

      case "GET /api/brain/code-links": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, codeLinks(projectRoot(dir)));
      }

      case "GET /api/brain/note": {
        const dir = searchParams.get("dir");
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, readNote(projectRoot(dir), searchParams.get("name") ?? ""));
      }

      case "POST /api/brain/note": {
        const { dir, name, content, mode } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, writeBrainNote(projectRoot(dir), name, { text: content, mode, keepOld: false }));
      }

      case "POST /api/brain/trash": {
        const { dir, name } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, trashNote(projectRoot(dir), name));
      }

      case "POST /api/brain/restore": {
        const { dir, path } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, restoreNote(projectRoot(dir), path));
      }

      case "POST /api/brain/empty-trash": {
        // Permanent: only the page's own button (after the user confirms) calls this; no AI tool does.
        const { dir, path } = await readJson(req);
        if (!dir || !isKnownProject(dir)) return json(res, 404, { error: "Add this project first" });
        return json(res, 200, emptyTrash(projectRoot(dir), path));
      }

      // ---- PDF page: a section's frames as the pages of one document ----
      case "GET /api/pdf/sections": {
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        const doc = await send("get_document_info");
        const sections = [];
        for (const top of doc.topLevel ?? []) {
          if (top.type !== "SECTION") continue;
          let node;
          try { node = await send("get_node", { nodeId: top.id, depth: 1 }); } catch { continue; } // section went away
          const { pages, warnings, issues } = orderPages(node.children);
          if (pages.length) sections.push({ id: top.id, name: top.name, pages, warnings, issues });
        }
        return json(res, 200, { file: doc.fileName, page: doc.currentPage?.name, sections });
      }

      case "GET /api/pdf/download": {
        // Built in memory and sent as a download; nothing is written on this machine.
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        // id=<one> or ids=<a,b,…>: several sections become one document, in the order given
        const ids = (searchParams.get("ids") ?? searchParams.get("id") ?? "").split(",").map((s) => s.trim()).filter(Boolean);
        if (!ids.length) return json(res, 400, { error: "id or ids is required" });
        const { pdf, title } = await designPdf(send, ids);
        const name = pdfFileName(title);
        res.writeHead(200, {
          "Content-Type": "application/pdf",
          "Content-Length": pdf.length,
          "Content-Disposition": `attachment; filename="${name.replace(/[^\x20-\x7e]/g, "_")}"; filename*=UTF-8''${encodeURIComponent(name)}`,
          "Cache-Control": "no-store",
        });
        return res.end(pdf);
      }

      case "GET /api/thumb": {
        // Screen preview for the graph side panel; not recorded in the command log.
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        const id = searchParams.get("id");
        if (!id) return json(res, 400, { error: "id is required" });
        const { base64 } = await send("export_node_image", { nodeId: id, scale: 1, maxSize: 640 }, 60_000);
        res.writeHead(200, { "Content-Type": "image/png", "Cache-Control": "no-store" });
        return res.end(Buffer.from(base64, "base64"));
      }

      case "GET /api/outline": {
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected" });
        const id = searchParams.get("id");
        if (!id) return json(res, 400, { error: "id is required" });
        const depth = Math.min(6, Math.max(1, Number(searchParams.get("depth")) || 3));
        return json(res, 200, await send("frame_outline", { nodeId: id, depth, maxChildren: 40 }, 60_000));
      }

      case "GET /api/graph": {
        if (!pluginConnected()) return json(res, 503, { error: "Figma plugin is not connected — run Plugins → Development → wridge" });
        const pageId = searchParams.get("pageId") || undefined;
        const refresh = searchParams.get("refresh") === "1";
        const map = await send("design_map", { pageId, refresh }, 120_000);
        let flows = null;
        try {
          flows = await send("check_flows", { pageId, limit: 1 }, 120_000);
        } catch {
          // Issue highlighting is optional; the graph still renders without it.
        }
        return json(res, 200, toGraphData(map, flows));
      }

      case "GET /guide":
        res.writeHead(200, { "Content-Type": "text/plain; charset=utf-8", "Cache-Control": "no-store" });
        return res.end(readGuide());

      // ---- skills page (read-only; works without Figma) ----
      case "GET /api/skills":
        return json(
          res,
          200,
          loadSkills(SKILLS_DIR).map(({ name, description, area, file, tokens, body }) => ({
            name,
            description,
            area: area === "design" || area === "code" ? area : "both", // no area = listed in every session
            file: `skills/${file}`,
            tokens,
            body,
          })),
        );

      case "POST /api/skills/open": {
        // Open one of the shipped skill files in VS Code. Only names from the skills list are accepted.
        const { file } = await readJson(req);
        const skill = loadSkills(SKILLS_DIR).find((s) => `skills/${s.file}` === file || s.file === file);
        if (!skill) return json(res, 404, { error: "No such skill" });
        const path = join(SKILLS_DIR, skill.file);
        if (/["%^&|<>]/.test(path)) return json(res, 400, { error: "This file name can't be opened safely from here" });
        const win = process.platform === "win32";
        const child = spawn("code", ["-g", win ? `"${path}"` : path], { shell: win, windowsHide: true, stdio: "ignore" });
        const ok = await new Promise((done) => {
          const timer = setTimeout(() => done(true), 5000);
          child.once("error", () => { clearTimeout(timer); done(false); });
          child.once("exit", (code) => { clearTimeout(timer); done(code === 0); });
        });
        return ok ? json(res, 200, { opened: path.replaceAll("\\", "/") }) : json(res, 503, { error: "VS Code's \"code\" command was not found — in VS Code run: Shell Command: Install 'code' command in PATH" });
      }

      case "GET /api/status":
        refreshFileName();
        if (state.today.day !== new Date().toDateString()) state.today = newDay();
        return json(res, 200, {
          version: "1.7.4",
          fileName: state.fileName,
          openFiles: openFiles(),
          today: state.today,
          coreToolCount: profileTools("core").length,
          skillCount: loadSkills(SKILLS_DIR).length,
          guidePath: GUIDE_PATH.replaceAll("\\", "/"),
          cliPath: join(HERE, "wara.mjs").replaceAll("\\", "/"),
          pid: process.pid,
          startedAt: state.startedAt,
          paused: state.paused,
          pluginConnected: pluginConnected(),
          ports: { plugin: WS_PORT, http: HTTP_PORT },
          toolCount: TOOLS.length,
          history: state.history,
        });

      case "GET /api/tools":
        return json(
          res,
          200,
          TOOLS.map((t) => ({
            name: t.name,
            description: t.description,
            params: z.toJSONSchema(z.object(t.inputSchema), { io: "input" }),
          })),
        );

      case "POST /api/tool": {
        const { name, args } = await readJson(req);
        const t = findTool(name);
        if (!t) return json(res, 404, { isError: true, content: [{ type: "text", text: `Unknown tool "${name}"` }] });
        const parsed = z.object(t.inputSchema).safeParse(args ?? {});
        if (!parsed.success) {
          return json(res, 400, { isError: true, content: [{ type: "text", text: z.prettifyError(parsed.error) }] });
        }
        return json(res, 200, await runTool(t, parsed.data, "CLI"));
      }

      case "POST /api/pause": {
        const { paused } = await readJson(req);
        state.paused = Boolean(paused);
        log(state.paused ? "Paused AI commands" : "Resumed AI commands");
        notifyPlugin();
        return json(res, 200, { paused: state.paused });
      }

      case "POST /api/restart":
        json(res, 200, { restarting: true });
        return restart();

      case "POST /api/stop":
        json(res, 200, { stopping: true });
        log("Stopped from dashboard");
        return shutdown();

      default:
        if (req.method === "GET" && !pathname.startsWith("/api/") && (req.headers.accept ?? "").includes("text/html")) {
          res.writeHead(404, { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" });
          return res.end(readFileSync(join(WEB, "404.html"), "utf8"));
        }
        return json(res, 404, { error: "not found" });
    }
  } catch (err) {
    log(`${route} failed: ${err.message}`);
    if (!res.headersSent) json(res, 500, { error: err.message });
  }
});

// ---- lifecycle ----

function shutdown() {
  rejectAllPending("wridge server stopped");
  for (const socket of plugins.keys()) socket.close();
  wss.close();
  wsHttp.close();
  http.close();
  setTimeout(() => process.exit(0), 300);
}

function restart() {
  log("Restarting…");
  // Run by the wridge app: it owns the process and the log, so hand the restart to it (exit code 75).
  if (process.env.WRIDGE_MANAGED) {
    shutdown();
    setTimeout(() => process.exit(75), 250); // before shutdown()'s own exit(0) at 300 ms
    return;
  }
  // Started by start-wara.vbs, wara.log is held open by `cmd >> wara.log` (EBUSY on Windows): the new server
  // then writes through the same, inherited handle.
  let out = "inherit";
  try { out = openSync(join(HERE, "wara.log"), "a"); } catch { /* keep "inherit" */ }
  spawn(process.execPath, [fileURLToPath(import.meta.url)], {
    cwd: HERE,
    detached: true,
    windowsHide: true,
    stdio: ["ignore", out, out],
    env: { ...process.env, WARA_WAIT_FOR_PORTS: "1" },
  }).unref();
  shutdown();
}

function listen(server, port, label) {
  return new Promise((resolve) => {
    let retries = process.env.WARA_WAIT_FOR_PORTS ? 40 : 0;
    const attempt = () => server.listen(port, "127.0.0.1");
    server.once("listening", resolve);
    server.on("error", (err) => {
      if (err.code !== "EADDRINUSE") return log(`${label} error: ${err.message}`);
      if (retries-- > 0) return setTimeout(attempt, 250);
      console.error(
        `Port ${port} (${label}) is already in use — wridge is probably already running: http://localhost:${HTTP_PORT}`,
      );
      process.exit(1);
    });
    attempt();
  });
}

// Run by the wridge app: if the app goes away without stopping us (killed, crashed), stop too, so no
// orphan keeps the ports.
if (process.env.WRIDGE_PARENT_PID) {
  const parent = Number(process.env.WRIDGE_PARENT_PID);
  setInterval(() => {
    try { process.kill(parent, 0); } catch { log("wridge app is gone — stopping"); shutdown(); }
  }, 2000).unref();
}

await listen(wsHttp, WS_PORT, "Figma plugin");
await listen(http, HTTP_PORT, "dashboard/MCP");
log(`wridge running (pid ${process.pid})`);
log(`  dashboard   http://localhost:${HTTP_PORT}`);
log(`  MCP         http://127.0.0.1:${HTTP_PORT}/mcp`);
log(`  plugin      ws://localhost:${WS_PORT}`);
