// Design → code helpers: token files (CSS / Tailwind v4) and the Figma → code component map.
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join } from "node:path";

export const COMPONENT_MAP_FILE = "wara.components.json";

export function requireAbsolute(path, label) {
  if (!isAbsolute(path)) throw new Error(`${label} must be an absolute path, got "${path}"`);
  return path;
}

export function kebab(name) {
  return name
    .normalize("NFKD")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export async function writeFileDeep(file, data) {
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, data);
}

// ---------- component map ----------

export async function readComponentMap(projectDir) {
  const file = join(requireAbsolute(projectDir, "projectDir"), COMPONENT_MAP_FILE);
  try {
    return JSON.parse(await readFile(file, "utf8"));
  } catch (err) {
    if (err.code === "ENOENT") return {};
    throw new Error(`Cannot read ${file}: ${err.message}`);
  }
}

export async function writeComponentMap(projectDir, map) {
  const sorted = Object.fromEntries(Object.entries(map).sort(([a], [b]) => a.localeCompare(b)));
  const file = join(projectDir, COMPONENT_MAP_FILE);
  await writeFileDeep(file, `${JSON.stringify(sorted, null, 2)}\n`);
  return file;
}

// Class mappings: base className + the classes of each "Prop=Value" in the instance's variant.
export function resolveEntry(entry, variant) {
  if (!entry?.className || !entry.variants) return entry;
  const extra = String(variant ?? "")
    .split(",")
    .map((pair) => entry.variants[pair.split("=").map((s) => s.trim()).join("=")])
    .filter(Boolean);
  const { variants, ...rest } = entry;
  return { ...rest, className: [entry.className, ...extra].join(" ") };
}

// Adds `code` to every instance that has a mapping; returns the component names still unmapped.
export function annotateComponents(root, map) {
  const unmapped = new Set();
  const visit = (node) => {
    if (node?.kind === "instance") {
      const { name, variant } = node.component;
      const entry = (variant && map[`${name}/${variant}`]) || map[name];
      if (entry) node.code = resolveEntry(entry, variant);
      else unmapped.add(name);
    }
    node?.children?.forEach(visit);
  };
  visit(root);
  return [...unmapped];
}

// ---------- responsive merge ----------

export const BREAKPOINTS = ["base", "sm", "md", "lg", "xl"];

function flatten(value, prefix = "", out = {}) {
  if (value && typeof value === "object" && !Array.isArray(value) && !("token" in value)) {
    for (const [k, v] of Object.entries(value)) flatten(v, prefix ? `${prefix}.${k}` : k, out);
  } else {
    out[prefix] = value;
  }
  return out;
}

// name differs by design (Home/Mobile vs Home/Desktop); template and children are merged recursively.
const OWN_KEYS_SKIPPED = new Set(["id", "name", "children", "template", "responsive", "hiddenAt", "onlyAt", "items"]);

function ownProps(node) {
  return flatten(Object.fromEntries(Object.entries(node).filter(([k]) => !OWN_KEYS_SKIPPED.has(k))));
}

const childKey = (n) => (n.kind === "repeat" ? `repeat:${n.template?.name}` : `${n.kind}:${n.name}`);

function keyed(children = []) {
  const seen = new Map();
  return children.map((child) => {
    const key = childKey(child);
    const n = seen.get(key) ?? 0;
    seen.set(key, n + 1);
    return [`${key}#${n}`, child];
  });
}

