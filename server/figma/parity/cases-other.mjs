// Parity cases for motion-figma, motion-handoff, figjam, motion, codegen and pdf.
// "<copy>" / "<write>" in args stand for a per-runtime copy of a temp folder (node-run.mjs and parity.rs
// each write into their own), so write paths are compared without one run seeing the other's files.
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { LOTTIE_PROJECT, REPORT_PDF } from "../../parity-projects.mjs";

export async function otherCases(ctx) {
  return [...motionFigmaCases(ctx), ...handoffCases(ctx), ...figjamCases(ctx), ...motionCases(ctx), ...codegenCases(ctx), ...(await pdfCases(ctx))];
}

const specOf = (repo, f) => JSON.parse(readFileSync(join(repo, "web/lottie", f), "utf8")).meta.wara.spec;

// A motion_read result as the plugin would report it for these tracks (what Figma stores after motion_animate).
function readFrom(rootName, layers, tracks, { duration = 1.4 } = {}) {
  const ease = (e) => {
    if (e == null) return undefined;
    const m = { linear: "LINEAR", in: "EASE_IN", out: "EASE_OUT", "in-out": "EASE_IN_AND_OUT", back: "EASE_OUT_BACK", "back-in": "EASE_IN_BACK", hold: "HOLD", spring: "BOUNCY", bounce: "BOUNCY" }[e];
    if (m) return m;
    const named = { standard: "0.2,0,0,1", decelerate: "0,0,0,1", accelerate: "0.3,0,1,1", emphasized: "0.05,0.7,0.1,1", ease: "0.25,0.1,0.25,1", anticipate: "0.68,-0.6,0.32,1.6" }[e];
    return named ? `cubic(${named})` : String(e).startsWith("cubic(") ? e : { spring: { mass: 1, stiffness: 200, damping: 12 } };
  };
  const nodes = layers.map((name, i) => ({ id: `9:${i + 10}`, name, parentId: "9:1", nth: 1, path: [], hint: { text: false, instances: 0, autoLayout: false, vectorOnly: true, image: false, component: null }, styles: [], tracks: [] }));
  for (const tr of tracks) {
    const pat = String(tr.layer ?? "all").toLowerCase().replace(/\*/g, ".*");
    const targets = nodes.filter((n) => pat === "all" || new RegExp(`^${pat}$`).test(n.name.toLowerCase()));
    targets.forEach((n, k) => {
      const off = (tr.stagger ?? 0) * k;
      const keys = Array.isArray(tr.keys) ? tr.keys : [[tr.start ?? 0, tr.from ?? 1, tr.ease], [tr.end ?? (tr.start ?? 0) + (tr.duration ?? 300), tr.to ?? 1]];
      const fieldsOf = { opacity: [["OPACITY", (v) => v]], rotate: [["ROTATION", (v) => (v * Math.PI) / 180]], draw: [["PATH_TRIM_END", (v) => v]],
        move: [["TRANSLATION_X", (v) => (Array.isArray(v) ? v[0] : v)], ["TRANSLATION_Y", (v) => (Array.isArray(v) ? v[1] : 0)]],
        scale: [["SCALE_X", (v) => (Array.isArray(v) ? v[0] : v)], ["SCALE_Y", (v) => (Array.isArray(v) ? v[1] : v)]] }[tr.prop] ?? [];
      for (const [field, conv] of fieldsOf) {
        n.tracks.push({ field, base: field.startsWith("TRANSLATION") ? 40 : field.startsWith("SCALE") || field === "OPACITY" ? 1 : 0, tracks: [{ op: "SET", keys: keys.map(([ms, v, e]) => ({ t: (ms + off) / 1000, v: conv(v), ...(e ? { ease: ease(e) } : {}) })) }] });
      }
    });
  }
  return { root: { id: "9:1", name: rootName }, timelines: [{ id: "tl", duration }], playhead: 0.25, animated: nodes.filter((n) => n.tracks.length) };
}

