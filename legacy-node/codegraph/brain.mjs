// The brain: a project's notes as plain Markdown files in <project>/brain/, linked with [[…]] like Obsidian.
// The files are the truth; .wara/brain.db is only an index built from them (delete it and it comes back).
// Links: [[Note]] another note · [[code:path#Name]] · [[figma:12:34]] · [[roblox:Game.Path]] · [[other-project:Note]]
// A front-matter key holding links is a named relation, e.g. "boss_of: [[CH1]]".
// Deleting moves a note to brain/.trash/ (Obsidian's own trash folder), never removes it; only the user
// empties the trash, from the web page.
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { projectRoot } from "./build.mjs";
import { WARA_DIR, ensureGitignored } from "./files.mjs";

export const BRAIN_DIR = "brain";
export const TRASH_DIR = ".trash";
const EXTERNAL = new Set(["code", "figma", "roblox", "url", "file"]);

const SCHEMA = `
CREATE TABLE IF NOT EXISTS notes (
  path TEXT PRIMARY KEY, name TEXT NOT NULL, title TEXT NOT NULL, aliases TEXT, tags TEXT, body TEXT,
  stamp TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS links (
  src TEXT NOT NULL, kind TEXT NOT NULL, target TEXT NOT NULL, rel TEXT, line INTEGER
);
CREATE INDEX IF NOT EXISTS links_src ON links(src);
CREATE INDEX IF NOT EXISTS links_target ON links(target);
`;

const slash = (p) => p.replaceAll("\\", "/");
const lower = (s) => String(s ?? "").toLowerCase();

// ---------- parsing ----------

