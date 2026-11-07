// Figma Motion → handoff: which animated layers become code (+ motion tokens) and which become a Lottie,
// lint for UI motion, the token set the code should use, and code snippets (CSS / Framer Motion).
// Input is always a motion_read result (with hints); see motion-figma.mjs for the track conversion.
import { nodeTracks } from "./motion-figma.mjs";

// ---------------------------------------------------------------- classify
const LOTTIE_ONLY = new Set(["draw", "fill", "stroke"]); // path drawing / colour morphs on art read better as Lottie

// context "ui" (screens in an app) or "landing" (landing / product / brand pages, storyboards — motion-landing):
// on a landing page the art is part of a scroll scene built in code, and long expressive moments are the point.
export const motionContext = (sectionName) => (/storyboard|landing|marketing|brand/i.test(String(sectionName ?? "")) ? "landing" : "ui");

/** → [{ id, name, parentId, to: "code"|"lottie", why, props, span: [ms, ms], tracks }] */
export function classify(r, warnings = [], { context = "ui" } = {}) {
  const landing = context === "landing";
  const rows = [];
  for (const n of r.animated) {
    if (!n.tracks.length && !n.styles.length) continue;
    const tracks = nodeTracks(n, warnings);
    const props = [...new Set(tracks.map((t) => t.prop))];
    const times = tracks.flatMap((t) => t.keys.map((k) => k[0]));
    const span = times.length ? [Math.min(...times), Math.max(...times)] : [0, 0];
    const h = n.hint ?? {};
    const complex = tracks.some((t) => t.keys.length > 2) || props.some((p) => LOTTIE_ONLY.has(p));
    let to, why;
    if (h.component?.startsWith("Lottie/")) [to, why] = ["lottie", `component ${h.component}`];
    else if (n.id === r.root.id) [to, why] = ["code", "the screen/frame itself — a transition"];
    else if (h.text) [to, why] = ["code", "has text (must stay real, translatable text)"];
    else if (h.component || h.instances) [to, why] = ["code", `UI component${h.component ? ` ${h.component}` : ""}`];
    else if (h.autoLayout) [to, why] = ["code", "auto-layout container (UI)"];
    else if (h.image) [to, why] = ["code", "image — CSS moves it fine"];
    else if (landing && h.vectorOnly) [to, why] = ["code", "part of the scroll scene — inline SVG driven by GSAP / ScrollTrigger"];
    else if (h.vectorOnly && complex) [to, why] = ["lottie", "vector art with path/colour/multi-step motion"];
    else if (h.vectorOnly) [to, why] = ["code", "vector with a simple transform — CSS is enough"];
    else [to, why] = ["code", "UI layer"];
    rows.push({ id: n.id, name: n.name, parentId: n.parentId, nth: n.nth, to, why, props, span, tracks, styles: n.styles.map((s) => s.name) });
  }
  // Vector pieces animating together under one parent are one illustration: if any is Lottie, all are.
  const lottieParents = new Set(landing ? [] : rows.filter((x) => x.to === "lottie" && !x.why.startsWith("component")).map((x) => x.parentId));
  for (const x of rows) {
    const h = r.animated.find((n) => n.id === x.id)?.hint ?? {};
    if (x.to === "code" && h.vectorOnly && lottieParents.has(x.parentId) && x.id !== r.root.id) [x.to, x.why] = ["lottie", "part of the same illustration"];
  }
  return rows;
}