function motionFigmaCases({ repo }) {
  const cases = [];
  const add = (fn, ...args) => cases.push({ module: "motion-figma", fn, args });
  // unit-test fixtures
  add("tracksToItems", [{ layer: "Box", prop: "rotate", from: 0, to: 90, start: 200, end: 1000, ease: "back" }]);
  add("tracksToItems", [{ prop: "move", from: [0, 20], to: [0, 0] }, { prop: "scale", from: 0.5 }, { prop: "fill", from: "#ff0000", to: "#00ff0080" }]);
  add("tracksToItems", [{ prop: "opacity", keys: [[0, 0, "hold"], [300, 1], [600, 0.5]], ease: "out" }]);
  add("tracksToItems", [{ layer: "Fold1", prop: "move", keys: [[0, [0, 0]], [1300, [-283, 0]]] }]);
  add("tracksToItems", [{ prop: "skew" }]);
  add("tracksToItems", [{ prop: "x", from: 5, to: 10, ease: "wobble" }]);
  add("tracksToItems", [{ prop: "fill", from: "red", to: "#fff" }]);
  add("tracksToItems", [{ prop: "fill" }]);
  add("tracksToItems", [{ prop: "width", from: 100, to: 200, ease: [0.1, 0.2, 0.3, 0.4], stagger: 30, layer: "Card*" }, { prop: "radius", from: 0, to: 12, duration: 450, ease: "cubic(0.4,0,0.2)" }, { prop: "height", from: 4 }]);
  add("tracksToItems", [{ prop: "draw", from: 0, to: 1, ease: "gentle" }, { prop: "y", keys: [[0, 10], [200]] }, { prop: "scale", keys: [[0, [0.5]], [100, 1, "slow"], [200, [1, 2]]] }]);
  for (const f of ["logo-intro.json", "loading.json", "success.json", "plugin-connected.json"]) add("tracksToItems", specOf(repo, f).tracks);
  for (const e of ["cubic(0.2,0,0,1)", "standard", "spring", "back-in-out", [0, 0, 1, 1], ["linear"], "wobble", null, 3, "quick"]) add("toEasing", e);
  add("presetsToItems", [{ layer: "Box", style: "opacity", delay: 500, duration: 800, stagger: 100 }], [{ styleId: "Opacity", name: "motion.preset_name.opacity" }]);
  add("presetsToItems", [{ style: "Wiggle" }], []);
  add("presetsToItems", [{ style: "position", props: { distance: 24, delay: 9 }, delay: 120 }, { style: "SCALE" }], [{ styleId: "Opacity", name: "motion.preset_name.opacity" }, { styleId: "Pos1", name: "motion.preset_name.position" }, { styleId: "Scale", name: "x" }]);
  add("presetsToItems", [{ style: "nope" }], [{ styleId: "A", name: "a" }, { name: "b", styleId: "" }]);

  const read = {
    root: { id: "1:1", name: "Card" }, timelines: [{ id: "t", duration: 1.5 }],
    animated: [
      { id: "1:1", name: "Card", parentId: "0:1", nth: 1, tracks: [{ field: "OPACITY", base: 1, tracks: [{ op: "SET", keys: [{ t: 0, v: 0 }, { t: 1, v: 1 }] }] }], styles: [] },
      { id: "1:2", name: "Dot", parentId: "1:1", nth: 2, tracks: [
        { field: "ROTATION", base: 0, tracks: [{ op: "SET", keys: [{ t: 0.2, v: 0, ease: "EASE_OUT_BACK" }, { t: 1, v: Math.PI / 2, ease: "cubic(0.5,0,0.5,1)" }] }] },
        { field: "OPACITY", base: 1, tracks: [{ op: "SCALE", keys: [{ t: 0, v: 0, ease: "HOLD" }, { t: 0.8, v: 1 }] }] },
        { field: "TRANSLATION_Y", base: 0, tracks: [{ op: "SET", keys: [{ t: 0, v: 20, ease: "GENTLE" }, { t: 0.5, v: 0 }] }] },
      ], styles: [] },
      { id: "1:3", name: "Inner", parentId: "1:2", nth: 1, tracks: [{ field: "OPACITY", base: 1, tracks: [{ op: "SET", keys: [{ t: 0, v: 0 }, { t: 1, v: 1 }] }] }], styles: [] },
    ],
  };
  const nested = { root: { id: "F", name: "Scene" }, timelines: [{ duration: 1 }], animated: [
    { id: "F2", name: "Lens L", parentId: "G", nth: 1, path: ["Glasses"], tracks: [{ field: "TRANSLATION_Y", base: 10, tracks: [{ op: "SET", keys: [{ t: 0, v: 50, ease: "EASE_OUT" }, { t: 0.5, v: 0 }] }] }], styles: [] },
    { id: "G", name: "Glasses", parentId: "F", nth: 1, path: [], tracks: [{ field: "SCALE_XY", base: [1, 1], tracks: [{ op: "OFFSET", keys: [{ t: 0, v: [0.5, 0.5], ease: { spring: { stiffness: 100 } } }, { t: 1, v: [0, 0], ease: { variable: "x" } }] }] }], styles: [] },
  ] };
  const folds = { root: { id: "F", name: "Folds" }, timelines: [{ duration: 2 }], animated: [
    { id: "F1", name: "Fold1", parentId: "F", nth: 1, path: [], tracks: [{ field: "TRANSLATION_X", base: 480, tracks: [{ op: "SET", keys: [{ t: 0, v: 0 }, { t: 1.3, v: -283 }] }] }], styles: [] },
  ] };
  const odd = { root: { id: "R", name: "Odd" }, animationStyles: [{ name: "Fade in", styleId: "S:1", props: { duration: 1, delay: 0 }, description: "Fades\nsecond line" }, { name: "Bare", styleId: "S:2" }], animated: [
    { id: "R1", name: "Blob", type: "VECTOR", parentId: "R", nth: 3, path: [], styles: [{ name: "Fade in", props: { duration: 0.4 } }, { name: "Bare" }], tracks: [
      { field: "SCALE_X", base: 2, tracks: [{ op: "SCALE", keys: [{ t: 0, v: 0.5, ease: "QUICK" }, { t: 0.3, v: 1, ease: "EASE_IN_AND_OUT_BACK" }, { t: 0.6, v: 1.2 }] }, { op: "SET", keys: [] }] },
      { field: "SCALE_Y", base: 2, tracks: [{ op: "OFFSET", keys: [{ t: 0.1, v: 0.5, ease: "SLOW" }, { t: 0.6, v: 0 }] }] },
      { field: "fills[0]", base: "#ff0000", tracks: [{ op: "SET", keys: [{ t: 0, v: "#ff0000", ease: { mystery: 1 } }, { t: 0.5, v: "#0000ff" }] }] },
      { field: "BLUR", base: 0, tracks: [{ op: "SET", keys: [{ t: 0, v: 4 }] }] },
      { field: "WIDTH", base: 100, tracks: [{ op: "OFFSET", keys: [{ t: 0, v: -20, ease: "LINEAR" }, { t: 0.4, v: 0, ease: "SOMETHING" }] }] },
      { field: "TRANSLATION_XY", base: [10, 20], tracks: [{ op: "SET", keys: [{ t: 0, v: [5, 5] }, { t: 0.2, v: [0, 0] }] }] },
    ] },
  ] };
  const logo = readFrom("wridge Motion · logo-intro", ["LeftEar", "RightEar", "LeftHead", "RightHead", "Bridge", "EyeL", "EyeR"], specOf(repo, "logo-intro.json").tracks);
  const loading = readFrom("loading", ["Cat", "EyeL", "EyeR", "Dot1", "Dot2", "Dot3"], specOf(repo, "loading.json").tracks, { duration: 1.2 });
  for (const r of [read, nested, folds, odd, logo, loading, { root: { id: "E", name: "Empty" }, animated: [] }]) {
    add("motionText", r);
    add("motionToSpec", r);
  }
  return cases;
}

