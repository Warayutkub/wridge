// The sunset scene on the website's "Bend shapes" card: drawn in Figma only from rectangles, ellipses and lines,
// bent by wridge's own tools (bend_shape · vector_edit · set_stroke · boolean_shapes).
// Needs the wridge server and the plugin running in the target file: node tools/site/shapes-scene.mjs <figma file name>
// Then export the frame at 2× and save it as site/assets/img/figma-shapes.webp (1600×900).
import { writeFileSync } from "node:fs";
const FILE = process.argv[2] ?? "utils";
async function call(name, args = {}) {
  const r = await fetch("http://127.0.0.1:3156/api/tool", { method: "POST", headers: { "Content-Type": "application/json", "X-Wridge-Session": "site-shapes" }, body: JSON.stringify({ name, args }) });
  return r.json();
}
const J = (r) => { const t = r.content?.[0]?.text; try { return JSON.parse(t); } catch { return t; } };
async function go(n, a, tries = 4) {
  const r = await call(n, a);
  if (r.isError) {
    const m = r.content?.[0]?.text ?? "";
    if (/Unable to establish connection/.test(m) && tries > 1) { await new Promise((ok) => setTimeout(ok, 2000)); return go(n, a, tries - 1); }
    throw new Error(`${n}: ${m}`);
  }
  return J(r);
}
await go("use_file", { file: FILE });
const info = await go("get_document_info");
for (const n of info.topLevel.filter((n) => n.name === "wridge · Shapes showcase v3")) await go("delete_node", { nodeId: n.id });
const W = 1600, H = 900;
const F = (await go("create_frame", { name: "wridge · Shapes showcase v3", x: Math.min(...info.topLevel.map((n) => n.x)) - 2200, y: 0, width: W, height: H, fill: "#FCEFDF", layoutMode: "NONE" })).id;

const rect = async (name, x, y, w, h, fill, extra = {}) => (await go("create_rectangle", { name, x, y, width: w, height: h, parentId: F, fill, ...extra })).id;
const ell = async (name, x, y, w, h, fill) => (await go("create_ellipse", { name, x, y, width: w, height: h, parentId: F, fill })).id;
const line = async (name, x, y, len, color, weight) => (await go("create_line", { name, x, y, length: len, parentId: F, color, weight })).id;
const edit = async (id, ops) => (await go("vector_edit", { nodeId: id, ops })).id;
const bend = async (id, a) => (await go("bend_shape", { nodeId: id, ...a })).id;
const fade = (id, opacity) => go("set_props", { nodeId: id, opacity });
const peak = async (name, x, y, w, h, fill) => edit(await rect(name, x, y, w, h, fill), [{ op: "remove", points: [1] }, { op: "move", points: [0], x: w / 2, y: 0 }]);
const ridge = async (name, x, y, w, h, fill, pts) => {
  const ops = []; let prev = 0;
  pts.forEach(([px], k) => { ops.push({ op: "split", segment: k === 0 ? 0 : 3 + k, at: (px - prev) / (w - prev) }); prev = px; });
  pts.forEach(([, dy], k) => ops.push({ op: "move", points: [4 + k], dy }));
  return edit(await rect(name, x, y, w, h, fill), ops);
};
const jag = async (id, side, depths) => {
  const r = await go("vector_read", { nodeId: id });
  const seg = r.sides[side].segments[0], n = depths.length + 1;
  const V = r.points.length, S = r.segments.length, ops = [];
  for (let k = 0; k < n - 1; k++) ops.push({ op: "split", segment: k === 0 ? seg : S + k - 1, at: 1 / (n - k) });
  depths.forEach((d, k) => d && ops.push({ op: "move", points: [V + k], dy: side === "bottom" ? -d : d }));
  return edit(id, ops);
};
// mist: a sky-coloured band with a soft top, see-through
// mist: thin see-through layers stacked downward, so it thickens toward the bottom with no hard edge
const mist = async (y, h, c, o, amount = 6, count = 5, layers = 4) => {
  for (let i = 0; i < layers; i++) {
    const top = y + (i * h) / (layers + 1);
    await fade(await bend(await rect("Mist", -40, top, W + 80, y + h - top + 40, c), { effect: "wave", side: "top", amount: amount * (1 - i * 0.2), count: count + i }), o / layers * 1.4);
  }
};

// ---- sky: eight fine bands, so it reads as a gradient
// 15 steps from the top colour to the horizon colour: close enough to read as a gradient
const mix = (a, b, t) => "#" + [0, 2, 4].map((i) => Math.round(parseInt(a.slice(1 + i, 3 + i), 16) * (1 - t) + parseInt(b.slice(1 + i, 3 + i), 16) * t).toString(16).padStart(2, "0")).join("").toUpperCase();
const STEPS = 15, BAND = 46;
for (let i = 0; i < STEPS; i++) await rect("Sky", 0, i * BAND, W, i === STEPS - 1 ? H - i * BAND : BAND, mix("#EFBC97", "#FCEFDF", i / (STEPS - 1)));