// Small front matter: "key: value", "key: [a, b]", or "key:" followed by "- item" lines.
export function parseFrontMatter(text) {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  if (!m) return { data: {}, body: text, offset: 0 };
  const data = {};
  let key = null;
  for (const line of m[1].split(/\r?\n/)) {
    const item = /^\s*-\s+(.*)$/.exec(line);
    if (item && key) { data[key] = [].concat(data[key] ?? [], item[1].trim()); continue; }
    const kv = /^([\w฀-๿-]+)\s*:\s*(.*)$/.exec(line);
    if (!kv) continue;
    key = kv[1];
    const v = kv[2].trim();
    data[key] = v === "" ? [] : /^\[.*\]$/.test(v) && !/^\[\[/.test(v) ? v.slice(1, -1).split(",").map((s) => s.trim()).filter(Boolean) : v;
  }
  return { data, body: text.slice(m[0].length), offset: m[0].split("\n").length - 1 };
}

const LINK = /\[\[([^\]|#]+)(?:#([^\]|]*))?(?:\|[^\]]*)?\]\]/g;

// "[[x]]" → { kind, target }: a note of this project, another project's note, or something outside the brain.
export function linkTarget(raw) {
  const t = raw.trim();
  const i = t.indexOf(":");
  if (i > 0) {
    const prefix = lower(t.slice(0, i));
    if (EXTERNAL.has(prefix)) return { kind: prefix, target: t.slice(i + 1).trim() };
    return { kind: "project", target: `${t.slice(0, i).trim()}:${t.slice(i + 1).trim()}` };
  }
  return { kind: "note", target: t };
}

// [[x#part]]: for a note, #part is a heading (ignored); for code it names the symbol (code:path#Name), so it stays.
const linkOf = (m) => { const t = linkTarget(m[1]); return t.kind === "code" && m[2] ? { ...t, target: `${t.target}#${m[2].trim()}` } : t; };

export function parseNote(text, path) {
  const { data, body, offset } = parseFrontMatter(text);
  const name = path.replace(/\.md$/i, "").split("/").pop();
  const heading = /^#\s+(.+)$/m.exec(body);
  const tags = new Set([].concat(data.tags ?? []).map((t) => lower(t).replace(/^#/, "")).filter(Boolean));
  const links = [];
  body.split(/\r?\n/).forEach((line, i) => {
    if (!/^#{1,6}\s/.test(line)) for (const m of line.matchAll(/(?:^|\s)#([\p{L}\p{M}\p{N}_/-]+)/gu)) tags.add(lower(m[1]));
    for (const m of line.matchAll(LINK)) links.push({ ...linkOf(m), rel: null, line: offset + i + 1 });
  });
  for (const [key, value] of Object.entries(data)) {
    if (key === "tags" || key === "aliases") continue;
    for (const v of [].concat(value)) for (const m of String(v).matchAll(LINK)) links.push({ ...linkOf(m), rel: key, line: 1 });
  }
  return { path, name, title: heading ? heading[1].trim() : name, aliases: [].concat(data.aliases ?? []).map(String), tags: [...tags], body, links };
}

// ---------- index ----------

function walk(dir, base, out = []) {
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name.startsWith(".")) continue; // .trash, .obsidian
    const full = join(dir, e.name);
    if (e.isDirectory()) walk(full, base, out);
    else if (/\.md$/i.test(e.name)) out.push(slash(relative(base, full)));
  }
  return out;
}

function openIndex(root) {
  ensureGitignored(root); // before .wara/ exists
  mkdirSync(join(root, WARA_DIR), { recursive: true });
  const db = new DatabaseSync(join(root, WARA_DIR, "brain.db"));
  db.exec("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;");
  db.exec(SCHEMA);
  return db;
}

// Re-reads only notes whose file changed (edits by hand, in Obsidian, or from git all count).
function sync(db, root) {
  const base = join(root, BRAIN_DIR);
  const files = walk(base, base);
  const known = new Map(db.prepare("SELECT path, stamp FROM notes").all().map((r) => [r.path, r.stamp]));
  const put = db.prepare("INSERT OR REPLACE INTO notes (path, name, title, aliases, tags, body, stamp) VALUES (?, ?, ?, ?, ?, ?, ?)");
  const link = db.prepare("INSERT INTO links (src, kind, target, rel, line) VALUES (?, ?, ?, ?, ?)");
  const drop = (p) => { db.prepare("DELETE FROM notes WHERE path = ?").run(p); db.prepare("DELETE FROM links WHERE src = ?").run(p); };
  for (const p of files) {
    const st = statSync(join(base, p));
    const stamp = `${st.mtimeMs}:${st.size}`;
    if (known.get(p) === stamp) continue;
    const n = parseNote(readFileSync(join(base, p), "utf8"), p);
    drop(p);
    put.run(p, n.name, n.title, n.aliases.join("\n"), n.tags.join(","), n.body, stamp);
    for (const l of n.links) link.run(p, l.kind, l.target, l.rel, l.line);
  }
  const present = new Set(files);
  for (const p of known.keys()) if (!present.has(p)) drop(p);
}

function withBrain(projectDir, fn) {
  const root = projectRoot(projectDir);
  const db = openIndex(root);
  try {
    sync(db, root);
    return fn(db, root);
  } finally {
    db.close();
  }
}

// A note name (file name, title, alias or path) → its row.
function resolve(db, name) {
  const n = lower(String(name).trim().replace(/\.md$/i, ""));
  const rows = db.prepare("SELECT path, name, title, aliases, tags, body FROM notes").all();
  return rows.find((r) => lower(r.path.replace(/\.md$/i, "")) === n)
    ?? rows.find((r) => lower(r.name) === n)
    ?? rows.find((r) => lower(r.title) === n)
    ?? rows.find((r) => r.aliases.split("\n").some((a) => lower(a) === n));
}

const label = (r) => (lower(r.title) === lower(r.name) ? r.title : `${r.title} (${r.name})`);

// ---------- reading ----------

export function brainSummary(projectDir) {
  const root = projectRoot(projectDir);
  if (!existsSync(join(root, BRAIN_DIR))) return null;
  return withBrain(root, (db) => {
    const count = db.prepare("SELECT COUNT(*) AS n FROM notes").get().n;
    const tagCount = {};
    for (const r of db.prepare("SELECT tags FROM notes").all()) for (const t of r.tags.split(",").filter(Boolean)) tagCount[t] = (tagCount[t] ?? 0) + 1;
    const root = projectRoot(projectDir);
    const codeBroken = withCodeGraph(root, (gdb) => db.prepare("SELECT target FROM links WHERE kind = 'code'").all().filter((l) => !codeExists(root, l.target, gdb)).length);
    const broken = brokenLinks(db).length + codeBroken;
    return { count, tags: Object.entries(tagCount).sort((a, b) => b[1] - a[1]).slice(0, 12), broken };
  });
}

function brokenLinks(db) {
  return db.prepare("SELECT src, target, line FROM links WHERE kind = 'note'").all().filter((l) => !resolve(db, l.target));
}

export function findNotes(projectDir, query, { limit = 8 } = {}) {
  if (!existsSync(join(projectRoot(projectDir), BRAIN_DIR))) return []; // no brain: nothing to search, and no index made
  return withBrain(projectDir, (db) => {
    const q = lower(query).trim();
    const words = q.split(/\s+/).filter(Boolean);
    return db.prepare("SELECT path, name, title, aliases, tags, body FROM notes").all().map((r) => {
      const head = lower(`${r.title} ${r.name} ${r.aliases}`), tags = lower(r.tags), body = lower(r.body);
      let s = (head + " " + tags + " " + body).includes(q) ? 5 : 0;
      for (const w of words) s += (head.includes(w) ? 3 : 0) + (tags.includes(w) ? 2 : 0) + (body.includes(w) ? 1 : 0);
      // the best matching line, so the answer stays short
      const line = r.body.split(/\r?\n/).filter((l) => l.trim() && !/^#\s/.test(l)).map((l) => [l, words.filter((w) => lower(l).includes(w)).length]).sort((a, b) => b[1] - a[1])[0];
      return { r, s, line: line && line[1] ? line[0].trim().slice(0, 160) : "" };
    }).filter((x) => x.s > 0).sort((a, b) => b.s - a.s).slice(0, limit)
      .map(({ r, line }) => ({ note: label(r), path: `${BRAIN_DIR}/${r.path}`, tags: r.tags ? r.tags.split(",") : [], line }));
  });
}

export function readNote(projectDir, name) {
  return withBrain(projectDir, (db) => {
    const r = resolve(db, name);
    if (!r) throw new Error(`No note "${name}" in ${BRAIN_DIR}/ — project_memory find lists what exists`);
    const out = db.prepare("SELECT kind, target, rel FROM links WHERE src = ?").all(r.path).map((l) => {
      if (l.kind === "code") return { ...l, missing: withCodeGraph(projectRoot(projectDir), (gdb) => !codeExists(projectRoot(projectDir), l.target, gdb)) || undefined };
      if (l.kind !== "note") return { ...l };
      const hit = resolve(db, l.target);
      return { ...l, target: hit ? hit.name : l.target, missing: !hit || undefined };
    });
    const keys = new Set([r.name, r.title, r.path.replace(/\.md$/i, ""), ...r.aliases.split("\n").filter(Boolean)].map(lower));
    const backPaths = [...new Set(db.prepare("SELECT src, target FROM links WHERE kind = 'note'").all().filter((l) => keys.has(lower(l.target)) && l.src !== r.path).map((l) => l.src))];
    const backRows = backPaths.map((p) => db.prepare("SELECT path, name, title FROM notes WHERE path = ?").get(p)).filter(Boolean);
    const backlinks = backRows.map(label);
    return { note: label(r), path: `${BRAIN_DIR}/${r.path}`, text: readFileSync(join(projectRoot(projectDir), BRAIN_DIR, r.path), "utf8"), links: out, backlinks, backlinkPaths: backRows.map((b) => b.path) };
  });
}

// Everything for the web page's graph: notes, what they point at, and the lines between them.
export function brainGraph(projectDir) {
  return withBrain(projectDir, (db) => {
    const root = projectRoot(projectDir);
    const gdbFile = join(root, WARA_DIR, "code-graph.db");
    let gdb = null;
    try { if (existsSync(gdbFile)) gdb = new DatabaseSync(gdbFile, { readOnly: true }); } catch { gdb = null; }
    const codeGone = (t) => !codeExists(root, t, gdb);
    const notes = db.prepare("SELECT path, name, title, tags, body FROM notes ORDER BY path").all();
    // the first line of text, links shown as their words — the list's preview under each title
    const firstLine = (body) => body.split(/\r?\n/).map((l) => l.trim()).find((l) => l && !/^#/.test(l))?.replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (m, a, b) => b ?? a).replace(/\*\*|`/g, "").slice(0, 140) ?? "";
    const nodes = notes.map((n) => ({ id: `note:${n.path}`, kind: "note", label: n.title, name: n.name, path: n.path, tags: n.tags ? n.tags.split(",") : [], line: firstLine(n.body) }));
    const seen = new Set(nodes.map((n) => n.id));
    const edges = [];
    for (const l of db.prepare("SELECT src, kind, target, rel FROM links").all()) {
      let to;
      if (l.kind === "note") {
        const hit = resolve(db, l.target);
        to = hit ? `note:${hit.path}` : `missing:${l.target}`;
        if (!hit && !seen.has(to)) { seen.add(to); nodes.push({ id: to, kind: "missing", label: l.target }); }
      } else {
        to = `${l.kind}:${l.target}`;
        if (!seen.has(to)) { seen.add(to); nodes.push({ id: to, kind: l.kind, label: l.target, ...(l.kind === "code" && codeGone(l.target) ? { missing: true } : {}) }); }
      }
      edges.push({ from: `note:${l.src}`, to, rel: l.rel });
    }
    gdb?.close();
    return { nodes, edges, trash: listTrash(projectDir) };
  });
}

// ---------- writing ----------

const safeName = (name) => String(name).trim().replace(/\.md$/i, "").replace(/[<>:"|?*\\]+/g, "-").replace(/^\/+|\/+$/g, "").replace(/\.\.+/g, ".");

function noteFile(root, name) {
  const rel = `${safeName(name)}.md`;
  if (!safeName(name)) throw new Error("A note needs a name");
  return { rel, full: join(root, BRAIN_DIR, rel) };
}

// mode: create (fails if it exists) | replace (the old text goes to the trash first, unless keepOld is false —
// the web editor saves in place like any editor) | append
export function writeNote(projectDir, name, { text, mode = "create", tags, keepOld = true } = {}) {
  const root = projectRoot(projectDir);
  if (!String(text ?? "").trim()) throw new Error("write needs text");
  const existing = withBrain(root, (db) => resolve(db, name));
  const { rel, full } = existing ? { rel: existing.path, full: join(root, BRAIN_DIR, existing.path) } : noteFile(root, name);
  if (existing && mode === "create") throw new Error(`"${label(existing)}" already exists (${BRAIN_DIR}/${rel}) — use mode append or replace`);
  mkdirSync(dirname(full), { recursive: true });
  let body = String(text).trim();
  if (mode === "append" && existing) {
    writeFileSync(full, `${readFileSync(full, "utf8").replace(/\s*$/, "")}\n\n${body}\n`);
  } else {
    if (mode === "replace" && existing && keepOld) toTrash(root, rel, { copy: true });
    if (!/^---\r?\n/.test(body) && !/^#\s/m.test(body)) body = `# ${safeName(name).split("/").pop()}\n\n${body}`;
    if (tags?.length && !/^---\r?\n/.test(body)) body = `---\ntags: [${tags.join(", ")}]\n---\n${body}`;
    writeFileSync(full, `${body}\n`);
  }
  return withBrain(root, (db) => {
    const r = db.prepare("SELECT path FROM notes WHERE path = ?").get(rel);
    const broken = brokenLinks(db).filter((l) => l.src === rel).map((l) => l.target);
    return { saved: `${BRAIN_DIR}/${r.path}`, mode: existing ? mode : "create", ...(broken.length ? { linksToMissingNotes: broken } : {}) };
  });
}

// Into brain/.trash/ keeping the folder path; the file's time becomes "when it was deleted".
function toTrash(root, rel, { copy = false } = {}) {
  const from = join(root, BRAIN_DIR, rel);
  let to = join(root, BRAIN_DIR, TRASH_DIR, rel);
  for (let n = 2; existsSync(to); n++) to = join(root, BRAIN_DIR, TRASH_DIR, rel.replace(/\.md$/i, ` (${n}).md`));
  mkdirSync(dirname(to), { recursive: true });
  if (copy) writeFileSync(to, readFileSync(from));
  else renameSync(from, to);
  const t = new Date();
  utimesSync(to, t, t);
  return slash(relative(join(root, BRAIN_DIR, TRASH_DIR), to));
}

export function trashNote(projectDir, name) {
  const root = projectRoot(projectDir);
  const r = withBrain(root, (db) => resolve(db, name));
  if (!r) throw new Error(`No note "${name}"`);
  const backlinks = readNote(root, r.path).backlinks;
  const at = toTrash(root, r.path);
  return { trashed: `${BRAIN_DIR}/${r.path}`, restore: at, ...(backlinks.length ? { brokenLinksFrom: backlinks } : {}) };
}

export function listTrash(projectDir) {
  const root = projectRoot(projectDir);
  const base = join(root, BRAIN_DIR, TRASH_DIR);
  return walk(base, base).map((p) => ({ path: p, deletedAt: statSync(join(base, p)).mtime.toISOString() })).sort((a, b) => b.deletedAt.localeCompare(a.deletedAt));
}

// path: as listed by listTrash. Back to where it was ("x (2).md" → "x.md"); refuses to overwrite.
export function restoreNote(projectDir, path) {
  const root = projectRoot(projectDir);
  const from = join(root, BRAIN_DIR, TRASH_DIR, path);
  if (!existsSync(from) || slash(relative(join(root, BRAIN_DIR, TRASH_DIR), from)).startsWith("..")) throw new Error(`Not in the trash: ${path}`);
  const rel = path.replace(/ \(\d+\)\.md$/i, ".md");
  const to = join(root, BRAIN_DIR, rel);
  if (existsSync(to)) throw new Error(`${BRAIN_DIR}/${rel} exists again — rename or trash it first`);
  mkdirSync(dirname(to), { recursive: true });
  renameSync(from, to);
  return { restored: `${BRAIN_DIR}/${rel}` };
}

// The web page's "empty trash" button only (the user's own click); the AI tool has no way to call this.
export function emptyTrash(projectDir, path) {
  const root = projectRoot(projectDir);
  const base = join(root, BRAIN_DIR, TRASH_DIR);
  if (path) {
    const file = join(base, path);
    if (slash(relative(base, file)).startsWith("..") || !existsSync(file)) throw new Error(`Not in the trash: ${path}`);
    rmSync(file);
    return { removed: 1 };
  }
  const n = listTrash(root).length;
  rmSync(base, { recursive: true, force: true });
  return { removed: n };
}

// ---------- links into the code graph ----------

const normCode = (t) => String(t).trim().replaceAll("\\", "/").replace(/^\.\//, "");

// Is a [[code:…]] target still there? path (on disk), path#Name (the graph knows the symbol, when it parsed
// the file), or a bare Name (some symbol has it). Unparsed languages (Lua…) are checked on disk only.
function codeExists(root, target, gdb) {
  const [p, name] = normCode(target).split("#");
  if (!p) return true;
  if (!/[/.]/.test(p) && !name) return gdb ? Boolean(gdb.prepare("SELECT 1 FROM symbols WHERE name = ? LIMIT 1").get(p)) : true;
  if (!existsSync(join(root, p))) return false;
  if (name && gdb) {
    const f = gdb.prepare("SELECT id FROM files WHERE path = ?").get(p);
    if (f && gdb.prepare("SELECT COUNT(*) AS n FROM symbols WHERE file_id = ?").get(f.id).n) return Boolean(gdb.prepare("SELECT 1 FROM symbols WHERE file_id = ? AND name = ?").get(f.id, name));
  }
  return true;
}
function withCodeGraph(root, fn) {
  const file = join(root, WARA_DIR, "code-graph.db");
  let gdb = null;
  try { if (existsSync(file)) gdb = new DatabaseSync(file, { readOnly: true }); } catch { gdb = null; }
  try { return fn(gdb); } finally { gdb?.close(); }
}
export const codeLinkMissing = (projectDir, target) => {
  const root = projectRoot(projectDir);
  return withCodeGraph(root, (gdb) => !codeExists(root, target, gdb));
};

// Notes whose [[code:…]] links point at this file or symbol — the signpost code_graph adds to its answers.
export function notesAbout(projectDir, { path, name } = {}) {
  const root = projectRoot(projectDir);
  if (!existsSync(join(root, BRAIN_DIR))) return [];
  return withBrain(root, (db) => {
    const p = path ? normCode(path) : null;
    const hits = new Set();
    for (const l of db.prepare("SELECT src, target FROM links WHERE kind = 'code'").all()) {
      const t = normCode(l.target);
      const [tp, tn] = t.split("#");
      if ((p && (tp === p) && (!tn || !name || tn === name)) || (name && (t === name || (tn === name && (!p || tp === p))))) hits.add(l.src);
    }
    return [...hits].map((src) => db.prepare("SELECT path, name, title, body FROM notes WHERE path = ?").get(src)).filter(Boolean).map((r) => ({
      note: label(r),
      line: r.body.split(/\r?\n/).map((x) => x.trim()).find((x) => x && !/^#/.test(x) && !/^---$/.test(x))?.replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (m, a, b) => b ?? a).slice(0, 120) ?? "",
    }));
  });
}

// Every [[code:…]] link with its note — the code graph page shows them beside the file or symbol.
export function codeLinks(projectDir) {
  const root = projectRoot(projectDir);
  if (!existsSync(join(root, BRAIN_DIR))) return [];
  return withBrain(root, (db) => db.prepare("SELECT l.target, n.path, n.name, n.title FROM links l JOIN notes n ON n.path = l.src WHERE l.kind = 'code'").all()
    .map((r) => ({ target: normCode(r.target), note: label(r), notePath: r.path.replace(/\.md$/i, "") })));
}