// Mobile-first: `base` keeps the smallest layout; each larger breakpoint records only what changes.
function mergeNode(base, other, bp) {
  if (Array.isArray(base.onlyAt)) base.onlyAt.push(bp);
  const a = ownProps(base);
  const b = ownProps(other);
  const diff = {};
  for (const path of new Set([...Object.keys(a), ...Object.keys(b)])) {
    if (JSON.stringify(a[path]) !== JSON.stringify(b[path])) diff[path] = b[path] ?? null;
  }
  if (Object.keys(diff).length) (base.responsive ??= {})[bp] = diff;

  if (base.kind === "repeat" && other.kind === "repeat" && base.template && other.template) {
    mergeNode(base.template, other.template, bp);
  }

  const otherChildren = new Map(keyed(other.children));
  for (const [key, child] of keyed(base.children)) {
    const match = otherChildren.get(key);
    if (match) {
      mergeNode(child, match, bp);
      otherChildren.delete(key);
    } else if (!child.onlyAt) {
      (child.hiddenAt ??= []).push(bp);
    }
  }
  for (const extra of otherChildren.values()) {
    (base.children ??= []).push({ ...structuredClone(extra), onlyAt: [bp] });
  }
}

/** contexts: [{ bp, context }] in breakpoint order; the first one is the base layout. */
export function mergeBreakpoints(contexts) {
  const [first, ...rest] = contexts;
  const root = structuredClone(first.context.root);
  for (const { bp, context } of rest) mergeNode(root, context.root, bp);

  const assets = new Map();
  for (const { context } of contexts) for (const a of context.assets) assets.set(`${a.name}.${a.format}`, a);
  return {
    breakpoints: Object.fromEntries(contexts.map(({ bp, context }) => [bp, context.root?.width])),
    note: "Mobile-first: root holds the base layout; responsive[bp] lists only properties that change from that breakpoint up (null = back to default). hiddenAt/onlyAt mark visibility per breakpoint.",
    root,
    tokens: Object.assign({}, ...contexts.map(({ context }) => context.tokens)),
    components: [...new Map(contexts.flatMap(({ context }) => context.components.map((c) => [c.name, c]))).values()],
    fonts: [...new Set(contexts.flatMap(({ context }) => context.fonts))],
    assets: [...assets.values()],
  };
}

// ---------- tokens ----------

const UNITLESS = /(weight|opacity|z-index|zindex|ratio|scale)/;
const WEIGHTS = {
  thin: 100, hairline: 100, extralight: 200, ultralight: 200, light: 300, regular: 400, normal: 400, book: 400,
  medium: 500, semibold: 600, demibold: 600, bold: 700, extrabold: 800, ultrabold: 800, black: 900, heavy: 900,
};

const fontWeight = (style) => WEIGHTS[style.toLowerCase().replace(/italic|oblique|[\s_-]/g, "")] ?? 400;
const lineHeight = (v) => (v.endsWith("%") ? String(parseFloat(v) / 100) : v);
const strip = (token, re) => token.replace(re, "") || token;

function tailwindVar(token, type) {
  if (type === "COLOR") return `--color-${strip(token, /^colou?rs?-/)}`;
  if (type === "STRING" && /font|family|typeface/.test(token)) return `--font-${strip(token, /^(font-family|font|typeface|family)-/)}`;
  if (type === "FLOAT") {
    if (/radius|radii|corner|rounded/.test(token)) return `--radius-${strip(token, /^(radius|radii|corner-radius|corner|rounded)-/)}`;
    if (/font-size|text-size|fontsize|type-size/.test(token)) return `--text-${strip(token, /^(font-size|text-size|fontsize|type-size|text)-/)}`;
    if (/font-weight|weight/.test(token)) return `--font-weight-${strip(token, /^(font-weight|weight)-/)}`;
    if (/line-height|leading/.test(token)) return `--leading-${strip(token, /^(line-height|leading)-/)}`;
    if (/spacing|space|gap|padding|margin/.test(token)) return `--spacing-${strip(token, /^(spacing|space|gap|padding|margin)-/)}`;
  }
  return `--${token}`;
}

function shadowCss(shadows) {
  return shadows.map((s) => `${s.inset ? "inset " : ""}${s.x}px ${s.y}px ${s.blur}px ${s.spread}px ${s.color}`).join(", ");
}

// ---------- Flutter (Dart) tokens ----------

