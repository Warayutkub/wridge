// Figma Motion (beta Plugin API) on the server side: turning what the plugin reads into compact text, and
// turning the short motion spec (the same one create_lottie takes) into Figma keyframe tracks / presets.
// Kept in one file so a change in the beta API is fixed in one place (plugin side: the "Figma Motion
// helpers" block in code.ts).

const ms = (s) => Math.round(Number(s) * 1000);
const val = (v) => (Array.isArray(v) ? `[${v.join(",")}]` : typeof v === "string" ? v : JSON.stringify(v));

// motion_read result → one line per animated field: "OPACITY base 1 · 0ms 0 → 300ms 1 EASE_OUT".
export function motionText(r) {
  const lines = [`Motion · ${r.root.name} (${r.root.id})${r.timelines?.length ? ` · timeline ${r.timelines.map((t) => `${ms(t.duration)}ms`).join(", ")}` : " · no timeline"}${r.playhead != null ? ` · playhead ${ms(r.playhead)}ms` : ""}`];
  if (!r.animated.length) lines.push("Nothing animated in this frame yet.");
  for (const n of r.animated) {
    lines.push("", `${n.name} (${n.id}, ${n.type})`);
    for (const s of n.styles) lines.push(`  style ${s.name}${s.props ? ` ${JSON.stringify(s.props)}` : ""}`);
    for (const f of n.tracks) {
      for (const t of f.tracks) {
        const keys = t.keys.map((k) => `${ms(k.t)}ms ${val(k.v)}${k.ease && k.ease !== "LINEAR" ? ` ${typeof k.ease === "string" ? k.ease : JSON.stringify(k.ease)}` : ""}`).join(" → ");
        // translation keys are offsets from the layer's own position, which Figma reports as the base
        const base = /^TRANSLATION/.test(f.field) ? `at ${val(f.base)}, offset` : `base ${val(f.base)}`;
        lines.push(`  ${f.field}${t.op !== "SET" ? ` (${t.op.toLowerCase()})` : ""} ${base} · ${keys}`);
      }
    }
  }
  if (r.animationStyles) {
    lines.push("", `## Animation styles in this file (${r.animationStyles.length})`);
    for (const s of r.animationStyles) lines.push(`  ${s.name} (${s.styleId})${s.props ? ` props: ${Object.keys(s.props).join(", ")}` : ""}${s.description ? ` — ${s.description.split("\n")[0]}` : ""}`);
  }
  return lines.join("\n");
}

