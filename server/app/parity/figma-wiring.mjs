// The tools and routes wired to wridge-figma (codegen, lottie, motion, motion-figma, motion-handoff, figjam,
// pdf), Rust vs a private Node instance: a fake plugin with real-looking answers (Motion timelines, SVG art,
// PDF pages, tokens, a FigJam board) on each server, the same steps on both, the answers diffed, then the
// files each server wrote compared (PDFs by page count, page sizes and title; everything else byte for byte)
// and the commands each plugin received.
//   node server/app/parity/figma-wiring.mjs 3255 3256 3355 3356 [--keep]
// Each server writes only into its own temp folder (deleted afterwards unless --keep). Never the user's
// ports (3155/3156, 3055/3056): refused.
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { request } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { fakeAnswers, startFakePlugin } from "./fake-plugin.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(HERE, "..", "..", "..", "legacy-node", "package.json"));
const { PDFDocument } = require("pdf-lib");

const argv = process.argv.slice(2);
const keep = argv.includes("--keep");
const ports = argv.filter((a) => /^\d+$/.test(a)).map(Number);
if (ports.length !== 4) throw new Error("usage: figma-wiring.mjs <rustWs> <rustHttp> <nodeWs> <nodeHttp>");
if (ports.some((p) => [3155, 3156, 3055, 3056].includes(p))) throw new Error("refusing the user's server ports");
const [rustWs, rustHttp, nodeWs, nodeHttp] = ports;

// ---------------------------------------------------------------- fixtures the fake plugin answers with

const key = (t, v, ease) => ({ t, v, ...(ease ? { ease } : {}) });
const field = (name, keys, op = "SET", base = 1) => ({ field: name, base, tracks: [{ op, keys }] });
const vec = { text: false, instances: 0, autoLayout: false, vectorOnly: true, image: false, component: null };
const ui = { ...vec, vectorOnly: false, autoLayout: true, text: true };

const READS = {
  // an app screen: text + a component go to code, a Lottie/… component to Lottie
  "1:1": { root: { id: "1:1", name: "Home" }, timelines: [{ id: "t", duration: 2 }], playhead: 0.25, animated: [
    { id: "3:1", name: "Title", type: "TEXT", parentId: "1:1", nth: 1, path: [], hint: ui, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.2, 1)]), field("TRANSLATION_Y", [key(0, 16, "EASE_OUT"), key(0.2, 0)], "SET", 24)] },
    { id: "3:2", name: "Card", type: "INSTANCE", parentId: "1:1", nth: 1, path: [], hint: { ...ui, component: "Card" }, styles: [{ name: "Fade in", props: { duration: 0.4 } }], tracks: [field("OPACITY", [key(0.1, 0, "EASE_OUT"), key(0.4, 1)]), field("WIDTH", [key(0.1, 200, "EASE_OUT"), key(0.8, 320)], "SET", 320)] },
    { id: "3:4", name: "Card", type: "INSTANCE", parentId: "1:1", nth: 2, path: [], hint: { ...ui, component: "Card" }, styles: [], tracks: [field("OPACITY", [key(0.16, 0, "cubic(0.2,0,0,1)"), key(0.46, 1)])] },
    { id: "3:3", name: "Confetti", type: "INSTANCE", parentId: "1:1", nth: 1, path: [], hint: { ...vec, component: "Lottie/Confetti" }, styles: [], tracks: [field("OPACITY", [key(0, 0), key(1.5, 1)])] },
  ] },
  // vector art: Lottie
  "1:3": { root: { id: "1:3", name: "Success" }, timelines: [{ duration: 1.2 }], animated: [
    { id: "4:1", name: "Ring", type: "GROUP", parentId: "1:3", nth: 1, path: [], hint: vec, styles: [], tracks: [field("SCALE_X", [key(0, 0, "BOUNCY"), key(0.5, 1)]), field("SCALE_Y", [key(0, 0, "BOUNCY"), key(0.5, 1)])] },
    { id: "4:2", name: "Check", type: "VECTOR", parentId: "1:3", nth: 1, path: [], hint: vec, styles: [], tracks: [field("PATH_TRIM_END", [key(0.3, 0, "cubic(0.2,0,0,1)"), key(0.7, 1)])] },
    { id: "4:3", name: "Check", type: "VECTOR", parentId: "1:3", nth: 2, path: [], hint: vec, styles: [], tracks: [field("ROTATION", [key(0, 0, "EASE_IN_AND_OUT"), key(0.6, Math.PI / 4)], "SET", 0), field("BLUR", [key(0, 4), key(0.6, 0)], "SET", 0)] },
  ] },
  // inside a Storyboard section: landing motion
  "7:1": { root: { id: "7:1", name: "Hero" }, timelines: [{ duration: 1.5 }], animated: [
    { id: "7:2", name: "Glasses", type: "VECTOR", parentId: "7:1", nth: 1, path: [], hint: vec, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.9, 1)])] },
    { id: "7:3", name: "Headline", type: "TEXT", parentId: "7:1", nth: 1, path: [], hint: ui, styles: [], tracks: [field("TRANSLATION_Y", [key(0.2, 40, "cubic(0.16,1,0.3,1)"), key(0.8, 0)], "SET", 300), field("OPACITY", [key(0.2, 0, "cubic(0.16,1,0.3,1)"), key(0.8, 1)])] },
  ] },
  "1:4": { root: { id: "1:4", name: "Static" }, timelines: [], animated: [] },
  // a nested layer from an old plugin (no path): nothing Lottie can take
  "9:9": { root: { id: "9:9", name: "Scene" }, timelines: [{ duration: 1 }], animated: [
    { id: "9:10", name: "Lens", type: "VECTOR", parentId: "9:11", nth: 1, hint: vec, styles: [], tracks: [field("OPACITY", [key(0, 0), key(0.5, 1)])] },
    { id: "9:9", name: "Scene", type: "FRAME", parentId: "0:1", nth: 1, hint: ui, styles: [], tracks: [field("OPACITY", [key(0, 0), key(0.5, 1)])] },
  ] },
  "1:6": { root: { id: "1:6", name: "Weird" } }, // no animated list
  "1:7": { root: { id: "1:7", name: "Odd answer" }, timelines: [{ duration: 1 }], animated: [] },
};
const STYLES = [
  { name: "motion.preset_name.opacity", styleId: "Opacity", props: { duration: 0.4, delay: 0 }, description: "Fade in\nsecond line" },
  { name: "motion.preset_name.position", styleId: "Pos1", props: { distance: 24 } },
];