const camel = (token) => token.replace(/-([a-z0-9])/g, (_, c) => c.toUpperCase());
const pascal = (s) => camel(kebab(s)).replace(/^./, (c) => c.toUpperCase());

function dartIdent(token, prefixRe, fallbackPrefix) {
  const ident = camel(strip(token, prefixRe));
  return /^[0-9]/.test(ident) ? `${fallbackPrefix}${ident}` : ident;
}

function dartColor(hex) {
  const h = hex.replace("#", "").toUpperCase();
  const alpha = h.length === 8 ? h.slice(6, 8) : "FF";
  return `Color(0x${alpha}${h.slice(0, 6)})`;
}

function dartGroup(v) {
  if (v.type === "COLOR") return ["AppColors", /^colou?rs?-/, "c"];
  if (v.type === "STRING") return ["AppStrings", /^(font-family|font|family|string)-/, "s"];
  if (/radius|radii|corner|rounded/.test(v.token)) return ["AppRadius", /^(radius|radii|corner-radius|corner|rounded)-/, "r"];
  if (/spacing|space|gap|padding|margin/.test(v.token)) return ["AppSpacing", /^(spacing|space|gap|padding|margin)-/, "s"];
  if (/font-size|text-size|fontsize/.test(v.token)) return ["AppFontSizes", /^(font-size|text-size|fontsize)-/, "f"];
  return ["AppDimens", /^$/, "d"];
}

function formatDart(data) {
  const map = {};
  const classes = new Map();
  const add = (cls, line) => {
    if (!classes.has(cls)) classes.set(cls, []);
    classes.get(cls).push(line);
  };
  const defaultMode = new Map(data.collections.map((c) => [c.name, c.defaultMode]));
  const variables = data.variables.filter((v) => v.type !== "BOOLEAN");
  for (const v of variables) {
    const [cls, re, prefix] = dartGroup(v);
    map[v.token] = `${cls}.${dartIdent(v.token, re, prefix)}`;
  }
  const byToken = new Map(variables.map((v) => [v.token, v]));
  const modeClass = (cls, collection, mode) => (mode === defaultMode.get(collection) ? cls : `${cls}${pascal(mode)}`);
  const dartValue = (raw, v, mode) => {
    if (raw && typeof raw === "object" && "alias" in raw) {
      const target = byToken.get(raw.alias);
      if (!target) return map[raw.alias] ?? "null";
      // An alias inside a mode points at the same mode of its target (Dark → AppColorsDark.x).
      const [cls] = dartGroup(target);
      const sameMode = mode in target.values ? mode : defaultMode.get(target.collection);
      return `${modeClass(cls, target.collection, sameMode)}.${map[raw.alias].split(".")[1]}`;
    }
    if (v.type === "COLOR") return dartColor(raw);
    if (v.type === "STRING") return JSON.stringify(String(raw));
    return Number.isInteger(raw) ? `${raw}.0` : String(raw);
  };
  for (const v of variables) {
    const [cls] = dartGroup(v);
    const ident = map[v.token].split(".")[1];
    const type = v.type === "FLOAT" ? "double " : "";
    for (const [mode, raw] of Object.entries(v.values)) {
      add(modeClass(cls, v.collection, mode), `  static const ${type}${ident} = ${dartValue(raw, v, mode)};`);
    }
  }
  for (const s of data.paintStyles) {
    const v = s.value;
    if (typeof v === "string" || v?.token) {
      const ident = dartIdent(s.token, /^colou?rs?-/, "c");
      map[s.token] = `AppColors.${ident}`;
      add("AppColors", `  static const ${ident} = ${typeof v === "string" ? dartColor(v) : map[v.token]};`);
    } else if (v?.gradient === "linear") {
      const ident = dartIdent(s.token, /^gradients?-/, "g");
      map[s.token] = `AppGradients.${ident}`;
      const colors = v.stops.map((st) => dartColor(st.color)).join(", ");
      const stops = v.stops.map((st) => st.position).join(", ");
      add("AppGradients", `  static const ${ident} = LinearGradient(begin: Alignment.topCenter, end: Alignment.bottomCenter, colors: [${colors}], stops: [${stops}]);`);
    }
  }
  for (const s of data.textStyles) {
    const ident = dartIdent(s.token, /^(text|type|typography|font)-/, "t");
    map[s.token] = `AppTextStyles.${ident}`;
    const parts = [`fontFamily: ${JSON.stringify(s.family)}`, `fontSize: ${s.size}`, `fontWeight: FontWeight.w${fontWeight(s.style)}`];
    if (s.lineHeight.endsWith("%")) parts.push(`height: ${parseFloat(s.lineHeight) / 100}`);
    else if (s.lineHeight.endsWith("px")) parts.push(`height: ${Math.round((parseFloat(s.lineHeight) / s.size) * 1000) / 1000}`);
    const ls = parseFloat(s.letterSpacing);
    if (ls) parts.push(`letterSpacing: ${s.letterSpacing.endsWith("em") ? Math.round(ls * s.size * 100) / 100 : ls}`);
    add("AppTextStyles", `  static const ${ident} = TextStyle(${parts.join(", ")});`);
  }
  for (const s of data.effectStyles) {
    if (!s.shadows.length) continue;
    const ident = dartIdent(s.token, /^(shadow|elevation|effect)-/, "e");
    map[s.token] = `AppShadows.${ident}`;
    const shadows = s.shadows
      .filter((sh) => !sh.inset)
      .map((sh) => `BoxShadow(offset: Offset(${sh.x}, ${sh.y}), blurRadius: ${sh.blur}, spreadRadius: ${sh.spread}, color: ${dartColor(sh.color)})`);
    add("AppShadows", `  static const ${ident} = [${shadows.join(", ")}];`);
  }
  const header = `// Generated by wridge from Figma "${data.file}". Re-run get_tokens instead of editing by hand.\nimport 'package:flutter/material.dart';`;
  const body = [...classes].map(([cls, lines]) => `abstract final class ${cls} {\n${lines.join("\n")}\n}`);
  return { content: `${[header, ...body].join("\n\n")}\n`, map };
}

