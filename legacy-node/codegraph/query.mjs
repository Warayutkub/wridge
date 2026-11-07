// Questions the AI asks the code graph. Answers are compact plain text (far fewer tokens than JSON).
import { findEntity } from "./data/model.mjs";
import { flowsText, traceText } from "./flows.mjs";
import { componentLinks, designSnapshot, tokenTable } from "./design.mjs";
import { allNotes, missingNotes, notesFor } from "./notes.mjs";
import { findNotes, notesAbout } from "./brain.mjs";
import { routeMatches } from "./routes-next.mjs";

export const QUERIES = ["overview", "find", "file", "used_by", "uses", "impact", "route", "design", "tokens", "notes", "data", "table", "flows", "trace"];

export function runQuery(db, query, target, { depth, limit = 40, root } = {}) {
  const g = graph(db);
  const text = answer(g, query, target, { depth, limit, root });
  return root && ["find", "file", "used_by", "uses", "impact"].includes(query) ? text + brainSignpost(g, root, query, target) : text;
}

// The brain's notes about what was asked, one line each: the AI sees that a note exists and reads it only
// if it matters (project_memory read). find also matches notes by words.
function brainSignpost(g, root, query, target) {
  const block = (title, rows) => (rows.length ? `\n\n## ${title} (${rows.length}) — project_memory read <note>\n${rows.map((r) => `- ${r.note}${r.line ? ` — ${r.line}` : ""}`).join("\n")}` : "");
  try {
    if (query === "find") return block("Brain notes", findNotes(root, target, { limit: 3 }));
    const t = String(target ?? "").trim();
    const hit = g.lookup(t);
    const sym = hit.symbols?.length === 1 ? hit.symbols[0] : null;
    const path = hit.file?.path ?? sym?.file?.path ?? t.split("#")[0];
    return block("Brain notes linking here", notesAbout(root, { path, name: sym?.name ?? t.split("#")[1] }));
  } catch {
    return ""; // no brain, or an unclear target: the code answer stands alone
  }
}

function answer(g, query, target, { depth, limit, root }) {
  switch (query) {
    case "overview": return overview(g, limit);
    case "find": return find(g, need(target, query), limit);
    case "file": return fileInfo(g, need(target, query));
    case "used_by": return usedBy(g, need(target, query), limit);
    case "uses": return uses(g, need(target, query), limit);
    case "impact": return impact(g, need(target, query), depth ?? 3, limit * 4);
    case "route": return route(g, need(target, query), depth ?? 3);
    case "design": return design(g, root, limit);
    case "tokens": return tokens(g, target, limit);
    case "notes": return notes(g, limit);
    case "data": return dataOverview(g, limit);
    case "table": return tableInfo(g, need(target, query), limit);
    case "flows": return flowsText(g, root, target, limit);
    case "trace": return traceText(g, need(target, query));
    default: throw new Error(`Unknown query "${query}". Use one of: ${QUERIES.join(", ")}`);
  }
}

function need(target, query) {
  if (!target) throw new Error(`"${query}" needs a target: a symbol name (Button), a file path (components/ui/button.tsx), path#Symbol, or a route (/jobs/[id])`);
  return String(target).trim();
}

// ---------- graph access with per-query memoization ----------

