// Links the code graph to the Figma design: components (wara.components.json + name matching) and tokens.
// A snapshot of the Figma side is stored in the graph (meta "design") so links work while Figma is closed.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { COMPONENT_MAP_FILE, formatTokens, writeComponentMap } from "../codegen.mjs";
import { createDartResolver } from "./build.mjs";
import { getMeta, setMeta } from "./db.mjs";
import { createResolver } from "./resolve-ts.mjs";
import { normColor, twToVars } from "./styles.mjs";

// ---------- snapshot from Figma (needs the plugin) ----------

export async function syncDesign(db, send) {
  const info = await send("get_document_info", {});
  const components = await send("get_components", {});
  const data = await send("get_tokens", { source: "auto" }, 120_000);
  const css = formatTokens(data, "tailwind").map;
  const dart = formatTokens(data, "dart").map;
  const defaultMode = new Map((data.collections ?? []).map((c) => [c.name, c.defaultMode]));
  const tokens = [];
  for (const v of data.variables ?? []) {
    if (v.type === "BOOLEAN") continue;
    const raw = v.values?.[defaultMode.get(v.collection)] ?? Object.values(v.values ?? {})[0];
    tokens.push({ token: v.token, type: v.type, value: typeof raw === "object" ? `→ ${raw?.alias}` : String(raw), css: css[v.token], dart: dart[v.token] });
  }
  for (const s of data.paintStyles ?? []) {
    if (typeof s.value === "string") tokens.push({ token: s.token, type: "COLOR", value: s.value, css: css[s.token], dart: dart[s.token] });
  }
  for (const s of data.textStyles ?? []) tokens.push({ token: s.token, type: "TEXT", value: `${s.family} ${s.style} ${s.size}px`, css: css[s.token], dart: dart[s.token] });
  for (const s of data.effectStyles ?? []) tokens.push({ token: s.token, type: "SHADOW", value: `${s.shadows?.length ?? 0} shadow(s)`, css: css[s.token], dart: dart[s.token] });
  const snapshot = {
    file: info.fileName,
    fetchedAt: new Date().toISOString(),
    components: components.filter((c) => c.type === "COMPONENT" || c.type === "COMPONENT_SET").map((c) => ({ name: c.name, id: c.id, type: c.type })),
    tokens,
  };
  setMeta(db, "design", JSON.stringify(snapshot));
  return { file: snapshot.file, components: snapshot.components.length, tokens: tokens.length };
}

export function designSnapshot(db) {
  try {
    return JSON.parse(getMeta(db, "design") ?? "null");
  } catch {
    return null;
  }
}

// ---------- components ----------

const norm = (s) => String(s).toLowerCase().replace(/[^a-z0-9]/g, "");

function readMap(root) {
  try {
    return JSON.parse(readFileSync(join(root, COMPONENT_MAP_FILE), "utf8"));
  } catch {
    return {};
  }
}