// ---------------------------------------------------------------- lint (UI motion that ships as code)
export function lint(rows, r, { context = "ui" } = {}) {
  const out = [];
  const landing = context === "landing";
  const code = rows.filter((x) => x.to === "code");
  for (const x of code) {
    const len = x.span[1] - x.span[0];
    if (!landing && len > 500 && x.id !== r.root.id) out.push(`${x.name}: ${len}ms of UI motion feels slow — keep UI moves 150–400ms`);
    if (x.props.some((p) => p === "width" || p === "height")) out.push(`${x.name}: animates width/height (re-layout every frame) — use scale, or a layout animation in code`);
    if (x.tracks.some((t) => t.keys.some((k) => k[2] === "spring" || k[2] === "anticipate"))) out.push(`${x.name}: bouncy easing on UI — fine for a playful taste, check it matches the project's motion taste`);
  }
  const lastEnd = Math.max(0, ...code.filter((x) => x.id !== r.root.id).map((x) => x.span[1]));
  if (!landing && lastEnd > 1000) out.push(`the screen keeps moving until ${lastEnd}ms — content should be usable within ~1s (shorten the stagger)`);
  // landing: a moment can be long, but when it plays on time (load) it must not hold the reader
  if (landing && lastEnd > 3000) out.push(`this moment runs ${lastEnd}ms — on load keep it under ~2s; longer stories belong on scroll (pinned + scrub)`);
  const { durations, easings } = collect(code);
  if (durations.length > 4) out.push(`${durations.length} different durations (${durations.join(", ")}ms) — snap to 3–4 tokens`);
  if (easings.length > (landing ? 4 : 3)) out.push(`${easings.length} different easings (${easings.join(", ")}) — a project usually needs 2–3`);
  const mixed = new Set(rows.filter((x) => x.to === "lottie").map((x) => x.parentId));
  for (const p of mixed) if (rows.some((x) => x.to === "code" && x.parentId === p)) out.push(`frame ${p} mixes Lottie art and UI layers — group the art into its own frame and export that frame with motion_to_lottie`);
  out.push(...aiSmell(rows.filter((x) => x.id !== r.root.id && x.tracks.length)));
  return out;
}

// The tells of generated motion (motion-craft §6): every layer the same, no leader, wrong curves.
function aiSmell(rows) {
  const out = [];
  const tag = (s) => `AI smell: ${s} (motion-craft)`;
  const first = (x, prop) => x.tracks.find((t) => t.prop === prop)?.keys;
  const len = (x) => x.span[1] - x.span[0];
  const lateEntrances = rows.filter((x) => { const op = first(x, "opacity"); return op && op[0][1] < op.at(-1)[1] && ["in", "linear", "accelerate", "back-in"].includes(op[0][2]); });
  if (lateEntrances.length) out.push(tag(`${lateEntrances.map((x) => x.name).join(", ")} appear${lateEntrances.length === 1 ? "s" : ""} with ${first(lateEntrances[0], "opacity")[0][2]} — entrances ease out`));
  if (rows.length < 3) return out;
  if (new Set(rows.map(len)).size === 1) out.push(tag(`all ${rows.length} layers last ${len(rows[0])}ms — give the leader more time and the followers less`));
  const eases = new Set(rows.flatMap((x) => x.tracks.flatMap((t) => t.keys.slice(0, -1).map((k) => k[2] ?? "standard"))));
  if (eases.size === 1) out.push(tag(`one easing (${[...eases][0]}) on every layer — shape the leader's curve differently`));
  if (new Set(rows.map((x) => x.span[0])).size === 1) out.push(tag("everything starts at once — let one element lead"));
  // fade + rise of the same distance on everything
  const rise = rows.map((x) => { const m = first(x, "move"); const o = first(x, "opacity"); return m && o && o[0][1] === 0 ? JSON.stringify(m[0][1]) : null; });
  if (rise.every(Boolean) && new Set(rise).size === 1) out.push(tag(`every layer fades in and moves from ${rise[0]} — vary distance by weight, or move only the leader`));
  // the same offset between every start, over a long run
  const starts = [...new Set(rows.map((x) => x.span[0]))].sort((a, b) => a - b);
  const gaps = new Set(starts.slice(1).map((s, i) => s - starts[i]));
  if (starts.length > 6 && gaps.size === 1) out.push(tag(`${starts.length} items staggered evenly — cap the stagger at ~5–6 items, the rest arrive together`));
  return out;
}

// ---------------------------------------------------------------- tokens
const CSS_EASE = {
  linear: "linear", in: "cubic-bezier(0.42, 0, 1, 1)", out: "cubic-bezier(0, 0, 0.58, 1)", "in-out": "cubic-bezier(0.42, 0, 0.58, 1)",
  standard: "cubic-bezier(0.2, 0, 0, 1)", decelerate: "cubic-bezier(0, 0, 0, 1)", accelerate: "cubic-bezier(0.3, 0, 1, 1)",
  emphasized: "cubic-bezier(0.05, 0.7, 0.1, 1)", ease: "ease", back: "cubic-bezier(0.34, 1.56, 0.64, 1)", "back-in": "cubic-bezier(0.36, 0, 0.66, -0.56)",
  anticipate: "cubic-bezier(0.68, -0.6, 0.32, 1.6)", spring: "cubic-bezier(0.34, 1.56, 0.64, 1)", hold: "steps(1, end)",
};
const cssEase = (e) => CSS_EASE[e ?? "standard"] ?? (/^cubic\(([^)]+)\)$/.exec(e) ? `cubic-bezier(${/^cubic\(([^)]+)\)$/.exec(e)[1].split(",").join(", ")})` : CSS_EASE.standard);
const easeName = (e) => (e?.startsWith?.("cubic(") ? null : e ?? "standard");