function handoffCases({ repo }) {
  const cases = [];
  const add = (read, context = "ui") => cases.push({ module: "motion-handoff", fn: "pipeline", args: [read, context] });
  const vec = { text: false, instances: 0, autoLayout: false, vectorOnly: true, image: false, component: null };
  const ui = { ...vec, vectorOnly: false, autoLayout: true, text: true };
  const key = (t, v, ease) => ({ t, v, ...(ease ? { ease } : {}) });
  const field = (name, keys, op = "SET", base = 1) => ({ field: name, base, tracks: [{ op, keys }] });
  const art = { root: { id: "A", name: "Success" }, timelines: [{ duration: 1.2 }], animated: [
    { id: "A1", name: "Ring", parentId: "A", nth: 1, hint: vec, styles: [], tracks: [field("SCALE_X", [key(0, 0, "BOUNCY"), key(0.5, 1)]), field("SCALE_Y", [key(0, 0, "BOUNCY"), key(0.5, 1)])] },
    { id: "A2", name: "Check", parentId: "A", nth: 1, hint: vec, styles: [], tracks: [field("PATH_TRIM_END", [key(0.3, 0, "cubic(0.2,0,0,1)"), key(0.7, 1)])] },
  ] };
  const screen = { root: { id: "S", name: "Home" }, timelines: [{ duration: 2 }], animated: [
    { id: "S1", name: "Title", parentId: "S", nth: 1, hint: ui, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.2, 1)]), field("TRANSLATION_Y", [key(0, 16, "EASE_OUT"), key(0.2, 0)], "SET", 24)] },
    { id: "S2", name: "Card", parentId: "S", nth: 1, hint: { ...ui, component: "Card" }, styles: [], tracks: [field("OPACITY", [key(0.1, 0, "EASE_OUT"), key(0.4, 1)]), field("WIDTH", [key(0.1, 200, "EASE_OUT"), key(0.8, 320)], "SET", 320)] },
    { id: "S3", name: "Confetti", parentId: "S", nth: 1, hint: { ...vec, component: "Lottie/Confetti" }, styles: [], tracks: [field("OPACITY", [key(0, 0), key(1.5, 1)])] },
  ] };
  const card = (id, nth, t) => ({ id, name: "Card", parentId: "H", nth, hint: ui, styles: [], tracks: [
    field("OPACITY", [key(t, 0, "cubic(0.2,0,0,1)"), key(t + 0.3, 1)]),
    field("TRANSLATION_Y", [key(t, 24, "cubic(0.2,0,0,1)"), key(t + 0.3, 0)], "SET", 76 + nth * 72),
  ] });
  const home = { root: { id: "H", name: "Home" }, timelines: [{ duration: 0.8 }], animated: [card("H1", 1, 0.1), card("H2", 2, 0.16), card("H3", 3, 0.22)] };
  const item = (id, t, dist, dur, ease) => ({ id, name: `Item ${id}`, parentId: "P", nth: 1, hint: ui, styles: [], tracks: [
    field("OPACITY", [key(t, 0, ease), key(t + dur, 1)]), field("TRANSLATION_Y", [key(t, dist, ease), key(t + dur, 0)], "SET", 100),
  ] });
  const page = (animated) => ({ root: { id: "P", name: "Page" }, timelines: [{ duration: 2 }], animated });
  const flat = page(["a", "b", "c", "d"].map((id) => item(id, 0, 20, 0.3, "EASE_IN")));
  const led = page([item("a", 0, 32, 0.42, "cubic(0.16,1,0.3,1)"), item("b", 0.12, 12, 0.26, "EASE_OUT"), item("c", 0.16, 12, 0.26, "EASE_OUT")]);
  const slow = { root: { id: "L", name: "Hero" }, timelines: [{ duration: 1.5 }], animated: [
    { id: "L1", name: "Glasses", parentId: "L", nth: 1, hint: vec, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.9, 1)])] },
  ] };
  const many = page(Array.from({ length: 8 }, (_, i) => item(String.fromCharCode(97 + i), i * 0.05, 16, 0.3, ["EASE_OUT", "EASE_IN", "LINEAR", "BOUNCY", "EASE_OUT_BACK", "HOLD"][i % 6])));
  const mixed = { root: { id: "M", name: "Mixed screen · Landing" }, timelines: [{ duration: 4 }], animated: [
    { id: "M", name: "Mixed screen", parentId: "0", nth: 1, hint: ui, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.3, 1)])] },
    { id: "M1", name: "Hero art", parentId: "M", nth: 1, hint: vec, styles: [{ name: "Float" }], tracks: [field("ROTATION", [key(0, 0, "EASE_IN_AND_OUT"), key(1, 0.2, "HOLD"), key(2, 0.2, "LINEAR"), key(3.2, 0)])] },
    { id: "M2", name: "Photo", parentId: "M", nth: 1, hint: { ...vec, vectorOnly: false, image: true }, styles: [], tracks: [field("SCALE_X", [key(0, 1.2, "EASE_OUT"), key(1.2, 1)]), field("SCALE_Y", [key(0, 1.1, "EASE_OUT"), key(1.2, 1)]), field("CORNER_RADIUS", [key(0, 0), key(1.2, 24)], "SET", 0)] },
    { id: "M3", name: "Badge", parentId: "M", nth: 2, hint: { ...vec, vectorOnly: false, instances: 2 }, styles: [], tracks: [field("fills[0]", [key(0, "#ff0000", "EASE_OUT"), key(0.5, "#00ff00")], "SET", "#ff0000"), field("HEIGHT", [key(0, 10), key(0.5, 40)], "SET", 40)] },
    { id: "M4", name: "Only style", parentId: "M", nth: 1, hint: ui, styles: [{ name: "Pulse" }], tracks: [] },
    { id: "M5", name: "Plain", parentId: "M", nth: 1, styles: [], tracks: [field("TRANSLATION_X", [key(0.5, -40, "EASE_OUT"), key(0.9, 0)], "SET", 10), field("TRANSLATION_Y", [key(0.5, 0, "EASE_OUT"), key(0.7, 8), key(0.9, 0)], "SET", 10)] },
  ] };
  const logo = readFrom("wridge Motion · logo-intro", ["LeftEar", "RightEar", "LeftHead", "RightHead", "Bridge", "EyeL", "EyeR"], specOf(repo, "logo-intro.json").tracks);
  const success = readFrom("success", ["Disc", "Spark1", "Spark2", "Spark3", "Spark4", "CheckInk"], specOf(repo, "success.json").tracks, { duration: 1.1 });
  const uiLogo = structuredClone(logo);
  uiLogo.animated.forEach((n, i) => (n.hint = i % 2 ? ui : { ...ui, component: "Logo" }));
  for (const r of [art, screen, home, flat, led, slow, many, mixed, logo, success, uiLogo]) add(r);
  for (const r of [art, slow, mixed, logo]) add(r, "landing");
  for (const name of ["Eyewear Atelier landing · Storyboard · Draft", "Checkout · Prototype · Draft", "BRAND page", null, 42]) cases.push({ module: "motion-handoff", fn: "motionContext", args: [name] });
  return cases;
}