export function graph(db) {
  const files = new Map(db.prepare("SELECT * FROM files").all().map((f) => [f.id, f]));
  const byPath = new Map([...files.values()].map((f) => [f.path, f]));
  const symbolsOf = new Map();
  const symbolById = new Map();
  for (const s of db.prepare("SELECT * FROM symbols").all()) {
    s.file = files.get(s.file_id);
    symbolById.set(s.id, s);
    if (!symbolsOf.has(s.file_id)) symbolsOf.set(s.file_id, []);
    symbolsOf.get(s.file_id).push(s);
  }
  const importsOf = groupBy(db.prepare("SELECT * FROM imports").all(), "file_id");
  const refsOf = groupBy(db.prepare("SELECT * FROM refs").all(), "file_id");
  const routes = db.prepare("SELECT * FROM routes").all().map((r) => ({ ...r, file: files.get(r.file_id) }));
  const defCache = new Map();

  // Where an exported name of a file is really declared, following re-exports and barrels.
  function definition(fileId, name, seen = new Set()) {
    const key = `${fileId}|${name}`;
    if (defCache.has(key)) return defCache.get(key);
    if (seen.has(key)) return null;
    seen.add(key);
    let out = null;
    const syms = symbolsOf.get(fileId) ?? [];
    const own = syms.find((s) => s.exported && s.kind !== "alias" && (name === "default" ? s.is_default : s.name === name));
    const alias = !own && syms.find((s) => s.exported && s.kind === "alias" && s.name === name);
    if (own) out = own;
    else if (alias) out = syms.find((s) => s.name === JSON.parse(alias.props).of && s.kind !== "alias") ?? null;
    else {
      for (const imp of importsOf.get(fileId) ?? []) {
        if (imp.kind === "reexport" && imp.local === name && imp.target_file_id) out = definition(imp.target_file_id, imp.imported, seen);
        if (out) break;
      }
      if (!out) {
        for (const imp of importsOf.get(fileId) ?? []) {
          if (imp.kind === "reexport_all" && imp.target_file_id && name !== "default") out = definition(imp.target_file_id, name, seen);
          if (out) break;
        }
      }
    }
    defCache.set(key, out);
    return out;
  }

  // What a local identifier in a file points at: a symbol, a file, or an external package.
  function bindingTarget(fileId, local) {
    const imp = (importsOf.get(fileId) ?? []).find((i) => i.kind === "import" && i.local === local);
    if (imp) {
      if (imp.external) return { external: imp.external, imported: imp.imported };
      if (!imp.target_file_id) return { unresolved: imp.source };
      if (imp.imported === "*") return { file: files.get(imp.target_file_id), namespace: true };
      return { symbol: definition(imp.target_file_id, imp.imported), file: files.get(imp.target_file_id), imported: imp.imported };
    }
    const own = (symbolsOf.get(fileId) ?? []).find((s) => s.name === local && s.kind !== "alias");
    if (own) return { symbol: own };
    // Dart: `import 'x.dart'` brings in every public name of x (and of what x exports).
    if (files.get(fileId)?.lang === "dart") {
      for (const imp of importsOf.get(fileId) ?? []) {
        if (imp.kind !== "import" || imp.local || !imp.target_file_id) continue;
        const def = definition(imp.target_file_id, local);
        if (def) return { symbol: def, file: files.get(imp.target_file_id) };
      }
    }
    return null;
  }

  // Reverse index built once per query: symbol id → import bindings that point at it.
  let bindingIndex = null;
  let namespaceImports = null;
  function buildIndex() {
    bindingIndex = new Map();
    namespaceImports = [];
    for (const [fileId, imps] of importsOf) {
      for (const imp of imps) {
        if (imp.kind !== "import" || !imp.target_file_id || !imp.local) continue;
        if (imp.imported === "*") { namespaceImports.push({ fileId, imp }); continue; }
        const def = definition(imp.target_file_id, imp.imported);
        if (!def) continue;
        if (!bindingIndex.has(def.id)) bindingIndex.set(def.id, []);
        bindingIndex.get(def.id).push({ fileId, local: imp.local });
      }
    }
    // Dart files have no named imports: resolve each referenced name through the file's imports.
    for (const [fileId, refs] of refsOf) {
      if (files.get(fileId)?.lang !== "dart") continue;
      const seen = new Set();
      for (const r of refs) {
        if (seen.has(r.local)) continue;
        seen.add(r.local);
        const b = bindingTarget(fileId, r.local);
        if (!b?.symbol || b.symbol.file_id === fileId) continue;
        if (!bindingIndex.has(b.symbol.id)) bindingIndex.set(b.symbol.id, []);
        bindingIndex.get(b.symbol.id).push({ fileId, local: r.local });
      }
    }
  }

  // Every place that uses a symbol: same-file refs and refs through any import (incl. barrels and namespaces).
  function referencesTo(sym) {
    if (!bindingIndex) buildIndex();
    const out = [];
    const push = (fileId, local, member) => {
      for (const r of refsOf.get(fileId) ?? []) {
        if (r.local !== local) continue;
        if (member !== undefined && r.member !== member && !(r.member ?? "").startsWith(`${member}.`)) continue;
        if (r.symbol_id === sym.id) continue;
        out.push({ ...r, file: files.get(fileId), owner: symbolById.get(r.symbol_id) ?? null });
      }
    };
    push(sym.file_id, sym.name);
    for (const b of bindingIndex.get(sym.id) ?? []) push(b.fileId, b.local);
    for (const { fileId, imp } of namespaceImports) {
      if (definition(imp.target_file_id, sym.name) === sym) push(fileId, imp.local, sym.name);
    }
    return out;
  }

  // Resolve a user-supplied target to symbols, a file or routes.
  function lookup(target, { routesFirst = false } = {}) {
    if (target.startsWith("/") && !target.includes(".")) {
      const rs = routes.filter((r) => r.path === target || routeMatches(r.path, target));
      if (rs.length) return { routes: rs };
    }
    if (routesFirst) {
      const screens = routes.filter((r) => r.framework === "flutter" && r.path === target);
      if (screens.length) return { routes: screens };
    }
    const [pathPart, symPart] = target.split("#");
    const file = fileByPath(pathPart);
    if (file && symPart) {
      const s = (symbolsOf.get(file.id) ?? []).filter((x) => x.name === symPart || (symPart === "default" && x.is_default));
      return s.length ? { symbols: s } : { error: `No symbol "${symPart}" in ${file.path}` };
    }
    if (file) return { file };
    const exact = [...symbolById.values()].filter((s) => s.name === target && s.kind !== "alias");
    if (exact.length) return { symbols: exact };
    return { error: `Nothing named "${target}". Try find ${target}.` };
  }

  function fileByPath(p) {
    if (!p) return null;
    const norm = p.replaceAll("\\", "/").replace(/^\.\//, "");
    if (byPath.has(norm)) return byPath.get(norm);
    const hits = [...byPath.values()].filter((f) => f.path.endsWith(`/${norm}`));
    return hits.length === 1 ? hits[0] : null;
  }

  // Dart classes that extend a project widget (class Card extends AppCard) are widgets too.
  for (let pass = 0; pass < 3; pass++) {
    for (const s of symbolById.values()) {
      if (s.kind !== "class" || !s.props) continue;
      const ext = JSON.parse(s.props).extends;
      const base = ext && bindingTarget(s.file_id, ext)?.symbol;
      if (base?.kind === "widget") s.kind = "widget";
    }
  }

  // Anything rendered as <Name /> is a component, even when its own body has no JSX (e.g. it returns useRender()).
  for (const [fileId, refs] of refsOf) {
    for (const r of refs) {
      if (r.kind !== "jsx") continue;
      const b = bindingTarget(fileId, r.local);
      if (b?.symbol && !r.member && (b.symbol.kind === "function" || b.symbol.kind === "const")) b.symbol.kind = "component";
    }
  }

  return { db, files, symbolsOf, symbolById, importsOf, refsOf, routes, definition, bindingTarget, referencesTo, lookup, fileByPath };
}

function groupBy(rows, key) {
  const m = new Map();
  for (const r of rows) {
    if (!m.has(r[key])) m.set(r[key], []);
    m.get(r[key]).push(r);
  }
  return m;
}

// ---------- formatting ----------

const where = (s) => `${s.file.path}:${s.line}`;
const label = (s) => `${s.name === "default" ? `${s.file.path} (default export)` : s.name} [${s.kind}${s.exported ? "" : ", private"}]`;
const more = (n, shown) => (n > shown ? [`… ${n - shown} more (raise limit)`] : []);
function routesOfFile(g, fileId) {
  return g.routes.filter((r) => r.file_id === fileId).map((r) => `${r.kind} ${r.path}`);
}
function ambiguous(syms) {
  return [`"${syms[0].name}" is declared in ${syms.length} places — pick one with path#Name:`, ...syms.map((s) => `  ${s.file.path}#${s.name} [${s.kind}] :${s.line}`)].join("\n");
}

// ---------- data model (tables, columns, who reads/writes them) ----------

function dataModel(g) {
  if (g.dataModelCache !== undefined) return g.dataModelCache;
  try {
    g.dataModelCache = JSON.parse(g.db.prepare("SELECT value FROM meta WHERE key = 'data_model'").get()?.value ?? "null");
  } catch {
    g.dataModelCache = null;
  }
  return g.dataModelCache;
}

function dataRefs(g, entity) {
  const rows = entity
    ? g.db.prepare("SELECT * FROM data_refs WHERE entity = ? AND op != 'ddl'").all(entity)
    : g.db.prepare("SELECT * FROM data_refs WHERE op != 'ddl'").all();
  return rows.map((r) => ({ ...r, file: g.files.get(r.file_id), columns: r.columns ? JSON.parse(r.columns) : null, filters: r.filters ? JSON.parse(r.filters) : null }));
}

// Refs to an entity by its model name or its table name (Prisma User / users).
function dataRefsFor(g, ent) {
  return [...new Set([ent.name, ent.table].filter(Boolean))].flatMap((n) => dataRefs(g, n)).sort((a, b) => a.file.path.localeCompare(b.file.path) || a.line - b.line);
}

// The top-level symbol whose lines contain a data ref (the function/component doing the query).
function ownerAt(g, fileId, line) {
  return (g.symbolsOf.get(fileId) ?? []).filter((s) => s.kind !== "alias" && s.line <= line && (s.end_line ?? s.line) >= line)
    .sort((a, b) => (a.end_line - a.line) - (b.end_line - b.line))[0] ?? null;
}

const colFlags = (c) => [c.db && `column ${c.db}`, c.pk && "PK", c.notNull && !c.pk && "not null", c.unique && "unique", c.ref && `→ ${c.ref.table}.${c.ref.column}`, c.default && `= ${c.default}`].filter(Boolean).join(", ");

function dataOverview(g, limit) {
  const dm = dataModel(g);
  if (!dm?.stores?.length) return "No data model found. Read: Supabase migrations, schema.prisma, .sql files, and CREATE/ALTER TABLE strings in TS/JS/Dart code (tests excluded). Not read: Drizzle, Mongo/NoSQL, Go struct ORMs.";
  const refs = dataRefs(g);
  const lines = [];
  for (const st of dm.stores) {
    const ents = Object.values(st.entities);
    lines.push(`# ${st.label} — from ${st.sources.length} source file(s)${st.unparsed.length ? ` · ${st.unparsed.length} statement(s) not understood` : ""}`);
    if (st.skipped?.length) lines.push(`Skipped database dumps (snapshots, not schema history): ${st.skipped.join(", ")}`);
    for (const u of st.unparsed.slice(0, 3)) lines.push(`Not understood: ${u.file}:${u.line} ${u.sql}`);
    const use = (e) => {
      const names = new Set([e.name, e.table].filter(Boolean));
      const rs = refs.filter((r) => names.has(r.entity));
      const files = (op) => new Set(rs.filter((r) => r.op === op).map((r) => r.file_id)).size;
      return rs.length ? ` · read in ${files("read")} file(s), written in ${files("write")}${files("call") ? `, called in ${files("call")}` : ""}` : " · not used in code";
    };
    const tables = ents.filter((x) => x.kind === "table" && !x.external);
    // Several schema files (e.g. a control DB and a per-tenant DB): group tables by the file that creates them.
    const groups = new Map();
    for (const e of tables) {
      const k = st.kind === "supabase" ? "all" : e.source?.file ?? "(never created in a schema file)";
      if (!groups.has(k)) groups.set(k, []);
      groups.get(k).push(e);
    }
    let shown = 0;
    for (const [file, list] of groups) {
      lines.push("", groups.size > 1 ? `## Tables in ${file} (${list.length})` : "## Tables");
      for (const e of list) {
        if (shown++ >= limit) break;
        const rls = st.kind !== "supabase" ? "" : e.outsideMigrations ? (e.rls ? ", RLS on" : ", RLS unknown") : e.rls ? `, RLS ${e.policies.length} policies` : ", NO RLS";
        lines.push(`${e.name} (${e.columns.length} cols${rls})${use(e)}${e.outsideMigrations ? " · ⚠ never created in a migration/schema file — columns incomplete" : ""}${e.unknownColumns ? ` · ⚠ indexes columns not defined: ${e.unknownColumns.join(", ")}` : ""}`);
      }
    }
    lines.push(...more(tables.length, limit));
    const views = ents.filter((x) => x.kind === "view");
    if (views.length) lines.push("", "## Views", ...views.map((v) => `${v.name}${use(v)}`));
    const fns = ents.filter((x) => x.kind === "function");
    if (fns.length) {
      const called = new Set(refs.filter((r) => r.op === "call").map((r) => r.entity));
      lines.push("", `## Functions (${fns.length}) — called from code: ${fns.filter((f) => called.has(f.name)).map((f) => f.name).join(", ") || "none"}`);
    }
    const ext = ents.filter((x) => x.external).map((x) => x.name);
    if (ext.length) lines.push("", `Platform tables referenced: ${ext.join(", ")}`);
  }
  const known = new Set(dm.stores.flatMap((st) => Object.values(st.entities).flatMap((e) => [e.name, e.table].filter(Boolean))));
  // Prisma-looking calls on other objects (stripe.customers.create) are not tables: only real models count.
  const unknown = [...new Set(refs.filter((r) => !known.has(r.entity) && r.form !== "prisma").map((r) => r.entity))];
  if (unknown.length) lines.push("", `## Used in code but not in any schema source (${unknown.length}) — made outside migrations, misspelled, a CTE/alias, or created elsewhere`, unknown.join(", "));
  return lines.join("\n");
}

function tableInfo(g, name, limit) {
  const hit = findEntity(dataModel(g), name.replace(/^table:/, ""));
  if (!hit) return `No table/view/function "${name}" in the data model. Try: code_graph data`;
  const { ent } = hit;
  const origin = ent.source ? ` — created ${ent.source.file}:${ent.source.line}` : ent.outsideMigrations ? " — ⚠ created outside migrations (dashboard?); only later changes are in code, so columns/RLS below are incomplete" : " (platform table, not created by this project)";
  const lines = [`${ent.kind} ${ent.name}${origin}${ent.changedIn?.length ? `, changed in ${ent.changedIn.join(", ")}` : ""}`];
  if (ent.columns.length) lines.push("", "## Columns", ...ent.columns.map((c) => `${c.name}  ${c.type}${colFlags(c) ? `  (${colFlags(c)})` : ""}`));
  if (ent.unknownColumns) lines.push(`⚠ Indexed but not defined in any migration: ${ent.unknownColumns.join(", ")}`);
  if (ent.indexes.length) lines.push("", "## Indexes", ...ent.indexes.map((i) => `${i.unique ? "unique " : ""}${i.name ?? "(inline)"} (${i.columns.join(", ")})`));
  const incoming = Object.values(hit.store.entities).flatMap((o) => o.fks.filter((f) => f.table === ent.name).map((f) => `${o.name}.${f.columns.join(",")}`));
  if (ent.fks.length || incoming.length) lines.push("", "## Relations", ...ent.fks.map((f) => `→ ${f.columns.join(",")} references ${f.table}.${f.refColumns.join(",")}`), ...incoming.map((x) => `← ${x}`));
  if (ent.kind === "table" && !ent.external && hit.store.kind === "supabase") {
    if (ent.rls) lines.push("", `## RLS on — ${ent.policies.length} policies${ent.policies.length ? "" : " (none: only the service role can access it)"}`);
    else if (ent.outsideMigrations) lines.push("", "## RLS unknown — not enabled in migrations; check the dashboard and add it to a migration");
    else lines.push("", "## RLS OFF — anyone with the anon key can read/write it");
  }
  for (const p of ent.policies) lines.push(`${p.name} [${p.cmd}${p.roles ? ` to ${p.roles}` : ""}]${p.using ? ` using (${p.using.slice(0, 100)})` : ""}${p.check ? ` check (${p.check.slice(0, 80)})` : ""}`);
  if (ent.reads?.length) lines.push("", `Reads: ${ent.reads.join(", ")}`);
  const refs = dataRefsFor(g, ent);
  if (refs.length) {
    lines.push("", `## Used in code (${refs.length})`);
    for (const r of refs.slice(0, limit)) {
      const owner = ownerAt(g, r.file_id, r.line);
      lines.push(`${r.op.padEnd(5)} ${r.detail}${r.columns ? ` [${r.columns.join(", ")}]` : ""}${r.filters ? ` by ${r.filters.join(", ")}` : ""}${r.via ? ` (embedded from ${r.via})` : ""}  ${r.file.path}:${r.line}${owner ? ` in ${owner.name}` : ""}`);
    }
    lines.push(...more(refs.length, limit));
    // Filters on columns without an index: candidates for one (see backend-performance).
    const indexed = new Set(ent.indexes.map((i) => i.columns[0]).concat(ent.columns.filter((c) => c.pk || c.unique).map((c) => c.name)));
    const unindexed = [...new Set(refs.flatMap((r) => r.filters ?? []).filter((c) => !indexed.has(c) && c !== "order"))];
    if (unindexed.length && ent.kind === "table") lines.push("", `Filtered on without a leading index: ${unindexed.join(", ")}`);
  } else lines.push("", "Not used in code.");
  return lines.join("\n");
}

// "orders" or "orders.status" → every query that touches it, then who calls those functions.
function dataImpact(g, ent, column, depth, cap) {
  let refs = dataRefsFor(g, ent);
  if (column) refs = refs.filter((r) => !r.columns || r.columns.includes("*") || r.columns.includes(column) || r.filters?.includes(column));
  const lines = [`Impact of changing ${ent.name}${column ? `.${column}` : ""}:`];
  const store = findEntity(dataModel(g), ent.name).store;
  const related = Object.values(store.entities).flatMap((o) => [
    ...o.fks.filter((f) => f.table === ent.name && (!column || f.refColumns.includes(column))).map((f) => `FK ${o.name}.${f.columns.join(",")}`),
    ...((o.reads ?? []).includes(ent.name) ? [`${o.kind} ${o.name}`] : []),
  ]);
  const policies = ent.policies.filter((p) => !column || `${p.using ?? ""} ${p.check ?? ""}`.includes(column)).map((p) => `policy ${p.name}`);
  if (related.length || policies.length) lines.push(`## In the database`, [...related, ...policies].join(" · "));
  if (!refs.length) { lines.push("No code touches it (columns unknown where a query uses a variable)."); return lines.join("\n"); }
  lines.push(`## Queries (${refs.length})`);
  const owners = new Map();
  for (const r of refs.slice(0, cap)) {
    const owner = ownerAt(g, r.file_id, r.line);
    const rs = routesOfFile(g, r.file_id);
    lines.push(`${r.op} ${r.file.path}:${r.line}${owner ? ` in ${owner.name}` : ""}${r.columns?.includes("*") && column ? " (select *)" : ""}${rs.length ? `  ⚑ ${rs.join(", ")}` : ""}`);
    if (owner) owners.set(owner.id, owner);
  }
  // Callers of the functions/components that run those queries.
  const seen = new Set(owners.keys());
  let frontier = [...owners.values()];
  for (let level = 1; level < depth && frontier.length; level++) {
    const next = [];
    const shown = [];
    for (const sym of frontier) {
      for (const r of g.referencesTo(sym)) {
        const key = r.owner ? r.owner.id : `m${r.file_id}`;
        if (seen.has(key)) continue;
        seen.add(key);
        const rs = routesOfFile(g, r.file_id);
        shown.push(`${r.owner ? r.owner.name : `${r.file.path} (module)`} ${r.file.path}:${r.line} ← ${sym.name}${rs.length ? `  ⚑ ${rs.join(", ")}` : ""}`);
        if (r.owner) next.push(r.owner);
      }
    }
    if (shown.length) lines.push(`## Callers, level ${level} (${shown.length})`, ...shown.slice(0, cap));
    frontier = next;
  }
  return lines.join("\n");
}

function dataTarget(g, target) {
  const dm = dataModel(g);
  if (!dm) return null;
  const t = target.replace(/^table:/, "");
  const whole = findEntity(dm, t);
  if (whole) return { ent: whole.ent, column: null, explicit: target.startsWith("table:") };
  const dot = t.lastIndexOf(".");
  if (dot > 0) {
    const hit = findEntity(dm, t.slice(0, dot));
    if (hit) return { ent: hit.ent, column: t.slice(dot + 1), explicit: true };
  }
  return null;
}

// ---------- queries ----------

function overview(g, limit) {
  const files = [...g.files.values()];
  const byLang = countBy(files, (f) => f.lang);
  const syms = [...g.symbolById.values()];
  const byKind = countBy(syms, (s) => s.kind);
  const lines = [
    `Files: ${files.length} (${fmtCounts(byLang)}) · Symbols: ${fmtCounts(byKind)}`,
    `Updated: ${g.db.prepare("SELECT value FROM meta WHERE key = 'updated_at'").get()?.value ?? "never"}`,
  ];
  // Usage counts for components/hooks: how many places reference each.
  const used = new Map();
  for (const s of syms) if (s.kind === "component" || s.kind === "hook" || s.kind === "widget") used.set(s, 0);
  for (const s of used.keys()) used.set(s, new Set(g.referencesTo(s).map((r) => r.file_id)).size);
  const top = [...used].filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1]).slice(0, 15);
  if (top.length) lines.push("", "## Most used components/hooks (files using them)", top.map(([s, n]) => `${s.name} ${n}`).join(" · "));
  if (g.routes.length) {
    lines.push("", `## Routes (${g.routes.length})`);
    for (const r of g.routes.slice(0, limit)) lines.push(`${r.kind.padEnd(8)} ${r.path}  → ${r.file.path}`);
    lines.push(...more(g.routes.length, limit));
  }
  const routeFiles = new Set(g.routes.map((r) => r.file_id));
  const unused = [...used].filter(([s, n]) => n === 0 && s.exported && !routeFiles.has(s.file_id)).map(([s]) => `${s.name} (${s.file.path})`);
  if (unused.length) lines.push("", `## Exported components/hooks used nowhere (${unused.length})`, unused.slice(0, 20).join(" · "));
  const ext = countBy(
    [...g.importsOf.values()].flat().filter((i) => i.external),
    (i) => i.external,
  );
  const topExt = Object.entries(ext).sort((a, b) => b[1] - a[1]).slice(0, 15);
  if (topExt.length) lines.push("", "## Packages (imports)", topExt.map(([k, v]) => `${k} ${v}`).join(" · "));
  const dm = dataModel(g);
  for (const st of dm?.stores ?? []) {
    const ents = Object.values(st.entities).filter((e) => !e.external);
    lines.push("", `## Data: ${st.label} — ${ents.filter((e) => e.kind === "table").length} tables, ${ents.filter((e) => e.kind === "view").length} views, ${ents.filter((e) => e.kind === "function").length} functions (code_graph data / table <name>)`);
  }
  const unresolved = [...g.importsOf.values()].flat().filter((i) => !i.target_file_id && !i.external);
  if (unresolved.length) {
    lines.push("", `## Unresolved imports (${unresolved.length}) — usually generated or ignored files`);
    lines.push(...unresolved.slice(0, 10).map((i) => `${g.files.get(i.file_id).path}: "${i.source}"`));
  }
  const ns = allNotes(g.db, g);
  const dirNotes = ns.filter((n) => n.kind === "dir" && n.target.split("/").length <= 2);
  lines.push("", `## Notes (${ns.length}${ns.some((n) => n.status !== "ok") ? `, ${ns.filter((n) => n.status !== "ok").length} stale` : ""}) — see the notes query`);
  for (const n of dirNotes.slice(0, 12)) lines.push(`${n.label}: ${n.text.split("\n")[0].slice(0, 140)}${n.status !== "ok" ? " (stale)" : ""}`);
  const errors = files.filter((f) => f.parse_error);
  if (errors.length) lines.push("", "## Parse errors", ...errors.map((f) => `${f.path}: ${f.parse_error}`));
  return lines.join("\n");
}

