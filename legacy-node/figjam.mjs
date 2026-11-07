// Lays out a system-design diagram for FigJam: the AI sends boxes and arrows (with lanes for who does
// each step), tables, or participants and messages; this file decides where everything goes, so no
// coordinates are spent in the conversation. The plugin's figjam_draw then creates the section, lane
// bands, shapes, stickies and connectors. Placement uses ELK (layered): fewer crossings, room for labels.
import ELK from "elkjs/lib/elk.bundled.js";

// Look of each kind of node. Colours follow the board conventions in the system-design skill.
const KIND = {
  start: { shape: "ELLIPSE", w: 56, h: 56, fill: "#0B0D12", text: "" },
  end: { shape: "ELLIPSE", w: 56, h: 56, fill: "#FFFFFF", stroke: "#0B0D12", text: "" },
  step: { shape: "ROUNDED_RECTANGLE" },
  wait: { shape: "ROUNDED_RECTANGLE", prefix: "⏳ " },
  decision: { shape: "DIAMOND" },
  state: { shape: "ELLIPSE" },
  table: { shape: "SQUARE" },
  question: { sticky: "#FDE68A" },
  risk: { sticky: "#FECACA" },
  agreed: { sticky: "#BBF7D0" },
  note: { sticky: "#E2E8F0" },
};
// existing = already in the product (grey), new = proposed (blue), changed (amber), removed (red outline)
const STATUS = {
  existing: { fill: "#F1F5F9", stroke: "#CBD5E1", textColor: "#334155" },
  new: { fill: "#DBEAFE", stroke: "#2563EB", textColor: "#0B0D12" },
  changed: { fill: "#FEF3C7", stroke: "#D97706", textColor: "#0B0D12" },
  removed: { fill: "#FFFFFF", stroke: "#DC2626", textColor: "#991B1B" },
};
export const NODE_KINDS = Object.keys(KIND);
export const NODE_STATUS = Object.keys(STATUS);
export const DIAGRAM_TYPES = ["flow", "state", "er", "sequence"];

const elk = new ELK();
// TITLE leaves room for FigJam's own section label, which sits inside the section's top-left corner.
const LANE_LABEL = 200, PAD = 64, LANE_PAD = 36, TITLE = 140, NOTE = 280;

// Box size from its text: FigJam wraps inside the shape, so make the shape fit a few short lines.
// Thai marks above/below a letter take no width, so they are not counted.
const visible = (t) => String(t ?? "").replace(/[ัิ-ฺ็-๎]/g, "");
function fit(text, { fs = 16, minW, maxW, minH }) {
  const per = fs * 0.62; // average glyph width at this size
  const chars = visible(text).length;
  const w = Math.max(minW, Math.min(maxW, chars * per + 48));
  const lines = Math.max(1, Math.ceil((chars * per) / (w - 48)));
  return { w: Math.round(w), h: Math.round(Math.max(minH, lines * fs * 1.45 + 40)) };
}

// Edges that close a loop (retry, go back) — drawn dashed.
function backEdges(nodes, edges) {
  const out = new Map(nodes.map((n) => [n.id, []]));
  for (const e of edges) if (out.has(e.from) && out.has(e.to)) out.get(e.from).push(e.to);
  const state = new Map(), back = new Set();
  const visit = (id) => {
    state.set(id, 1);
    for (const to of out.get(id)) {
      if (state.get(to) === 1) back.add(`${id}>${to}`);
      else if (!state.has(to)) visit(to);
    }
    state.set(id, 2);
  };
  const hasIn = new Set(edges.map((e) => e.to));
  for (const n of nodes) if (!hasIn.has(n.id) && !state.has(n.id)) visit(n.id);
  for (const n of nodes) if (!state.has(n.id)) visit(n.id);
  return back;
}