// ---- a low sun with a soft halo of six rings
const SX = 1110, SY = 382;
// the halo: see-through rings of one warm colour, so they blend into the sky instead of sitting on it
for (const r of [360, 300, 245, 195, 150, 118]) await fade(await ell("Sun glow", SX - r, SY - r, r * 2, r * 2, "#F6A876"), 0.16);
await ell("Sun", SX - 90, SY - 90, 180, 180, "#EC773C");

// ---- thin stratus: four flat ellipses joined, so the top is lumpy and the ends taper unevenly
const streak = async (x, y, w, h, c, o) => {
  const parts = [];
  for (const [dx, dy, fw, fh] of [[0, 0, 1, 1], [0.16, -0.5, 0.52, 1.3], [0.56, -0.2, 0.34, 1.1], [-0.08, 0.3, 0.42, 0.75]]) parts.push(await ell("Streak part", x + dx * w, y + dy * h, fw * w, fh * h, c));
  const u = await go("boolean_shapes", { nodeIds: parts, operation: "UNION", name: "Stratus" });
  await go("set_fill", { nodeId: u.id, color: c });
  await fade(u.id, o);
};
for (const [x, y, w, h, c, o] of [[120, 262, 440, 16, "#FFF3E6", 0.9], [880, 344, 560, 12, "#FCE0C8", 0.85], [990, 374, 330, 8, "#FCE0C8", 0.8], [1230, 236, 330, 12, "#FFF3E6", 0.9], [560, 300, 220, 8, "#FFF3E6", 0.7]]) await streak(x, y, w, h, c, o);

// ---- cumulus: ellipses joined into one cloud, the lower part shaded (light from the right)
const cumulus = async (cx, cy, s, body, belly) => {
  const parts = [];
  for (const [dx, dy, w, h] of [[-150, 6, 110, 70], [-100, -34, 130, 112], [-20, -66, 150, 146], [70, -40, 130, 116], [128, -2, 100, 76], [-170, 26, 360, 44]]) {
    parts.push(await ell("Cloud part", cx + dx * s, cy + dy * s, w * s, h * s, body));
  }
  // the flat base is the wide ellipse; join them all
  const u = await go("boolean_shapes", { nodeIds: parts, operation: "UNION", name: "Cloud" });
  await go("set_fill", { nodeId: u.id, color: body });
  // the shaded belly: the cloud minus itself moved up and to the right — a crescent that follows its own edge
  const shade = await go("duplicate_node", { nodeId: u.id, parentId: F, name: "Cloud belly" });
  const lift = await go("duplicate_node", { nodeId: u.id, parentId: F, name: "Cloud lift" });
  const at = await go("get_node", { nodeId: u.id, depth: 0 });
  await go("move_resize", { nodeId: lift.id, x: at.x + 16 * s, y: at.y - 18 * s });
  const b = await go("boolean_shapes", { nodeIds: [shade.id, lift.id], operation: "SUBTRACT", name: "Cloud belly" });
  await go("set_fill", { nodeId: b.id, color: belly });
  return u.id;
};
const cloudA = await cumulus(300, 190, 1, "#FFF8EF", "#F3DDC8");
await cumulus(1410, 286, 0.62, "#FFF6EC", "#F4DCC6");

// ---- distant ranges, each sinking into mist
await ridge("Ridge/far", -40, 420, 1700, 360, "#EAD2C7", [[110, -70], [240, -24], [400, -104], [520, -44], [700, -86], [860, -30], [1040, -120], [1190, -52], [1360, -92], [1520, -36], [1640, -58]]);
await mist(470, 160, "#F8E3D2", 0.6);
await ridge("Ridge/near", -40, 490, 1700, 320, "#D8C0C0", [[160, -40], [330, -92], [470, -34], [640, -68], [930, -22], [1120, -76], [1280, -34], [1450, -104], [1590, -46]]);
await mist(540, 160, "#F6DED0", 0.55, 5, 4);