function collect(code) {
  const durations = new Set(), easings = new Set();
  for (const x of code) {
    if (x.span[1] > x.span[0]) durations.add(x.span[1] - x.span[0]);
    for (const t of x.tracks) for (const k of t.keys.slice(0, -1)) easings.add(k[2] ?? "standard");
  }
  return { durations: snap([...durations]), easings: [...easings].filter((e) => e !== "hold") };
}
// merge durations closer than 30ms
function snap(list) {
  const out = [];
  for (const d of list.sort((a, b) => a - b)) if (!out.length || d - out.at(-1) > 30) out.push(d);
  return out;
}
const DURATION_NAMES = [["base"], ["fast", "base"], ["fast", "base", "slow"], ["fast", "base", "slow", "slower"], ["instant", "fast", "base", "slow", "slower"]];

/** → { duration: { name: ms }, easing: { name: css }, css, json } from the code-bound layers */
export function tokens(rows) {
  const code = rows.filter((x) => x.to === "code");
  const { durations, easings } = collect(code);
  const names = DURATION_NAMES[durations.length - 1] ?? durations.map((_, i) => `d${i + 1}`);
  const duration = Object.fromEntries(durations.map((d, i) => [names[i], d]));
  let custom = 0;
  const easing = Object.fromEntries(easings.map((e) => [easeName(e) ?? `custom-${++custom}`, cssEase(e)]));
  const css = [":root {", ...Object.entries(duration).map(([k, v]) => `  --motion-duration-${k}: ${v}ms;`), ...Object.entries(easing).map(([k, v]) => `  --motion-easing-${k}: ${v};`), "}"].join("\n");
  return { duration, easing, css, json: JSON.stringify({ motion: { duration: Object.fromEntries(Object.entries(duration).map(([k, v]) => [k, `${v}ms`])), easing } }, null, 2) };
}
const durationVar = (tk, ms) => {
  const hit = Object.entries(tk.duration).find(([, v]) => Math.abs(v - ms) <= 30);
  return hit ? `var(--motion-duration-${hit[0]})` : `${ms}ms`;
};
const easingVar = (tk, e) => {
  const css = cssEase(e);
  const hit = Object.entries(tk.easing).find(([, v]) => v === css);
  return hit ? `var(--motion-easing-${hit[0]})` : css;
};

// ---------------------------------------------------------------- code
const lerp = (a, b, t) => (Array.isArray(a) ? a.map((x, i) => x + (b[i] - x) * t) : typeof a === "number" ? a + (b - a) * t : t < 1 ? a : b);
function at(keys, ms) {
  if (ms <= keys[0][0]) return keys[0][1];
  for (let i = 1; i < keys.length; i++) {
    const [t0, v0, e] = keys[i - 1], [t1, v1] = keys[i];
    if (ms <= t1) return e === "hold" && ms < t1 ? v0 : lerp(v0, v1, (ms - t0) / (t1 - t0));
  }
  return keys.at(-1)[1];
}
const slug = (s) => String(s).toLowerCase().replace(/[^a-z0-9ก-๙]+/g, "-").replace(/^-|-$/g, "") || "layer";
const r3 = (n) => Math.round(n * 1000) / 1000;
const REST = { opacity: 1, move: [0, 0], scale: [1, 1], rotate: 0 };
const vec = (v) => (Array.isArray(v) ? v : [v, v]);