// ELK layered layout, left → right: fewer crossings, and room between columns for the arrow labels.
async function elkPlace(boxes, edges, { nodeGap = 56, layerGap = 120 } = {}) {
  const g = await elk.layout({
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.spacing.nodeNode": String(nodeGap),
      "elk.layered.spacing.nodeNodeBetweenLayers": String(layerGap),
      "elk.layered.spacing.edgeNodeBetweenLayers": "40",
      "elk.spacing.edgeLabel": "10",
      "elk.layered.crossingMinimization.strategy": "LAYER_SWEEP",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
      "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
      "elk.layered.cycleBreaking.strategy": "DEPTH_FIRST",
    },
    children: boxes.map((b) => ({ id: b.id, width: b.w, height: b.h })),
    edges: edges.map((e, i) => ({ id: `elk${i}`, sources: [e.from], targets: [e.to], labels: e.label ? [{ text: e.label, width: visible(e.label).length * 8 + 20, height: 22 }] : [] })),
  });
  return { at: new Map(g.children.map((c) => [c.id, { x: c.x, y: c.y }])), width: g.width, height: g.height };
}

// Stickies in up to two columns beside the diagram (shared by every type).
function placeNotes(notes, x0, top) {
  return notes.map((nt, i) => {
    const kind = nt.kind ?? "note";
    if (!KIND[kind]?.sticky) throw new Error(`Note kind "${kind}" — use question, risk, agreed or note`);
    return { id: nt.id ?? `note${i + 1}`, kind: "sticky", role: kind, x: x0 + (i % 2) * NOTE, y: top + Math.floor(i / 2) * NOTE, text: nt.text, fill: KIND[kind].sticky };
  });
}
const notesSize = (notes) => (notes.length ? { w: Math.min(2, notes.length) * NOTE, h: Math.ceil(notes.length / 2) * NOTE } : { w: 0, h: 0 });
function section(spec, fallback, width, height) {
  const ns = notesSize(spec.notes ?? []);
  return { name: spec.title ?? fallback, ...(spec.x != null ? { x: spec.x, y: spec.y ?? 0 } : {}), width: Math.round(width + (ns.w ? ns.w + 24 : 0) + PAD), height: Math.round(Math.max(height, TITLE + ns.h) + PAD), fill: "#FFFFFF" };
}

/** Dispatch on spec.type: flow (default, lanes = actors), state (state machine), er (tables), sequence. */
export async function layoutDiagram(spec) {
  const type = spec.type ?? "flow";
  if (type === "flow" || type === "state") return layoutFlow(spec);
  if (type === "er") return layoutEr(spec);
  if (type === "sequence") return layoutSequence(spec);
  throw new Error(`Unknown diagram type "${type}" — use ${DIAGRAM_TYPES.join(", ")}`);
}