const SUCCESS_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" viewBox="0 0 200 200">
  <g id="Ring"><circle cx="100" cy="100" r="80" fill="none" stroke="#22c55e" stroke-width="12"/></g>
  <path id="Check" d="M60 104 L88 132 L142 74" fill="none" stroke="#22c55e" stroke-width="14" stroke-linecap="round"/>
  <path id="Check_2" d="M50 50 L60 60" stroke="#16a34a" stroke-width="4"/>
</svg>`;
const HERO_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="300" height="120"><rect id="Glasses" x="10" y="10" width="120" height="40" rx="20" fill="#111"/><text id="Headline" x="10" y="100">Hi</text></svg>`;
const ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path d="M6 6l12 12M18 6L6 18" stroke="#000" stroke-width="2"/></svg>`;

// export_assets: name and kind per node
const ASSETS = {
  "2:1": { name: "Icon / Close", vector: true },
  "2:2": { name: "icon close", vector: true }, // same kebab name as 2:1
  "2:3": { name: "Hero Photo" },
  "2:4": { name: "" },
  "2:5": { name: "ไทย", vector: true }, // kebab() leaves nothing
  "2:6": { name: "Hero photo" },
  "1:3": { name: "Success", vector: true, svg: SUCCESS_SVG },
  "7:1": { name: "Hero", vector: true, svg: HERO_SVG },
  "1:4": { name: "Static", vector: true, svg: ICON_SVG },
};
// frames that become PDF pages
const FRAMES = {
  "6:1": { name: "2 Inside", w: 595, h: 842, x: 700, y: 0 },
  "6:2": { name: "1 Cover", w: 595, h: 842, x: 0, y: 0 },
  "6:3": { name: "2 Also", w: 842, h: 595, x: 1400, y: 0 },
  "6:4": { name: "Notes", w: 420, h: 595, x: 0, y: 1000 },
  "6:5": { name: "3 Hidden", w: 595, h: 842, x: 2400, y: 0, visible: false },
  "6:6": { name: "1 ปก", w: 500, h: 700, x: 0, y: 0 },
  "6:7": { name: "2 เนื้อหา", w: 500.5, h: 700.25, x: 600, y: 0 },
};
const frameNode = (id) => ({ id, name: FRAMES[id].name, type: "FRAME", x: FRAMES[id].x, y: FRAMES[id].y, width: FRAMES[id].w, height: FRAMES[id].h, visible: FRAMES[id].visible ?? true });
const SECTIONS = {
  "5:1": { name: "Book · Slides · Draft", children: ["6:1", "6:2", "6:3", "6:4", "6:5"].map(frameNode).concat([{ id: "6:9", name: "4 caption", type: "TEXT", x: 0, y: 0 }]) },
  "5:2": { name: "Hero · Storyboard · Draft", children: [{ id: "7:1", name: "Hero", type: "FRAME", x: 0, y: 0, width: 1440, height: 900.4 }, { id: "7:9", name: "Spec", type: "FRAME", x: 1500, y: 0, width: 400, height: 300 }] },
  "5:3": { name: "Empty · Slides", children: [{ id: "8:1", name: "t", type: "TEXT", x: 0, y: 0 }] },
  "5:5": { name: "หนังสือ 📘 \"เล่ม/1\" · Slides", children: ["6:7", "6:6"].map(frameNode) },
};

// a design context per frame / breakpoint (component instances for annotateComponents)
const designContext = (w, id = `bp:${w}`) => ({
  root: { id, name: `Home/${w}`, kind: "frame", width: w, layout: { mode: w > 700 ? "row" : "column", gap: w > 700 ? 24 : 16, padding: [16, 16] }, fill: { token: "surface" }, children: [
    { id: "2", kind: "text", name: "Title", text: "สวัสดี", style: { size: w > 700 ? 48 : 32, weight: 700 } },
    ...(w > 700 ? [{ id: "9", kind: "frame", name: "Sidebar", width: 280 }] : [{ id: "8", kind: "instance", name: "Menu", component: { name: "Burger", variant: "Type=Open" } }]),
    { id: "3", kind: "instance", name: "Btn", component: { name: "Button", variant: "Type=Danger, Size=Small" } },
    { id: "4", kind: "instance", name: "Card", component: { name: "Card", variant: "Size=Small" } },
    { id: "5", kind: "instance", name: "Chip", component: { name: "Chip" } },
    { id: "6", kind: "repeat", count: w > 700 ? 6 : 3, template: { id: "7", kind: "frame", name: "Row", width: w > 700 ? 300 : null, children: [{ id: "10", kind: "text", name: "Label", text: "x" }] } },
  ] },
  tokens: { surface: "#fff", [`w${w}`]: w, 10: "ten" }, components: [{ name: "Burger", id: "c1" }, { name: `C${w}`, id: "c2" }], fonts: ["Kanit"], assets: [{ name: "logo", format: "svg", nodeId: `a${w}` }],
});
const CONTEXTS = { "1:1": designContext(375, "1:1"), "1:3": designContext(200, "1:3"), "bp:390": designContext(390), "bp:768": designContext(768), "bp:1280": designContext(1280) };

const TOKENS_FILE = {
  sources: ["Foundations page", "variables", "styles"],
  file: "book-lock · Foundations",
  collections: [{ name: "Primitives", defaultMode: "Value" }, { name: "Theme", defaultMode: "Light" }, { name: "Size", defaultMode: "Default" }],
  variables: [
    { token: "blue-500", type: "COLOR", collection: "Primitives", values: { Value: "#2f6bff" } },
    { token: "white", type: "COLOR", collection: "Primitives", values: { Value: "#ffffffcc" } },
    { token: "colors-primary", type: "COLOR", collection: "Theme", values: { Light: { alias: "blue-500" }, Dark: { alias: "white" } } },
    { token: "radius-md", type: "FLOAT", collection: "Size", values: { Default: 8, Compact: 6.5 } },
    { token: "spacing-4", type: "FLOAT", collection: "Size", values: { Default: 16 } },
    { token: "font-family-body", type: "STRING", collection: "Size", values: { Default: "IBM Plex Sans Thai" } },
    { token: "feature-x", type: "BOOLEAN", collection: "Size", values: { Default: true } },
  ],
  paintStyles: [{ name: "Brand/Primary", token: "color-brand", value: "#2f6bff" }, { name: "Sunset", token: "gradient-sunset", value: { gradient: "linear", stops: [{ color: "#ff7a00", position: 0 }, { color: "#ff2d55", position: 0.555 }] } }],
  textStyles: [{ token: "text-display", family: "IBM Plex Sans Thai", style: "SemiBold Italic", size: 48, lineHeight: "120%", letterSpacing: "-0.02em" }, { token: "caption", family: "JetBrains Mono", style: "Regular", size: 12.5, lineHeight: "auto", letterSpacing: "0.5px" }],
  effectStyles: [{ token: "shadow-sm", shadows: [{ x: 0, y: 1, blur: 2, spread: 0, color: "#0000001a" }, { x: 0, y: 0, blur: 0, spread: 1, color: "#00000014", inset: true }] }],
};
const TOKENS_FOUNDATIONS = {
  sources: ["Foundations page"], file: "t", collections: [{ name: "c", defaultMode: "Light" }],
  variables: [{ token: "color-tile", type: "COLOR", collection: "c", values: { Light: "#ffffff" } }],
  paintStyles: [], textStyles: [{ token: "text-tile", family: "Kanit", style: "Bold", size: 24, lineHeight: "130%", letterSpacing: "0px" }], effectStyles: [],
};

const BOARD = {
  file: "book-lock board", editorType: "figjam",
  loose: [{ id: "9:1", kind: "sticky", color: "yellow", x: 0, y: 0, w: 100, h: 100, text: "loose" }, { id: "9:2", kind: "connector", from: "9:1", to: "2:9" }],
  sections: [{
    id: "1:1", name: "จองบูธ · System design · Draft", lanes: [{ name: "ร้านค้า", x: 0, y: 72, w: 1100, h: 160 }, { name: "server", x: 0, y: 232, w: 1100, h: 160 }],
    items: [
      { id: "2:1", kind: "shape", role: "step", waraId: "pick", x: 300, y: 100, w: 220, h: 96, text: "เลือกบูธ" },
      { id: "2:2", kind: "shape", role: "step", waraId: "hold", x: 600, y: 260, w: 220, h: 96, text: "ล็อกบูธ 15 นาที" },
      { id: "2:4", kind: "shape", shape: "DIAMOND", x: 600, y: 100, w: 220, h: 96, text: "ok?\n\"quoted\"" },
      { id: "2:3", kind: "sticky", role: "question", x: 1200, y: 72, w: 200, h: 200, text: "แก้สลิปได้กี่ครั้ง?" },
      { id: "3:1", kind: "connector", from: "2:1", to: "2:2", label: "ยืนยัน" }, { id: "3:2", kind: "connector", from: "2:2", to: "2:4", dashed: true },
    ],
  }, { id: "1:2", name: "Empty", items: [] }],
};

// PDF pages: made once per size, so both servers merge the same bytes
const pdfCache = new Map();
async function pdfPage(w, h) {
  const k = `${w}x${h}`;
  if (!pdfCache.has(k)) {
    const d = await PDFDocument.create();
    d.addPage([w, h]);
    pdfCache.set(k, Buffer.from(await d.save({ useObjectStreams: false })).toString("base64"));
  }
  return pdfCache.get(k);
}

function figmaAnswers(fileName, state) {
  const base = fakeAnswers(fileName);
  const doc = () => ({
    fileName,
    currentPage: { id: "0:1", name: "Page 1" },
    pages: [{ id: "0:1", name: "Page 1" }],
    topLevel: [
      { id: "1:1", name: "Home", type: "FRAME", x: 0, y: 0, width: 375.4, height: 812 },
      { id: "1:2", name: "Detail", type: "FRAME", x: 500, y: 0, width: 375, height: 812.5 },
      { id: "1:3", name: "Success", type: "COMPONENT", x: 1000, y: 0, width: 200, height: 200 },
      { id: "1:4", name: "Static", type: "FRAME", x: 1300, y: 0, width: 100, height: 100 },
      { id: "1:5", name: "Label", type: "TEXT", x: 0, y: 900, width: 80, height: 20 },
      { id: "9:9", name: "Scene", type: "GROUP", x: 1500, y: 0 },
      ...(state.weird ? [{ id: "1:6", name: "Weird", type: "FRAME", x: 0, y: 2000, width: 10, height: 10 }] : []),
      { id: "5:1", name: SECTIONS["5:1"].name, type: "SECTION", x: 0, y: 3000, width: 3000, height: 2000 },
      { id: "5:2", name: SECTIONS["5:2"].name, type: "SECTION", x: 0, y: 6000, width: 2000, height: 1000 },
      { id: "5:3", name: SECTIONS["5:3"].name, type: "SECTION", x: 0, y: 8000, width: 500, height: 500 },
      { id: "5:4", name: "Gone", type: "SECTION", x: 0, y: 9000, width: 500, height: 500 },
      { id: "5:5", name: SECTIONS["5:5"].name, type: "SECTION", x: 0, y: 10000, width: 1200, height: 800 },
    ],
  });
  return async (command, params) => {
    switch (command) {
      case "get_document_info": return doc();
      case "get_node": {
        const id = params.nodeId;
        if (SECTIONS[id]) return { id, name: SECTIONS[id].name, type: "SECTION", children: SECTIONS[id].children };
        if (FRAMES[id]) return frameNode(id);
        if (id === "1:5") return { id, name: "Label", type: "TEXT", characters: "Hi" };
        if (id === "5:4" || id === "boom") throw new Error(`Node ${id} not found`);
        return base(command, params);
      }
      case "motion_read": {
        if (state.noMotion) throw new Error("Figma Motion is not available in this editor");
        const r = READS[params.nodeId ?? "1:1"];
        if (!r) throw new Error("Figma Motion is not available in this editor");
        return params.styles ? { ...r, animationStyles: STYLES } : r;
      }
      case "motion_animate": {
        if (params.rootId === "1:7") return {};
        const missing = params.items.filter((i) => i.layer === "Nope");
        return {
          applied: params.items.filter((i) => i.layer !== "Nope").map((i) => ({ layer: i.layer, field: i.field?.name ?? i.field?.collection ?? null, style: i.styleId })),
          missing: missing.map((i) => i.layer),
          layers: ["Title", "Card", "Confetti"],
          ...(params.replace ? { idChanges: ["replace", "keyframes"] } : {}),
        };
      }
      case "export_assets":
        return Promise.all(params.items.map(async (it) => {
          if (it.nodeId === "boom") throw new Error("Node boom not found");
          const a = ASSETS[it.nodeId] ?? (FRAMES[it.nodeId] && { name: FRAMES[it.nodeId].name, w: FRAMES[it.nodeId].w, h: FRAMES[it.nodeId].h }) ?? { name: "Node" };
          const format = it.format === "auto" ? (a.vector ? "svg" : "png") : it.format;
          if (it.nodeId === "nob64") return { nodeId: it.nodeId, name: "No data", format };
          let b64;
          if (format === "pdf") b64 = await pdfPage(a.w ?? 100, a.h ?? 100);
          else if (format === "svg") b64 = Buffer.from(a.svg ?? ICON_SVG).toString("base64");
          else b64 = Buffer.from(`PNG ${it.nodeId} @${it.scale}x`).toString("base64");
          return { nodeId: it.nodeId, name: a.name, format, base64: b64 };
        }));
      case "get_design_context": return CONTEXTS[params.nodeId] ?? base(command, params);
      case "get_tokens": return params.source === "foundations" ? TOKENS_FOUNDATIONS : TOKENS_FILE;
      case "board_map": return params.sectionId ? { ...BOARD, loose: [], sections: BOARD.sections.filter((s) => s.id === params.sectionId) } : BOARD;
      case "figjam_draw": {
        const ids = { $section: "20:0" };
        params.nodes.forEach((n, i) => { if (n.id) ids[n.id] = `20:${i + 1}`; });
        return { ids, section: { id: "20:0", name: params.section.name, type: "SECTION" } };
      }
      default: return base(command, params);
    }
  };
}

// ---------------------------------------------------------------- per-server temp folders

const TMP = mkdtempSync(join(tmpdir(), "wridge-figma-wiring-"));
function prepare(root) {
  mkdirSync(join(root, "proj", "art"), { recursive: true });
  mkdirSync(join(root, "proj", "public", "lottie"), { recursive: true });
  writeFileSync(join(root, "proj", "wara.components.json"), `${JSON.stringify({
    Button: { import: "@/ui/button", component: "Button" },
    "Burger/Type=Open": { import: "@/ui/burger", component: "Burger", props: { open: true } },
    Card: { element: "div", className: "card", variants: { "Size=Small": "card-sm", "Size=Large": "card-lg" } },
    10: { component: "Ten" },
  }, null, 2)}\n`);
  writeFileSync(join(root, "proj", "art", "success.svg"), SUCCESS_SVG);
  // a Lottie already sent to Figma: rebuilding it keeps the link (meta.wara.figma)
  writeFileSync(join(root, "proj", "public", "lottie", "linked.json"), JSON.stringify({ v: "5.7.4", fr: 60, ip: 0, op: 60, w: 10, h: 10, layers: [], meta: { g: "wara-bridge", wara: { source: "spec", spec: {}, figma: { componentId: "77:1", file: "utils", sentAt: 1762400000000, hash: "abc" } } } }));
  mkdirSync(join(root, "bad"), { recursive: true });
  writeFileSync(join(root, "bad", "wara.components.json"), "{ not json");
  mkdirSync(join(root, "empty"), { recursive: true });
  return root;
}
const ROOTS = { rust: prepare(join(TMP, "rust")), node: prepare(join(TMP, "node")) };

// ---------------------------------------------------------------- HTTP

async function pdfSummary(buf) {
  try {
    const d = await PDFDocument.load(buf, { updateMetadata: false });
    return { pdf: true, pages: d.getPageCount(), sizes: d.getPages().map((p) => { const { width, height } = p.getSize(); return [width, height]; }), title: d.getTitle() ?? null, producer: d.getProducer() ?? null };
  } catch (err) {
    return `not a PDF: ${err.message}`;
  }
}

function http(port, method, path, body) {
  return new Promise((resolve, reject) => {
    const req = request({ host: "127.0.0.1", port, method, path, headers: { host: `127.0.0.1:${port}`, "content-type": "application/json", accept: "application/json, text/event-stream" } }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", async () => {
        const buf = Buffer.concat(chunks);
        const type = String(res.headers["content-type"] ?? "");
        let out;
        if (type.startsWith("application/json")) out = JSON.parse(buf.toString("utf8"));
        else if (type.startsWith("text/event-stream")) out = buf.toString("utf8").split("\n").filter((l) => l.startsWith("data: ")).map((l) => JSON.parse(l.slice(6)));
        else if (type === "application/pdf") {
          out = await pdfSummary(buf);
          const headers = Object.fromEntries(["content-type", "content-disposition", "cache-control"].map((h) => [h, res.headers[h]]));
          headers["content-length matches"] = Number(res.headers["content-length"]) === buf.length;
          return resolve({ status: res.statusCode, headers, body: out });
        } else out = buf.toString("utf8");
        resolve({ status: res.statusCode, body: out });
      });
    });
    req.on("error", reject);
    if (body !== undefined) req.write(JSON.stringify(body));
    req.end();
  });
}

// ---------------------------------------------------------------- the session

async function session(wsPort, httpPort, root) {
  const out = [];
  const fwd = root.replaceAll("\\", "/");
  // the server's own temp folder → <tmp>, in every spelling (backslashes, slashes, JSON-escaped)
  const variants = [root.replaceAll("\\", "\\\\"), root, fwd].sort((a, b) => b.length - a.length);
  const clean = (v) => JSON.parse(JSON.stringify(v, (k, x) => {
    if (["time", "ms", "totalMs"].includes(k)) return `<${k}>`;
    if (typeof x !== "string") return x;
    for (const p of variants) x = x.split(p).join("<tmp>");
    return x;
  }));
  const step = async (label, fn, fix = (v) => v) => {
    try {
      out.push({ label, value: clean(fix(await fn())) });
    } catch (err) {
      out.push({ label, value: `THREW ${err.message}` });
    }
  };
  const tool = (name, args) => http(httpPort, "POST", "/api/tool", { name, args });
  let mcpId = 0;
  const mcp = (name, args) => http(httpPort, "POST", "/mcp?profile=all", { jsonrpc: "2.0", id: ++mcpId, method: "tools/call", params: { name, arguments: args } });
  const get = (path) => http(httpPort, "GET", path);
  // a merged PDF's byte count depends on the writer (pdf-lib vs lopdf): compared through the file instead
  const pdfBytes = (r) => JSON.parse(JSON.stringify(r, (k, x) => (typeof x === "string" ? x.replace(/"bytes":\d+/g, '"bytes":"<pdf bytes>"') : x)));

  const state = {};
  const plugin = await startFakePlugin(wsPort, { fileName: "utils", answer: figmaAnswers("utils", state) });
  const P = fwd; // paths handed to the tools: slashes here, backslashes below where it matters
  const B = root;

  await step("get_document_info (lets writes through)", () => tool("get_document_info", {}));

  // ---- get_design_context: component map, breakpoints, motion summary
  await step("gdc nodeId with motion", () => tool("get_design_context", { nodeId: "1:1" }));
  await step("gdc nodeId without Motion API", () => tool("get_design_context", { nodeId: "1:2", depth: 3 }));
  await step("gdc nodeId art (Lottie rows)", () => tool("get_design_context", { nodeId: "1:3" }));
  await step("gdc projectDir with map", () => tool("get_design_context", { nodeId: "1:1", projectDir: `${P}/proj` }));
  await step("gdc projectDir backslashes", () => mcp("get_design_context", { nodeId: "1:1", projectDir: `${B}\\proj`, expandInstances: true }));
  await step("gdc projectDir without map", () => tool("get_design_context", { nodeId: "1:1", projectDir: `${P}/empty` }));
  await step("gdc projectDir bad JSON", () => tool("get_design_context", { nodeId: "1:1", projectDir: `${P}/bad` }));
  await step("gdc projectDir relative", () => tool("get_design_context", { nodeId: "1:1", projectDir: "proj" }));
  await step("gdc context without root + projectDir", () => tool("get_design_context", { nodeId: "1:9", projectDir: `${P}/proj` }));
  await step("gdc breakpoints base/md/xl + projectDir", () => tool("get_design_context", { breakpoints: { base: "bp:390", md: "bp:768", xl: "bp:1280" }, projectDir: `${P}/proj` }));
  await step("gdc breakpoints out of order (motion of the first)", () => tool("get_design_context", { breakpoints: { lg: "1:1", md: "bp:768" } }));
  await step("gdc breakpoints with motion on base", () => tool("get_design_context", { breakpoints: { md: "bp:768", base: "1:1" }, userApproved: true }));
  await step("gdc breakpoints all empty", () => tool("get_design_context", { breakpoints: { base: "" } }));

  // ---- export_assets
  await step("export_assets web auto", () => tool("export_assets", { nodeIds: ["2:1", "2:2", "2:3", "2:4", "2:5"], outDir: `${P}/assets/web` }));
  await step("export_assets web png scale 3", () => tool("export_assets", { nodeIds: ["2:1", "2:6"], outDir: `${B}\\assets\\png`, format: "png", scale: 3 }));
  await step("export_assets flutter", () => mcp("export_assets", { nodeIds: ["2:3", "2:1", "2:6", "2:4"], outDir: `${P}/assets/flutter`, platform: "flutter" }));
  await step("export_assets svg", () => tool("export_assets", { nodeIds: ["2:3"], outDir: `${P}/assets/svg/`, format: "svg" }));
  await step("export_assets relative outDir", () => tool("export_assets", { nodeIds: ["2:1"], outDir: "assets" }));
  await step("export_assets no base64", () => tool("export_assets", { nodeIds: ["nob64"], outDir: `${P}/assets/bad` }));
  await step("export_assets plugin error", () => tool("export_assets", { nodeIds: ["2:1", "boom"], outDir: `${P}/assets/bad` }));

  // ---- export_pdf
  await step("export_pdf section → folder", () => tool("export_pdf", { nodeId: "5:1", out: `${P}/pdf` }), pdfBytes);
  await step("export_pdf frame → .PDF file", () => tool("export_pdf", { nodeId: "6:2", out: `${B}\\pdf\\one page.PDF` }), pdfBytes);
  await step("export_pdf Thai section → folder", () => mcp("export_pdf", { nodeId: "5:5", out: `${P}/pdf/thai/` }), pdfBytes);
  await step("export_pdf a text node", () => tool("export_pdf", { nodeId: "1:5", out: `${P}/pdf` }));
  await step("export_pdf section without frames", () => tool("export_pdf", { nodeId: "5:3", out: `${P}/pdf` }));
  await step("export_pdf missing node", () => tool("export_pdf", { nodeId: "boom", out: `${P}/pdf` }));
  await step("export_pdf relative out", () => tool("export_pdf", { nodeId: "5:1", out: "book.pdf" }));

  // ---- get_tokens
  await step("get_tokens tailwind", () => tool("get_tokens", {}));
  await step("get_tokens css → file", () => tool("get_tokens", { format: "css", outFile: `${P}/tokens/tokens.css` }));
  await step("get_tokens dart → file", () => mcp("get_tokens", { format: "dart", outFile: `${B}\\tokens\\tokens.dart` }));
  await step("get_tokens json", () => tool("get_tokens", { format: "json", source: "file" }));
  await step("get_tokens foundations", () => tool("get_tokens", { source: "foundations", format: "css" }));
  await step("get_tokens empty outFile", () => tool("get_tokens", { outFile: "" }));
  await step("get_tokens relative outFile", () => tool("get_tokens", { outFile: "tokens.css" }));

  // ---- component map
  await step("get_component_map", () => tool("get_component_map", { projectDir: `${P}/proj` }));
  await step("get_component_map none", () => tool("get_component_map", { projectDir: `${P}/empty` }));
  await step("get_component_map bad JSON", () => mcp("get_component_map", { projectDir: `${B}\\bad` }));
  await step("get_component_map relative", () => tool("get_component_map", { projectDir: "proj" }));
  await step("map_component component", () => tool("map_component", { projectDir: `${P}/proj`, figmaName: "Avatar", import: "@/ui/avatar", component: "Avatar", props: { size: "lg" } }));
  await step("map_component classes", () => tool("map_component", { projectDir: `${P}/proj`, figmaName: "Chip", element: "span", className: "chip", variants: { "Tone=Muted": "chip-m" } }));
  await step("map_component neither", () => tool("map_component", { projectDir: `${P}/proj`, figmaName: "X", element: "div" }));
  await step("map_component both", () => tool("map_component", { projectDir: `${P}/proj`, figmaName: "X", import: "a", component: "B", className: "c" }));
  await step("map_component component without import", () => tool("map_component", { projectDir: `${P}/proj`, figmaName: "X", component: "B" }));
  await step("map_component remove", () => tool("map_component", { projectDir: `${P}/proj`, figmaName: "Button", remove: true }));
  await step("map_component remove missing", () => mcp("map_component", { projectDir: `${B}\\proj`, figmaName: "Nope", remove: true }));
  await step("map_component new project", () => tool("map_component", { projectDir: `${P}/fresh/app`, figmaName: "Ünïcode/Type=Big", import: "@/x", component: "X" }));
  await step("map_component bad JSON", () => tool("map_component", { projectDir: `${P}/bad`, figmaName: "X", import: "a", component: "B" }));
  await step("get_component_map after edits", () => tool("get_component_map", { projectDir: `${P}/proj` }));
  await step("gdc with the edited map", () => tool("get_design_context", { nodeId: "1:1", projectDir: `${P}/proj` }));

  // ---- Figma Motion
  await step("motion_read screen + styles", () => tool("motion_read", { nodeId: "1:1", styles: true }));
  await step("motion_read art", () => mcp("motion_read", { nodeId: "1:3" }));
  await step("motion_read selection", () => tool("motion_read", {}));
  await step("motion_read nothing animated", () => tool("motion_read", { nodeId: "1:4" }));
  await step("motion_read no Motion API", () => tool("motion_read", { nodeId: "1:2" }));
  await step("motion_read odd answer", () => tool("motion_read", { nodeId: "1:6" }));
  await step("motion_animate tracks + presets", () => tool("motion_animate", { nodeId: "1:1", duration: 1200, tracks: [
    { layer: "Title", prop: "opacity", from: 0, to: 1, duration: 300, ease: "out" },
    { layer: "Card*", prop: "move", from: [0, 24], to: [0, 0], start: 100, end: 500, stagger: 60, ease: [0.2, 0, 0, 1] },
    { layer: "Card", prop: "fill", keys: [[0, "#ff0000"], [300, "#00ff0080", "hold"], [600, "#0000ff"]] },
  ], presets: [{ layer: "Confetti", style: "opacity", delay: 200, duration: 400 }, { style: "position", props: { distance: 12 } }] }));
  await step("motion_animate replace + missing layer", () => mcp("motion_animate", { nodeId: "1:1", replace: true, tracks: [{ layer: "Nope", prop: "scale", from: 0.5 }, { layer: "Title", prop: "rotate", from: -10, to: 0 }] }));
  await step("motion_animate unknown ease", () => tool("motion_animate", { nodeId: "1:1", tracks: [{ prop: "x", from: 0, to: 4, ease: "wobble" }] }));
  await step("motion_animate unknown preset", () => tool("motion_animate", { nodeId: "1:1", presets: [{ style: "Wiggle" }] }));
  await step("motion_animate odd plugin answer", () => tool("motion_animate", { nodeId: "1:7", tracks: [{ prop: "opacity", from: 0 }] }));
  await step("motion_check screen (auto → ui)", () => tool("motion_check", { nodeId: "1:1" }));
  await step("motion_check storyboard (auto → landing)", () => mcp("motion_check", { nodeId: "7:1" }));
  await step("motion_check art as landing", () => tool("motion_check", { nodeId: "1:3", context: "landing" }));
  await step("motion_check no Motion API", () => tool("motion_check", { nodeId: "1:2", context: "ui" }));
  await step("motion_check odd answer", () => tool("motion_check", { nodeId: "1:6" }));
  await step("motion_tokens two frames", () => tool("motion_tokens", { nodeIds: ["1:1", "7:1"] }));
  await step("motion_tokens art only", () => tool("motion_tokens", { nodeIds: ["1:3"], context: "ui" }));
  await step("motion_to_code css", () => tool("motion_to_code", { nodeId: "1:1" }));
  await step("motion_to_code framer", () => mcp("motion_to_code", { nodeId: "1:1", target: "framer-motion" }));
  await step("motion_to_code css with tokenFrames", () => tool("motion_to_code", { nodeId: "7:1", tokenFrames: ["1:1", "7:1"] }));
  await step("motion_to_code art", () => tool("motion_to_code", { nodeId: "1:3" }));
  await step("motion_to_code no Motion API", () => tool("motion_to_code", { nodeId: "1:2", context: "landing" }));

  // ---- Lottie
  const tracks = [{ layer: "Ring", prop: "scale", from: 0, to: 1, duration: 400, ease: "back" }, { layer: "Check*", prop: "draw", from: 0, to: 1, start: 300, end: 700, ease: "out" }];
  await step("create_lottie inline svg", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "public/lottie/inline.json", svg: SUCCESS_SVG, duration: 900, tracks }));
  await step("create_lottie svgFile + name + poster", () => tool("create_lottie", { projectDir: `${B}\\proj`, out: "public\\lottie\\from-file.JSON", svgFile: "art/success.svg", name: "Success check", duration: 1200, fps: 30, poster: 0, width: 100, tracks }));
  await step("create_lottie from node", () => mcp("create_lottie", { projectDir: `${P}/proj`, out: "assets/lottie/node.json", nodeId: "1:3", duration: 800, tracks: [{ layer: "all", prop: "opacity", from: 0, to: 1, stagger: 100 }] }));
  await step("create_lottie keeps the Figma link", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "public/lottie/linked.json", svg: SUCCESS_SVG, duration: 500, tracks: [{ layer: "Ring", prop: "rotate", keys: [[0, 0], [500, 360, "linear"]] }] }));
  await step("create_lottie unknown layer", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "public/lottie/warn.json", svg: SUCCESS_SVG, duration: 500, tracks: [{ layer: "Nope", prop: "opacity", from: 0 }] }));
  await step("create_lottie svgFile missing", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "x.json", svgFile: "art/missing.svg", duration: 500 }));
  await step("create_lottie out outside", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "../outside.json", svg: SUCCESS_SVG, duration: 500 }));
  await step("create_lottie no art", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "x.json", svg: "", duration: 500 }));
  await step("create_lottie not svg", () => tool("create_lottie", { projectDir: `${P}/proj`, out: "x.json", svg: "<div>nope</div>", duration: 500 }));
  await step("motion_to_lottie art", () => tool("motion_to_lottie", { projectDir: `${P}/proj`, out: "public/lottie/success.json", nodeId: "1:3", fps: 30 }));
  await step("motion_to_lottie nothing animated", () => tool("motion_to_lottie", { projectDir: `${P}/proj`, out: "public/lottie/static.json", nodeId: "1:4" }));
  await step("motion_to_lottie only warnings", () => mcp("motion_to_lottie", { projectDir: `${P}/proj`, out: "public/lottie/scene.json", nodeId: "9:9", name: "Scene" }));
  await step("motion_to_lottie no Motion API", () => tool("motion_to_lottie", { projectDir: `${P}/proj`, out: "public/lottie/detail.json", nodeId: "1:2" }));
  await step("motion_to_lottie out outside", () => tool("motion_to_lottie", { projectDir: `${P}/proj`, out: "../x.json", nodeId: "1:3" }));

  // ---- FigJam
  const BOOKING = {
    title: "จองบูธ · System design · Draft", lanes: ["ร้านค้า", "server"],
    nodes: [
      { id: "s", kind: "start", lane: "ร้านค้า" }, { id: "pick", text: "เลือกบูธ", lane: "ร้านค้า" }, { id: "hold", text: "ล็อกบูธ 15 นาที", lane: "server" },
      { id: "ok", text: "สลิปตรงไหม", kind: "decision", lane: "server" }, { id: "10", text: "ออกตั๋ว", lane: "server", status: "changed" }, { id: "e", kind: "end", lane: "ร้านค้า" },
    ],
    edges: [{ from: "s", to: "pick" }, { from: "pick", to: "hold" }, { from: "hold", to: "ok" }, { from: "ok", to: "10", label: "ตรง" }, { from: "ok", to: "pick", label: "ไม่ตรง" }, { from: "10", to: "e" }],
    notes: [{ text: "ร้านแก้สลิปได้กี่ครั้ง?", kind: "question" }, { text: "ล็อก 15 นาที", kind: "agreed" }],
  };
  await step("figjam_diagram flow with lanes", () => tool("figjam_diagram", BOOKING));
  await step("figjam_diagram state", () => mcp("figjam_diagram", { type: "state", title: "booking", nodes: [{ id: "hold", text: "hold" }, { id: "paid", text: "paid" }, { id: "2", text: "refunded" }], edges: [{ from: "hold", to: "paid", label: "slip ok" }, { from: "paid", to: "2", label: "refund", dashed: true }], x: 100, y: 200 }));
  await step("figjam_diagram er", () => tool("figjam_diagram", { type: "er", title: "Booth data", tables: [
    { name: "bookings", status: "changed", columns: [{ name: "id", type: "uuid", pk: true }, { name: "booth_id", type: "uuid", fk: "booths.id" }] },
    { name: "booths", columns: [{ name: "id", type: "uuid", pk: true }, { name: "label", type: "text", note: "A1, A2…" }] },
  ], notes: [{ text: "soft delete?", kind: "question" }] }));
  await step("figjam_diagram sequence", () => tool("figjam_diagram", { type: "sequence", title: "Slip check", participants: ["web", "server", "Slip2Go"], messages: [
    { from: "web", to: "server", text: "upload slip" }, { from: "server", to: "Slip2Go", text: "verify" }, { from: "Slip2Go", to: "server", text: "result", kind: "reply" }, { from: "server", to: "server", text: "save" },
  ] }));
  await step("figjam_diagram unknown edge node", () => tool("figjam_diagram", { title: "x", nodes: [{ id: "a", text: "x" }], edges: [{ from: "a", to: "b" }] }));
  await step("figjam_diagram duplicate id", () => tool("figjam_diagram", { title: "x", nodes: [{ id: "a" }, { id: "a" }] }));
  await step("figjam_diagram lane not listed", () => tool("figjam_diagram", { title: "x", lanes: ["A"], nodes: [{ id: "a", text: "x", lane: "B" }] }));
  await step("figjam_diagram er without tables", () => tool("figjam_diagram", { title: "x", type: "er" }));
  await step("figjam_diagram sequence too short", () => tool("figjam_diagram", { title: "x", type: "sequence", participants: ["a"], messages: [] }));
  await step("board_map whole board", () => tool("board_map", {}));
  await step("board_map one section", () => mcp("board_map", { sectionId: "1:1" }));

  // ---- routes
  await step("GET /api/motion/figma-frames", () => get("/api/motion/figma-frames"));
  await step("GET /api/motion/figma-preview art", () => get("/api/motion/figma-preview?id=1:3"));
  await step("GET /api/motion/figma-preview storyboard", () => get("/api/motion/figma-preview?id=7:1"));
  await step("GET /api/motion/figma-preview nothing animated", () => get("/api/motion/figma-preview?id=1:4"));
  await step("GET /api/motion/figma-preview nested only", () => get("/api/motion/figma-preview?id=9:9"));
  await step("GET /api/motion/figma-preview no Motion API", () => get("/api/motion/figma-preview?id=1:2"));
  await step("GET /api/motion/figma-preview odd answer", () => get("/api/motion/figma-preview?id=1:6"));
  await step("GET /api/pdf/sections", () => get("/api/pdf/sections"));
  await step("GET /api/pdf/download id", () => get("/api/pdf/download?id=5:1"));
  await step("GET /api/pdf/download ids (two sections + a frame)", () => get(`/api/pdf/download?ids=${encodeURIComponent(" 5:5, 5:1 ,,6:4")}`));
  await step("GET /api/pdf/download ids beats id", () => get("/api/pdf/download?id=5:1&ids=6:2"));
  await step("GET /api/pdf/download a text node", () => get("/api/pdf/download?id=1:5"));
  await step("GET /api/pdf/download section without frames", () => get("/api/pdf/download?ids=5:1,5:3"));
  await step("GET /api/pdf/download missing node", () => get("/api/pdf/download?id=boom"));
  state.weird = true;
  await step("GET /api/motion/figma-frames with an odd frame", () => get("/api/motion/figma-frames"));
  await step("GET /api/pdf/sections with an odd frame", () => get("/api/pdf/sections"));
  state.weird = false;
  state.noMotion = true;
  await step("GET /api/motion/figma-frames without Motion API", () => get("/api/motion/figma-frames"));
  await step("gdc without Motion API", () => tool("get_design_context", { nodeId: "1:1" }));
  state.noMotion = false;

  await step("plugin received", () => plugin.received);
  await plugin.close();
  await new Promise((r) => setTimeout(r, 150)); // the server sees the plugin go before the next check starts
  return out;
}

// ---------------------------------------------------------------- written files

async function manifest(root) {
  const files = {};
  const walk = async (dir) => {
    for (const e of readdirSync(dir).sort()) {
      const p = join(dir, e);
      if (statSync(p).isDirectory()) await walk(p);
      else {
        const rel = relative(root, p).replaceAll("\\", "/");
        const buf = readFileSync(p);
        files[rel] = /\.pdf$/i.test(rel) ? await pdfSummary(buf) : `${buf.length} bytes sha256:${createHash("sha256").update(buf).digest("hex").slice(0, 16)}`;
      }
    }
  };
  await walk(root);
  return files;
}

function report(a, b, labelA, labelB) {
  let same = 0;
  const diffs = [];
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = JSON.stringify(a[i]?.value), y = JSON.stringify(b[i]?.value);
    if (x === y) { same++; continue; }
    let j = 0;
    while (j < x.length && x[j] === y?.[j]) j++;
    diffs.push(`DIFF ${a[i]?.label ?? b[i]?.label}\n    ${labelA}: …${String(x).slice(Math.max(0, j - 100), j + 200)}\n    ${labelB}: …${String(y).slice(Math.max(0, j - 100), j + 200)}`);
  }
  return { same, total: Math.max(a.length, b.length), diffs };
}

let exitCode = 1;
try {
  const [rust, node] = await Promise.all([session(rustWs, rustHttp, ROOTS.rust), session(nodeWs, nodeHttp, ROOTS.node)]);
  const steps = report(rust, node, "rust", "node");
  const [mr, mn] = await Promise.all([manifest(ROOTS.rust), manifest(ROOTS.node)]);
  const files = report(Object.entries(mr).map(([label, value]) => ({ label, value })), Object.entries(mn).map(([label, value]) => ({ label, value })), "rust", "node");
  for (const d of [...steps.diffs, ...files.diffs]) console.log(d);
  if (process.env.PARITY_VERBOSE) for (const s of rust) console.log(`${s.label}: ${JSON.stringify(s.value).slice(0, 300)}`);
  console.log(`figma wiring: ${steps.same}/${steps.total} steps identical, ${files.same}/${files.total} written files identical (:${rustHttp} vs :${nodeHttp})`);
  exitCode = steps.diffs.length || files.diffs.length ? 1 : 0;
} finally {
  if (keep) console.log(`kept ${TMP}`);
  else rmSync(TMP, { recursive: true, force: true });
}
process.exit(exitCode);
