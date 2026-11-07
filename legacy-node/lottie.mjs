// SVG art + a short motion spec → a Lottie (bodymovin) JSON file.
// The AI sends what moves and how ("check: draw 0→1, 0–400ms, out"); this file writes the keyframes,
// so a few hundred tokens of spec stand in for thousands of tokens of Lottie JSON.
//
// Layers = the top-level elements of the SVG (a lone wrapping <g> is unwrapped). Names come from
// id / data-name (Figma exports layer names as ids), else "<tag>-<n>". Transforms are baked into the
// points; every layer pivots around its own centre, so scale and rotate look right without setup.

// ---------------------------------------------------------------- SVG → tree
const ENTITY = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };
const unescape = (s) => s.replace(/&(#x?[0-9a-f]+|\w+);/gi, (m, e) => (e[0] === "#" ? String.fromCodePoint(e[1] === "x" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : ENTITY[e] ?? m));

export function parseSvg(text) {
  const root = { tag: "#root", attrs: {}, children: [] };
  const stack = [root];
  const re = /<!--[\s\S]*?-->|<\?[\s\S]*?\?>|<!\[CDATA\[[\s\S]*?\]\]>|<!DOCTYPE[^>]*>|<(\/?)([\w:.-]+)((?:\s+[\w:.-]+\s*=\s*(?:"[^"]*"|'[^']*'))*)\s*(\/?)>/gi;
  for (const m of text.matchAll(re)) {
    if (!m[2]) continue;
    const [, close, tag, rawAttrs, selfClose] = m;
    if (close) { if (stack.length > 1) stack.pop(); continue; }
    const attrs = {};
    for (const a of rawAttrs.matchAll(/([\w:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) attrs[a[1]] = unescape(a[2] ?? a[3]);
    const node = { tag: tag.toLowerCase(), attrs, children: [] };
    stack.at(-1).children.push(node);
    if (!selfClose) stack.push(node);
  }
  const svg = root.children.find((c) => c.tag === "svg");
  if (!svg) throw new Error("Not an SVG: no <svg> element");
  return svg;
}

// ---------------------------------------------------------------- transforms
const IDENTITY = [1, 0, 0, 1, 0, 0];
const mul = (m, n) => [m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1], m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3], m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]];
const apply = (m, [x, y]) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
const applyVec = (m, [x, y]) => [m[0] * x + m[2] * y, m[1] * x + m[3] * y];
function parseTransform(s) {
  let m = IDENTITY;
  for (const [, fn, args] of String(s ?? "").matchAll(/(\w+)\s*\(([^)]*)\)/g)) {
    const a = args.split(/[\s,]+/).filter(Boolean).map(Number);
    let t = IDENTITY;
    if (fn === "matrix" && a.length === 6) t = a;
    else if (fn === "translate") t = [1, 0, 0, 1, a[0] ?? 0, a[1] ?? 0];
    else if (fn === "scale") t = [a[0], 0, 0, a[1] ?? a[0], 0, 0];
    else if (fn === "rotate") {
      const r = ((a[0] ?? 0) * Math.PI) / 180, c = Math.cos(r), sn = Math.sin(r);
      t = [c, sn, -sn, c, 0, 0];
      if (a.length === 3) t = mul(mul([1, 0, 0, 1, a[1], a[2]], t), [1, 0, 0, 1, -a[1], -a[2]]);
    } else if (fn === "skewX") t = [1, 0, Math.tan((a[0] * Math.PI) / 180), 1, 0, 0];
    else if (fn === "skewY") t = [1, Math.tan((a[0] * Math.PI) / 180), 0, 1, 0, 0];
    m = mul(m, t);
  }
  return m;
}

// ---------------------------------------------------------------- path data → cubic subpaths
// Subpath: { closed, pts: [[x,y]...], segs: [[c1, c2]...] } (segs[i] joins pts[i] → pts[i+1]; closing seg last).
function arcToCubics(x1, y1, rx, ry, phi, large, sweep, x2, y2) {
  if (!rx || !ry) return [[[x1, y1], [x2, y2], [x2, y2]]];
  const rad = (phi * Math.PI) / 180, cos = Math.cos(rad), sin = Math.sin(rad);
  const dx = (x1 - x2) / 2, dy = (y1 - y2) / 2;
  const x1p = cos * dx + sin * dy, y1p = -sin * dx + cos * dy;
  rx = Math.abs(rx); ry = Math.abs(ry);
  const lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
  if (lam > 1) { rx *= Math.sqrt(lam); ry *= Math.sqrt(lam); }
  const sign = large === sweep ? -1 : 1;
  const num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
  const co = sign * Math.sqrt(Math.max(0, num / (rx * rx * y1p * y1p + ry * ry * x1p * x1p)));
  const cxp = (co * rx * y1p) / ry, cyp = (-co * ry * x1p) / rx;
  const cx = cos * cxp - sin * cyp + (x1 + x2) / 2, cy = sin * cxp + cos * cyp + (y1 + y2) / 2;
  const ang = (ux, uy, vx, vy) => { const a = Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy); return a; };
  let t1 = ang(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry);
  let dt = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
  if (!sweep && dt > 0) dt -= 2 * Math.PI;
  if (sweep && dt < 0) dt += 2 * Math.PI;
  const n = Math.ceil(Math.abs(dt) / (Math.PI / 2) - 1e-9) || 1;
  const step = dt / n, k = (4 / 3) * Math.tan(step / 4);
  const pt = (t) => [cx + rx * Math.cos(t) * cos - ry * Math.sin(t) * sin, cy + rx * Math.cos(t) * sin + ry * Math.sin(t) * cos];
  const der = (t) => [-rx * Math.sin(t) * cos - ry * Math.cos(t) * sin, -rx * Math.sin(t) * sin + ry * Math.cos(t) * cos];
  const out = [];
  for (let i = 0; i < n; i++) {
    const a = t1 + i * step, b = a + step;
    const p0 = pt(a), p1 = pt(b), d0 = der(a), d1 = der(b);
    out.push([[p0[0] + k * d0[0], p0[1] + k * d0[1]], [p1[0] - k * d1[0], p1[1] - k * d1[1]], p1]);
  }
  return out;
}

export function parsePath(d) {
  const toks = String(d ?? "").match(/[a-df-z]|[-+]?(?:\d*\.\d+|\d+\.?)(?:e[-+]?\d+)?/gi) ?? [];
  const subs = [];
  let i = 0, cmd = "", cur = [0, 0], start = [0, 0], lastCtrl = null, lastCmd = "", sub = null;
  const num = () => Number(toks[i++]);
  const flag = () => Number(toks[i++]);
  const begin = (p) => { sub = { closed: false, pts: [p], segs: [] }; subs.push(sub); };
  const cubic = (c1, c2, p) => { if (!sub) begin(cur); sub.segs.push([c1, c2]); sub.pts.push(p); cur = p; };
  const line = (p) => cubic(cur, p, p);
  while (i < toks.length) {
    if (/[a-z]/i.test(toks[i])) cmd = toks[i++];
    else if (!cmd) { i++; continue; }
    const rel = cmd === cmd.toLowerCase() && cmd !== "z" && cmd !== "Z";
    const P = (x, y) => (rel ? [cur[0] + x, cur[1] + y] : [x, y]);
    const C = cmd.toUpperCase();
    if (C === "M") { const p = P(num(), num()); begin(p); cur = start = p; cmd = rel ? "l" : "L"; }
    else if (C === "L") line(P(num(), num()));
    else if (C === "H") { const x = num(); line([rel ? cur[0] + x : x, cur[1]]); }
    else if (C === "V") { const y = num(); line([cur[0], rel ? cur[1] + y : y]); }
    else if (C === "C") { const c1 = P(num(), num()), c2 = P(num(), num()), p = P(num(), num()); cubic(c1, c2, p); lastCtrl = c2; }
    else if (C === "S") {
      const c1 = /[CS]/i.test(lastCmd) && lastCtrl ? [2 * cur[0] - lastCtrl[0], 2 * cur[1] - lastCtrl[1]] : cur;
      const c2 = P(num(), num()), p = P(num(), num()); cubic(c1, c2, p); lastCtrl = c2;
    } else if (C === "Q" || C === "T") {
      const q = C === "Q" ? P(num(), num()) : /[QT]/i.test(lastCmd) && lastCtrl ? [2 * cur[0] - lastCtrl[0], 2 * cur[1] - lastCtrl[1]] : cur;
      const p = P(num(), num());
      cubic([cur[0] + (2 / 3) * (q[0] - cur[0]), cur[1] + (2 / 3) * (q[1] - cur[1])], [p[0] + (2 / 3) * (q[0] - p[0]), p[1] + (2 / 3) * (q[1] - p[1])], p);
      lastCtrl = q;
    } else if (C === "A") {
      const rx = num(), ry = num(), phi = num(), large = flag(), sweep = flag(), p = P(num(), num());
      for (const [c1, c2, q] of arcToCubics(cur[0], cur[1], rx, ry, phi, large, sweep, p[0], p[1])) cubic(c1, c2, q);
    } else if (C === "Z") {
      if (sub) {
        const first = sub.pts[0], last = sub.pts.at(-1);
        // A closing point equal to the first one is dropped: the closing segment takes its curve.
        if (sub.pts.length > 1 && Math.hypot(first[0] - last[0], first[1] - last[1]) < 1e-6) sub.pts.pop();
        else sub.segs.push([last, first]);
        sub.closed = true;
      }
      cur = start; sub = null;
    } else { i++; }
    if (!/[CS]/i.test(C) && !/[QT]/i.test(C)) lastCtrl = null;
    lastCmd = C;
  }
  return subs.filter((s) => s.pts.length > 1 || s.closed);
}

// Basic shapes as path data, so everything goes through one converter.
function shapeToPath(n) {
  const a = n.attrs, f = (k, d = 0) => Number.parseFloat(a[k] ?? d) || 0;
  if (n.tag === "path") return a.d;
  if (n.tag === "rect") {
    const x = f("x"), y = f("y"), w = f("width"), h = f("height");
    let rx = a.rx != null ? f("rx") : f("ry"), ry = a.ry != null ? f("ry") : rx;
    rx = Math.min(rx, w / 2); ry = Math.min(ry, h / 2);
    if (!rx && !ry) return `M${x} ${y}H${x + w}V${y + h}H${x}Z`;
    return `M${x + rx} ${y}H${x + w - rx}A${rx} ${ry} 0 0 1 ${x + w} ${y + ry}V${y + h - ry}A${rx} ${ry} 0 0 1 ${x + w - rx} ${y + h}H${x + rx}A${rx} ${ry} 0 0 1 ${x} ${y + h - ry}V${y + ry}A${rx} ${ry} 0 0 1 ${x + rx} ${y}Z`;
  }
  if (n.tag === "circle" || n.tag === "ellipse") {
    const cx = f("cx"), cy = f("cy"), rx = n.tag === "circle" ? f("r") : f("rx"), ry = n.tag === "circle" ? f("r") : f("ry");
    return `M${cx - rx} ${cy}A${rx} ${ry} 0 1 0 ${cx + rx} ${cy}A${rx} ${ry} 0 1 0 ${cx - rx} ${cy}Z`;
  }
  if (n.tag === "line") return `M${f("x1")} ${f("y1")}L${f("x2")} ${f("y2")}`;
  if (n.tag === "polyline" || n.tag === "polygon") {
    const p = String(a.points ?? "").trim().split(/[\s,]+/).map(Number);
    if (p.length < 4) return null;
    return `M${p[0]} ${p[1]}${Array.from({ length: p.length / 2 - 1 }, (_, k) => `L${p[2 * k + 2]} ${p[2 * k + 3]}`).join("")}${n.tag === "polygon" ? "Z" : ""}`;
  }
  return null;
}

// ---------------------------------------------------------------- paint
const NAMED = { black: "#000000", white: "#ffffff", red: "#ff0000", green: "#008000", blue: "#0000ff", transparent: "none", currentcolor: "#000000" };
export function parseColor(v) {
  if (v == null) return null;
  v = String(v).trim().toLowerCase();
  v = NAMED[v] ?? v;
  if (v === "none" || v === "") return "none";
  let m = /^#([0-9a-f]{3,8})$/.exec(v);
  if (m) {
    let h = m[1];
    if (h.length <= 4) h = [...h].map((c) => c + c).join("");
    return [parseInt(h.slice(0, 2), 16) / 255, parseInt(h.slice(2, 4), 16) / 255, parseInt(h.slice(4, 6), 16) / 255, h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1];
  }
  m = /^rgba?\(([^)]+)\)$/.exec(v);
  if (m) { const p = m[1].split(/[\s,/]+/).filter(Boolean).map((x) => (x.endsWith("%") ? (parseFloat(x) * 2.55) : parseFloat(x))); return [p[0] / 255, p[1] / 255, p[2] / 255, p[3] ?? 1]; }
  if (v.startsWith("url(")) return { ref: /#([^)'"]+)/.exec(v)?.[1] };
  return null;
}
const round = (n) => Math.round(n * 1000) / 1000;

// Inherited presentation attributes (style="" wins over attributes).
const PAINT_KEYS = ["fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "fill-opacity", "stroke-opacity", "opacity", "fill-rule", "display", "visibility"];
function ownStyle(n) {
  const s = {};
  for (const k of PAINT_KEYS) if (n.attrs[k] != null) s[k] = n.attrs[k];
  for (const part of String(n.attrs.style ?? "").split(";")) { const [k, v] = part.split(":").map((x) => x?.trim()); if (k && v && PAINT_KEYS.includes(k)) s[k] = v; }
  return s;
}

// ---------------------------------------------------------------- SVG → layers of drawable items
const DRAWABLE = new Set(["path", "rect", "circle", "ellipse", "line", "polyline", "polygon"]);
const SKIP = new Set(["defs", "clippath", "mask", "lineargradient", "radialgradient", "pattern", "filter", "style", "title", "desc", "metadata", "symbol", "marker"]);

// opts.split: names of groups to open — their children become layers of their own (a nested layer that
// animates in Figma Motion lives inside a group in the SVG export).
export function svgLayers(text, opts = {}) {
  const norm = (s) => String(s).toLowerCase().replace(/[^a-z0-9ก-๙]/g, "");
  const split = new Set((opts.split ?? []).map(norm));
  const svg = parseSvg(text);
  const warnings = new Set();
  const gradients = new Map();
  const walkDefs = (n) => { if (/gradient$/.test(n.tag) && n.attrs.id) gradients.set(n.attrs.id, n); n.children.forEach(walkDefs); };
  walkDefs(svg);
  const vb = String(svg.attrs.viewBox ?? "").split(/[\s,]+/).map(Number);
  const width = Number.parseFloat(svg.attrs.width) || vb[2] || 100;
  const height = Number.parseFloat(svg.attrs.height) || vb[3] || 100;
  // viewBox → output size
  let base = IDENTITY;
  if (vb.length === 4 && vb[2] && vb[3]) base = mul([width / vb[2], 0, 0, height / vb[3], 0, 0], [1, 0, 0, 1, -vb[0], -vb[1]]);

  // Gradient stops, following href chains (a gradient may borrow another's stops).
  const stopsOf = (g) => {
    const seen = new Set();
    let node = g;
    while (node && !node.children.some((c) => c.tag === "stop")) {
      const ref = String(node.attrs.href ?? node.attrs["xlink:href"] ?? "").replace(/^#/, "");
      if (!ref || seen.has(ref)) break;
      seen.add(ref);
      node = gradients.get(ref);
    }
    return (node?.children ?? []).filter((c) => c.tag === "stop").map((stop) => {
      const a = { ...stop.attrs };
      for (const part of String(a.style ?? "").split(";")) { const [k, v] = part.split(":").map((x) => x?.trim()); if (k && v) a[k] = v; }
      const c = parseColor(a["stop-color"] ?? "#000");
      const rgba = Array.isArray(c) ? c : [0, 0, 0, 1];
      const off = String(a.offset ?? 0).trim().endsWith("%") ? Number.parseFloat(a.offset) / 100 : Number.parseFloat(a.offset ?? 0) || 0;
      return [Math.min(1, Math.max(0, off)), rgba[0], rgba[1], rgba[2], rgba[3] * Number(a["stop-opacity"] ?? 1)];
    });
  };
  // A gradient as Lottie needs it: start/end points in the drawing's space (radial: centre and a point on the edge).
  const gradientOf = (g, m, localBox) => {
    const stops = stopsOf(g);
    if (!stops.length) return null;
    const v = (k, d) => { const x = g.attrs[k]; return x == null ? d : String(x).trim().endsWith("%") ? Number.parseFloat(x) / 100 : Number.parseFloat(x); };
    const radial = g.tag === "radialgradient";
    const [u1, v1, u2, v2] = radial ? [v("cx", 0.5), v("cy", 0.5), v("cx", 0.5) + v("r", 0.5), v("cy", 0.5)] : [v("x1", 0), v("y1", 0), v("x2", 1), v("y2", 0)];
    const gt = parseTransform(g.attrs.gradientTransform);
    const box = g.attrs.gradientUnits === "userSpaceOnUse" ? IDENTITY : [localBox.w, 0, 0, localBox.h, localBox.x, localBox.y];
    const toUser = mul(m, mul(box, gt));
    return { type: radial ? "radial" : "linear", s: apply(toUser, [u1, v1]), e: apply(toUser, [u2, v2]), stops };
  };
  const paintOf = (st, key, opacityKey, m, localBox) => {
    const c = parseColor(st[key]);
    const k = Number(st[opacityKey] ?? 1);
    if (c && c.ref) {
      const g = gradients.get(c.ref);
      const grad = g ? gradientOf(g, m, localBox) : null;
      return grad ? { gradient: grad, alpha: k } : null;
    }
    if (!c || c === "none") return null;
    return [...c.slice(0, 3), c[3] * k];
  };

  const items = (n, m, st, out) => {
    if (SKIP.has(n.tag)) return;
    const s = { ...st, ...ownStyle(n) };
    if (s.display === "none" || s.visibility === "hidden") return;
    const mm = n.attrs.transform ? mul(m, parseTransform(n.attrs.transform)) : m;
    const opacity = Number(st.__opacity ?? 1) * Number(ownStyle(n).opacity ?? 1);
    s.__opacity = opacity;
    if (n.attrs["clip-path"] || n.attrs.mask) warnings.add("clip-path / mask ignored");
    if (n.attrs.filter) warnings.add("filters (blur, shadow) ignored");
    if (n.tag === "use" || n.tag === "image" || n.tag === "text") { warnings.add(`<${n.tag}> not supported — outline it in Figma first`); return; }
    if (DRAWABLE.has(n.tag)) {
      const d = shapeToPath(n);
      if (!d) return;
      const local = parsePath(d);
      const subs = local.map((sp) => ({ closed: sp.closed, pts: sp.pts.map((p) => apply(mm, p)), segs: sp.segs.map(([a, b]) => [apply(mm, a), apply(mm, b)]) }));
      if (!subs.length) return;
      const lb = bounds([{ subs: local }]);
      const localBox = { x: lb.cx - lb.w / 2, y: lb.cy - lb.h / 2, w: lb.w || 1, h: lb.h || 1 };
      const fill = s.fill === undefined ? [0, 0, 0, 1] : paintOf(s, "fill", "fill-opacity", mm, localBox);
      const stroke = paintOf(s, "stroke", "stroke-opacity", mm, localBox);
      const sw = Number.parseFloat(s["stroke-width"] ?? 1) * Math.sqrt(Math.abs(mm[0] * mm[3] - mm[1] * mm[2]));
      if (!fill && !stroke) return;
      out.push({ subs, fill, stroke, strokeWidth: sw, cap: s["stroke-linecap"] ?? "butt", join: s["stroke-linejoin"] ?? "miter", evenodd: s["fill-rule"] === "evenodd", opacity });
      return;
    }
    for (const c of n.children) items(c, mm, s, out);
  };

  // Top level: unwrap single wrapping groups (Figma wraps the frame's content in one <g>).
  let top = svg.children.filter((c) => !SKIP.has(c.tag));
  let m0 = base, st0 = ownStyle(svg);
  while (top.length === 1 && top[0].tag === "g" && top[0].children.some((c) => !SKIP.has(c.tag))) {
    m0 = top[0].attrs.transform ? mul(m0, parseTransform(top[0].attrs.transform)) : m0;
    st0 = { ...st0, ...ownStyle(top[0]) };
    if (top[0].attrs["clip-path"] || top[0].attrs.mask) warnings.add("clip-path / mask ignored");
    top = top[0].children.filter((c) => !SKIP.has(c.tag));
  }
  const used = new Map();
  const layers = [];
  // open the groups named in split (recursively), keeping their transform and style for the children
  const flat = [];
  const open = (n, m, st) => {
    const name = n.attrs["data-name"] ?? n.attrs.id;
    if (name && split.has(norm(name)) && n.children.some((c) => !SKIP.has(c.tag)) && !DRAWABLE.has(n.tag)) {
      const mm = n.attrs.transform ? mul(m, parseTransform(n.attrs.transform)) : m;
      const s = { ...st, ...ownStyle(n), __opacity: Number(st.__opacity ?? 1) * Number(ownStyle(n).opacity ?? 1) };
      for (const c of n.children) if (!SKIP.has(c.tag)) open(c, mm, s);
    } else flat.push([n, m, st]);
  };
  for (const n of top) open(n, m0, st0);
  for (const [n, m, st] of flat) {
    const out = [];
    items(n, m, st, out);
    if (!out.length) continue;
    // Figma exports the 2nd "Dot" as id "Dot_2": a "dot*" track reaches both.
    const want = n.attrs["data-name"] ?? n.attrs.id ?? `${n.tag}-${layers.length + 1}`;
    const seen = (used.get(want) ?? 0) + 1;
    used.set(want, seen);
    let name = seen === 1 ? want : `${want}_${seen}`;
    while (layers.some((l) => l.name === name)) name += "_"; // an id that already looks like "Dot_2"
    layers.push({ name, items: out });
  }
  return { width, height, layers, warnings: [...warnings] };
}

// ---------------------------------------------------------------- motion
const EASE = {
  linear: [0, 0, 1, 1], ease: [0.25, 0.1, 0.25, 1], in: [0.42, 0, 1, 1], out: [0, 0, 0.58, 1], "in-out": [0.42, 0, 0.58, 1],
  standard: [0.2, 0, 0, 1], decelerate: [0, 0, 0, 1], accelerate: [0.3, 0, 1, 1], emphasized: [0.05, 0.7, 0.1, 1],
  back: [0.34, 1.56, 0.64, 1], "back-in": [0.36, 0, 0.66, -0.56], anticipate: [0.68, -0.6, 0.32, 1.6],
};
export const EASINGS = [...Object.keys(EASE), "spring", "bounce", "hold"];
function easeOf(e) {
  if (Array.isArray(e) && e.length === 4) return e.map(Number);
  const m = /^cubic\(([^)]+)\)$/.exec(String(e ?? ""));
  if (m) return m[1].split(",").map(Number);
  return EASE[e ?? "standard"] ?? null;
}
// Damped spring 0→1 sampled (Lottie has no spring curve).
function springCurve(t, stiffness = 170, damping = 16) {
  const w0 = Math.sqrt(stiffness), z = damping / (2 * Math.sqrt(stiffness));
  if (z >= 1) return 1 - Math.exp(-w0 * t) * (1 + w0 * t);
  const wd = w0 * Math.sqrt(1 - z * z);
  return 1 - Math.exp(-z * w0 * t) * (Math.cos(wd * t) + ((z * w0) / wd) * Math.sin(wd * t));
}
function bounceCurve(t) {
  const n = 7.5625, d = 2.75;
  if (t < 1 / d) return n * t * t;
  if (t < 2 / d) return n * (t -= 1.5 / d) * t + 0.75;
  if (t < 2.5 / d) return n * (t -= 2.25 / d) * t + 0.9375;
  return n * (t -= 2.625 / d) * t + 0.984375;
}

const PROPS = ["opacity", "scale", "move", "rotate", "draw", "fill", "stroke"];
const REST = { opacity: 1, scale: 1, move: [0, 0], rotate: 0, draw: 1 };
const lerp = (a, b, t) => (Array.isArray(a) ? a.map((x, i) => x + (b[i] - x) * t) : a + (b - a) * t);

// Value in spec units → Lottie units for one layer.
function toLottie(prop, v, layer) {
  if (prop === "opacity") return [round(Number(v) * 100)];
  if (prop === "scale") { const s = Array.isArray(v) ? v : [v, v]; return [round(s[0] * 100), round(s[1] * 100), 100]; }
  if (prop === "move") { const d = Array.isArray(v) ? v : [Number(v), 0]; return [round(layer.cx + d[0]), round(layer.cy + d[1]), 0]; }
  if (prop === "rotate") return [round(Number(v))];
  if (prop === "draw") return [round(Number(v) * 100)];
  if (prop === "fill" || prop === "stroke") { const c = parseColor(v); if (!Array.isArray(c)) throw new Error(`${prop}: "${v}" is not a colour`); return c.map(round); }
  throw new Error(`Unknown prop "${prop}" — use ${PROPS.join(", ")}`);
}

const matchName = (pattern, name) => {
  const norm = (s) => String(s).toLowerCase().replace(/[^a-z0-9ก-๙*]/g, "");
  const p = norm(pattern), n = norm(name);
  if (p === "all" || p === "*") return true;
  if (p.includes("*")) return new RegExp(`^${p.split("*").map((x) => x.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(".*")}$`).test(n);
  return p === n;
};

// tracks → { [layerName]: { [prop]: [{ t, v, ease, hold }] } } in frames.
function keyframes(tracks, layers, fps, totalMs, warnings) {
  const out = new Map(layers.map((l) => [l.name, {}]));
  const frame = (ms) => Math.round((Number(ms) / 1000) * fps);
  tracks.forEach((tr, ti) => {
    if (!PROPS.includes(tr.prop)) throw new Error(`track ${ti + 1}: unknown prop "${tr.prop}" — use ${PROPS.join(", ")}`);
    const targets = layers.filter((l) => matchName(tr.layer ?? "all", l.name));
    if (!targets.length) { warnings.push(`track ${ti + 1}: no layer named "${tr.layer}" (layers: ${layers.map((l) => l.name).join(", ")})`); return; }
    targets.forEach((layer, k) => {
      const off = (tr.stagger ?? 0) * k;
      let keys;
      if (Array.isArray(tr.keys) && tr.keys.length >= 2) keys = tr.keys.map(([ms, v, ease]) => ({ ms: ms + off, v, ease: ease ?? tr.ease }));
      else {
        const start = (tr.start ?? 0) + off, end = (tr.end ?? (tr.duration != null ? (tr.start ?? 0) + tr.duration : totalMs)) + off;
        const rest = tr.prop === "fill" || tr.prop === "stroke" ? null : REST[tr.prop];
        const from = tr.from ?? rest, to = tr.to ?? rest;
        if (from == null || to == null) throw new Error(`track ${ti + 1}: ${tr.prop} needs from and to`);
        keys = [{ ms: start, v: from, ease: tr.ease }, { ms: end, v: to }];
      }
      const list = (out.get(layer.name)[tr.prop] ??= []);
      if (list.length) list.at(-1).hold = true; // a later track on the same prop starts from its own "from", no drift in between
      for (let i = 0; i < keys.length; i++) {
        const a = keys[i], b = keys[i + 1];
        const ease = a.ease ?? tr.ease ?? "standard";
        if (b && (ease === "spring" || ease === "bounce")) {
          // sample the curve: one key every 2 frames, linear in between
          const f0 = frame(a.ms), f1 = frame(b.ms), n = Math.max(2, Math.round((f1 - f0) / 2));
          for (let s = 0; s < n; s++) {
            // the spring settles in ~0.6s of its own time: stretch that over the track
            const p = s / n, eased = ease === "spring" ? springCurve(p * 0.6) : bounceCurve(p);
            list.push({ t: f0 + Math.round((f1 - f0) * p), raw: true, v: lerpValues(tr.prop, a.v, b.v, eased), ease: EASE.linear });
          }
        } else if (b && ease === "hold") {
          list.push({ t: frame(a.ms), v: a.v, ease: EASE.linear, hold: true }); // keep the value, jump at the next key
        } else {
          const curve = b ? easeOf(ease) : EASE.linear; // the last key has no segment after it
          if (!curve) { warnings.push(`track ${ti + 1}: unknown ease "${ease}" — used standard (choices: ${EASINGS.join(", ")}, cubic(x1,y1,x2,y2))`); }
          list.push({ t: frame(a.ms), v: a.v, ease: curve ?? EASE.standard });
        }
      }
    });
  });
  return out;
}
// Interpolate spec values (colours as rgba arrays) for sampled curves.
function lerpValues(prop, a, b, t) {
  if (prop === "fill" || prop === "stroke") { const ca = parseColor(a), cb = parseColor(b); return `rgba(${lerp(ca, cb, t).map((x, i) => (i < 3 ? Math.round(x * 255) : round(x))).join(",")})`; }
  return lerp(Array.isArray(a) || Array.isArray(b) ? (Array.isArray(a) ? a : [a, a]) : a, Array.isArray(b) || Array.isArray(a) ? (Array.isArray(b) ? b : [b, b]) : b, t);
}

// A value that does not move: 1-D properties are plain numbers (opacity: 100, not [100]).
function still(prop, v, layer) {
  const k = toLottie(prop, v, layer);
  return { a: 0, k: k.length === 1 ? k[0] : k };
}

function animated(prop, keys, layer) {
  const sorted = [...keys].sort((a, b) => a.t - b.t);
  // two keys on the same frame: keep the later one
  const ks = sorted.filter((k, i) => !(sorted[i + 1] && sorted[i + 1].t === k.t));
  if (ks.length === 1) return still(prop, ks[0].v, layer);
  return {
    a: 1,
    k: ks.map((k, i) => {
      const s = toLottie(prop, k.v, layer);
      if (i === ks.length - 1) return { t: k.t, s };
      const [x1, y1, x2, y2] = k.ease;
      return { t: k.t, s, ...(k.hold ? { h: 1 } : { o: { x: [x1], y: [y1] }, i: { x: [x2], y: [y2] } }) };
    }),
  };
}

// ---------------------------------------------------------------- compile
const CAP = { butt: 1, round: 2, square: 3 }, JOIN = { miter: 1, round: 2, bevel: 3 };
const pathShape = (sp, cx, cy) => ({
  ty: "sh",
  ks: {
    a: 0,
    k: {
      c: sp.closed,
      // vertices relative to the layer anchor; tangents relative to their vertex
      v: sp.pts.map(([x, y]) => [round(x - cx), round(y - cy)]),
      o: sp.pts.map((p, i) => { const seg = sp.segs[i]; return seg ? [round(seg[0][0] - p[0]), round(seg[0][1] - p[1])] : [0, 0]; }),
      i: sp.pts.map((p, i) => { const seg = i === 0 ? (sp.closed ? sp.segs[sp.pts.length - 1] : null) : sp.segs[i - 1]; return seg ? [round(seg[1][0] - p[0]), round(seg[1][1] - p[1])] : [0, 0]; }),
    },
  },
});

// Gradient fill (gf) / stroke (gs): colour stops, then opacity stops when any stop is see-through.
function gradientPaint(ty, paint, it, l) {
  const { type, s, e, stops } = paint.gradient;
  const colors = stops.flatMap(([o, r, g, b]) => [o, r, g, b]);
  const alphas = stops.some((x) => x[4] < 1) ? stops.flatMap(([o, , , , a]) => [o, a]) : [];
  return {
    ty, nm: ty === "gf" ? "fill" : "stroke", o: { a: 0, k: round(paint.alpha * it.opacity * 100) }, r: it.evenodd ? 2 : 1, bm: 0,
    g: { p: stops.length, k: { a: 0, k: [...colors, ...alphas].map(round) } },
    s: { a: 0, k: [round(s[0] - l.cx), round(s[1] - l.cy)] }, e: { a: 0, k: [round(e[0] - l.cx), round(e[1] - l.cy)] },
    t: type === "radial" ? 2 : 1, h: { a: 0, k: 0 }, a: { a: 0, k: 0 },
  };
}

function bounds(items) {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const it of items) for (const sp of it.subs) for (const p of [...sp.pts, ...sp.segs.flat()]) { x0 = Math.min(x0, p[0]); y0 = Math.min(y0, p[1]); x1 = Math.max(x1, p[0]); y1 = Math.max(y1, p[1]); }
  return { cx: (x0 + x1) / 2, cy: (y0 + y1) / 2, w: x1 - x0, h: y1 - y0 };
}

/**
 * spec: { name?, width?, height?, duration (ms), fps = 60, svg (markup), tracks: [{ layer, prop, from?, to?, start?, end?, duration?, keys?, ease?, stagger? }] }
 * Returns { lottie, summary, warnings }.
 */
export function compileLottie(spec) {
  const fps = spec.fps ?? 60;
  const duration = Number(spec.duration ?? 1000);
  if (!(duration > 0)) throw new Error("duration (ms) must be > 0");
  const art = svgLayers(spec.svg, { split: spec.split });
  const width = spec.width ?? art.width, height = spec.height ?? art.height;
  const sx = width / art.width, sy = height / art.height;
  const warnings = [...art.warnings];
  const layers = art.layers.map((l) => {
    if (sx !== 1 || sy !== 1) for (const it of l.items) for (const sp of it.subs) { sp.pts = sp.pts.map(([x, y]) => [x * sx, y * sy]); sp.segs = sp.segs.map((s) => s.map(([x, y]) => [x * sx, y * sy])); }
    return { ...l, ...bounds(l.items) };
  });
  if (!layers.length) throw new Error("The SVG has nothing to draw (no filled or stroked shapes)");
  const frames = Math.max(1, Math.round((duration / 1000) * fps));
  const keys = keyframes(spec.tracks ?? [], layers, fps, duration, warnings);

  const out = layers.map((l, idx) => {
    const k = keys.get(l.name);
    const prop = (name, rest) => (k[name] ? animated(name, k[name], l) : still(name, rest, l));
    const groups = l.items.map((it, gi) => {
      const shapes = it.subs.map((sp) => pathShape(sp, l.cx, l.cy));
      const paints = [];
      if (k.draw) paints.push({ ty: "tm", s: { a: 0, k: 0 }, e: animated("draw", k.draw, l), o: { a: 0, k: 0 }, m: 1, nm: "draw" });
      if (it.fill?.gradient) paints.push(gradientPaint("gf", it.fill, it, l));
      else if (it.fill) paints.push({ ty: "fl", c: k.fill ? animated("fill", k.fill, l) : { a: 0, k: it.fill.slice(0, 3).concat(1).map(round) }, o: { a: 0, k: round(it.fill[3] * it.opacity * 100) }, r: it.evenodd ? 2 : 1, nm: "fill" });
      if (it.stroke?.gradient) paints.push({ ...gradientPaint("gs", it.stroke, it, l), w: { a: 0, k: round(it.strokeWidth) }, lc: CAP[it.cap] ?? 1, lj: JOIN[it.join] ?? 1, ml: 4 });
      else if (it.stroke) paints.push({ ty: "st", c: k.stroke ? animated("stroke", k.stroke, l) : { a: 0, k: it.stroke.slice(0, 3).concat(1).map(round) }, o: { a: 0, k: round(it.stroke[3] * it.opacity * 100) }, w: { a: 0, k: round(it.strokeWidth) }, lc: CAP[it.cap] ?? 1, lj: JOIN[it.join] ?? 1, ml: 4, nm: "stroke" });
      return {
        ty: "gr", nm: `${l.name}-${gi + 1}`,
        it: [...shapes, ...paints, { ty: "tr", p: { a: 0, k: [0, 0] }, a: { a: 0, k: [0, 0] }, s: { a: 0, k: [100, 100] }, r: { a: 0, k: 0 }, o: { a: 0, k: 100 }, sk: { a: 0, k: 0 }, sa: { a: 0, k: 0 } }],
      };
    });
    return {
      ddd: 0, ind: idx + 1, ty: 4, nm: l.name, sr: 1, ao: 0, ip: 0, op: frames, st: 0, bm: 0,
      ks: { o: prop("opacity", 1), r: prop("rotate", 0), p: prop("move", [0, 0]), a: { a: 0, k: [0, 0, 0] }, s: prop("scale", 1) },
      shapes: groups.reverse(), // Lottie draws the first group on top; SVG draws the last on top
    };
  });
  const lottie = { v: "5.7.4", fr: fps, ip: 0, op: frames, w: Math.round(width), h: Math.round(height), nm: spec.name ?? "animation", ddd: 0, assets: [], layers: out.reverse(), markers: [] };
  const animatedLayers = [...keys].filter(([, p]) => Object.keys(p).length).map(([n, p]) => `${n}: ${Object.keys(p).join(", ")}`);
  const summary = [
    `${lottie.nm} · ${lottie.w}×${lottie.h} · ${duration}ms @ ${fps}fps (${frames} frames) · ${layers.length} layer(s)`,
    `layers: ${layers.map((l) => l.name).join(", ")}`,
    animatedLayers.length ? `animated: ${animatedLayers.join(" · ")}` : "nothing animated (no track matched a layer)",
  ];
  return { lottie, summary: summary.join("\n"), warnings };
}