// ---------------------------------------------------------------- spec → Figma Motion items
// UNITS: how our spec values map to what Figma stores. Checked against what Figma's own presets write
// (motion_read on a preset) — keep every unit decision in this table.
export const UNITS = {
  rotateToFigma: (deg) => (deg * Math.PI) / 180, // ROTATION: radians (the Rotation preset notes "underlying rotate is in radians")
  drawToFigma: (v) => v, // PATH_TRIM_END: 0–1
  scaleToFigma: (v) => v, // SCALE_X / SCALE_Y: factor, 1 = 100%
};
const PROP = {
  opacity: [["OPACITY", (v) => v]],
  move: [["TRANSLATION_X", (v) => (Array.isArray(v) ? v[0] : v)], ["TRANSLATION_Y", (v) => (Array.isArray(v) ? v[1] : 0)]],
  x: [["TRANSLATION_X", (v) => v]],
  y: [["TRANSLATION_Y", (v) => v]],
  scale: [["SCALE_X", (v) => UNITS.scaleToFigma(Array.isArray(v) ? v[0] : v)], ["SCALE_Y", (v) => UNITS.scaleToFigma(Array.isArray(v) ? v[1] : v)]],
  rotate: [["ROTATION", (v) => UNITS.rotateToFigma(v)]],
  draw: [["PATH_TRIM_END", (v) => UNITS.drawToFigma(v)]],
  width: [["WIDTH", (v) => v]],
  height: [["HEIGHT", (v) => v]],
  radius: [["CORNER_RADIUS", (v) => v]],
};
export const MOTION_PROPS = [...Object.keys(PROP), "fill"];
const REST = { opacity: 1, move: [0, 0], x: 0, y: 0, scale: 1, rotate: 0, draw: 1 };
const EASE = {
  linear: { type: "LINEAR" }, in: { type: "EASE_IN" }, out: { type: "EASE_OUT" }, "in-out": { type: "EASE_IN_AND_OUT" },
  back: { type: "EASE_OUT_BACK" }, "back-in": { type: "EASE_IN_BACK" }, "back-in-out": { type: "EASE_IN_AND_OUT_BACK" },
  spring: { type: "BOUNCY" }, bounce: { type: "BOUNCY" }, gentle: { type: "GENTLE" }, quick: { type: "QUICK" }, slow: { type: "SLOW" }, hold: { type: "HOLD" },
};
const BEZIER = { standard: [0.2, 0, 0, 1], decelerate: [0, 0, 0, 1], accelerate: [0.3, 0, 1, 1], emphasized: [0.05, 0.7, 0.1, 1], ease: [0.25, 0.1, 0.25, 1], anticipate: [0.68, -0.6, 0.32, 1.6] };
export const MOTION_EASES = [...Object.keys(EASE), ...Object.keys(BEZIER), "cubic(x1,y1,x2,y2)"];
const bezier = ([x1, y1, x2, y2]) => ({ type: "CUSTOM_CUBIC_BEZIER", easingFunctionCubicBezier: { x1, y1, x2, y2 } });
export function toEasing(e) {
  if (e == null) return undefined;
  if (Array.isArray(e) && e.length === 4) return bezier(e.map(Number));
  const m = /^cubic\(([^)]+)\)$/.exec(String(e));
  if (m) return bezier(m[1].split(",").map(Number));
  if (EASE[e]) return EASE[e];
  if (BEZIER[e]) return bezier(BEZIER[e]);
  throw new Error(`Unknown ease "${e}" — use ${MOTION_EASES.join(", ")}`);
}
const hexRgba = (h) => {
  const m = /^#?([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(String(h));
  if (!m) throw new Error(`fill: "${h}" is not a hex colour`);
  const n = parseInt(m[1], 16);
  return { r: ((n >> 16) & 255) / 255, g: ((n >> 8) & 255) / 255, b: (n & 255) / 255, a: m[2] ? parseInt(m[2], 16) / 255 : 1 };
};

/**
 * tracks: [{ layer, prop, from?, to?, start?, end?, duration?, ease?, stagger?, keys? [[ms, value, ease?]] }]
 * → plugin items [{ layer, stagger (s), field, keys: [{ t (s), value, easing }] }].
 * A keyframe's easing shapes the segment that LEAVES it (Figma: HOLD "holds its value until the next
 * keyframe"), so a from/to ease goes on the start key; in keys form an ease on [ms, v, ease] leaves that key.
 */
export function tracksToItems(tracks = []) {
  const items = [];
  for (const [i, tr] of tracks.entries()) {
    if (!MOTION_PROPS.includes(tr.prop)) throw new Error(`track ${i + 1}: prop "${tr.prop}" — use ${MOTION_PROPS.join(", ")}`);
    let keys;
    if (Array.isArray(tr.keys) && tr.keys.length >= 2) keys = tr.keys.map(([t, v, ease], k) => ({ ms: t, v, ease: k === tr.keys.length - 1 ? ease : ease ?? tr.ease }));
    else {
      const start = tr.start ?? 0, end = tr.end ?? start + (tr.duration ?? 300);
      const from = tr.from ?? REST[tr.prop], to = tr.to ?? REST[tr.prop];
      if (from == null || to == null) throw new Error(`track ${i + 1}: ${tr.prop} needs from and to`);
      keys = [{ ms: start, v: from, ease: tr.ease ?? "standard" }, { ms: end, v: to }];
    }
    const stagger = (tr.stagger ?? 0) / 1000;
    const layer = tr.layer ?? "all";
    if (tr.prop === "fill") {
      items.push({ layer, stagger, field: { type: "INDEXED_ITEM", collection: "fills", index: 0 }, keys: keys.map((k) => ({ t: k.ms / 1000, value: { type: "COLOR", value: hexRgba(k.v) }, easing: toEasing(k.ease) })) });
      continue;
    }
    for (const [name, conv] of PROP[tr.prop]) {
      // TRANSLATION_X/Y keys are offsets from where the layer sits (its base is its x / y), so move goes in as given
      items.push({ layer, stagger, field: { type: "PROPERTY", name }, keys: keys.map((k) => ({ t: k.ms / 1000, value: { type: "FLOAT", value: conv(k.v) }, easing: toEasing(k.ease) })) });
    }
  }
  return items;
}