function cssFrame(get) {
  const parts = [];
  const tf = [];
  const m = get("move"), s = get("scale"), rot = get("rotate");
  if (m) tf.push(`translate(${r3(vec(m)[0])}px, ${r3(vec(m)[1])}px)`);
  if (s) { const [x, y] = vec(s); tf.push(x === y ? `scale(${r3(x)})` : `scale(${r3(x)}, ${r3(y)})`); }
  if (rot != null) tf.push(`rotate(${r3(rot)}deg)`);
  if (tf.length) parts.push(`transform: ${tf.join(" ")}`);
  const o = get("opacity"); if (o != null) parts.push(`opacity: ${r3(o)}`);
  const w = get("width"); if (w != null) parts.push(`width: ${r3(w)}px`);
  const h = get("height"); if (h != null) parts.push(`height: ${r3(h)}px`);
  const rad = get("radius"); if (rad != null) parts.push(`border-radius: ${r3(rad)}px`);
  const f = get("fill"); if (f != null) parts.push(`background-color: ${f}`);
  return parts;
}

// Same-named layers with the same motion, started at even steps (a list appearing one by one) → one
// animation plus a per-item delay, instead of a copy per item.
function staggerGroups(rows) {
  const sig = (x) => JSON.stringify(x.tracks.map((t) => [t.prop, t.keys.map((k) => [k[0] - x.span[0], k[1], k[2]])]));
  const groups = [];
  for (const x of rows.filter((r) => r.to === "code" && r.tracks.length)) {
    const g = groups.find((gr) => gr.name === x.name && gr.sig === sig(x));
    if (g) g.members.push(x); else groups.push({ name: x.name, sig: sig(x), members: [x] });
  }
  return groups.flatMap((g) => {
    const starts = g.members.map((m) => m.span[0]);
    const step = starts.length > 1 ? starts[1] - starts[0] : 0;
    const even = starts.every((st, i) => Math.abs(st - (starts[0] + step * i)) <= 1);
    return even ? [{ x: g.members[0], count: g.members.length, step }] : g.members.map((x) => ({ x, count: 1, step: 0 }));
  });
}

/** CSS @keyframes + animation per code layer, using the token variables. */
export function toCss(rows, tk, warnings = []) {
  const out = [tk.css, ""];
  for (const { x, count, step } of staggerGroups(rows)) {
    const name = slug(x.name) + (x.nth > 1 && count === 1 ? `-${x.nth}` : "");
    const [t0, t1] = x.span, len = Math.max(1, t1 - t0);
    const times = [...new Set(x.tracks.flatMap((t) => t.keys.map((k) => k[0])))].sort((a, b) => a - b);
    const keyTimes = x.tracks.map((t) => t.keys.map((k) => k[0]).join(","));
    if (new Set(keyTimes).size > 1) warnings.push(`${x.name}: props have keys at different times — one @keyframes blends them linearly between keys; Framer Motion output keeps them exact`);
    out.push(`/* ${x.name} — ${x.why}${count > 1 ? ` · ${count} items, ${step}ms apart: set style="--i: 0", 1, 2… on each` : ""} */`, `@keyframes ${name} {`);
    for (const ms of times) {
      const ease = x.tracks.map((t) => t.keys.find((k) => k[0] === ms)).find((k) => k && k[2])?.[2];
      const get = (p) => { const t = x.tracks.find((tr) => tr.prop === p); return t ? at(t.keys, ms) : null; };
      const parts = cssFrame(get);
      if (ms < t1 && ease) parts.push(`animation-timing-function: ${easingVar(tk, ease)}`);
      out.push(`  ${r3(((ms - t0) / len) * 100)}% { ${parts.join("; ")}; }`);
    }
    const delay = count > 1 ? `calc(${t0}ms + var(--i, 0) * ${step}ms) ` : t0 ? `${t0}ms ` : "";
    out.push("}", `.${name} {`, `  animation: ${name} ${durationVar(tk, len)} ${delay}both;`, "}", "@media (prefers-reduced-motion: reduce) {", `  .${name} { animation: none; }`, "}", "");
  }
  return out.join("\n");
}

