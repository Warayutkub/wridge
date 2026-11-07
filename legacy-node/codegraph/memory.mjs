// Project memory: what was agreed and where work stands, so nothing is lost when a chat is compacted
// or a new session starts.
//   - Project decisions: docs/decisions.md (committed, the team reads it) — indexed into .wara/memory.db
//   - Tasks (handoff) and working notes: only in .wara/memory.db (personal, gitignored)
//   - The brain: notes as Markdown files in brain/ (brain.mjs) — overview and find cover it too
// The AI never reads the files whole: queries return only what matches, so cost does not grow with the project.
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync, appendFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { projectRoot } from "./build.mjs";
import { WARA_DIR, ensureGitignored } from "./files.mjs";
import { BRAIN_DIR, brainSummary, findNotes } from "./brain.mjs";

export const DECISIONS_FILE = "docs/decisions.md";
const SCHEMA = `
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS decisions (
  id TEXT PRIMARY KEY, date TEXT, title TEXT NOT NULL, tags TEXT, body TEXT, supersedes TEXT, line INTEGER
);
CREATE TABLE IF NOT EXISTS tasks (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, stage TEXT, done TEXT, next TEXT, questions TEXT,
  status TEXT NOT NULL DEFAULT 'open', updated_at TEXT
);
CREATE TABLE IF NOT EXISTS task_history (task_id INTEGER, snapshot TEXT, at TEXT);
CREATE TABLE IF NOT EXISTS notes (
  id INTEGER PRIMARY KEY, task_id INTEGER, text TEXT NOT NULL, tags TEXT, created_at TEXT, archived INTEGER NOT NULL DEFAULT 0
);
`;

const now = () => new Date().toISOString();
const today = () => now().slice(0, 10);

function open(root, { create = true } = {}) {
  const file = join(root, WARA_DIR, "memory.db");
  if (!create && !existsSync(file)) return null;
  const gitignore = create ? ensureGitignored(root) : { added: false }; // before .wara/ exists
  mkdirSync(join(root, WARA_DIR), { recursive: true });
  const db = new DatabaseSync(file);
  db.exec("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;");
  db.exec(SCHEMA);
  db.gitignore = gitignore;
  return db;
}

const getMeta = (db, k) => db.prepare("SELECT value FROM meta WHERE key = ?").get(k)?.value;
const setMeta = (db, k, v) => db.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)").run(k, String(v));

// ---------- docs/decisions.md ----------

// "## D-12 · 2026-10-06 · Title" (id and date optional, so hand-written entries still count).
const HEADING = /^##\s+(?:(D-\d+)\s*[·|–-]\s*)?(?:(\d{4}-\d{2}-\d{2})\s*[·|–-]\s*)?(.+?)\s*$/;
const FIELD = /^\**(Tags|Supersedes|Why|Decision)\**\s*:\**\s*(.*)$/i;

export function parseDecisions(text) {
  const out = [];
  let cur = null;
  let n = 0;
  text.split(/\r?\n/).forEach((line, i) => {
    const h = HEADING.exec(line);
    if (h && line.startsWith("## ")) {
      n++;
      cur = { id: h[1] ?? `H${n}`, date: h[2] ?? null, title: h[3], tags: "", supersedes: "", body: [], line: i + 1 };
      out.push(cur);
      return;
    }
    if (!cur) return;
    const f = FIELD.exec(line.trim());
    if (f && /^tags$/i.test(f[1])) cur.tags = f[2].toLowerCase();
    else if (f && /^supersedes$/i.test(f[1])) cur.supersedes = f[2].trim();
    else if (line.trim()) cur.body.push(line.trim());
  });
  return out.map((d) => ({ ...d, body: d.body.join("\n") }));
}

// Re-index docs/decisions.md only when it changed (also catches edits by hand or from git pulls).
function syncDecisions(db, root) {
  const file = join(root, DECISIONS_FILE);
  const st = statSync(file, { throwIfNoEntry: false });
  const stamp = st ? `${st.mtimeMs}:${st.size}` : "none";
  if (getMeta(db, "decisions_stamp") === stamp) return;
  db.exec("DELETE FROM decisions");
  if (st) {
    const put = db.prepare("INSERT OR REPLACE INTO decisions (id, date, title, tags, body, supersedes, line) VALUES (?, ?, ?, ?, ?, ?, ?)");
    for (const d of parseDecisions(readFileSync(file, "utf8"))) put.run(d.id, d.date, d.title, d.tags, d.body, d.supersedes, d.line);
  }
  setMeta(db, "decisions_stamp", stamp);
}

const supersededIds = (db) =>
  new Set(db.prepare("SELECT supersedes FROM decisions WHERE supersedes != ''").all().flatMap((r) => r.supersedes.split(/[,\s]+/).filter(Boolean)));