// flow / state: { lanes?, nodes: [{ id, text, kind?, lane?, status? }], edges: [{ from, to, label?, dashed? }] }
async function layoutFlow(spec) {
  const ids = new Set();
  const nodes = (spec.nodes ?? []).map((n, i) => {
    const id = String(n.id ?? `n${i + 1}`);
    if (ids.has(id)) throw new Error(`Duplicate node id "${id}"`);
    ids.add(id);
    const kind = n.kind ?? (spec.type === "state" ? "state" : "step");
    if (!KIND[kind]) throw new Error(`Unknown kind "${kind}" for ${id} — use ${NODE_KINDS.join(", ")}`);
    if (KIND[kind].sticky) throw new Error(`"${kind}" is a sticky: put it in notes, not nodes`);
    const k = KIND[kind];
    const text = k.text ?? `${k.prefix ?? ""}${n.text ?? ""}`;
    const size = kind === "start" || kind === "end" ? { w: k.w, h: k.h }
      : kind === "decision" ? fit(text, { fs: 14, minW: 190, maxW: 240, minH: 130 })
      : kind === "state" ? fit(text, { fs: 16, minW: 190, maxW: 250, minH: 84 })
      : fit(text, { fs: 16, minW: 200, maxW: 260, minH: 88 });
    return { ...n, id, kind, text, ...size };
  });
  const edgesIn = spec.edges ?? [];
  for (const e of edgesIn) if (!ids.has(e.from) || !ids.has(e.to)) throw new Error(`Edge ${e.from} → ${e.to}: unknown node (ids: ${[...ids].join(", ")})`);

  const laneNames = spec.lanes?.length ? spec.lanes : [...new Set(nodes.map((n) => n.lane).filter(Boolean))];
  const hasLanes = laneNames.length > 0;
  for (const n of nodes) if (hasLanes && n.lane && !laneNames.includes(n.lane)) throw new Error(`Node ${n.id}: lane "${n.lane}" is not in lanes (${laneNames.join(", ")})`);
  const laneOf = (n) => (hasLanes ? Math.max(0, laneNames.indexOf(n.lane ?? laneNames[0])) : 0);

  const back = backEdges(nodes, edgesIn);
  const isState = spec.type === "state";
  const { at, width: elkW, height: elkH } = await elkPlace(nodes, edgesIn, { nodeGap: isState ? 72 : 56, layerGap: isState ? 150 : 110 });
  const left = (hasLanes ? LANE_LABEL : 0) + PAD;
  const width = left + elkW;
  const pos = new Map();
  let lanes = [], diagramH;
  if (!hasLanes) {
    for (const n of nodes) pos.set(n.id, { x: left + at.get(n.id).x, y: TITLE + at.get(n.id).y });
    diagramH = TITLE + elkH;
  } else {
    // Keep ELK's left → right order; each box goes into its actor's lane. Boxes of one lane that would
    // overlap side by side take another row of that lane (in ELK's top → bottom order).
    const GAP = 40;
    const rowsOf = laneNames.map(() => []); // per lane: rows of [x0, x1] spans
    const slot = new Map();
    for (const n of [...nodes].sort((a, b) => at.get(a.id).y - at.get(b.id).y)) {
      const l = laneOf(n), x0 = at.get(n.id).x, x1 = x0 + n.w;
      let r = rowsOf[l].findIndex((row) => row.every(([a, b]) => x1 + GAP <= a || x0 >= b + GAP));
      if (r < 0) { rowsOf[l].push([]); r = rowsOf[l].length - 1; }
      rowsOf[l][r].push([x0, x1]);
      slot.set(n.id, r);
    }
    const rowH = Math.max(...nodes.map((n) => n.h)) + 40;
    let y = TITLE;
    lanes = laneNames.map((name, l) => {
      const lane = { name, x: 0, y, width: width + PAD, height: Math.max(1, rowsOf[l].length) * rowH + LANE_PAD * 2, fill: l % 2 ? "#FFFFFF" : "#F8FAFC" };
      y += lane.height;
      return lane;
    });
    for (const n of nodes) {
      const lane = lanes[laneOf(n)];
      pos.set(n.id, { x: left + at.get(n.id).x, y: lane.y + LANE_PAD + slot.get(n.id) * rowH + (rowH - n.h) / 2 });
    }
    diagramH = y;
  }
  const out = nodes.map((n) => {
    const k = KIND[n.kind], st = STATUS[n.status ?? "new"] ?? STATUS.new;
    const p = pos.get(n.id);
    return {
      id: n.id, kind: "shape", shape: k.shape, role: n.kind, x: Math.round(p.x), y: Math.round(p.y), width: n.w, height: n.h, text: n.text,
      fill: k.fill ?? st.fill, stroke: k.stroke ?? st.stroke, textColor: n.kind === "start" ? "#FFFFFF" : st.textColor, fontSize: n.kind === "decision" ? 14 : 16,
    };
  });
  return {
    section: section(spec, "System design · Draft", width + PAD, diagramH),
    lanes, nodes: [...out, ...placeNotes(spec.notes ?? [], width + PAD + 24, TITLE)],
    edges: edgesIn.map((e, i) => ({ id: e.id ?? `e${i + 1}`, from: e.from, to: e.to, label: e.label, dashed: e.dashed || back.has(`${e.from}>${e.to}`) || undefined, color: back.has(`${e.from}>${e.to}`) ? "#94A3B8" : "#334155" })),
  };
}