function find(g, q, limit) {
  const needle = q.toLowerCase();
  const syms = [...g.symbolById.values()].filter((s) => s.name.toLowerCase().includes(needle) && s.kind !== "alias");
  syms.sort((a, b) => (a.name.toLowerCase() === needle ? -1 : 0) - (b.name.toLowerCase() === needle ? -1 : 0) || b.exported - a.exported);
  const files = [...g.files.values()].filter((f) => f.path.toLowerCase().includes(needle));
  const routes = g.routes.filter((r) => r.path.toLowerCase().includes(needle));
  const lines = [];
  if (syms.length) lines.push(`## Symbols (${syms.length})`, ...syms.slice(0, limit).map((s) => `${label(s)}  ${where(s)}`), ...more(syms.length, limit));
  if (files.length) lines.push(`## Files (${files.length})`, ...files.slice(0, limit).map((f) => f.path), ...more(files.length, limit));
  if (routes.length) lines.push(`## Routes (${routes.length})`, ...routes.slice(0, limit).map((r) => `${r.kind} ${r.path} → ${r.file.path}`));
  // Data: which table/column already holds this — check before adding a table or column.
  const ents = (dataModel(g)?.stores ?? []).flatMap((st) => Object.values(st.entities).filter((e) => !e.external));
  const tables = ents.filter((e) => e.name.toLowerCase().includes(needle) || e.table?.toLowerCase().includes(needle));
  const cols = ents.flatMap((e) => e.columns.filter((c) => c.name.toLowerCase().includes(needle) || c.db?.toLowerCase().includes(needle)).map((c) => ({ e, c })));
  if (tables.length) lines.push(`## Tables (${tables.length}) — code_graph table <name> for details`, ...tables.slice(0, limit).map((e) => `${e.kind} ${e.name}${e.table && e.table !== e.name ? ` (table ${e.table})` : ""} · ${e.columns.length} cols${e.source ? ` · ${e.source.file}:${e.source.line}` : ""}`));
  if (cols.length) lines.push(`## Columns (${cols.length})`, ...cols.slice(0, limit).map(({ e, c }) => `${e.name}.${c.name}  ${c.type}${colFlags(c) ? `  (${colFlags(c)})` : ""}`), ...more(cols.length, limit));
  return lines.join("\n") || `Nothing matches "${q}".`;
}