/**
 * Formats get_tokens output. Returns { content, map } where map is design-token → CSS variable,
 * so tokens seen in get_design_context can be translated to the generated variables.
 */
export function formatTokens(data, format) {
  if (format === "json") return { content: JSON.stringify(data, null, 2), map: {} };
  if (format === "dart") return formatDart(data);
  const tailwind = format === "tailwind";
  const map = {};
  const cssVar = (token, type) => (map[token] ??= tailwind ? tailwindVar(token, type) : `--${token}`);

  for (const v of data.variables) cssVar(v.token, v.type);
  const value = (raw, v) => {
    if (raw && typeof raw === "object" && "alias" in raw) return `var(${map[raw.alias] ?? `--${raw.alias}`})`;
    if (v.type === "FLOAT") return UNITLESS.test(v.token) ? String(raw) : `${raw}px`;
    if (v.type === "STRING") return JSON.stringify(String(raw));
    return String(raw);
  };

  const base = [];
  const modes = new Map();
  const defaultMode = new Map(data.collections.map((c) => [c.name, c.defaultMode]));
  for (const v of data.variables) {
    if (v.type === "BOOLEAN") continue;
    for (const [mode, raw] of Object.entries(v.values)) {
      const line = `  ${map[v.token]}: ${value(raw, v)};`;
      if (mode === defaultMode.get(v.collection)) base.push(line);
      else {
        if (!modes.has(mode)) modes.set(mode, []);
        modes.get(mode).push(line);
      }
    }
  }

  const skipped = [];
  for (const s of data.paintStyles) {
    const v = s.value;
    if (typeof v === "string") base.push(`  ${cssVar(s.token, "COLOR")}: ${v};`);
    else if (v?.token) base.push(`  ${cssVar(s.token, "COLOR")}: var(${map[v.token] ?? `--${v.token}`});`);
    else if (v?.gradient) {
      // Not a color: Tailwind would emit an invalid background-color; use bg-(image:--gradient-…).
      const name = (map[s.token] = `--gradient-${strip(s.token, /^gradients?-/)}`);
      const stops = v.stops.map((st) => `${st.color} ${Math.round(st.position * 100)}%`).join(", ");
      base.push(`  ${name}: ${v.gradient === "radial" ? "radial-gradient(" : "linear-gradient(180deg, "}${stops});`);
    } else skipped.push(s.name);
  }

  // Tailwind resolves `text-x` against both --color-x and --text-x (and `shadow-x` against --color-x and
  // --shadow-x), so a color and a text/shadow token with the same name make one utility ambiguous.
  const colorNames = new Set(Object.values(map).filter((v) => v.startsWith("--color-")).map((v) => v.slice("--color-".length)));
  const renamed = [];
  const unique = (prefix, alt, name, token) => {
    if (!tailwind || !colorNames.has(name)) return `${prefix}${name}`;
    renamed.push(`${token} → ${prefix}${alt}${name} (the color ${name} uses ${prefix.replace(/^--|-$/g, "")}-${name})`);
    return `${prefix}${alt}${name}`;
  };

  const textRules = [];
  const fonts = new Map();
  for (const s of data.textStyles) {
    const familyVar = `--font-${kebab(s.family)}`;
    fonts.set(familyVar, `${JSON.stringify(s.family)}, ui-sans-serif, system-ui, sans-serif`);
    if (tailwind) {
      const n = map[s.token] = unique("--text-", "type-", strip(s.token, /^(text|type|typography|font)-/), s.token);
      base.push(`  ${n}: ${s.size}px;`);
      base.push(`  ${n}--line-height: ${lineHeight(s.lineHeight)};`);
      base.push(`  ${n}--font-weight: ${fontWeight(s.style)};`);
      if (s.letterSpacing !== "0px" && s.letterSpacing !== "0em") base.push(`  ${n}--letter-spacing: ${s.letterSpacing};`);
    } else {
      const className = `text-${strip(s.token, /^(text|type|typography|font)-/)}`;
      map[s.token] = `.${className}`;
      textRules.push(
        `.${className} {\n  font-family: var(${familyVar});\n  font-size: ${s.size}px;\n  font-weight: ${fontWeight(s.style)};\n  line-height: ${lineHeight(s.lineHeight)};\n  letter-spacing: ${s.letterSpacing};\n}`,
      );
    }
  }
  for (const [name, stack] of fonts) base.push(`  ${name}: ${stack};`);

  for (const s of data.effectStyles) {
    if (!s.shadows.length) continue;
    const n = (map[s.token] = tailwind ? unique("--shadow-", "depth-", strip(s.token, /^(shadow|elevation|effect)-/), s.token) : `--${s.token}`);
    base.push(`  ${n}: ${shadowCss(s.shadows)};`);
  }

  const header = `/* Generated by wridge from Figma "${data.file}". Re-run get_tokens instead of editing by hand. */`;
  const blocks = [header, `${tailwind ? "@theme" : ":root"} {\n${base.join("\n")}\n}`];
  for (const [mode, lines] of modes) blocks.push(`[data-theme="${kebab(mode)}"] {\n${lines.join("\n")}\n}`);
  blocks.push(...textRules);
  if (skipped.length) blocks.push(`/* Not converted (image/mixed paints): ${skipped.join(", ")} */`);
  if (renamed.length) blocks.push(`/* Renamed to avoid clashing with a color of the same name — use the map:\n   ${renamed.join("\n   ")} */`);
  return { content: `${blocks.join("\n\n")}\n`, map };
}