function figjamCases() {
  const cases = [];
  const add = (spec) => cases.push({ module: "figjam", fn: "layoutDiagram", args: [spec] });
  const BOOKING = {
    title: "จองบูธ · System design · Draft", lanes: ["ร้านค้า", "server"],
    nodes: [
      { id: "s", kind: "start", lane: "ร้านค้า" }, { id: "pick", text: "เลือกบูธ", lane: "ร้านค้า" }, { id: "hold", text: "ล็อกบูธ 15 นาที", lane: "server" },
      { id: "ok", text: "สลิปตรงไหม", kind: "decision", lane: "server" }, { id: "ticket", text: "ออกตั๋ว", lane: "server" }, { id: "e", kind: "end", lane: "ร้านค้า" },
    ],
    edges: [{ from: "s", to: "pick" }, { from: "pick", to: "hold" }, { from: "hold", to: "ok" }, { from: "ok", to: "ticket", label: "ตรง" }, { from: "ok", to: "pick", label: "ไม่ตรง" }, { from: "ticket", to: "e" }],
    notes: [{ text: "ร้านแก้สลิปได้กี่ครั้ง?", kind: "question" }],
  };
  add(BOOKING);
  add({ nodes: [{ id: "a", text: "x" }], edges: [{ from: "a", to: "b" }] });
  add({ lanes: ["A"], nodes: [{ id: "a", text: "x", lane: "B" }] });
  add({ nodes: [{ id: "a", text: "x", kind: "question" }] });
  add({ nodes: [{ id: "a", text: "x", kind: "blob" }] });
  add({ nodes: [{ id: "a" }, { id: "a" }] });
  add({ notes: [{ text: "x", kind: "blob" }] });
  add({ type: "state", title: "booking", nodes: [{ id: "hold", text: "hold" }, { id: "paid", text: "paid" }], edges: [{ from: "hold", to: "paid", label: "slip ok" }] });
  add({ type: "er", tables: [
    { name: "bookings", columns: [{ name: "id", type: "uuid", pk: true }, { name: "booth_id", type: "uuid", fk: "booths.id" }] },
    { name: "booths", columns: [{ name: "id", type: "uuid", pk: true }, { name: "label", type: "text" }] },
  ] });
  add({ type: "er", tables: [{ name: "a", columns: [{ name: "x_id", fk: "x.id" }] }] });
  add({ type: "er" });
  add({ type: "sequence", participants: ["web", "server", "Slip2Go"], messages: [
    { from: "web", to: "server", text: "upload slip" }, { from: "server", to: "Slip2Go", text: "verify" }, { from: "Slip2Go", to: "server", text: "result", kind: "reply" },
  ] });
  add({ type: "sequence", participants: ["a", "b"], messages: [{ from: "a", to: "c", text: "x" }] });
  add({ type: "sequence", participants: ["a"], messages: [] });
  add({ type: "mindmap" });
  add({ title: "empty" });
  // realistic: a bigger flow with lanes, loops, statuses, waits and notes
  add({
    title: "ชำระเงิน · System design · Draft", x: 4000, lanes: ["ลูกค้า", "web", "server", "ธนาคาร"],
    nodes: [
      { id: "start", kind: "start", lane: "ลูกค้า" }, { id: "cart", text: "ตะกร้าสินค้า", lane: "ลูกค้า", status: "existing" },
      { id: "checkout", text: "กดชำระเงิน / Checkout", lane: "web", status: "changed" }, { id: "quote", text: "คำนวณราคา + ค่าส่ง + ส่วนลดคูปอง", lane: "server" },
      { id: "stock", text: "สต็อกพอไหม?", kind: "decision", lane: "server" }, { id: "reserve", text: "จองสต็อก 10 นาที", lane: "server" },
      { id: "qr", text: "สร้าง QR PromptPay", lane: "server" }, { id: "scan", text: "สแกนจ่าย", lane: "ลูกค้า" },
      { id: "wait", text: "รอธนาคารแจ้งผล (webhook)", kind: "wait", lane: "server" }, { id: "bank", text: "ยืนยันยอดเงิน", lane: "ธนาคาร", status: "existing" },
      { id: "paid", text: "จ่ายสำเร็จ?", kind: "decision", lane: "server" }, { id: "order", text: "สร้างออเดอร์", lane: "server", status: "new" },
      { id: "release", text: "ปล่อยสต็อก", lane: "server", status: "removed" }, { id: "done", kind: "end", lane: "ลูกค้า" },
      { id: "soldout", text: "แจ้งของหมด", lane: "web", status: "weird" },
    ],
    edges: [
      { from: "start", to: "cart" }, { from: "cart", to: "checkout" }, { from: "checkout", to: "quote" }, { from: "quote", to: "stock" },
      { from: "stock", to: "reserve", label: "พอ" }, { from: "stock", to: "soldout", label: "ไม่พอ" }, { from: "soldout", to: "cart", label: "แก้ตะกร้า" },
      { from: "reserve", to: "qr" }, { from: "qr", to: "scan" }, { from: "scan", to: "bank" }, { from: "bank", to: "wait", label: "webhook" },
      { from: "wait", to: "paid" }, { from: "paid", to: "order", label: "สำเร็จ", id: "ok-edge" }, { from: "paid", to: "release", label: "หมดเวลา", dashed: true },
      { from: "release", to: "cart", dashed: false }, { from: "order", to: "done" },
    ],
    notes: [{ text: "คูปองใช้ซ้ำได้ไหม?", kind: "question" }, { text: "webhook ซ้ำ → ต้อง idempotent", kind: "risk" }, { text: "QR หมดอายุ 10 นาที", kind: "agreed", id: "n-qr" }, { text: "ค่าส่งคิดตามน้ำหนัก" }],
  });
  add({ title: "No lanes", nodes: [{ id: "a", text: "Login" }, { id: "b", text: "OTP ok?", kind: "decision" }, { id: "c", text: "Home" }, { id: "d", text: "Locked out after five wrong tries in a row", kind: "wait" }],
    edges: [{ from: "a", to: "b" }, { from: "b", to: "c", label: "yes" }, { from: "b", to: "d", label: "no ×5" }, { from: "b", to: "a", label: "retry" }] });
  add({ type: "state", nodes: [{ id: "draft", text: "draft" }, { id: "review", text: "in review" }, { id: "ok", text: "approved", status: "existing" }, { id: "x", text: "rejected" }, { id: "s", kind: "start" }],
    edges: [{ from: "s", to: "draft" }, { from: "draft", to: "review", label: "submit" }, { from: "review", to: "ok", label: "approve" }, { from: "review", to: "x", label: "reject" }, { from: "x", to: "draft", label: "edit" }, { from: "ok", to: "ok", label: "re-save" }] });
  add({ type: "er", title: "Booth data", notes: [{ text: "soft delete?", kind: "question" }, { text: "uuid everywhere", kind: "agreed" }, { text: "money in satang", kind: "note" }], tables: [
    { name: "events", status: "existing", columns: [{ name: "id", type: "uuid", pk: true }, { name: "title", type: "text" }, { name: "starts_at", type: "timestamptz" }] },
    { name: "booths", columns: [{ name: "id", type: "uuid", pk: true }, { name: "event_id", type: "uuid", fk: "events.id" }, { name: "label", type: "text", note: "A1, A2…" }, { name: "price", type: "int", note: "satang" }] },
    { id: "bk", name: "bookings", status: "changed", columns: [{ name: "id", type: "uuid", pk: true }, { name: "booth_id", type: "uuid", fk: "booths.id" }, { name: "vendor_id", type: "uuid", fk: "vendors.id" }, { name: "status", type: "booking_status" }, { name: "slip_url" }] },
    { name: "vendors", columns: [{ name: "id", type: "uuid", pk: true }, { name: "line_user_id", type: "text" }, { name: "ชื่อร้าน", type: "text" }] },
    { name: "payments", status: "removed", columns: [{ name: "id", pk: true }, { name: "booking_id", fk: "bk.id" }] },
    { name: "audit" },
  ] });
  add({ type: "sequence", title: "Slip check", x: 0, y: 900, participants: ["ร้านค้า", "LIFF", "server", "Slip2Go", "LINE OA"], notes: [{ text: "timeout 30s", kind: "risk" }], messages: [
    { from: "ร้านค้า", to: "LIFF", text: "อัปโหลดสลิป" }, { from: "LIFF", to: "server", text: "POST /bookings/:id/slip (multipart)" }, { from: "server", to: "server", text: "ตรวจขนาดไฟล์" },
    { from: "server", to: "Slip2Go", text: "verify(qr payload)" }, { from: "Slip2Go", to: "server", text: "amount, ref, time", kind: "reply" }, { from: "server", to: "LINE OA", text: "push: จองสำเร็จ" },
    { from: "server", to: "LIFF", text: "200 OK", kind: "reply" },
  ] });
  cases.push({ module: "figjam", fn: "boardText", args: [{
    file: "book-lock board", editorType: "figjam", loose: [{ id: "9:1", kind: "sticky", color: "yellow", x: 0, y: 0, w: 100, h: 100, text: "loose" }, { id: "9:2", kind: "connector", from: "9:1", to: "2:9" }],
    sections: [{
      id: "1:1", name: "จองบูธ · System design · Draft", lanes: [{ name: "ร้านค้า", x: 0, y: 72, w: 1100, h: 160 }, { name: "server", x: 0, y: 232, w: 1100, h: 160 }],
      items: [
        { id: "2:1", kind: "shape", role: "step", waraId: "pick", x: 300, y: 100, w: 220, h: 96, text: "เลือกบูธ" },
        { id: "2:2", kind: "shape", role: "step", waraId: "hold", x: 600, y: 260, w: 220, h: 96, text: "ล็อกบูธ 15 นาที" },
        { id: "2:4", kind: "shape", shape: "DIAMOND", x: 600, y: 100, w: 220, h: 96, text: "ok?\n\"quoted\"" },
        { id: "2:3", kind: "sticky", role: "question", x: 1200, y: 72, w: 200, h: 200, text: "แก้สลิปได้กี่ครั้ง?" },
        { id: "2:5", kind: "text", x: 10, y: 5, w: 10, h: 10 },
        { id: "3:1", kind: "connector", from: "2:1", to: "2:2", label: "ยืนยัน" }, { id: "3:2", kind: "connector", from: "2:2", to: "2:4", dashed: true },
      ],
    }, { id: "1:2", name: "Empty", items: [] }],
  }] });
  return cases;
}