// presets: [{ layer, style: "Opacity" | "Position" | …, props?, delay? ms, duration? ms, stagger? ms }]
export function presetsToItems(presets = [], available = []) {
  return presets.map((pr, i) => {
    const want = String(pr.style).toLowerCase();
    const s = available.find((a) => a.styleId.toLowerCase() === want || a.name.toLowerCase().endsWith(`.${want}`));
    if (!s) throw new Error(`preset ${i + 1}: no animation style "${pr.style}" — this file has ${available.map((a) => a.styleId).join(", ")}`);
    const props = { ...(pr.props ?? {}), ...(pr.delay != null ? { delay: pr.delay / 1000 } : {}), ...(pr.duration != null ? { duration: pr.duration / 1000 } : {}) };
    return { layer: pr.layer ?? "all", stagger: (pr.stagger ?? 0) / 1000, kind: "style", styleId: s.styleId, config: { ...(pr.duration != null ? { duration: pr.duration / 1000 } : {}), props } };
  });
}

// ---------------------------------------------------------------- Figma Motion → create_lottie spec
// motion_read result → { duration ms, tracks (create_lottie spec), warnings }. Lottie animates the frame's
// top-level layers (the SVG export's groups), so only those are carried over; the rest becomes a warning.
const FROM_FIELD = {
  OPACITY: ["opacity"], ROTATION: ["rotate"], PATH_TRIM_END: ["draw"],
  TRANSLATION_X: ["move", 0], TRANSLATION_Y: ["move", 1], TRANSLATION_XY: ["move"],
  SCALE_X: ["scale", 0], SCALE_Y: ["scale", 1], SCALE_XY: ["scale"],
  "fills[0]": ["fill"], "strokes[0]": ["stroke"], WIDTH: ["width"], HEIGHT: ["height"], CORNER_RADIUS: ["radius"],
};
const FROM_UNITS = { rotate: (rad) => Math.round(((rad * 180) / Math.PI) * 1000) / 1000 };
const AXIS_REST = { move: 0, scale: 1 };
const FROM_EASE = {
  LINEAR: "linear", EASE_IN: "in", EASE_OUT: "out", EASE_IN_AND_OUT: "in-out",
  EASE_OUT_BACK: "back", EASE_IN_BACK: "back-in", EASE_IN_AND_OUT_BACK: "anticipate", HOLD: "hold", BOUNCY: "spring",
};
const NEAR_EASE = { GENTLE: "decelerate", SLOW: "decelerate", QUICK: "standard" }; // Figma springs without overshoot ≈ a curve

function easeFrom(e, warn) {
  if (e == null) return undefined;
  if (typeof e === "string") {
    if (e.startsWith("cubic(")) {
      // one of our named curves written as a custom bezier → its name again (so tokens get real names)
      const p = e.slice(6, -1).split(",").map(Number);
      const named = Object.entries(BEZIER).find(([, b]) => b.every((x, i) => Math.abs(x - p[i]) < 0.005));
      return named ? named[0] : e;
    }
    if (FROM_EASE[e]) return FROM_EASE[e];
    if (NEAR_EASE[e]) { warn(`${e} spring → "${NEAR_EASE[e]}" curve (Lottie has no springs)`); return NEAR_EASE[e]; }
  }
  if (e.spring) { warn("custom spring → \"spring\" (approximated)"); return "spring"; }
  if (e.variable) { warn("easing bound to a variable → \"standard\""); return "standard"; }
  warn(`unknown easing ${JSON.stringify(e)} → "standard"`);
  return "standard";
}
// Apply the keyframe operation: SET is the value, OFFSET adds to the base, SCALE multiplies it.
function opValue(op, base, v) {
  if (op === "OFFSET") return Array.isArray(v) ? v.map((x, i) => (base?.[i] ?? 0) + x) : (base ?? 0) + v;
  if (op === "SCALE") return Array.isArray(v) ? v.map((x, i) => (base?.[i] ?? 1) * x) : (base ?? 1) * v;
  return v;
}
// Value of a key list at time t (linear between keys) — only to fill an axis that has no key there.
function valueAt(keys, t) {
  if (t <= keys[0].ms) return keys[0].v;
  for (let i = 1; i < keys.length; i++) if (t <= keys[i].ms) { const a = keys[i - 1], b = keys[i]; return a.v + ((b.v - a.v) * (t - a.ms)) / (b.ms - a.ms); }
  return keys.at(-1).v;
}