// Figma component ↔ code component/widget. "linked" come from wara.components.json, "suggested" from names.
export function componentLinks(g, root, snapshot) {
  const map = readMap(root);
  const paths = [...g.files.values()].map((f) => f.path);
  const resolveTs = createResolver(root, paths);
  const resolveDart = createDartResolver(root, paths);
  const byPath = new Map([...g.files.values()].map((f) => [f.path, f]));
  const codeComponents = [...g.symbolById.values()].filter((s) => (s.kind === "component" || s.kind === "widget") && s.exported);

  const linked = [];
  const linkedSymbols = new Set();
  const classes = classIndex(g, root, Object.values(map));
  for (const [figmaName, entry] of Object.entries(map)) {
    if (entry.className) {
      const names = [...new Set([entry.className, ...Object.values(entry.variants ?? {})].join(" ").split(/\s+/).filter(Boolean))];
      const figma = snapshot?.components.find((c) => c.name === figmaName || c.name.split("/")[0] === figmaName) ?? null;
      linked.push({
        figmaName, figma, entry, symbol: null,
        classes: {
          defined: names.filter((n) => classes.defs.has(n)).map((n) => `.${n} ${classes.defs.get(n)}`),
          missing: names.filter((n) => !classes.defs.has(n)),
          files: classes.usedIn(entry.className.split(/\s+/)),
        },
      });
      continue;
    }
    const spec = entry.import ?? "";
    const r = spec.startsWith("package:") || spec.endsWith(".dart") ? resolveDart("lib/main.dart", spec) : resolveTs("index.ts", spec);
    const file = r.file ? byPath.get(r.file) : null;
    const symbol = file ? g.definition(file.id, entry.component) ?? null : codeComponents.find((s) => s.name === entry.component) ?? null;
    const figma = snapshot?.components.find((c) => c.name === figmaName || c.name.split("/")[0] === figmaName) ?? null;
    linked.push({ figmaName, figma, entry, symbol });
    if (symbol) linkedSymbols.add(symbol.id);
  }

  const mappedNames = new Set(Object.keys(map));
  const suggested = [];
  const figmaOnly = [];
  for (const c of snapshot?.components ?? []) {
    if (mappedNames.has(c.name) || mappedNames.has(c.name.split("/")[0])) continue;
    const parts = c.name.split("/").map((p) => p.split("=").pop().trim()).filter(Boolean);
    const candidates = [parts.join(""), [...parts].reverse().join(""), parts[0], parts.at(-1)].map(norm);
    let best = null;
    for (const [rank, cand] of candidates.entries()) {
      const hit = codeComponents.find((s) => norm(s.name) === cand || norm(s.name.replace(/(Widget|Component|View)$/, "")) === cand);
      if (hit) { best = { symbol: hit, rank }; break; }
    }
    if (best) suggested.push({ figma: c, symbol: best.symbol, exact: best.rank < 2 });
    else figmaOnly.push(c);
  }
  const suggestedIds = new Set(suggested.map((s) => s.symbol.id));
  const codeOnly = codeComponents.filter((s) => !linkedSymbols.has(s.id) && !suggestedIds.has(s.id));
  return { linked, suggested, figmaOnly, codeOnly };
}