function motionCases({ out, repo }) {
  const cases = [];
  // read-only on a real project with web/lottie files (listLottie + readLottie never write)
  const wb = LOTTIE_PROJECT;
  cases.push({ module: "motion", fn: "listLottie", args: [wb] });
  for (const p of ["server/package.json", "../secret.json", "C:/Windows/win.ini", "server", "nope.json"]) cases.push({ module: "motion", fn: "readLottie", args: [wb, p] });
  for (const p of ["../x", "a/../../x", ".", "", "sub/x.json", "..hidden/x.json", "C:\\Users", "a\\..\\b.json", "SUB/./y.json"]) cases.push({ module: "motion", fn: "insideProject", args: [wb, p] });
  // the wridge repo's own web/lottie (read-only)
  cases.push({ module: "motion", fn: "listLottie", args: [join(repo, "web")] });
  for (const f of ["logo-intro.json", "success.json"]) cases.push({ module: "motion", fn: "readLottie", args: [join(repo, "web"), `lottie/${f}`] });
  // write tests run on copies, one per runtime
  const tmp = join(out, "motion-project");
  rmSync(tmp, { recursive: true, force: true });
  mkdirSync(join(tmp, "assets/lottie"), { recursive: true });
  mkdirSync(join(tmp, "node_modules/pkg"), { recursive: true });
  const lottie = readFileSync(join(repo, "web/lottie/success.json"), "utf8");
  writeFileSync(join(tmp, "assets/lottie/success.json"), lottie);
  writeFileSync(join(tmp, "assets/lottie/plain.json"), JSON.stringify({ ...JSON.parse(lottie), meta: undefined, nm: "" }));
  writeFileSync(join(tmp, "node_modules/pkg/anim.json"), lottie);
  writeFileSync(join(tmp, "assets/data.json"), JSON.stringify({ layers: [], fr: "x", other: "not lottie".repeat(30) }));
  writeFileSync(join(tmp, "package.json"), "{}");
  for (const who of ["node", "rust"]) {
    const dir = join(out, `motion-project-${who}`);
    rmSync(dir, { recursive: true, force: true });
    cpSync(tmp, dir, { recursive: true });
  }
  cases.push({ module: "motion", fn: "listLottie", args: [tmp] });
  cases.push({ module: "motion", fn: "markSentToFigma", args: ["<copy>", "assets/lottie/success.json", { componentId: "12:34", file: "wridge" }] });
  cases.push({ module: "motion", fn: "markSentToFigma", args: ["<copy>", "assets/lottie/plain.json", { componentId: "5:6" }] });
  cases.push({ module: "motion", fn: "markSentToFigma", args: ["<copy>", "../outside.json", {}] });
  cases.push({ module: "motion", fn: "listLottie", args: ["<copy>"] });
  for (const j of [{ v: "5.7.4", fr: 60, layers: [], op: 10, w: 1 }, { v: 5, fr: 60, layers: [], op: 10, w: 1 }, { v: "5", fr: "30", layers: [], op: 1.5, w: "2" }, { v: "5", fr: 0, layers: [], op: 1, w: 1 }, null, [], { v: "5", fr: 60, layers: [], op: "10", w: 1 }]) {
    cases.push({ module: "motion", fn: "isLottie", args: [j] });
  }
  for (const j of [{ v: "5.7.4", fr: 60, op: 10, meta: { x: 1 }, layers: [{ nm: "a", "1": 2 }] }, { b: 0.1, a: [1e21, 1e-7, -0, 0.0000015], meta: 1, "10": "x", "2": "y", s: "é\u0001\"" }, JSON.parse(lottie)]) cases.push({ module: "motion", fn: "contentHash", args: [j] });
  return cases;
}