function nextDecisionId(db) {
  const max = db.prepare("SELECT id FROM decisions WHERE id LIKE 'D-%'").all().map((r) => Number(r.id.slice(2))).reduce((a, b) => Math.max(a, b), 0);
  return `D-${max + 1}`;
}

// ---------- queries ----------

const fmtDecision = (d, full) =>
  `${d.id} · ${d.date ?? "—"} · ${d.title}${d.tags ? ` [${d.tags}]` : ""}${full && d.body ? `\n  ${d.body.replaceAll("\n", "\n  ")}` : ""}${d.supersedes ? `\n  (replaces ${d.supersedes})` : ""}`;

function taskBlock(db, t) {
  const notes = db.prepare("SELECT id, text FROM notes WHERE task_id = ? AND archived = 0 ORDER BY id").all(t.id);
  return [
    `### ${t.name}${t.stage ? ` — ${t.stage}` : ""} (updated ${String(t.updated_at).slice(0, 16).replace("T", " ")})`,
    t.done && `Done: ${t.done}`,
    t.next && `Next: ${t.next}`,
    t.questions && `Waiting on the user: ${t.questions}`,
    notes.length && `Agreed while working:\n${notes.map((n) => `- (n${n.id}) ${n.text}`).join("\n")}`,
  ].filter(Boolean).join("\n");
}

export function overview(projectDir, { brief = false } = {}) {
  const root = projectRoot(projectDir);
  const db = open(root, { create: !brief || existsSync(join(root, DECISIONS_FILE)) || existsSync(join(root, BRAIN_DIR)) });
  if (!db) return "";
  try {
    syncDecisions(db, root);
    const tasks = db.prepare("SELECT * FROM tasks WHERE status = 'open' ORDER BY updated_at DESC").all();
    const old = supersededIds(db);
    const decisions = db.prepare("SELECT * FROM decisions ORDER BY line").all().filter((d) => !old.has(d.id));
    const brain = brainSummary(root);
    if (brief && !tasks.length && !decisions.length && !brain?.count) return "";
    const lines = [`# Project memory — ${root.replaceAll("\\", "/")}`];
    lines.push("", tasks.length ? `## Open tasks (${tasks.length})` : "## Open tasks: none");
    for (const t of tasks.slice(0, 5)) lines.push(taskBlock(db, t));
    const tagCount = {};
    for (const d of decisions) for (const tag of d.tags.split(/[,\s]+/).filter(Boolean)) tagCount[tag] = (tagCount[tag] ?? 0) + 1;
    lines.push("", `## Project decisions (${decisions.length} in ${DECISIONS_FILE})`);
    if (decisions.length <= 30) lines.push(...decisions.map((d) => fmtDecision(d, false)));
    else {
      lines.push(`By tag: ${Object.entries(tagCount).sort((a, b) => b[1] - a[1]).map(([t, c]) => `${t} ${c}`).join(" · ")}`, "Latest:");
      lines.push(...decisions.slice(-8).map((d) => fmtDecision(d, false)));
    }
    if (brain) {
      lines.push("", `## Brain (${brain.count} notes in ${BRAIN_DIR}/)`);
      if (brain.tags.length) lines.push(`Tags: ${brain.tags.map(([t, c]) => `${t} ${c}`).join(" · ")}`);
      if (brain.broken) lines.push(`${brain.broken} link(s) point at notes that don't exist.`);
    }
    lines.push("", `Details: project_memory find <topic> (never open ${DECISIONS_FILE} whole). Record agreements as they happen.`);
    return lines.join("\n");
  } finally {
    db.close();
  }
}

export function find(projectDir, query, { includeOld = false, limit = 8 } = {}) {
  const root = projectRoot(projectDir);
  const db = open(root);
  try {
    syncDecisions(db, root);
    const q = String(query ?? "").toLowerCase().trim();
    const words = q.split(/\s+/).filter(Boolean);
    const score = (title, tags, body) => {
      const t = `${title}`.toLowerCase(), g = `${tags ?? ""}`.toLowerCase(), b = `${body ?? ""}`.toLowerCase();
      let s = (t + " " + g + " " + b).includes(q) ? 5 : 0;
      for (const w of words) s += (t.includes(w) ? 3 : 0) + (g.includes(w) ? 2 : 0) + (b.includes(w) ? 1 : 0);
      return s;
    };
    const old = supersededIds(db);
    const decisions = db.prepare("SELECT * FROM decisions").all()
      .filter((d) => includeOld || !old.has(d.id))
      .map((d) => ({ d, s: score(d.title, d.tags, d.body) })).filter((x) => x.s > 0).sort((a, b) => b.s - a.s).slice(0, limit);
    const notes = db.prepare("SELECT n.*, t.name AS task FROM notes n LEFT JOIN tasks t ON t.id = n.task_id WHERE n.archived = 0 OR ?").all(includeOld ? 1 : 0)
      .map((n) => ({ n, s: score(n.text, n.tags, n.task) })).filter((x) => x.s > 0).sort((a, b) => b.s - a.s).slice(0, limit);
    const lines = [];
    if (decisions.length) lines.push(`## Decisions (${DECISIONS_FILE})`, ...decisions.map((x) => fmtDecision(x.d, true) + (old.has(x.d.id) ? "\n  [replaced]" : "")));
    if (notes.length) lines.push("", "## Working notes", ...notes.map((x) => `- (n${x.n.id}) [${x.n.task ?? "no task"}${x.n.archived ? ", closed" : ""}] ${x.n.text}`));
    const brain = existsSync(join(root, BRAIN_DIR)) ? findNotes(root, query, { limit }) : [];
    if (brain.length) lines.push("", "## Brain notes (project_memory read <note> for the whole note)", ...brain.map((b) => `- ${b.note}${b.tags.length ? ` [${b.tags.join(", ")}]` : ""}${b.line ? ` — ${b.line}` : ""}`));
    return lines.length ? lines.join("\n") : `Nothing recorded about "${query}".`;
  } finally {
    db.close();
  }
}