// Where CSS classes of class mappings are defined (stylesheets) and which source files use them.
// Read on demand: only projects with class mappings pay for it.
function classIndex(g, root, entries) {
  const defs = new Map();
  const sources = [];
  if (!entries.some((e) => e.className)) return { defs, usedIn: () => 0 };
  for (const f of g.files.values()) {
    const ext = f.path.split(".").pop();
    if (["css", "scss", "sass", "less"].includes(ext)) {
      readFileSafe(join(root, f.path)).split("\n").forEach((line, i) => {
        for (const m of line.matchAll(/\.(-?[A-Za-z_][\w-]*)/g)) if (!defs.has(m[1]) && /[{,]|^\s*\./.test(line)) defs.set(m[1], `${f.path}:${i + 1}`);
      });
    } else if (["tsx", "jsx", "ts", "js", "html", "vue", "svelte", "php"].includes(ext)) {
      sources.push(readFileSafe(join(root, f.path)));
    }
  }
  const usedIn = (names) => {
    const res = names.filter(Boolean).map((n) => new RegExp(`(^|[\\s"'\`{])${n.replace(/[-]/g, "\\-")}(?=[\\s"'\`}]|$)`, "m"));
    return sources.filter((text) => res.every((re) => re.test(text))).length;
  };
  return { defs, usedIn };
}

// The module path people already use for a file ("@/components/ui/button", "package:app/widgets/card.dart").
export function importPathFor(g, root, file) {
  const counts = new Map();
  for (const imps of g.importsOf.values()) {
    for (const i of imps) {
      if (i.target_file_id !== file.id || i.source.startsWith(".")) continue;
      counts.set(i.source, (counts.get(i.source) ?? 0) + 1);
    }
  }
  const best = [...counts].sort((a, b) => b[1] - a[1])[0]?.[0];
  if (best) return best;
  if (file.lang === "dart") {
    const pubspec = readFileSafe(join(root, "pubspec.yaml"));
    const pkg = /^name:\s*([A-Za-z0-9_]+)/m.exec(pubspec)?.[1];
    return pkg && file.path.startsWith("lib/") ? `package:${pkg}/${file.path.slice(4)}` : file.path;
  }
  return `./${file.path.replace(/\.(tsx?|jsx?|mjs)$/, "").replace(/\/index$/, "")}`;
}

const readFileSafe = (p) => {
  try {
    return readFileSync(p, "utf8");
  } catch {
    return "";
  }
};

// Confirm a Figma ↔ code pair: writes it to wara.components.json (the same file map_component uses).
export async function confirmLink(g, root, figmaName, symbolId) {
  const sym = g.symbolById.get(symbolId);
  if (!sym) throw new Error(`Unknown symbol ${symbolId}`);
  const map = readMap(root);
  map[figmaName] = { ...(map[figmaName] ?? {}), import: importPathFor(g, root, sym.file), component: sym.name };
  await writeComponentMap(root, map);
  return map[figmaName];
}

// ---------- tokens ----------

// Every token from the code (definitions) and/or Figma, with usage counts and mismatches.
export function tokenTable(db, g, snapshot) {
  const defs = db.prepare("SELECT d.*, f.path FROM style_defs d JOIN files f ON f.id = d.file_id").all();
  const uses = db.prepare("SELECT r.*, f.path FROM style_refs r JOIN files f ON f.id = r.file_id WHERE r.form != 'hex'").all();
  const rows = new Map(); // code name → row
  const row = (name) => {
    if (!rows.has(name)) rows.set(name, { name, defs: [], figma: null, uses: [], files: new Set(), count: 0 });
    return rows.get(name);
  };
  for (const d of defs) row(d.name).defs.push(d);
  for (const t of snapshot?.tokens ?? []) {
    const names = [t.css, t.dart].filter(Boolean);
    const existing = names.find((n) => rows.has(n));
    row(existing ?? names[0] ?? `figma:${t.token}`).figma = t;
  }
  // Usages: var(--x) and AppColors.x by name; Tailwind utilities through the variables they read.
  for (const u of uses) {
    const targets = u.form === "tw" ? twToVars(u.name) : [u.name];
    const name = targets.find((t) => rows.has(t));
    if (!name) continue;
    const r = rows.get(name);
    r.uses.push(u);
    r.files.add(u.path);
    r.count += u.count;
  }
  // var(--primary) → the value --primary holds (follow up to 5 hops), so aliases can be compared with Figma.
  const resolveValue = (v, hops = 0) => {
    const m = /^var\(\s*(--[\w-]+)\s*\)$/.exec(String(v ?? "").trim());
    if (!m || hops > 5) return v;
    return resolveValue(rows.get(m[1])?.defs[0]?.value ?? v, hops + 1);
  };
  for (const r of rows.values()) {
    r.resolved = r.defs[0] ? resolveValue(r.defs[0].value) : null;
    const code = normColor(r.resolved);
    const figma = normColor(r.figma?.value);
    r.mismatch = Boolean(code && figma && code !== figma);
  }
  const hardcoded = db.prepare("SELECT r.name, sum(r.count) AS n, count(DISTINCT r.file_id) AS files FROM style_refs r WHERE r.form = 'hex' GROUP BY r.name ORDER BY n DESC").all();
  const hardFiles = db.prepare("SELECT f.path, sum(r.count) AS n FROM style_refs r JOIN files f ON f.id = r.file_id WHERE r.form = 'hex' GROUP BY f.path ORDER BY n DESC").all();
  return { rows: [...rows.values()], hardcoded, hardFiles };
}