function fileInfo(g, target) {
  const f = g.fileByPath(target.split("#")[0]);
  if (!f) return `No file "${target}" in the graph (paths are relative to the project root).`;
  const lines = [`${f.path} · ${f.lang}${f.directive ? ` · "${f.directive}"` : ""}${f.parse_error ? ` · parse error: ${f.parse_error}` : ""}`];
  const rs = routesOfFile(g, f.id);
  if (rs.length) lines.push(`Routes: ${rs.join(", ")}`);
  lines.push(...noteLines(notesFor(g.db, g, f)));
  const syms = (g.symbolsOf.get(f.id) ?? []).filter((s) => s.kind !== "alias");
  if (syms.length) {
    lines.push("", "## Declares");
    for (const s of syms) {
      const props = s.props ? JSON.parse(s.props) : {};
      const extra = [
        props.names?.length ? `props: ${props.names.join(", ")}` : "",
        props.type ? `props type: ${props.type}` : "",
        props.extends ? `extends ${props.extends}` : "",
        props.statics?.length ? `${props.statics.length} static values` : "",
      ].filter(Boolean).join(" · ");
      lines.push(`${label(s)}${s.is_default ? " default" : ""} :${s.line}-${s.end_line}${extra ? `  ${extra}` : ""}`);
    }
  }
  const imps = g.importsOf.get(f.id) ?? [];
  if (imps.length) {
    lines.push("", "## Imports");
    const groups = groupBy(imps, "source");
    for (const [source, list] of groups) {
      const i = list[0];
      const to = i.target_file_id ? g.files.get(i.target_file_id).path : i.external ? `pkg ${i.external}` : "unresolved";
      const names = list.map((x) => (x.kind === "reexport_all" ? "* (re-export)" : x.imported === x.local || !x.local ? x.imported : `${x.imported} as ${x.local}`)).filter(Boolean);
      lines.push(`${source} → ${to}${names.length ? `: ${names.join(", ")}` : ""}${list.some((x) => x.kind.startsWith("reexport")) ? " (re-export)" : ""}`);
    }
  }
  const importers = new Set([...g.importsOf.values()].flat().filter((i) => i.target_file_id === f.id).map((i) => g.files.get(i.file_id).path));
  lines.push("", `## Imported by (${importers.size})`, ...[...importers].slice(0, 30));
  return lines.join("\n");
}