const FM_EASE = { linear: "linear", in: "easeIn", out: "easeOut", "in-out": "easeInOut", back: "backOut", "back-in": "backIn", anticipate: "anticipate" };
const fmEase = (e) => {
  if (FM_EASE[e]) return JSON.stringify(FM_EASE[e]);
  const c = /^cubic-bezier\(([^)]+)\)$/.exec(cssEase(e));
  return c ? `[${c[1]}]` : '"easeOut"';
};
/** Framer Motion: one motion element per code layer, a transition per prop (exact key times and easings). */
export function toFramer(rows, tk) {
  const out = [`// Motion tokens: ${Object.entries(tk.duration).map(([k, v]) => `${k} ${v}ms`).join(", ")}`, `// Respect reduced motion: wrap the app in <MotionConfig reducedMotion="user">`, ""];
  for (const { x, count, step } of staggerGroups(rows)) {
    const animate = [], transition = [];
    const delayOf = (t0) => (count > 1 ? `${r3(t0 / 1000)} + i * ${r3(step / 1000)}` : r3(t0 / 1000));
    for (const t of x.tracks) {
      const keys = t.keys.flatMap((k, i) => (k[2] === "hold" && t.keys[i + 1] ? [k, [t.keys[i + 1][0] - 1, k[1]]] : [k])); // hold = stay, then jump
      const t0 = keys[0][0], t1 = keys.at(-1)[0], len = Math.max(1, t1 - t0);
      const fields = t.prop === "move" ? [["x", (v) => vec(v)[0]], ["y", (v) => vec(v)[1]]]
        : t.prop === "scale" ? (keys.every((k) => vec(k[1])[0] === vec(k[1])[1]) ? [["scale", (v) => vec(v)[0]]] : [["scaleX", (v) => vec(v)[0]], ["scaleY", (v) => vec(v)[1]]])
        : [[{ fill: "backgroundColor", radius: "borderRadius" }[t.prop] ?? t.prop, (v) => v]];
      for (const [field, get] of fields) {
        if (fields.length > 1 && keys.every((k) => get(k[1]) === get(keys[0][1]))) continue; // an axis that never moves
        animate.push(`${field}: [${keys.map((k) => JSON.stringify(typeof get(k[1]) === "number" ? r3(get(k[1])) : get(k[1]))).join(", ")}]`);
        transition.push(`${field}: { duration: ${r3(len / 1000)}, delay: ${delayOf(t0)}, times: [${keys.map((k) => r3((k[0] - t0) / len)).join(", ")}], ease: [${keys.slice(0, -1).map((k) => (k[2] === "hold" ? '"linear"' : fmEase(k[2]))).join(", ")}] }`);
      }
    }
    const el = [`<motion.div${count > 1 ? " key={item.id}" : ""}`, `  animate={{ ${animate.join(", ")} }}`, `  transition={{ ${transition.join(",\n    ")} }}`, `/>`];
    out.push(`{/* ${x.name} — ${x.why}${count > 1 ? ` · ${count} items, ${step}ms apart` : ""} */}`, ...(count > 1 ? ["{items.map((item, i) => (", ...el.map((l) => `  ${l}`), "))}"] : el), "");
  }
  return out.join("\n");
}

// ---------------------------------------------------------------- report
export function checkText(r, rows, lints, context = "ui") {
  const lines = [`Motion handoff · ${r.root.name} (${r.root.id}) · ${rows.length} animated layer(s) · context ${context}${context === "landing" ? " (motion-landing rules)" : ""}`, "", "| layer | animates | → | why |", "|---|---|---|---|"];
  for (const x of rows) lines.push(`| ${x.name}${x.nth > 1 ? ` #${x.nth}` : ""} (${x.id}) | ${x.props.join(", ") || x.styles.join(", ")} ${x.span[1] ? `${x.span[0]}–${x.span[1]}ms` : ""} | **${x.to === "code" ? "code + tokens" : "Lottie"}** | ${x.why} |`);
  const lottie = rows.filter((x) => x.to === "lottie");
  if (lottie.length) {
    const parents = [...new Set(lottie.map((x) => x.parentId))];
    lines.push("", `Lottie: motion_to_lottie on ${parents.map((p) => (p === r.root.id ? `${r.root.name} (${p})` : p)).join(", ")}${lottie.some((x) => x.why.startsWith("component")) ? " · Lottie/… components: export their main component's frame" : ""}`);
  }
  if (rows.some((x) => x.to === "code")) lines.push("Code: motion_tokens for the token set, motion_to_code for the snippets.");
  if (lints.length) lines.push("", "## Check", ...lints.map((l) => `- ${l}`));
  lines.push("", "Show this table to the user and wait for approval before writing code or files.");
  return lines.join("\n");
}