// One layer's Motion tracks in spec form: [{ prop, keys: [[ms, value, ease?]] }] (X/Y axes merged).
export function nodeTracks(n, warnings) {
  const where = `${n.name} (${n.id})`;
  const out = [];
  const axes = {}; // move / scale split by axis, merged below
  for (const f of n.tracks) {
    const map = FROM_FIELD[f.field];
    if (!map) { warnings.push(`${where}: ${f.field} is not supported for export yet; skipped`); continue; }
    if (f.tracks.length > 1) warnings.push(`${where}: ${f.field} has ${f.tracks.length} stacked tracks — used the first`);
    const t = f.tracks[0];
    const warn = (w) => warnings.push(`${where} ${f.field}: ${w}`);
    const [prop, axis] = map;
    const conv = FROM_UNITS[prop] ?? ((v) => v);
    // translation keys are already offsets from the layer's position (base = its x / y), so they read as move as-is
    const keys = t.keys.map((k) => ({ ms: ms(k.t), v: conv(prop === "move" ? k.v : opValue(t.op, f.base, k.v)), ease: easeFrom(k.ease, warn) }));
    if (axis == null) out.push({ prop, keys: keys.map((k) => (k.ease ? [k.ms, k.v, k.ease] : [k.ms, k.v])) });
    else (axes[prop] ??= [null, null])[axis] = keys;
  }
  for (const [prop, [kx, ky]] of Object.entries(axes)) {
    const times = [...new Set([...(kx ?? []), ...(ky ?? [])].map((k) => k.ms))].sort((a, b) => a - b);
    if (kx && ky && (kx.length !== ky.length || kx.some((k, i) => k.ms !== ky[i].ms))) warnings.push(`${where}: ${prop} X and Y keys are at different times — merged with linear in-betweens`);
    const at = (keys, t) => (keys ? valueAt(keys, t) : AXIS_REST[prop]);
    out.push({ prop, keys: times.map((t) => {
      const ease = (kx?.find((k) => k.ms === t) ?? ky?.find((k) => k.ms === t))?.ease;
      const v = [at(kx, t), at(ky, t)];
      return ease ? [t, v, ease] : [t, v];
    }) });
  }
  return out;
}

const LOTTIE_PROPS = new Set(["opacity", "scale", "move", "rotate", "draw", "fill", "stroke"]);
export function motionToSpec(r) {
  const warnings = [];
  const split = new Set();
  const duration = ms(r.timelines?.[0]?.duration ?? 1);
  const tracks = [];
  for (const n of r.animated) {
    const where = `${n.name} (${n.id})`;
    if (!n.tracks.length) continue;
    if (n.id === r.root.id) { warnings.push(`${where}: the frame itself is animated — Lottie only moves its layers; skipped`); continue; }
    if (n.parentId !== r.root.id) {
      // nested: open the groups above it in the SVG (needs the plugin's path; older plugins don't send it)
      if (!Array.isArray(n.path)) { warnings.push(`${where}: nested layer — reopen the Figma plugin so wara can animate layers inside groups; skipped`); continue; }
      for (const g of n.path) split.add(g);
    }
    const layer = n.nth > 1 ? `${n.name}_${n.nth}` : n.name;
    for (const t of nodeTracks(n, warnings)) {
      if (LOTTIE_PROPS.has(t.prop)) tracks.push({ layer, ...t });
      else warnings.push(`${where}: ${t.prop} has no Lottie equivalent; skipped`);
    }
  }
  // a group that animates itself and is also opened for its children loses its own motion in Lottie
  for (const n of r.animated) if (n.tracks.length && split.has(n.name)) warnings.push(`${n.name} (${n.id}): animates and has animated layers inside — Lottie can't nest them, its own motion is dropped (animate the children, or wrap it)`);
  return { duration, tracks, warnings, ...(split.size ? { split: [...split] } : {}) };
}
