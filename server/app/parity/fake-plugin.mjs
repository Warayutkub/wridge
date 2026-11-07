// A fake Figma plugin window: connects to a server's plugin WebSocket like figma-plugin/ui.html does
// (hello on connect, { id, result } / { id, error } replies) and answers commands from canned data.
// Never point it at the user's servers: ports 3155 and 3055 are refused.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const require = createRequire(join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "legacy-node", "package.json"));
const WebSocket = require("ws");

// 1×1 transparent PNG
export const PNG_1PX = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

export function fakeAnswers(fileName) {
  let next = 100;
  const doc = {
    fileName,
    currentPage: { id: "0:1", name: "Page 1" },
    pages: [{ id: "0:1", name: "Page 1" }],
    topLevel: [
      { id: "1:1", name: "Home", type: "FRAME", x: 0, y: 0, width: 375.4, height: 812 },
      { id: "1:2", name: "Detail", type: "FRAME", x: 500, y: 0, width: 375, height: 812 },
      { id: "5:1", name: "Book · Slides · Draft", type: "SECTION", x: 0, y: 1000, width: 2000, height: 1200 },
    ],
  };
  const designMap = {
    file: fileName,
    foundations: false,
    pages: [{ id: "0:1", name: "Page 1", flows: [{ name: "Main", nodeId: "1:1" }], screens: [
      { id: "1:1", name: "Home", size: "375×812", status: "review", components: [["Button", 2], ["Card", 1]], to: ["Detail"], toIds: ["1:2"] },
      { id: "1:2", name: "Detail", size: "375×812", components: [], to: [], toIds: [] },
    ] }],
    components: [["Button", 2, 1], ["Card", 1, 1]],
    unusedComponents: ["Tag"],
    breakpoints: { Home: { base: "1:1", md: "1:2" } },
    otherWork: [{ name: "Book", kind: "slides" }],
  };
  return (command, params) => {
    switch (command) {
      case "get_document_info": return doc;
      case "file_name": return { fileName };
      case "design_map": return designMap;
      case "check_flows": return { deadEnds: [{ id: "1:2", name: "Detail" }], orphans: [], unreachable: [], broken: [], noInteraction: [] };
      case "get_node":
        if (params.nodeId === "5:1") return { id: "5:1", name: "Book · Slides · Draft", type: "SECTION", children: [{ id: "6:1", name: "2 Inside", type: "FRAME", x: 0, y: 0, width: 595, height: 842, visible: true }, { id: "6:2", name: "1 Cover", type: "FRAME", x: 700, y: 0, width: 595, height: 842, visible: true }] };
        if (params.nodeId === "boom") throw new Error("Node boom not found");
        return { id: params.nodeId, name: "Node", type: "FRAME", depth: params.depth, width: 100.5, height: 0.1 + 0.2 };
      case "get_selection": return [{ id: "1:1", name: "Home", type: "FRAME", depth: params.depth }];
      case "export_node_image": return { base64: PNG_1PX, width: 1, height: 1 };
      case "frame_outline": return { id: params.nodeId, depth: params.depth, maxChildren: params.maxChildren, outline: "Home\n  Header" };
      case "motion_read": throw new Error("Figma Motion is not available in this editor");
      case "delete_node":
        if (params.nodeId === "boom") throw new Error("Node boom not found");
        return { deleted: params.nodeId };
      case "set_image_fill": return { nodeId: params.nodeId, imageHash: "abc", base64Length: params.base64.length };
      case "scan_design_values":
        if (params.nodeId === "hang") return new Promise(() => {}); // never answers
        return { colors: [] };
      case "get_tokens": return { sources: ["variables"], variables: [] };
      default:
        if (/^(create|duplicate|group|combine)_/.test(command)) return { id: `9:${next++}`, name: params.name ?? command, type: "FRAME", params };
        return { ok: true, command, params };
    }
  };
}

/** Connects; resolves once the server has sent its first status message. */
export function startFakePlugin(port, { fileName = "utils", editorType = "figma", answer = fakeAnswers(fileName) } = {}) {
  if (port === 3155 || port === 3055) throw new Error("never connect a fake plugin to the user's servers");
  const received = [];
  const statuses = [];
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://localhost:${port}`);
    ws.on("error", reject);
    ws.on("open", () => ws.send(JSON.stringify({ type: "hello", fileName, editorType })));
    let ready = false;
    ws.on("message", async (raw) => {
      const msg = JSON.parse(raw.toString());
      if (msg.type === "status") {
        statuses.push(msg);
        if (!ready) {
          ready = true;
          resolve({ ws, received, statuses, close: () => new Promise((r) => { ws.once("close", r); ws.close(); }) });
        }
        return;
      }
      received.push({ command: msg.command, params: msg.params });
      try {
        const result = await answer(msg.command, msg.params ?? {});
        ws.send(JSON.stringify({ id: msg.id, result }));
      } catch (err) {
        ws.send(JSON.stringify({ id: msg.id, error: err.message }));
      }
    });
  });
}