function symbolsFor(g, target) {
  const hit = g.lookup(target);
  if (hit.error) return { text: hit.error };
  if (hit.routes) return { text: `"${target}" is a route — use the route query.` };
  if (hit.file) return { file: hit.file };
  if (hit.symbols.length > 1) return { text: ambiguous(hit.symbols) };
  return { symbol: hit.symbols[0] };
}

function usedBy(g, target, limit) {
  const t = symbolsFor(g, target);
  if (t.text) return t.text;
  if (t.file) {
    const imps = [...g.importsOf.values()].flat().filter((i) => i.target_file_id === t.file.id);
    if (!imps.length) return `${t.file.path} is not imported anywhere.`;
    const byFile = groupBy(imps, "file_id");
    return [`${t.file.path} is imported by ${byFile.size} file(s):`, ...[...byFile].slice(0, limit).map(([id, list]) => `${g.files.get(id).path}: ${list.map((i) => i.imported ?? "(side effect)").join(", ")}`)].join("\n");
  }
  const s = t.symbol;
  const refs = g.referencesTo(s);
  const sn = symbolNote(g, s);
  if (sn) return [noteLines([sn])[0], usedByBody(g, s, refs, limit)].join("\n");
  return usedByBody(g, s, refs, limit);
}

function usedByBody(g, s, refs, limit) {
  if (!refs.length) return `${label(s)} ${where(s)} — not used anywhere${s.exported ? " (exported but unused)" : ""}.`;
  const files = new Set(refs.map((r) => r.file_id));
  const lines = [`${label(s)} ${where(s)} — used in ${refs.length} place(s) across ${files.size} file(s):`];
  for (const r of refs.slice(0, limit)) {
    const owner = r.owner ? (r.owner.name === "default" ? "default export" : r.owner.name) : "module";
    const rs = routesOfFile(g, r.file_id);
    lines.push(`${r.file.path}:${r.line}  in ${owner}  ${r.kind}${r.count > 1 ? ` ×${r.count}` : ""}${rs.length ? `  (${rs.join(", ")})` : ""}`);
  }
  lines.push(...more(refs.length, limit));
  return lines.join("\n");
}