// ER: one box per table (name, then a line per column: 🔑 PK, → FK). ELK places them with the referenced
// tables to the left; FK arrows point from the referencing table to the referenced one.
async function layoutEr(spec) {
  const tables = spec.tables ?? [];
  if (!tables.length) throw new Error("er needs tables: [{ id, name, columns: [{ name, type, pk?, fk? }] }]");
  const byName = new Map(tables.map((t) => [t.id ?? t.name, t]));
  const edges = [];
  for (const t of tables) for (const c of t.columns ?? []) {
    if (!c.fk) continue;
    const target = String(c.fk).split(".")[0];
    if (!byName.has(target)) throw new Error(`${t.name}.${c.name} → ${c.fk}: table "${target}" is not in tables`);
    edges.push({ from: t.id ?? t.name, to: target, label: c.name });
  }
  const boxes = tables.map((t) => {
    const lines = (t.columns ?? []).map((c) => `${c.pk ? "🔑 " : c.fk ? "→ " : "• "}${c.name}  ${c.type ?? ""}${c.fk ? `  (${c.fk})` : ""}${c.note ? ` · ${c.note}` : ""}`);
    const longest = Math.max(visible(t.name).length + 4, ...lines.map((l) => visible(l).length));
    return { id: t.id ?? t.name, t, lines, w: Math.round(Math.min(400, Math.max(240, longest * 8.2 + 40))), h: Math.round(64 + Math.max(1, lines.length) * 21 + 24) };
  });
  // laid out referenced → referencing, so referenced tables come first (left)
  const { at, width: elkW, height: elkH } = await elkPlace(boxes, edges.map((e) => ({ from: e.to, to: e.from, label: e.label })), { nodeGap: 48, layerGap: 140 });
  const out = boxes.map((b) => {
    const st = STATUS[b.t.status ?? "new"] ?? STATUS.new;
    return { id: b.id, kind: "shape", shape: "SQUARE", role: "table", x: Math.round(PAD + at.get(b.id).x), y: Math.round(TITLE + at.get(b.id).y), width: b.w, height: b.h, text: `${b.t.name}\n\n${b.lines.join("\n")}`, fill: st.fill, stroke: st.stroke, textColor: st.textColor, fontSize: 13 };
  });
  const width = PAD + elkW + PAD;
  return {
    section: section(spec, "Data · System design · Draft", width, TITLE + elkH),
    lanes: [], nodes: [...out, ...placeNotes(spec.notes ?? [], width + 24, TITLE)],
    edges: edges.map((e, i) => ({ id: `fk${i + 1}`, from: e.from, to: e.to, label: e.label, color: "#2563EB", lineType: "ELBOWED" })),
  };
}