function codegenCases({ out }) {
  const cases = [];
  const add = (fn, ...args) => cases.push({ module: "codegen", fn, args });
  const entry = { element: "button", className: "btn", variants: { "Type=Primary": "btn-p", "Type=Danger": "btn-d", "Size=Small": "btn-sm" } };
  add("resolveEntry", entry, "Type=Danger, State=Hover, Size=Small");
  add("resolveEntry", { import: "@/ui/button", component: "Button" }, "Type=Primary");
  add("resolveEntry", entry, null);
  add("resolveEntry", null, "x");
  add("resolveEntry", { className: "a", variants: {}, z: 1 }, " Type = Primary ");
  for (const n of ["Home Screen", "  Ünïcødé — Name ", "ﬁle №1", "already-kebab", "", "ไทย", "A/B\\C", "Ǆemal ǅ"]) add("kebab", n);
  for (const p of ["C:/x", "relative/x", "/abs", "C:\\a\\b", "\\\\server\\share", "C:"]) add("requireAbsolute", p, "outDir");
  const tw = {
    file: "t", collections: [{ name: "c", defaultMode: "Light" }],
    variables: [{ token: "color-tile", type: "COLOR", collection: "c", values: { Light: "#ffffff" } }, { token: "color-card", type: "COLOR", collection: "c", values: { Light: "#eeeeee" } }],
    paintStyles: [],
    textStyles: [{ token: "text-tile", family: "Kanit", style: "Bold", size: 24, lineHeight: "130%", letterSpacing: "0px" }, { token: "text-body", family: "Kanit", style: "Regular", size: 16, lineHeight: "150%", letterSpacing: "0px" }],
    effectStyles: [{ token: "shadow-card", shadows: [{ x: 0, y: 2, blur: 4, spread: 0, color: "#0000001a" }] }],
  };
  // a realistic design system: modes, aliases, floats, strings, booleans, gradients, image paints, shadows, text styles
  const ds = {
    file: "book-lock · Foundations",
    collections: [{ name: "Primitives", defaultMode: "Value" }, { name: "Theme", defaultMode: "Light" }, { name: "Size", defaultMode: "Default" }],
    variables: [
      { token: "blue-500", type: "COLOR", collection: "Primitives", values: { Value: "#2f6bff" } },
      { token: "gray-900", type: "COLOR", collection: "Primitives", values: { Value: "#111111" } },
      { token: "white", type: "COLOR", collection: "Primitives", values: { Value: "#ffffffcc" } },
      { token: "colors-primary", type: "COLOR", collection: "Theme", values: { Light: { alias: "blue-500" }, Dark: { alias: "white" }, "High contrast": "#0000ff" } },
      { token: "color-text", type: "COLOR", collection: "Theme", values: { Light: { alias: "gray-900" }, Dark: { alias: "white" } } },
      { token: "color-ghost", type: "COLOR", collection: "Theme", values: { Light: { alias: "missing-token" } } },
      { token: "radius-md", type: "FLOAT", collection: "Size", values: { Default: 8, Compact: 6.5 } },
      { token: "corner-radius-lg", type: "FLOAT", collection: "Size", values: { Default: 16 } },
      { token: "spacing-4", type: "FLOAT", collection: "Size", values: { Default: 16, Compact: 12 } },
      { token: "gap-2", type: "FLOAT", collection: "Size", values: { Default: { alias: "spacing-4" } } },
      { token: "font-size-body", type: "FLOAT", collection: "Size", values: { Default: 16 } },
      { token: "font-weight-bold", type: "FLOAT", collection: "Size", values: { Default: 700 } },
      { token: "line-height-tight", type: "FLOAT", collection: "Size", values: { Default: 1.2 } },
      { token: "opacity-disabled", type: "FLOAT", collection: "Size", values: { Default: 0.38 } },
      { token: "z-index-modal", type: "FLOAT", collection: "Size", values: { Default: 100 } },
      { token: "elevation", type: "FLOAT", collection: "Size", values: { Default: 3 } },
      { token: "2xl", type: "FLOAT", collection: "Size", values: { Default: 40 } },
      { token: "font-family-body", type: "STRING", collection: "Size", values: { Default: "IBM Plex Sans Thai" } },
      { token: "brand-name", type: "STRING", collection: "Size", values: { Default: "book \"lock\"" } },
      { token: "feature-x", type: "BOOLEAN", collection: "Size", values: { Default: true } },
    ],
    paintStyles: [
      { name: "Brand/Primary", token: "color-brand", value: "#2f6bff" },
      { name: "Surface", token: "surface", value: { token: "white" } },
      { name: "Sunset", token: "gradient-sunset", value: { gradient: "linear", stops: [{ color: "#ff7a00", position: 0 }, { color: "#ff2d55", position: 0.555 }] } },
      { name: "Glow", token: "glow", value: { gradient: "radial", stops: [{ color: "#ffffff", position: 0 }, { color: "#ffffff00", position: 1 }] } },
      { name: "Photo", token: "photo", value: null },
      { name: "Mixed", token: "mixed", value: { image: true } },
    ],
    textStyles: [
      { token: "text-display", family: "IBM Plex Sans Thai", style: "SemiBold Italic", size: 48, lineHeight: "120%", letterSpacing: "-0.02em" },
      { token: "type-body", family: "IBM Plex Sans Thai", style: "Regular", size: 16, lineHeight: "24px", letterSpacing: "0px" },
      { token: "caption", family: "JetBrains Mono", style: "Ultra-Bold", size: 12.5, lineHeight: "auto", letterSpacing: "0.5px" },
      { token: "text-primary", family: "Kanit", style: "Black", size: 20, lineHeight: "150%", letterSpacing: "0em" },
      { token: "font-9", family: "Kanit", style: "Weird", size: 9, lineHeight: "11px", letterSpacing: "1.5em" },
    ],
    effectStyles: [
      { token: "shadow-sm", shadows: [{ x: 0, y: 1, blur: 2, spread: 0, color: "#0000001a" }, { x: 0, y: 0, blur: 0, spread: 1, color: "#00000014", inset: true }] },
      { token: "elevation-primary", shadows: [{ x: 0, y: 8, blur: 24, spread: -4, color: "#2f6bff40" }] },
      { token: "blur-only", shadows: [] },
    ],
  };
  for (const data of [tw, ds]) for (const f of ["tailwind", "css", "dart", "json"]) add("formatTokens", data, f);
  // annotateComponents: a design-context tree
  const ctxTree = { kind: "frame", name: "Home", children: [
    { kind: "instance", name: "Btn", component: { name: "Button", variant: "Type=Danger, Size=Small" } },
    { kind: "instance", name: "Btn2", component: { name: "Button", variant: "Type=Primary" } },
    { kind: "instance", name: "Chip", component: { name: "Chip" } },
    { kind: "frame", name: "Row", children: [{ kind: "instance", name: "Avatar", component: { name: "Avatar", variant: "Size=L" } }, { kind: "instance", name: "Ic", component: { name: "Icon/Close" } }, { kind: "instance", name: "Ic2", component: { name: "Icon/Close" } }] },
  ] };
  add("annotateComponents", ctxTree, { Button: entry, "Avatar/Size=L": { import: "@/ui/avatar", component: "Avatar", props: { size: "lg" } }, Chip: { component: "Chip" } });
  add("annotateComponents", ctxTree, {});
  // mergeBreakpoints: mobile / tablet / desktop of one screen
  const mk = (w, extra = {}) => ({ root: { id: "1", name: `Home/${w}`, kind: "frame", width: w, layout: { mode: w > 700 ? "row" : "column", gap: w > 700 ? 24 : 16, padding: [16, 16] }, fill: { token: "surface" }, children: [
    { id: "2", kind: "text", name: "Title", text: "สวัสดี", style: { size: w > 700 ? 48 : 32, weight: 700 } },
    ...(w > 700 ? [{ id: "9", kind: "frame", name: "Sidebar", width: 280 }] : [{ id: "8", kind: "instance", name: "Menu", component: { name: "Burger" } }]),
    { id: "3", kind: "repeat", count: w > 700 ? 6 : 3, template: { id: "4", kind: "frame", name: "Card", width: w > 700 ? 300 : null, children: [{ id: "5", kind: "text", name: "Label", text: "x" }, ...(w > 1000 ? [{ id: "6", kind: "image", name: "Thumb" }] : [])] } },
    { id: "7", kind: "text", name: "Title", text: "second title" },
    ...(extra.empty ? [{ id: "10", kind: "frame", name: "Empty", props: {} }] : []),
  ] }, tokens: { surface: "#fff", [`w${w}`]: w, "10": "ten" }, components: [{ name: "Burger", id: "c1" }, { name: `C${w}`, id: "c2" }], fonts: ["Kanit", w > 700 ? "Inter" : "Kanit"], assets: [{ name: "logo", format: "svg", nodeId: `a${w}` }, { name: `img${w}`, format: "png" }] });
  add("mergeBreakpoints", [{ bp: "base", context: mk(390) }, { bp: "md", context: mk(768, { empty: true }) }, { bp: "xl", context: mk(1280) }]);
  add("mergeBreakpoints", [{ bp: "base", context: { root: { kind: "frame", name: "A", onlyAt: ["base"], children: [{ kind: "text", name: "t", onlyAt: [] }] }, tokens: null, components: [], fonts: [], assets: [] } }, { bp: "lg", context: { root: { kind: "frame", name: "A" }, tokens: {}, components: [], fonts: [], assets: [] } }]);
  // component map files (temp folders)
  const dir = join(out, "codegen-project");
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(join(dir, "bad"), { recursive: true });
  writeFileSync(join(dir, "wara.components.json"), JSON.stringify({ Zeta: { component: "Z" }, alpha: { component: "A" }, Émile: { c: 1 }, _x: {}, "10": {}, "Button/Type=Primary": { className: "btn" } }, null, 2));
  writeFileSync(join(dir, "bad/wara.components.json"), "{ not json");
  add("readComponentMap", dir);
  add("readComponentMap", join(dir, "none"));
  add("readComponentMap", "relative/dir");
  for (const who of ["node", "rust"]) {
    rmSync(join(out, `codegen-write-${who}`), { recursive: true, force: true });
    mkdirSync(join(out, `codegen-write-${who}`), { recursive: true });
  }
  add("writeComponentMap", "<write>", { Zeta: { component: "Z" }, alpha: { component: "A" }, "b-2": 1, B: 2, "a b": 3, _y: 4, Ábc: 5, "10": 6, "9": 7, zeta: 8, "Button/Size=Small": { className: "x" }, ab: 9, "a-b": 10, "a.b": 11, "ก": 12, "ข้าว": 13 });
  return cases;
}