function uses(g, target, limit) {
  const t = symbolsFor(g, target);
  if (t.text) return t.text;
  if (t.file) return fileInfo(g, t.file.path);
  const s = t.symbol;
  const sn = symbolNote(g, s);
  const refs = (g.refsOf.get(s.file_id) ?? []).filter((r) => r.symbol_id === s.id);
  if (sn && refs.length) return [noteLines([sn])[0], usesBody(g, s, refs, limit)].join("\n");
  return usesBody(g, s, refs, limit);
}

function usesBody(g, s, refs, limit) {
  if (!refs.length) return `${label(s)} ${where(s)} uses nothing from this project or packages.`;
  const lines = [`${label(s)} ${where(s)} uses:`];
  for (const r of refs.slice(0, limit)) lines.push(`${r.kind.padEnd(4)} ${describe(g, s.file_id, r)}${r.count > 1 ? ` ×${r.count}` : ""}`);
  lines.push(...more(refs.length, limit));
  return lines.join("\n");
}

function describe(g, fileId, r) {
  const name = r.member ? `${r.local}.${r.member}` : r.local;
  const b = g.bindingTarget(fileId, r.local);
  if (!b) return name;
  if (b.external) return `${name} ← pkg ${b.external}`;
  if (b.unresolved) return `${name} ← unresolved "${b.unresolved}"`;
  if (b.namespace) return `${name} ← ${b.file.path}`;
  if (b.symbol) return `${name} ← ${b.symbol.name === name ? "" : `${b.symbol.name} `}[${b.symbol.kind}] ${where(b.symbol)}`;
  return `${name} ← ${b.file?.path ?? "?"} (not found there)`;
}