// Sequence: participants across the top, a dashed lifeline under each, then one row per message
// (arrows between lifelines, top to bottom in time order). reply = dashed.
async function layoutSequence(spec) {
  const parts = spec.participants ?? [];
  const msgs = spec.messages ?? [];
  if (parts.length < 2 || !msgs.length) throw new Error("sequence needs participants (2+) and messages: [{ from, to, text, kind? }]");
  const at = new Map(parts.map((p, i) => [p, i]));
  for (const m of msgs) if (!at.has(m.from) || !at.has(m.to)) throw new Error(`Message "${m.text}": ${!at.has(m.from) ? m.from : m.to} is not a participant (${parts.join(", ")})`);
  // columns wide enough for the longest message between neighbours
  const longest = Math.max(...msgs.map((m) => visible(m.text).length + 4));
  const COL = Math.max(300, Math.min(460, longest * 8 + 60)), HEAD_W = 200, HEAD_H = 64, ROW = 80, TOP = TITLE + HEAD_H + 48;
  const cx = (i) => PAD + i * COL + HEAD_W / 2;
  const bottom = TOP + msgs.length * ROW + 40;
  const nodes = parts.map((p, i) => ({ id: `p${i + 1}`, kind: "shape", shape: "ROUNDED_RECTANGLE", role: "participant", x: PAD + i * COL, y: TITLE, width: HEAD_W, height: HEAD_H, text: p, fill: "#0B0D12", textColor: "#FFFFFF", fontSize: 16 }));
  const edges = parts.map((_, i) => ({ id: `life${i + 1}`, fromPoint: { x: cx(i), y: TITLE + HEAD_H }, toPoint: { x: cx(i), y: bottom }, dashed: true, color: "#CBD5E1", noArrow: true }));
  msgs.forEach((m, k) => {
    const y = TOP + k * ROW + ROW / 2;
    const a = at.get(m.from), b = at.get(m.to);
    const kind = m.kind ?? "call";
    if (a === b) {
      // a step inside one participant: a short hook to the right
      edges.push({ id: `m${k + 1}`, fromPoint: { x: cx(a), y: y - 12 }, toPoint: { x: cx(a) + 48, y: y + 12 }, label: `${k + 1}. ${m.text}`, lineType: "ELBOWED", color: "#334155" });
      return;
    }
    edges.push({ id: `m${k + 1}`, fromPoint: { x: cx(a), y }, toPoint: { x: cx(b), y }, label: `${k + 1}. ${m.text}`, dashed: kind === "reply" || undefined, lineType: "STRAIGHT", color: kind === "reply" ? "#64748B" : "#2563EB" });
  });
  const width = PAD + (parts.length - 1) * COL + HEAD_W + PAD;
  return {
    section: section(spec, "Sequence · System design · Draft", width, bottom),
    lanes: [], nodes: [...nodes, ...placeNotes(spec.notes ?? [], width + 24, TITLE)], edges,
  };
}

// board_map → plain text: one line per item, connectors as "a → b [label]". Far cheaper than JSON.
export function boardText(map) {
  const lines = [`Board "${map.file}" (${map.editorType})`];
  const nameOf = new Map();
  const all = [...map.sections.flatMap((s) => s.items), ...map.loose];
  for (const it of all) if (it.kind !== "connector") nameOf.set(it.id, it.waraId ?? it.id);
  const item = (it, lanes) => {
    if (it.kind === "connector") return `  ${nameOf.get(it.from) ?? it.from ?? "?"} → ${nameOf.get(it.to) ?? it.to ?? "?"}${it.label ? ` [${it.label}]` : ""}${it.dashed ? " (dashed)" : ""}`;
    // inside a lane band both ways (stickies beside the diagram belong to no lane)
    const cx = it.x + it.w / 2, cy = it.y + it.h / 2;
    const lane = lanes?.find((l) => cy >= l.y && cy < l.y + l.h && (l.w == null || (cx >= l.x && cx < l.x + l.w)))?.name;
    const tag = it.kind === "sticky" ? `sticky ${it.role ?? it.color ?? ""}`.trim() : it.kind === "shape" ? `${it.role ?? it.shape?.toLowerCase()}` : it.kind;
    return `  ${it.waraId ?? it.id} · ${tag}${lane ? ` @${lane}` : ""} · ${JSON.stringify(it.text ?? "")}`;
  };
  for (const s of map.sections) {
    lines.push("", `## ${s.name} (${s.id})${s.lanes?.length ? ` · lanes: ${s.lanes.map((l) => l.name).join(" | ")}` : ""}`);
    const shapes = s.items.filter((i) => i.kind !== "connector").sort((a, b) => a.x - b.x || a.y - b.y);
    for (const it of shapes) lines.push(item(it, s.lanes));
    for (const it of s.items.filter((i) => i.kind === "connector")) lines.push(item(it));
  }
  if (map.loose.length) {
    lines.push("", "## Outside sections");
    for (const it of map.loose) lines.push(item(it));
  }
  return lines.join("\n");
}