// ---- the two peaks: rock lit from the right, the shadow on the left face, ragged snow
const mountain = async (name, x, y, w, h, rock, capH, teeth) => {
  const ax = x + w / 2, slope = (w / 2) / h;
  await peak(`${name}/rock`, x, y, w, h, rock);
  const cw = 2 * slope * capH;
  await jag(await peak(`${name}/snow`, ax - cw / 2, y, cw, capH, "#FFFCF7"), "bottom", teeth.map((t) => t * capH));
  // shadow face: apex, base-left, and a point a little left of the middle
  const sh = await edit(await rect(`${name}/shade`, x, y, w / 2, h, "#26304A"), [{ op: "remove", points: [0] }, { op: "move", points: [1], dx: -w * 0.06 }]);
  await fade(sh, 0.24);
};
await mountain("Peak/back", 560, 210, 640, 600, "#B6C0D2", 116, [0.34, 0, 0.18, 0.05, 0.3, 0, 0.22]);
await mountain("Peak/front", 110, 300, 600, 500, "#8493AF", 96, [0.26, 0.04, 0.32, 0, 0.2, 0.06]);
await mist(600, 130, "#F5DACA", 0.5, 7, 5);

// ---- hills, pines in three bough tiers, a headland on the right
const pine = async (x, base, hgt, c, trunk = true) => {
  if (trunk) await rect("Pine/trunk", x - hgt * 0.035, base - hgt * 0.12, hgt * 0.07, hgt * 0.18, "#4A3A31");
  for (const [k, wf, hf] of [[0, 0.62, 0.46], [1, 0.5, 0.42], [2, 0.36, 0.4]]) {
    const w = hgt * wf, h = hgt * hf, top = base - hgt * (0.08 + k * 0.3) - h;
    const t = await peak("Pine/tier", x - w / 2, top + h * 0.0, w, h, c);
    await bend(t, { effect: "wave", side: "bottom", amount: h * 0.07, count: 2 });
  }
};
const HX = -160, HY = 560, HW = 1100, HDROP = 190, HB = 70;
await edit(await rect("Hill/back", HX, HY, HW, 340, "#9BB68E"), [{ op: "move", points: "top-right", dy: HDROP }, { op: "curve", segments: [0], amount: HB }]);
const hillY = (x) => { const t = (x - HX) / HW; return HY + HDROP * t - 4 * t * (1 - t) * HB; };
for (const [x, h, c] of [[300, 62, "#7E9D82"], [338, 50, "#7E9D82"], [520, 56, "#7E9D82"], [40, 92, "#557B5D"], [86, 118, "#4E7356"], [134, 96, "#557B5D"], [182, 74, "#5C8263"], [560, 64, "#5C8263"]]) await pine(x, hillY(x) + 8, h, c);
const KX = 1290, KW = 420;
await edit(await rect("Headland", KX, 612, KW, 200, "#A7B79A"), [{ op: "move", points: "top-left", dy: 74 }, { op: "curve", segments: [0], amount: 34 }]);
const landY = (x) => { const t = (x - KX) / KW; return 686 - 74 * t - 4 * t * (1 - t) * 34; };
for (const [x, h] of [[1478, 56], [1512, 70], [1550, 52], [1604, 64]]) await pine(x, landY(x) + 6, h, "#6B8A6F");
await edit(await rect("Hill/front", -220, 676, 800, 240, "#6E9772"), [{ op: "move", points: "top-right", dy: 120 }, { op: "curve", segments: [0], amount: 56 }]);
for (const [x, h] of [[60, 128], [118, 104]]) await pine(x, 676 + 8 + 120 * ((x + 220) / 800) - 4 * ((x + 220) / 800) * (1 - (x + 220) / 800) * 56, h, "#3F6449");

// ---- water: almost flat, three tones, light streaks and the sun's path
await bend(await rect("Sea/back", -40, 694, 1700, 230, "#93AFD0"), { effect: "wave", side: "top", amount: 1.2, count: 20 });
await bend(await rect("Sea/mid", -40, 732, 1700, 200, "#7196C2"), { effect: "wave", side: "top", amount: 2, count: 20 });
for (const [dx, y, w, h, o] of [[0, 704, 220, 6, 0.85], [-14, 716, 170, 5, 0.85], [10, 728, 140, 5, 0.8], [-6, 742, 110, 4, 0.8], [8, 756, 84, 4, 0.75]]) await fade(await ell("Sun path", SX - w / 2 + dx, y, w, h, "#FAD3AC"), o);

