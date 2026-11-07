// The plugin path end to end: a fake plugin (fake-plugin.mjs) answers commands while tools run through
// /api/tool and MCP, the HTTP routes ask the plugin, a second window (FigJam) joins and use_file switches.
//   node server/app/parity/plugin-roundtrip.mjs 3255 3256                  → run against the Rust server
//   node server/app/parity/plugin-roundtrip.mjs 3255 3256 3355 3356        → also against a second server
//                                                                           (a private Node instance) and diff
// Never use the user's ports (3155/3156, 3055/3056): refused.
import { request } from "node:http";
import { startFakePlugin } from "./fake-plugin.mjs";

const ports = process.argv.slice(2).map(Number);
if (ports.some((p) => [3155, 3156, 3055, 3056].includes(p))) throw new Error("refusing the user's server ports");
const [rustWs = 3255, rustHttp = 3256, otherWs, otherHttp] = ports;

function http(port, method, path, body, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = request({ host: "127.0.0.1", port, method, path, headers: { host: `127.0.0.1:${port}`, "content-type": "application/json", accept: "application/json, text/event-stream", ...headers } }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const buf = Buffer.concat(chunks);
        const type = String(res.headers["content-type"] ?? "");
        let out;
        if (type.startsWith("application/json")) out = JSON.parse(buf.toString("utf8"));
        else if (type.startsWith("text/event-stream")) out = buf.toString("utf8").split("\n").filter((l) => l.startsWith("data: ")).map((l) => JSON.parse(l.slice(6)));
        else if (type.startsWith("image/")) out = `${type} ${buf.length} bytes ${buf.toString("base64").slice(0, 24)}`;
        else out = buf.toString("utf8");
        resolve({ status: res.statusCode, body: out });
      });
    });
    req.on("error", reject);
    if (body !== undefined) req.write(JSON.stringify(body));
    req.end();
  });
}

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// Normalise what differs between runs: request ids, times, pids, ports.
function clean(v) {
  return JSON.parse(JSON.stringify(v, (k, x) => {
    if (["pid", "startedAt", "time", "ms", "totalMs", "cliPath"].includes(k)) return `<${k}>`;
    if (k === "ports") return "<ports>";
    if (k === "dashboard" && typeof x === "string") return x.replace(/\d+$/, "<port>");
    // the private Node server runs from a temp copy of legacy-node (private-node.mjs): the file name only
    if (k === "guidePath" && typeof x === "string") return x.replace(/^.*\//, "<root>/");
    if (typeof x === "string") return x.replace(/localhost:\d{4}/g, "localhost:<port>");
    return x;
  }));
}

async function session(wsPort, httpPort) {
  const out = [];
  const step = async (label, fn) => {
    try {
      out.push({ label, value: clean(await fn()) });
    } catch (err) {
      out.push({ label, value: `THREW ${err.message}` });
    }
  };
  const tool = (name, args) => http(httpPort, "POST", "/api/tool", { name, args });
  let mcpId = 0;
  const mcp = (name, args, profile = "all") => http(httpPort, "POST", `/mcp?profile=${profile}`, { jsonrpc: "2.0", id: ++mcpId, method: "tools/call", params: { name, arguments: args } });

  await step("status before plugin", () => http(httpPort, "GET", "/api/status"));
  await step("tool without plugin", () => tool("get_document_info", {}));
  const a = await startFakePlugin(wsPort, { fileName: "utils" });
  await wait(100);
  await step("plugin A first status", () => a.statuses[0]);
  await step("status triggers file name refresh", () => http(httpPort, "GET", "/api/status"));
  await wait(150);
  await step("status after refresh", () => http(httpPort, "GET", "/api/status"));
  await step("write before check (guard)", () => tool("create_frame", { name: "Card" }));
  await step("get_document_info", () => tool("get_document_info", {}));
  await step("create_frame with defaults", () => tool("create_frame", { name: "Card", fill: "#ffffff", extra: "dropped" }));
  await step("create_text via MCP", () => mcp("create_text", { text: "สวัสดี", lineHeight: "150%", parentId: "9:100" }));
  await step("create_instance needs id", () => tool("create_instance", {}));
  await step("add_interaction needs target", () => tool("add_interaction", { nodeId: "1:1" }));
  await step("add_interaction URL", () => tool("add_interaction", { nodeId: "1:1", action: "URL", url: "  https://example.com/x  " }));
  await step("create_from_svg needs svg", () => tool("create_from_svg", { svg: "" }));
  await step("create_from_svg not svg", () => tool("create_from_svg", { svg: "<div/>" }));
  await step("create_from_svg", () => tool("create_from_svg", { svg: "<svg viewBox='0 0 1 1'></svg>", color: "#000000", file: "ignored" }));
  await step("run_batch with refs", () => tool("run_batch", { steps: [
    { tool: "create_frame", as: "hero", args: { name: "Hero", widthSizing: "FILL" } },
    { tool: "create_text", args: { parentId: "$hero", text: "Hi", width: "$hero.params.width" } },
    { tool: "mcp_wara-bridge_select_nodes", args: { nodeIds: ["$hero", "$1"] } },
    { tool: "get_node", as: "missing", args: { nodeId: "boom" } },
    { tool: "get_node", args: { nodeId: "$nope" } },
  ], stopOnError: false }));
  await step("run_batch stops on error", () => tool("run_batch", { steps: [{ tool: "delete_node", args: { nodeId: "boom" } }, { tool: "get_selection" }] }));
  await step("call_tool forwards", () => tool("call_tool", { name: "create_rectangle", args: { cornerRadius: 4 } }));
  await step("call_tool invalid args", () => tool("call_tool", { name: "create_rectangle", args: { cornerRadius: -4 } }));
  await step("get_selection via MCP", () => mcp("get_selection", {}));
  await step("run_batch via MCP", () => mcp("run_batch", { steps: [{ tool: "create_ellipse", as: "dot", args: {} }, { tool: "set_fill", args: { nodeId: "$dot", color: "none" } }] }));
  await step("run_batch via MCP fails", () => mcp("run_batch", { steps: [{ tool: "get_node", args: { nodeId: "boom" } }] }));
  await step("call_tool via MCP", () => mcp("call_tool", { name: "wara_bridge.create_polygon", args: { pointCount: 6 } }, "core"));
  await step("get_node via MCP (core)", () => mcp("get_node", { nodeId: "1:1" }, "core"));
  await step("export_node_image via MCP", () => mcp("export_node_image", { nodeId: "1:1", maxSize: 256 }));
  await step("design_map text", () => mcp("design_map", {}));
  await step("design_map json", () => mcp("design_map", { format: "json", pageId: "0:1" }));
  await step("design_map maxScreens 1", () => tool("design_map", { maxScreens: 1 }));
  await step("check_flows", () => tool("check_flows", { limit: 3 }));
  await step("component_usage", () => tool("component_usage", { component: "Button" }));
  await step("plugin error", () => tool("delete_node", { nodeId: "boom" }));
  await step("set_image_fill bad file", () => tool("set_image_fill", { nodeId: "1:1", source: "C:/no/such/file.png" }));
  await step("set_effects defaults", () => tool("set_effects", { nodeId: "1:1", shadows: [{}, { color: "#00000080", inner: true }] }));
  await step("create_variables", () => tool("create_variables", { collectionId: "c", variables: [{ name: "Color/A", type: "COLOR", values: { Light: "#ffffff", Dark: { alias: "Color/B" } } }] }));
  await step("get_design_context nodeId", () => tool("get_design_context", { nodeId: "1:1" }));
  await step("get_design_context nothing", () => tool("get_design_context", {}));
  await step("GET /api/graph", () => http(httpPort, "GET", "/api/graph"));
  await step("GET /api/graph pageId", () => http(httpPort, "GET", "/api/graph?pageId=0:1&refresh=1"));
  await step("GET /api/thumb", () => http(httpPort, "GET", "/api/thumb?id=1:1"));
  await step("GET /api/thumb no id", () => http(httpPort, "GET", "/api/thumb"));
  await step("GET /api/outline clamp", () => http(httpPort, "GET", "/api/outline?id=1:1&depth=9"));
  await step("GET /api/outline default", () => http(httpPort, "GET", "/api/outline?id=1:1&depth=abc"));
  await step("GET /api/motion/figma-frames (no Motion API)", () => http(httpPort, "GET", "/api/motion/figma-frames"));
  await step("GET /api/motion/figma-preview no id", () => http(httpPort, "GET", "/api/motion/figma-preview"));
  await step("GET /api/pdf/download no id", () => http(httpPort, "GET", "/api/pdf/download?ids=,,"));

  // a FigJam window joins: commands still go to the first one
  const b = await startFakePlugin(wsPort, { fileName: "Board", editorType: "figjam" });
  await wait(100);
  await step("plugin B first status", () => b.statuses[0]);
  await step("plugin A status after B joined", () => a.statuses.at(-1));
  await step("get_document_info lists open files", () => tool("get_document_info", {}));
  await step("use_file no match", () => tool("use_file", { file: "nothing" }));
  await step("use_file figjam", () => tool("use_file", { file: "figjam" }));
  await step("write right after switching (guard)", () => tool("create_sticky", { text: "hi" }));
  await step("check the board", () => tool("get_document_info", {}));
  await step("write to the board", () => tool("create_sticky", { text: "hi", color: "#FFEE00" }));
  await step("use_file by name part", () => tool("use_file", { file: "UTIL" }));
  await step("status with two files", () => http(httpPort, "GET", "/api/status"));
  await step("pause", () => http(httpPort, "POST", "/api/pause", { paused: 1 }));
  await step("plugin status while paused", () => a.statuses.at(-1));
  await step("tool while paused", () => tool("get_selection", {}));
  await step("MCP while paused", () => mcp("get_selection", {}));
  await step("resume", () => http(httpPort, "POST", "/api/pause", { paused: false }));
  if (process.env.PARITY_SLOW) await step("plugin never answers (30 s timeout)", () => tool("scan_design_values", { nodeId: "hang" }));
  await a.close();
  await wait(150);
  const pendingCall = tool("scan_design_values", { nodeId: "hang" });
  await wait(150);
  await b.close();
  await step("plugin closed while a command waits", () => pendingCall);
  const b2 = await startFakePlugin(wsPort, { fileName: "Board", editorType: "figjam" });
  await wait(100);
  await step("B takes over after A closed", () => b.statuses.at(-1));
  await step("status after A closed", () => http(httpPort, "GET", "/api/status"));
  await step("plugin A received", () => a.received);
  await step("plugin B received", () => b.received);
  await b2.close();
  await wait(100);
  await step("status after both closed", () => http(httpPort, "GET", "/api/status"));
  await step("WS port answers plain HTTP with 426", () => http(wsPort, "GET", "/"));
  return out;
}

if (!otherWs) {
  const rust = await session(rustWs, rustHttp);
  for (const s of rust) console.log(`${s.label}: ${JSON.stringify(s.value).slice(0, 400)}`);
  process.exit(0);
}
const [rust, other] = await Promise.all([session(rustWs, rustHttp), session(otherWs, otherHttp)]);
let same = 0;
for (let i = 0; i < rust.length; i++) {
  const x = JSON.stringify(rust[i].value), y = JSON.stringify(other[i]?.value);
  if (x === y) { same++; continue; }
  let j = 0;
  while (j < x.length && x[j] === y[j]) j++;
  console.log(`DIFF ${rust[i].label}\n    rust : …${x.slice(Math.max(0, j - 80), j + 160)}\n    other: …${String(y).slice(Math.max(0, j - 80), j + 160)}`);
}
console.log(`plugin round trip: ${same}/${rust.length} identical (:${rustHttp} vs :${otherHttp})`);
process.exit(same === rust.length ? 0 : 1);