// Everything that would be affected by changing the target, level by level (reverse dependencies).
function impact(g, target, depth, cap) {
  // A table / table.column (or "table:x") goes to the data impact, unless a code symbol has that exact name.
  const data = dataTarget(g, target);
  if (data && (data.explicit || !g.lookup?.(target)?.symbols?.length)) return dataImpact(g, data.ent, data.column, depth, cap);
  const t = symbolsFor(g, target);
  if (t.text) return t.text;
  const start = t.symbol ?? null;
  const lines = [];
  const seen = new Set();
  let frontier = start ? [{ sym: start }] : (g.symbolsOf.get(t.file.id) ?? []).filter((s) => s.exported && s.kind !== "alias").map((sym) => ({ sym }));
  lines.push(`Impact of changing ${start ? `${label(start)} ${where(start)}` : t.file.path} (${depth} level${depth > 1 ? "s" : ""}):`);
  frontier.forEach((f) => seen.add(f.sym.id));
  let count = 0;
  for (let level = 1; level <= depth && frontier.length; level++) {
    const next = [];
    const shown = [];
    for (const { sym } of frontier) {
      for (const r of g.referencesTo(sym)) {
        const owner = r.owner;
        const key = owner ? owner.id : `m${r.file_id}`;
        if (seen.has(key)) continue;
        seen.add(key);
        const rs = routesOfFile(g, r.file_id);
        shown.push(`${owner ? (owner.name === "default" ? `${r.file.path} (default export)` : `${owner.name} [${owner.kind}]`) : `${r.file.path} (module code)`} ${r.file.path}:${r.line}  ← ${sym.name}${rs.length ? `  ⚑ ${rs.join(", ")}` : ""}`);
        if (owner) next.push({ sym: owner });
      }
    }
    if (shown.length) {
      lines.push(`## Level ${level} (${shown.length})`, ...shown.slice(0, Math.max(0, cap - count)));
      count += shown.length;
      if (count >= cap) { lines.push(`… stopped at ${cap} items`); break; }
    }
    frontier = next;
  }
  if (lines.length === 1) lines.push("Nothing depends on it.");
  return lines.join("\n");
}

// A route: its files (page + layouts…) and the component tree its page renders.
function route(g, url, depth) {
  // Flutter screens are named by widget ("TaxScreen"); web routes are paths ("/jobs/[id]").
  let hit = g.lookup(url, { routesFirst: true });
  if (!hit.routes && !url.startsWith("/")) hit = g.lookup(`/${url}`, { routesFirst: true });
  if (!hit.routes) return `No route matches "${url}". Routes: ${g.routes.map((r) => r.path).filter((p, i, a) => a.indexOf(p) === i).slice(0, 30).join(", ")}`;
  const lines = [];
  for (const r of hit.routes.filter((x) => x.kind !== "layout")) {
    lines.push(`## ${r.kind} ${r.path} → ${r.file.path}${r.file.directive ? ` ("${r.file.directive}")` : ""}`);
    // Layouts wrapping this route: same router root, path is a prefix.
    // Layouts wrap pages only; API route handlers have none.
    const layouts = r.kind === "route" ? [] : g.routes.filter((l) => l.kind === "layout" && l.root === r.root && (l.path === "/" || r.path === l.path || r.path.startsWith(`${l.path}/`)));
    if (layouts.length) lines.push(`Layouts: ${layouts.map((l) => l.file.path).join(" → ")}`);
    const syms = g.symbolsOf.get(r.file_id) ?? [];
    const entry = r.framework === "flutter" ? syms.find((s) => s.name === r.root) : syms.find((s) => s.is_default) ?? syms.find((s) => s.exported);
    if (!entry) continue;
    if (r.kind === "route") {
      const handlers = (g.symbolsOf.get(r.file_id) ?? []).filter((s) => s.exported && /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)$/.test(s.name));
      for (const h of handlers) {
        lines.push(`${h.name}:`);
        for (const ref of (g.refsOf.get(r.file_id) ?? []).filter((x) => x.symbol_id === h.id)) lines.push(`  ${describe(g, r.file_id, ref)}`);
      }
      continue;
    }
    lines.push(r.framework === "flutter" ? `Widget: ${entry.name} ${where(entry)}` : "", "Renders:");
    const dart = r.file.lang === "dart";
    const seen = new Set();
    const walk = (sym, indent, level) => {
      for (const ref of (g.refsOf.get(sym.file_id) ?? []).filter((x) => x.symbol_id === sym.id && x.kind === "jsx")) {
        const b = g.bindingTarget(sym.file_id, ref.local);
        const name = ref.member ? `${ref.local}.${ref.member}` : ref.local;
        // Flutter builds are mostly SDK widgets (Column, Text…); only project widgets are worth listing.
        if (dart && (!b?.symbol || b.symbol.kind !== "widget")) continue;
        if (b?.symbol) {
          const again = seen.has(b.symbol.id);
          lines.push(`${indent}${name}  ${where(b.symbol)}${again ? " (see above)" : ""}`);
          if (!again && level < depth) { seen.add(b.symbol.id); walk(b.symbol, `${indent}  `, level + 1); }
        } else lines.push(`${indent}${name}${b?.external ? `  (pkg ${b.external})` : ""}`);
      }
    };
    walk(entry, "  ", 1);
    // Screens and overlays this one opens (Navigator.push, showDialog …)
    const opens = (g.refsOf.get(entry.file_id) ?? []).filter((x) => x.symbol_id === entry.id && (x.kind === "nav" || x.kind === "overlay"));
    if (opens.length) lines.push(`Opens: ${opens.map((x) => `${x.local}${x.kind === "overlay" ? " (overlay)" : ""}`).join(", ")}`);
  }
  return lines.filter((l) => l !== "").join("\n");
}

function countBy(items, fn) {
  const out = {};
  for (const x of items) out[fn(x)] = (out[fn(x)] ?? 0) + 1;
  return out;
}
const fmtCounts = (o) => Object.entries(o).sort((a, b) => b[1] - a[1]).map(([k, v]) => `${k} ${v}`).join(", ");

// ---------- design link ----------