// ---------- writes ----------

export function decide(projectDir, { title, decision, why, tags, supersedes }) {
  const root = projectRoot(projectDir);
  const db = open(root);
  try {
    syncDecisions(db, root);
    const id = nextDecisionId(db);
    const file = join(root, DECISIONS_FILE);
    const created = !existsSync(file);
    if (created) {
      mkdirSync(dirname(file), { recursive: true });
      writeFileSync(file, "# Project decisions\n\nAgreed choices and rules for this project, newest last. Each entry: `## D-n · date · title`, then Tags / Decision / Why (and Supersedes when it replaces an older one). Edit freely; the AI reads it through wridge `project_memory`.\n");
    }
    const entry = [
      "", `## ${id} · ${today()} · ${title.trim()}`,
      tags?.length && `Tags: ${[].concat(tags).join(", ").toLowerCase()}`,
      `Decision: ${decision.trim()}`,
      why && `Why: ${why.trim()}`,
      supersedes && `Supersedes: ${supersedes}`,
    ].filter(Boolean).join("\n");
    appendFileSync(file, `\n${entry.trimStart()}\n`);
    syncDecisions(db, root);
    return { id, file: `${DECISIONS_FILE}${created ? " (created — commit it with the work)" : ""}`, gitignore: db.gitignore.added ? `added ${db.gitignore.pattern} to .gitignore` : undefined };
  } finally {
    db.close();
  }
}

function taskId(db, name, create) {
  const t = db.prepare("SELECT id FROM tasks WHERE name = ?").get(name);
  if (t) return t.id;
  if (!create) throw new Error(`No task "${name}" — start it with action "handoff"`);
  return Number(db.prepare("INSERT INTO tasks (name, updated_at) VALUES (?, ?)").run(name, now()).lastInsertRowid);
}

export function note(projectDir, { task, text, tags }) {
  const root = projectRoot(projectDir);
  const db = open(root);
  try {
    const id = task ? taskId(db, task, true) : null;
    const r = db.prepare("INSERT INTO notes (task_id, text, tags, created_at) VALUES (?, ?, ?, ?)").run(id, text.trim(), [].concat(tags ?? []).join(", "), now());
    return { note: `n${r.lastInsertRowid}`, task: task ?? null };
  } finally {
    db.close();
  }
}

export function handoff(projectDir, { task, stage, done, next, questions, close }) {
  const root = projectRoot(projectDir);
  const db = open(root);
  try {
    const id = taskId(db, task, !close);
    const prev = db.prepare("SELECT * FROM tasks WHERE id = ?").get(id);
    db.prepare("INSERT INTO task_history (task_id, snapshot, at) VALUES (?, ?, ?)").run(id, JSON.stringify(prev), now());
    const v = (val, old) => (val === undefined ? old : val);
    db.prepare("UPDATE tasks SET stage = ?, done = ?, next = ?, questions = ?, status = ?, updated_at = ? WHERE id = ?")
      .run(v(stage, prev.stage), v(done, prev.done), v(next, prev.next), v(questions, prev.questions), close ? "closed" : "open", now(), id);
    if (close) db.prepare("UPDATE notes SET archived = 1 WHERE task_id = ?").run(id);
    const t = db.prepare("SELECT * FROM tasks WHERE id = ?").get(id);
    return close ? `Closed "${task}". Its working notes are archived (find … includeOld shows them).` : taskBlock(db, t);
  } finally {
    db.close();
  }
}

// ---------- hook entry: find the project from a working directory ----------

export function findProjectRoot(cwd) {
  for (let dir = resolve(cwd); ; dir = dirname(dir)) {
    if (existsSync(join(dir, WARA_DIR, "memory.db")) || existsSync(join(dir, DECISIONS_FILE)) || existsSync(join(dir, BRAIN_DIR))) return dir;
    if (dirname(dir) === dir) return null;
  }
}