async function pdfCases({ repo, out }) {
  const cases = [];
  const add = (fn, ...args) => cases.push({ module: "pdf", fn, args });
  const frame = (name, x = 0, y = 0, extra = {}) => ({ id: name, name, type: "FRAME", x, y, width: 595, height: 842, ...extra });
  for (const n of ["01 Cover", "100 ภาคผนวก", "  7", "Cover", "A1", "12abc", "1.5 x", "007", "99999999999999999999 big", "١٢ arabic digits", 42, null]) add("pageNumber", n);
  add("orderPages", [frame("1000 z"), frame("10 c"), frame("2 b", 900), frame("100 d"), frame("01 a", 2000)]);
  add("orderPages", [frame("note", 0, 900), frame("2 b"), frame("cover", 0, 0), frame("1 a"), frame("3 hidden", 0, 0, { visible: false }), { id: "t", name: "4 text", type: "TEXT", x: 0, y: 0 }]);
  add("orderPages", [frame("2 right", 700), frame("2 left", 0), frame("1")]);
  add("orderPages", [frame("cover", 10, 10), frame("back", 0, 10), { ...frame("x"), type: "COMPONENT" }, { ...frame("3 y"), type: "INSTANCE" }, frame("3 z", 0, -5), frame("3 w", 0, -5, { visible: false })]);
  add("orderPages", [frame("only loose", 5, 5), frame("hidden", 0, 0, { visible: false })]);
  add("orderPages", []);
  add("orderPages", null);
  for (const n of ['a/b: "c"', "Laravel DOC 1 · Doc · Draft", "   ", "x<y>|z?*", "tab\there  two"]) add("pdfFileName", n);
  // PDFs to merge: made by pdf-lib (like Figma's exports) plus a real document from disk (read-only)
  const { PDFDocument, StandardFonts, rgb } = await import(pathToFileURL(join(repo, "legacy-node/node_modules/pdf-lib/cjs/index.js")).href);
  const dir = join(out, "pdf");
  mkdirSync(dir, { recursive: true });
  const make = async (file, sizes, { title, text } = {}) => {
    const d = await PDFDocument.create();
    if (title) d.setTitle(title);
    const font = await d.embedFont(StandardFonts.Helvetica);
    for (const [w, h] of sizes) {
      const p = d.addPage([w, h]);
      p.drawRectangle({ x: 10, y: 10, width: w / 2, height: h / 3, color: rgb(0.18, 0.42, 1) });
      if (text) p.drawText(`${text} ${w}x${h}`, { x: 20, y: h - 40, size: 14, font });
    }
    const f = join(dir, file);
    writeFileSync(f, await d.save({ useObjectStreams: false }));
    return f;
  };
  const a4 = await make("a4.pdf", [[595, 842]], { text: "Cover" });
  const land = await make("land.pdf", [[842, 595]], { text: "Wide", title: "own title" });
  const small = await make("small.pdf", [[100, 200]]);
  const multi = await make("multi.pdf", [[595.28, 841.89], [300, 300], [612, 792]], { text: "multi" });
  const objStreams = join(dir, "objstreams.pdf");
  writeFileSync(objStreams, await (await PDFDocument.load(readFileSync(multi))).save({ useObjectStreams: true }));
  add("mergePdfs", [a4, land, small], "Book");
  add("mergePdfs", [multi, a4], "ภาษาไทย · Doc");
  add("mergePdfs", [objStreams], null);
  add("mergePdfs", [], "Empty");
  const real = REPORT_PDF;
  if (real && existsSync(real)) add("mergePdfs", [real, a4, real], "Report");
  const nodes = {
    s: { id: "s", name: "Laravel DOC 1 · Doc · Draft", type: "SECTION", children: [frame("2 b"), frame("1 a")] },
    t: { id: "t", name: "Book · Cover", type: "SECTION", children: [frame("cover"), frame("2 two", 0, 0, { visible: false }), frame("1 one")] },
    f: { id: "f", name: "Single frame", type: "FRAME", width: 842, height: 595, x: 0, y: 0 },
    x: { name: "x", type: "TEXT" },
    e: { name: "empty", type: "SECTION", children: [] },
    __pdf: { "1 a": a4, "2 b": land, cover: multi, f: land, default: small },
  };
  add("designPdf", nodes, "s");
  add("designPdf", nodes, ["t", "f", "s"]);
  add("designPdf", nodes, "x");
  add("designPdf", nodes, "e");
  return cases;
}