function design(g, root, limit) {
  const snap = designSnapshot(g.db);
  const { linked, suggested, figmaOnly, codeOnly } = componentLinks(g, root, snap);
  const lines = [snap ? `Figma "${snap.file}" · ${snap.components.length} components · ${snap.tokens.length} tokens · synced ${snap.fetchedAt}` : "No Figma snapshot yet — run code_design_sync with the Figma plugin open (component names below come from wara.components.json only)."];
  lines.push("", `## Linked in wara.components.json (${linked.length})`);
  for (const l of linked.slice(0, limit)) {
    if (l.classes) {
      const variants = Object.entries(l.entry.variants ?? {}).map(([k, v]) => `${k}: ${v}`).join(", ");
      lines.push(`${l.figmaName} → <${l.entry.element ?? "div"} class="${l.entry.className}">${variants ? ` + {${variants}}` : ""} · used in ${l.classes.files} files${l.classes.missing.length ? ` · NOT DEFINED in CSS: ${l.classes.missing.map((n) => `.${n}`).join(" ")}` : ""}${snap && !l.figma ? " · not in Figma anymore" : ""}`);
      continue;
    }
    lines.push(`${l.figmaName} → ${l.symbol ? `${l.symbol.name} ${where(l.symbol)} · used in ${new Set(g.referencesTo(l.symbol).map((r) => r.file_id)).size} files` : `${l.entry.component} from "${l.entry.import}" — NOT FOUND in code`}${snap && !l.figma ? " · not in Figma anymore" : ""}`);
  }
  if (suggested.length) {
    lines.push("", `## Suggested pairs by name (${suggested.length}) — confirm with map_component`);
    for (const x of suggested.slice(0, limit)) lines.push(`${x.figma.name} (${x.figma.id}) ≈ ${x.symbol.name} ${where(x.symbol)}${x.exact ? "" : " (partial name match)"}`);
  }
  if (figmaOnly.length) lines.push("", `## In Figma, no code component found (${figmaOnly.length})`, figmaOnly.slice(0, limit).map((c) => c.name).join(" · "));
  if (codeOnly.length) lines.push("", `## Code components with no Figma counterpart (${codeOnly.length})`, codeOnly.slice(0, limit).map((s) => s.name).join(" · "));
  return lines.join("\n");
}

function tokens(g, target, limit) {
  const snap = designSnapshot(g.db);
  const { rows, hardcoded, hardFiles } = tokenTable(g.db, g, snap);
  if (target) {
    const r = rows.find((x) => x.name === target || x.figma?.token === target || x.name.endsWith(`-${target}`) || x.name.endsWith(`.${target}`));
    if (!r) return `No token "${target}". Try the tokens query without a target.`;
    const lines = [`${r.name}${r.figma ? ` · Figma "${r.figma.token}" = ${r.figma.value}` : " · not in Figma"}`];
    for (const d of r.defs) lines.push(`defined ${d.path}:${d.line} = ${d.value}`);
    if (r.mismatch) lines.push("⚠ code value differs from Figma");
    lines.push(`used ${r.count}× in ${r.files.size} file(s):`);
    const byFile = new Map();
    for (const u of r.uses) byFile.set(u.path, [...(byFile.get(u.path) ?? []), `${u.form === "tw" ? u.name : u.form}${u.count > 1 ? ` ×${u.count}` : ""} :${u.line}`]);
    for (const [p, list] of [...byFile].slice(0, limit)) lines.push(`  ${p}: ${list.join(", ")}`);
    return lines.join("\n");
  }
  const defined = rows.filter((r) => r.defs.length);
  const lines = [`Tokens: ${defined.length} defined in code${snap ? ` · ${snap.tokens.length} in Figma "${snap.file}"` : " · no Figma snapshot (run code_design_sync)"}`];
  const mism = rows.filter((r) => r.mismatch);
  if (mism.length) lines.push("", `## Value differs from Figma (${mism.length})`, ...mism.map((r) => `${r.name}: code ${r.defs[0].value} · Figma ${r.figma.value}`));
  const figmaOnly = rows.filter((r) => r.figma && !r.defs.length);
  if (figmaOnly.length) lines.push("", `## In Figma, not defined in code (${figmaOnly.length})`, figmaOnly.slice(0, limit).map((r) => `${r.figma.token}${r.name.startsWith("figma:") ? "" : ` (${r.name})`}`).join(" · "));
  const used = defined.filter((r) => r.count).sort((a, b) => b.count - a.count);
  lines.push("", `## Most used (${used.length})`, used.slice(0, limit).map((r) => `${r.name} ${r.count}×/${r.files.size}f`).join(" · "));
  const unused = defined.filter((r) => !r.count);
  if (unused.length) lines.push("", `## Defined but never used (${unused.length})`, unused.slice(0, limit).map((r) => r.name).join(" · "));
  if (hardcoded.length) {
    lines.push("", `## Raw colors in code instead of tokens (${hardcoded.reduce((s, h) => s + h.n, 0)})`, hardcoded.slice(0, 15).map((h) => `${h.name} ${h.n}×`).join(" · "));
    lines.push(`Files: ${hardFiles.slice(0, 10).map((f) => `${f.path} ${f.n}`).join(" · ")}`);
  }
  return lines.join("\n");
}

// ---------- notes ----------

function symbolNote(g, s) {
  return allNotes(g.db, g).find((n) => n.target === `symbol:${s.file.path}#${s.name}`) ?? null;
}

function noteLines(list) {
  return list.map((n) => `📝 ${n.kind === "dir" ? n.label : n.kind === "file" ? "file" : "note"}${n.status === "stale" ? " (STALE — code changed since written; verify and rewrite)" : ""}: ${n.text}`);
}

function notes(g, limit) {
  const ns = allNotes(g.db, g);
  const lines = [`Notes: ${ns.length} · ok ${ns.filter((n) => n.status === "ok").length} · stale ${ns.filter((n) => n.status === "stale").length} · orphan ${ns.filter((n) => n.status === "orphan").length}`];
  const bad = ns.filter((n) => n.status !== "ok");
  if (bad.length) {
    lines.push("", "## Needs rewriting (code changed / target gone)");
    for (const n of bad) lines.push(`[${n.status}] ${n.target}: ${n.text.slice(0, 160)}`);
  }
  const ok = ns.filter((n) => n.status === "ok");
  if (ok.length) {
    lines.push("", "## Current");
    for (const n of ok.slice(0, limit)) lines.push(`${n.target}: ${n.text.slice(0, 160)}`);
  }
  const miss = missingNotes(g.db, g, Math.min(limit, 15));
  if (miss.dirs.length || miss.symbols.length || miss.entries.length) {
    lines.push("", "## Worth a note (no note yet)");
    if (miss.dirs.length) lines.push(`Folders: ${miss.dirs.map((d) => `${d.target} (${d.files} files)`).join(" · ")}`);
    if (miss.entries.length) lines.push(`Entry points: ${miss.entries.map((e) => `${e.target} (${e.route})`).join(" · ")}`);
    if (miss.symbols.length) lines.push(`Widely used: ${miss.symbols.map((s) => `${s.target} (${s.usedInFiles} files)`).join(" · ")}`);
  }
  return lines.join("\n");
}
