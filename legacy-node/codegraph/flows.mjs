// Flows: how the project behaves, step by step, as small markdown files in docs/flows/ (committed).
// The file keeps only what the code graph can't know (names, order, actors, branches, waits, states, notes);
// the code path itself is traced live from the graph, so it never goes stale. Drafts for entry points
// that no flow covers yet are computed from the graph on demand (nothing stored).
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { findEntity } from "./data/model.mjs";
import { routeMatches } from "./routes-next.mjs";

export const FLOWS_DIR = "docs/flows";
const IGNORED_FILE = ".wara/flows-ignored.json";
const HTTP = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)$/;

// ---------- file format ----------
// # Flow: <name>
// entry: <free text>
// ## Steps
// 1. [user] text with `code refs`
//    - label → step 3 | → end
// 5a. [server] … (5a/5b run at the same time; 6 waits for both)
// ## States
// `table.column`: a → b | c · b → d (note)
// ## Notes
// free text

export const hashText = (text) => createHash("sha1").update(text).digest("hex").slice(0, 16);
const refsIn = (text) => [...String(text).matchAll(/`([^`]+)`/g)].map((m) => m[1]);

export function parseFlow(text) {
  const flow = { title: "", entry: "", steps: [], states: [], notes: "", extra: [] };
  let section = "head";
  let step = null;
  const notes = [];
  for (const raw of String(text).replace(/\r/g, "").split("\n")) {
    const line = raw.trimEnd();
    let m;
    if ((m = /^#\s+(?:flow:\s*)?(.+)$/i.exec(line)) && !flow.title) { flow.title = m[1].trim(); continue; }
    if ((m = /^##\s+(.+)$/.exec(line))) {
      const name = m[1].trim().toLowerCase();
      section = name.startsWith("step") ? "steps" : name.startsWith("state") ? "states" : name.startsWith("note") ? "notes" : "extra";
      if (section === "extra") flow.extra.push({ heading: m[1].trim(), lines: [] });
      step = null;
      continue;
    }
    if (section === "head") {
      if ((m = /^entry:\s*(.*)$/i.exec(line))) flow.entry = m[1].trim();
      continue;
    }
    if (section === "steps") {
      if ((m = /^\s*(\d+[a-z]?)[.)]\s*\[([^\]]+)\]\s*(.*)$/.exec(line))) {
        const actor = m[2].trim();
        const textPart = m[3].trim();
        step = {
          id: m[1],
          group: /[a-z]$/.test(m[1]) ? m[1].replace(/[a-z]$/, "") : null,
          actor: actor.replace(/^⏳\s*/, ""),
          text: textPart,
          wait: /⏳/.test(actor) || /^⏳/.test(textPart),
          refs: refsIn(textPart),
          branches: [],
        };
        flow.steps.push(step);
      } else if (step && (m = /^\s+-\s*(.+)$/.exec(line))) {
        const b = /^(.*?)\s*→\s*(end|step\s*(\d+[a-z]?))\s*$/i.exec(m[1]);
        step.branches.push(b ? { label: b[1].trim(), to: b[2].toLowerCase() === "end" ? "end" : b[3] } : { label: m[1].trim(), to: null });
      }
      continue;
    }
    if (section === "states") {
      if ((m = /^\s*-?\s*`?([\w.]+)`?\s*:\s*(.+)$/.exec(line))) flow.states.push(parseStates(m[1], m[2]));
      continue;
    }
    if (section === "notes") { notes.push(raw); continue; }
    flow.extra.at(-1)?.lines.push(raw);
  }
  flow.notes = notes.join("\n").trim();
  return flow;
}

function parseStates(column, text) {
  const transitions = [];
  const values = new Set();
  for (const chain of text.split("·")) {
    let note = null;
    const body = chain.replace(/\(([^)]*)\)\s*$/, (_, n) => ((note = n.trim()), "")).trim();
    const parts = body.split("→").map((p) => p.split("|").map((v) => v.trim()).filter(Boolean));
    parts.flat().forEach((v) => values.add(v));
    for (let i = 0; i < parts.length - 1; i++) {
      for (const from of parts[i]) for (const to of parts[i + 1]) transitions.push({ from, to, note: i === parts.length - 2 ? note : null });
    }
  }
  return { column, values: [...values], transitions };
}

/** Canonical markdown. Steps keep their order; ids are renumbered (5a/5b for a parallel group). */
export function formatFlow(flow) {
  const numbering = new Map();
  let n = 0;
  let lastGroup = null;
  let letter = 0;
  for (const s of flow.steps) {
    const group = s.group ?? null;
    if (group && group === lastGroup) numbering.set(s.id, `${n}${String.fromCharCode(97 + ++letter)}`);
    else if (group) { n += 1; letter = 0; numbering.set(s.id, `${n}a`); }
    else { n += 1; numbering.set(s.id, String(n)); }
    lastGroup = group;
  }
  const out = [`# Flow: ${flow.title || "Untitled"}`];
  if (flow.entry) out.push(`entry: ${flow.entry}`);
  out.push("", "## Steps");
  for (const s of flow.steps) {
    out.push(`${numbering.get(s.id)}. [${s.wait ? "⏳ " : ""}${s.actor || "server"}] ${s.text}`.trimEnd());
    for (const b of s.branches ?? []) {
      const to = b.to === "end" ? "→ end" : b.to && numbering.has(b.to) ? `→ step ${numbering.get(b.to)}` : "";
      out.push(`   - ${b.label}${to ? ` ${to}` : ""}`.trimEnd());
    }
  }
  if (flow.states?.length) {
    out.push("", "## States");
    for (const st of flow.states) {
      const chains = (st.transitions ?? []).map((t) => `${t.from} → ${t.to}${t.note ? ` (${t.note})` : ""}`);
      const lone = (st.values ?? []).filter((v) => !(st.transitions ?? []).some((t) => t.from === v || t.to === v));
      out.push(`\`${st.column}\`: ${[...chains, ...lone].join(" · ")}`);
    }
  }
  if (flow.notes?.trim()) out.push("", "## Notes", flow.notes.trim());
  for (const x of flow.extra ?? []) out.push("", `## ${x.heading}`, ...x.lines);
  return `${out.join("\n").trimEnd()}\n`;
}

