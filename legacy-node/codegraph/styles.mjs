// Design-token definitions and usages found in web code: CSS custom properties, Tailwind utilities and raw colors.
// Dart tokens come from the Dart parser instead (static fields of AppColors-like classes, AppColors.x references).

const WEB = /\.(tsx?|jsx?|mjs|cjs|vue|svelte|astro|html)$/;
const CSS = /\.(css|scss|sass|less)$/;

// Tailwind utility prefix → the @theme namespace it reads (Tailwind v4 naming).
export const TW_NAMESPACE = [
  [/^(bg|text|border|border-[trblxy]|ring|ring-offset|outline|fill|stroke|from|via|to|decoration|divide|placeholder|caret|accent|shadow|inset-shadow)$/, "color"],
  [/^(rounded|rounded-[trbl]{1,2}|rounded-[se][se]?)$/, "radius"],
  [/^(p|px|py|pt|pr|pb|pl|ps|pe|m|mx|my|mt|mr|mb|ml|ms|me|gap|gap-x|gap-y|space-x|space-y|w|h|min-w|min-h|max-w|max-h|size|inset|top|right|bottom|left|translate-x|translate-y|scroll-m|scroll-p)$/, "spacing"],
  [/^(shadow)$/, "shadow"],
  [/^(text)$/, "text"],
  [/^(font)$/, "font"],
  [/^(leading)$/, "leading"],
];
const TW_UTIL = /(?<![\w-])(?:[a-z0-9-]+:)*-?((?:inset-shadow|ring-offset|border-[trblxy]|rounded-[trbl]{1,2}|rounded-[se]{1,2}|space-[xy]|gap-[xy]|min-[wh]|max-[wh]|translate-[xy]|scroll-[mp]|bg|text|border|ring|outline|fill|stroke|from|via|to|decoration|divide|placeholder|caret|accent|shadow|rounded|p[xytrblse]?|m[xytrblse]?|gap|w|h|size|inset|top|right|bottom|left|font|leading))-([a-z][a-z0-9-]*?)(?:\/\d+)?(?![\w-])/g;

function lineAt(text, index) {
  let n = 1;
  for (let i = text.indexOf("\n"); i !== -1 && i < index; i = text.indexOf("\n", i + 1)) n++;
  return n;
}

// → { defs: [{ name, value, line }], refs: [{ form, name, line, count }] }
export function scanStyles(text, path) {
  const isCss = CSS.test(path);
  if (!isCss && !WEB.test(path)) return { defs: [], refs: [] };
  const defs = [];
  const refs = new Map();
  const add = (form, name, index) => {
    const key = `${form}|${name}`;
    const r = refs.get(key);
    if (r) r.count++;
    else refs.set(key, { form, name, line: lineAt(text, index), count: 1 });
  };
  const defLines = new Set();
  if (isCss) {
    for (const m of text.matchAll(/(?<![\w-])--([A-Za-z0-9_-]+)\s*:\s*([^;{}]+);/g)) {
      const line = lineAt(text, m.index);
      defLines.add(line);
      defs.push({ name: `--${m[1]}`, value: m[2].trim().slice(0, 80), line });
    }
    // @apply bg-primary … inside CSS
    for (const m of text.matchAll(/@apply\s+([^;]+);/g)) for (const u of m[1].matchAll(TW_UTIL)) add("tw", `${u[1]}-${u[2]}`, m.index);
  }
  for (const m of text.matchAll(/var\(\s*--([A-Za-z0-9_-]+)/g)) add("var", `--${m[1]}`, m.index);
  if (!isCss) {
    // Only inside string-ish class lists: className="…", cn("…"), cva("…"), tw`…`, class="…"
    for (const s of text.matchAll(/(["'`])((?:(?!\1)[^\\\n]|\\.){2,400}?)\1/g)) {
      if (!/[a-z]-[a-z]/.test(s[2])) continue;
      for (const u of s[2].matchAll(TW_UTIL)) add("tw", `${u[1]}-${u[2]}`, s.index);
    }
  }
  // Raw colors written in code (not token definitions)
  for (const m of text.matchAll(/#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3})(?![0-9a-zA-Z_-])/g)) {
    if (isCss && defLines.has(lineAt(text, m.index))) continue;
    if (!isCss && !/["'`]\s*#|:\s*["'`]?#|\(\s*["'`]?#/.test(text.slice(Math.max(0, m.index - 3), m.index + 1))) continue;
    add("hex", m[0].toLowerCase(), m.index);
  }
  return { defs, refs: [...refs.values()] };
}

// Tailwind "bg-primary" → candidate CSS variables it would read ("--color-primary").
export function twToVars(util) {
  for (let i = util.indexOf("-"); i !== -1; i = util.indexOf("-", i + 1)) {
    const prefix = util.slice(0, i), value = util.slice(i + 1);
    const hits = TW_NAMESPACE.filter(([re]) => re.test(prefix)).map(([, ns]) => `--${ns}-${value}`);
    if (hits.length) return hits;
  }
  return [];
}

// Classes that hold design tokens in Dart (same rule as the Dart parser).
export const DART_TOKEN_CLASS = /(Colou?rs?|Palette|Spacing|Spaces?|Radius|Radii|Sizes?|Shadows?|Elevations?|Typography|TextStyles?|Fonts?|Tokens?|Insets|Gaps|Durations|Breakpoints|Theme)$/;

// Comparable color: "#2563EB", "#2563ebff", "Color(0xFF2563EB)", "0xFF2563EB" → "#2563eb" (alpha kept only when not opaque).
export function normColor(v) {
  if (!v) return null;
  const s = String(v).trim();
  const dart = /0x([0-9a-f]{8})/i.exec(s);
  if (dart) {
    const [a, rgb] = [dart[1].slice(0, 2), dart[1].slice(2)];
    return `#${rgb}${a.toLowerCase() === "ff" ? "" : a}`.toLowerCase();
  }
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(s);
  if (!hex) return null;
  let h = hex[1].toLowerCase();
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  if (h.length === 8 && h.endsWith("ff")) h = h.slice(0, 6);
  return `#${h}`;
}