// ---- the boat
const BX = 1040;
await bend(await rect("Boat/hull", BX, 650, 210, 46, "#262A35"), { effect: "pinch", side: "bottom", amount: 34 });
await rect("Boat/trim", BX + 6, 656, 198, 4, "#C9773F", { cornerRadius: 2 });
await rect("Boat/mast", BX + 104, 466, 5, 188, "#262A35");
const sail = await edit(await rect("Boat/sail", BX + 110, 482, 98, 152, "#FFFCF7"), [{ op: "remove", points: [1] }]);
const sr = await go("vector_read", { nodeId: sail });
const hyp = sr.segments.find((g) => { const a = sr.points[g.from], b = sr.points[g.to]; return Math.abs(a.x - b.x) > 1 && Math.abs(a.y - b.y) > 1; });
await edit(sail, [{ op: "curve", segments: [hyp.i], amount: 12 }]);
// the sail's shaded fold near the mast
await fade(await edit(await rect("Boat/sail fold", BX + 110, 482, 30, 152, "#C9B9A6"), [{ op: "remove", points: [1] }, { op: "move", points: [1], dx: 0 }]), 0.35);
await edit(await rect("Boat/jib", BX + 42, 540, 58, 96, "#F6DCC0"), [{ op: "remove", points: [0] }]);
await edit(await rect("Boat/flag", BX + 109, 466, 26, 14, "#EC773C"), [{ op: "split", segments: "right", at: 0.5 }, { op: "move", points: [4], dx: -8 }]);
await fade(await bend(await rect("Boat/reflection", BX + 34, 697, 142, 26, "#1E2A3A"), { effect: "pinch", side: "bottom", amount: 26 }), 0.25);
const wake = await edit(await line("Wake", BX - 230, 712, 200, "#EAF3FC", 2), [{ op: "curve", segments: 0, amount: 10 }]);
await go("set_stroke", { nodeId: wake, dash: [12, 10], cap: "ROUND" });
await fade(wake, 0.8);

await bend(await rect("Sea/front", -40, 780, 1700, 160, "#4A74A3"), { effect: "wave", side: "top", amount: 4, count: 18 });
for (const [x, y, w, o] of [[120, 800, 260, 0.35], [520, 826, 180, 0.3], [860, 812, 220, 0.3], [1280, 836, 260, 0.35], [300, 858, 160, 0.25], [1020, 868, 200, 0.25]]) await fade(await ell("Streak", x, y, w, 4, "#B9D0EA"), o);
for (const [dx, y, w] of [[4, 792, 60], [-6, 806, 40]]) await fade(await ell("Sun path", SX - w / 2 + dx, y, w, 4, "#FAD3AC"), 0.6);

// ---- birds, a loose flock
for (const [x, y, w] of [[600, 214, 42], [654, 190, 34], [700, 228, 28], [742, 204, 22], [560, 246, 20]]) {
  const b = await edit(await line("Bird", x, y, w, "#2E2A2E", 2.6), [{ op: "split", segments: 0, at: 0.5 }, { op: "move", points: [2], dy: w * 0.24 }, { op: "curve", segments: [0, 1], amount: -w * 0.08 }]);
  await go("set_stroke", { nodeId: b, cap: "ROUND", join: "ROUND" });
}

// ---- callouts: what made what
const callout = async (tx, ty, ly, label) => {
  const up = ly < ty;
  const y0 = up ? ly + 26 : ty, y1 = up ? ty : ly;
  await fade(await rect("Callout/line", tx - 0.5, y0, 1, y1 - y0, "#2E2A2E"), 0.5);
  const dot = await ell("Callout/dot", tx - 4.5, ty - 4.5, 9, 9, "#2E2A2E");
  await go("set_stroke", { nodeId: dot, color: "#FFFCF7", weight: 2, align: "OUTSIDE" });
  const w = label.length * 7.9 + 26;
  const pill = await rect("Callout/pill", tx - w / 2, ly, w, 26, "#FFFCF7", { cornerRadius: 13 });
  await go("set_stroke", { nodeId: pill, color: "#2E2A2E22", weight: 1, align: "INSIDE" });
  await go("create_text", { text: label, x: tx - w / 2 + 13, y: ly + 4, parentId: F, fontFamily: "IBM Plex Mono", fontStyle: "Medium", fontSize: 13, fill: "#2E2A2E" });
};
await callout(250, 204, 300, "cloud · union of 6 ellipses");
await callout(900, 312, 132, "ragged snow · split + move");
await callout(BX + 140, 560, 410, "sail · curve +12");
await callout(1512, landY(1512) - 30, 494, "pine · wave edge");

// ---- caption
await go("create_text", { text: "◇  DRAWN WITH WRIDGE", x: 56, y: 44, parentId: F, fontFamily: "IBM Plex Mono", fontStyle: "Medium", fontSize: 16, fill: "#5A4A38", letterSpacing: "10%" });
await go("create_text", { text: "Every shape here began as a rectangle, an ellipse or a line.", x: 56, y: 70, parentId: F, fontFamily: "IBM Plex Mono", fontStyle: "Regular", fontSize: 14, fill: "#8A7660" });

await new Promise((r) => setTimeout(r, 1500));
const img = await call("export_node_image", { nodeId: F, scale: 2, maxSize: 3200 });
writeFileSync("shapes-scene.png", Buffer.from(img.content.find((c) => c.type === "image").data, "base64"));
console.log("done", F);