// ---------- files ----------

const SAFE_FILE = /^[a-z0-9][a-z0-9._-]*\.md$/i;
export const slugify = (name) => String(name).toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 60) || `flow-${Date.now().toString(36)}`;

export function readFlowFiles(root) {
  const dir = join(root, FLOWS_DIR);
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((f) => SAFE_FILE.test(f)).sort().map((file) => {
    const text = readFileSync(join(dir, file), "utf8");
    return { file: `${FLOWS_DIR}/${file}`, hash: hashText(text), ...parseFlow(text) };
  });
}

export class FlowConflict extends Error {
  constructor(current) {
    super("The flow file changed since it was opened");
    this.current = current;
  }
}

/** Write docs/flows/<name>.md. baseHash = hash when opened (null = must not exist yet). */
export function saveFlowFile(root, name, text, baseHash) {
  const file = String(name).replace(/^docs\/flows\//, "");
  if (!SAFE_FILE.test(file)) throw new Error(`Invalid flow file name "${name}"`);
  const dir = join(root, FLOWS_DIR);
  const path = join(dir, file);
  if (existsSync(path)) {
    const current = readFileSync(path, "utf8");
    if (baseHash !== hashText(current)) throw new FlowConflict(current);
  } else if (baseHash) {
    throw new FlowConflict(null);
  }
  mkdirSync(dir, { recursive: true });
  writeFileSync(path, text);
  return { file: `${FLOWS_DIR}/${file}`, hash: hashText(text) };
}

export function readIgnored(root) {
  try {
    return new Set(JSON.parse(readFileSync(join(root, IGNORED_FILE), "utf8")));
  } catch {
    return new Set();
  }
}
export function ignoreDraft(root, id) {
  const s = readIgnored(root);
  s.add(id);
  mkdirSync(join(root, ".wara"), { recursive: true });
  writeFileSync(join(root, IGNORED_FILE), JSON.stringify([...s], null, 1));
}

// ---------- checking refs against the graph ----------

function dataModel(g) {
  if (g.dataModelCache !== undefined) return g.dataModelCache;
  try {
    g.dataModelCache = JSON.parse(g.db.prepare("SELECT value FROM meta WHERE key = 'data_model'").get()?.value ?? "null");
  } catch {
    g.dataModelCache = null;
  }
  return g.dataModelCache;
}

/** One backtick ref → { ok, kind, where }. Lists ("payments, orders") are checked item by item. */
export function checkRef(g, ref) {
  const parts = ref.split(/\s*,\s*/).filter(Boolean);
  if (parts.length > 1) {
    const each = parts.map((p) => checkRef(g, p));
    return { ref, ok: each.every((x) => x.ok), kind: "list", parts: each };
  }
  const r = ref.replace(/:\d+$/, "").trim();
  const route = /^(?:(GET|POST|PUT|PATCH|DELETE)\s+|webhook\s+|cron\s+)?(\/\S*)$/i.exec(r);
  if (route) {
    const hits = g.routes.filter((x) => x.path === route[2] || routeMatches(x.path, route[2]));
    const method = route[1]?.toUpperCase();
    const ok = hits.length > 0 && (!method || hits.some((h) => h.kind !== "route" || (g.symbolsOf.get(h.file_id) ?? []).some((s) => s.name === method)));
    return { ref, ok, kind: "route", where: hits[0]?.file?.path };
  }
  if (/[/\\]/.test(r) && /\.\w+$/.test(r)) {
    const f = g.fileByPath(r);
    return { ref, ok: Boolean(f), kind: "file", where: f?.path };
  }
  const dm = dataModel(g);
  const [table, column] = r.split(".");
  const ent = dm && findEntity(dm, table);
  if (ent) {
    const ok = !column || ent.ent.columns.some((c) => c.name === column || c.db === column);
    return { ref, ok, kind: column ? "column" : "table", where: ent.ent.name };
  }
  const syms = [...g.symbolById.values()].filter((s) => s.name === r && s.kind !== "alias");
  if (syms.length) return { ref, ok: true, kind: "symbol", where: `${syms[0].file.path}:${syms[0].line}` };
  return { ref, ok: false, kind: /^[a-z_][\w]*$/i.test(r) ? "unknown" : "text" };
}

export function checkFlow(g, flow) {
  const steps = flow.steps.map((s) => ({ id: s.id, refs: s.refs.map((r) => checkRef(g, r)) }));
  const stale = steps.flatMap((s) => s.refs.filter((r) => !r.ok && r.kind !== "text").map((r) => ({ step: s.id, ref: r.ref })));
  return { steps, stale };
}

// ---------- trace: what the code really does from an entry point ----------

const ownerAt = (g, fileId, line) =>
  (g.symbolsOf.get(fileId) ?? []).filter((s) => s.kind !== "alias" && s.line <= line && (s.end_line ?? s.line) >= line)
    .sort((a, b) => (a.end_line - a.line) - (b.end_line - b.line))[0] ?? null;

function startSymbols(g, target) {
  const t = String(target).trim().replace(/:\d+$/, "");
  const m = /^(GET|POST|PUT|PATCH|DELETE)\s+(\/\S*)$/i.exec(t);
  const path = m ? m[2] : t;
  if (path.startsWith("/")) {
    const routes = g.routes.filter((r) => r.kind !== "layout" && (r.path === path || routeMatches(r.path, path)));
    const out = [];
    for (const r of routes) {
      const syms = (g.symbolsOf.get(r.file_id) ?? []).filter((s) => s.kind !== "alias");
      if (r.kind === "route") out.push(...syms.filter((s) => s.exported && HTTP.test(s.name) && (!m || s.name === m[1].toUpperCase())));
      else out.push(...syms.filter((s) => s.is_default).slice(0, 1));
    }
    return { routes, syms: out };
  }
  const hit = g.lookup(t);
  if (hit.symbols) return { routes: [], syms: hit.symbols };
  if (hit.file) return { routes: [], syms: (g.symbolsOf.get(hit.file.id) ?? []).filter((s) => s.exported && s.kind !== "alias"), file: hit.file };
  return { routes: [], syms: [], error: hit.error };
}

/** Calls (project functions, depth-limited), table reads/writes and packages reached from the target. */
export function trace(g, target, { depth = 3, max = 40 } = {}) {
  const start = startSymbols(g, target);
  if (!start.syms.length) return { target, error: start.error ?? `Nothing to trace for "${target}"`, calls: [], data: [], packages: [] };
  const seen = new Set(start.syms.map((s) => s.id));
  const calls = [];
  const packages = new Set();
  let frontier = start.syms.map((s) => ({ sym: s, level: 0 }));
  while (frontier.length && calls.length < max) {
    const next = [];
    for (const { sym, level } of frontier) {
      for (const r of (g.refsOf.get(sym.file_id) ?? []).filter((x) => x.symbol_id === sym.id && x.kind !== "type")) {
        const b = g.bindingTarget(sym.file_id, r.local);
        if (b?.external) { packages.add(b.external); continue; }
        const s = b?.symbol;
        if (!s || seen.has(s.id) || !["function", "hook", "class"].includes(s.kind)) continue;
        seen.add(s.id);
        calls.push({ from: sym.name, name: s.name, kind: s.kind, file: s.file.path, line: s.line, level: level + 1 });
        if (level + 1 < depth) next.push({ sym: s, level: level + 1 });
      }
    }
    frontier = next;
  }
  const ids = seen;
  const startFiles = new Set(start.syms.map((s) => s.file_id));
  const data = [];
  for (const d of g.db.prepare("SELECT * FROM data_refs WHERE op != 'ddl'").all()) {
    const owner = ownerAt(g, d.file_id, d.line ?? 0);
    if (!(owner ? ids.has(owner.id) : startFiles.has(d.file_id))) continue;
    data.push({ entity: d.entity, op: d.op, columns: d.columns ? JSON.parse(d.columns) : null, file: g.files.get(d.file_id)?.path, line: d.line, by: owner?.name ?? null });
  }
  return {
    target,
    start: start.syms.map((s) => ({ name: s.name, file: s.file.path, line: s.line })),
    routes: start.routes.map((r) => `${r.kind} ${r.path}`),
    calls,
    data,
    packages: [...packages],
  };
}

/** Values a status-like column is set to near the code that writes it ("status: 'paid'", "status = 'paid'"). */
export function statusValues(g, root, tableNames) {
  const found = new Map(); // "table.column" → Set(values)
  const rows = g.db.prepare("SELECT * FROM data_refs WHERE op = 'write'").all().filter((d) => tableNames.has(d.entity));
  const cache = new Map();
  for (const d of rows) {
    const f = g.files.get(d.file_id);
    if (!f) continue;
    if (!cache.has(f.path)) {
      try { cache.set(f.path, readFileSync(join(root, f.path), "utf8").split("\n")); } catch { cache.set(f.path, []); }
    }
    const lines = cache.get(f.path);
    const from = Math.max(0, (d.line ?? 1) - 3), to = Math.min(lines.length, (d.line ?? 1) + 8);
    for (const line of lines.slice(from, to)) {
      for (const m of line.matchAll(/\b(\w*status\w*|state)\b["']?\s*(?::|=)\s*['"]([a-z][\w-]{1,30})['"]/gi)) {
        const key = `${d.entity}.${m[1]}`;
        if (!found.has(key)) found.set(key, new Set());
        found.get(key).add(m[2]);
      }
    }
  }
  return [...found].map(([column, vals]) => ({ column, values: [...vals].sort(), transitions: [] }));
}

// ---------- drafts: entry points no flow covers yet ----------

const actorFor = (path) => (/\/cron\b|\/jobs?\b/.test(path) ? "cron" : /webhook|callback/.test(path) ? "webhook" : "web");

export function draftFlows(g, root, flows) {
  const covered = new Set(flows.flatMap((f) => f.steps.flatMap((s) => s.refs)).map((r) => r.replace(/^(GET|POST|PUT|PATCH|DELETE)\s+/i, "")));
  const ignored = readIgnored(root);
  const drafts = [];
  // Plumbing (auth checks, response helpers, the db wrapper) is called by most handlers: leave it out of
  // drafts so each one shows what is special about that route.
  const handlers = g.routes.filter((x) => x.kind === "route").flatMap((r) => (g.symbolsOf.get(r.file_id) ?? []).filter((s) => s.exported && HTTP.test(s.name)));
  const fanIn = new Map();
  for (const h of handlers) {
    const callees = new Set((g.refsOf.get(h.file_id) ?? []).filter((x) => x.symbol_id === h.id).map((x) => g.bindingTarget(h.file_id, x.local)?.symbol?.id).filter(Boolean));
    for (const id of callees) fanIn.set(id, (fanIn.get(id) ?? 0) + 1);
  }
  const plumbing = (name) => [...fanIn].some(([id, n]) => n >= Math.max(5, handlers.length * 0.1) && g.symbolById.get(id)?.name === name);
  for (const r of g.routes.filter((x) => x.kind === "route")) {
    for (const h of (g.symbolsOf.get(r.file_id) ?? []).filter((s) => s.exported && HTTP.test(s.name))) {
      const id = `${h.name} ${r.path}`;
      if (ignored.has(id) || covered.has(r.path) || covered.has(id)) continue;
      const t = trace(g, id, { depth: 2, max: 12 });
      const steps = [];
      const actor = actorFor(r.path);
      steps.push({ id: "1", actor: actor === "web" ? "web" : actor, text: `\`${id}\``, wait: actor !== "web", refs: [id], branches: [] });
      for (const c of t.calls.filter((c) => c.level === 1 && !plumbing(c.name)).slice(0, 4)) steps.push({ id: String(steps.length + 1), actor: "server", text: `\`${c.name}\``, wait: false, refs: [c.name], branches: [] });
      const writes = [...new Set(t.data.filter((d) => d.op === "write").map((d) => d.entity))];
      const reads = [...new Set(t.data.filter((d) => d.op === "read").map((d) => d.entity))].filter((e) => !writes.includes(e));
      if (writes.length) steps.push({ id: String(steps.length + 1), actor: "db", text: `writes \`${writes.join(", ")}\``, wait: false, refs: [writes.join(", ")], branches: [] });
      else if (reads.length) steps.push({ id: String(steps.length + 1), actor: "db", text: `reads \`${reads.slice(0, 4).join(", ")}\``, wait: false, refs: [reads.slice(0, 4).join(", ")], branches: [] });
      for (const p of t.packages.filter((p) => !/^(next|react|zod|@?types?)\b/.test(p)).slice(0, 2)) steps.push({ id: String(steps.length + 1), actor: p, text: `calls ${p}`, wait: false, refs: [], branches: [] });
      drafts.push({ id, title: id, file: null, draft: true, entry: `${actor} ${id}`, steps, states: statusValues(g, root, new Set(writes)), notes: "" });
    }
  }
  return drafts;
}

/** Everything the flows page shows. */
export function flowsOverview(g, root) {
  const flows = readFlowFiles(root).map((f) => ({ ...f, check: checkFlow(g, f) }));
  return { flows, drafts: draftFlows(g, root, flows) };
}

// ---------- text for the AI (code_graph flows / flow / trace) ----------

export function flowsText(g, root, target, limit = 40) {
  const { flows, drafts } = flowsOverview(g, root);
  if (!target) {
    if (!flows.length && !drafts.length) return `No flows yet. Flows live in ${FLOWS_DIR}/<name>.md (load the system-design skill for the format).`;
    const lines = [`# Flows (${flows.length}) — ${FLOWS_DIR}/`];
    for (const f of flows) lines.push(`${f.title} · ${f.file} · ${f.steps.length} steps${f.check.stale.length ? ` · ⚠ ${f.check.stale.length} step(s) point at code that no longer exists` : ""}`);
    if (drafts.length) {
      lines.push("", `# Entry points no flow covers yet (${drafts.length}) — drafts, not saved`);
      lines.push(...drafts.slice(0, limit).map((d) => `${d.title}${d.steps.length > 1 ? ` → ${d.steps.slice(1).map((s) => s.text.replace(/`/g, "")).join(" → ")}` : ""}`));
      if (drafts.length > limit) lines.push(`… ${drafts.length - limit} more`);
    }
    lines.push("", "Read one: code_graph flows <name or file>. Trace live code: code_graph trace <route|symbol>.");
    return lines.join("\n");
  }
  const q = target.toLowerCase();
  const f = flows.find((x) => x.file.toLowerCase().endsWith(`${q}.md`) || x.file.toLowerCase() === q || x.title.toLowerCase() === q)
    ?? flows.find((x) => x.title.toLowerCase().includes(q) || x.file.toLowerCase().includes(q));
  if (!f) {
    const d = drafts.find((x) => x.id.toLowerCase().includes(q));
    if (d) return [`Draft (not saved): ${d.title}`, ...d.steps.map((s) => `${s.id}. [${s.wait ? "⏳ " : ""}${s.actor}] ${s.text}`), ...d.states.map((st) => `states \`${st.column}\`: ${st.values.join(", ")} (values found in code; moves unknown)`)].join("\n");
    return `No flow matches "${target}". ${flows.length ? `Flows: ${flows.map((x) => x.title).join(", ")}` : "No flows yet."}`;
  }
  const staleBy = new Map(f.check.stale.map((s) => [`${s.step}|${s.ref}`, true]));
  const lines = [`# ${f.title} — ${f.file}${f.entry ? ` · entry: ${f.entry}` : ""}`];
  for (const s of f.steps) {
    const bad = s.refs.filter((r) => staleBy.has(`${s.id}|${r}`));
    lines.push(`${s.id}. [${s.wait ? "⏳ " : ""}${s.actor}] ${s.text}${bad.length ? `  ⚠ not in code: ${bad.join(", ")}` : ""}`);
    for (const b of s.branches) lines.push(`   - ${b.label}${b.to ? ` → ${b.to === "end" ? "end" : `step ${b.to}`}` : ""}`);
  }
  for (const st of f.states) lines.push(`states \`${st.column}\`: ${st.transitions.map((t) => `${t.from}→${t.to}${t.note ? ` (${t.note})` : ""}`).join(", ") || st.values.join(", ")}`);
  if (f.notes) lines.push("", f.notes);
  return lines.join("\n");
}

export function traceText(g, target) {
  const t = trace(g, target);
  if (t.error) return t.error;
  const lines = [`Trace ${target}${t.routes.length ? ` (${t.routes.join(", ")})` : ""} — from ${t.start.map((s) => `${s.name} ${s.file}:${s.line}`).join(", ")}`];
  if (t.calls.length) lines.push("## Calls", ...t.calls.map((c) => `${"  ".repeat(c.level - 1)}${c.from} → ${c.name} [${c.kind}] ${c.file}:${c.line}`));
  if (t.data.length) lines.push("## Data", ...t.data.map((d) => `${d.op} ${d.entity}${d.columns ? ` (${d.columns.join(", ")})` : ""} ${d.file}:${d.line}${d.by ? ` in ${d.by}` : ""}`));
  if (t.packages.length) lines.push(`## Packages: ${t.packages.join(", ")}`);
  if (!t.calls.length && !t.data.length) lines.push("No project calls or data access found from here.");
  return lines.join("\n");
}
