// Meaning written by AI on top of the structural graph: what a folder/file/component is for, rules, caveats.
// Each note stores a hash of the code it describes; when that code changes the note is reported as stale.
import { createHash } from "node:crypto";

const sha = (s) => createHash("sha1").update(s).digest("hex").slice(0, 12);

// target: "dir:app/create" | "app/create/page.tsx" | "path#Name" | "Name" → { key, kind, hash, label }
export function resolveNoteTarget(g, target) {
  const t = String(target).trim().replace(/\\/g, "/");
  if (t.startsWith("dir:") || t.endsWith("/")) {
    const dir = t.replace(/^dir:/, "").replace(/\/+$/, "").replace(/^\.\/?/, "");
    const files = [...g.files.values()].filter((f) => (dir ? f.path.startsWith(`${dir}/`) : true));
    if (!files.length) throw new Error(`No files under "${dir}"`);
    return { key: `dir:${dir}`, kind: "dir", label: `${dir || "(root)"}/`, hash: dirHash(files) };
  }
  const [pathPart, symPart] = t.split("#");
  const file = g.fileByPath(pathPart);
  if (file && !symPart) return { key: `file:${file.path}`, kind: "file", label: file.path, hash: fileHash(g, file) };
  const syms = file
    ? (g.symbolsOf.get(file.id) ?? []).filter((s) => s.name === symPart || (symPart === "default" && s.is_default))
    : [...g.symbolById.values()].filter((s) => s.name === t && s.kind !== "alias");
  if (!syms.length) throw new Error(`Nothing named "${t}" — use a file path, path#Name, dir:<folder>, or a unique symbol name`);
  if (syms.length > 1) throw new Error(`"${t}" is declared in ${syms.length} places — use path#Name: ${syms.map((s) => `${s.file.path}#${s.name}`).join(", ")}`);
  const s = syms[0];
  return { key: `symbol:${s.file.path}#${s.name}`, kind: "symbol", label: `${s.name} (${s.file.path})`, hash: s.hash };
}

// Structure of a file (what it declares and imports), not its formatting.
function fileHash(g, file) {
  const syms = (g.symbolsOf.get(file.id) ?? []).map((s) => `${s.name}:${s.hash}`).join("|");
  const imps = (g.importsOf.get(file.id) ?? []).map((i) => i.source).join("|");
  return sha(`${syms}#${imps}#${file.lang === "ts" || file.lang === "dart" ? "" : file.size}`);
}
// A folder note goes stale when files are added, removed or renamed in it.
const dirHash = (files) => sha(files.map((f) => f.path).sort().join("|"));

export function writeNote(db, g, target, text) {
  const t = resolveNoteTarget(g, target);
  if (!text?.trim()) {
    db.prepare("DELETE FROM notes WHERE target = ?").run(t.key);
    return { removed: t.key };
  }
  db.prepare("INSERT OR REPLACE INTO notes (target, text, code_hash, updated_at) VALUES (?, ?, ?, ?)").run(t.key, text.trim(), t.hash, new Date().toISOString());
  return { saved: t.key, label: t.label };
}

// All notes with their current status: ok | stale (code changed) | orphan (target no longer exists).
export function allNotes(db, g) {
  return db.prepare("SELECT * FROM notes ORDER BY target").all().map((n) => {
    const target = n.target.replace(/^(file|symbol):/, "");
    try {
      const t = resolveNoteTarget(g, n.target.startsWith("dir:") ? n.target : target);
      return { ...n, kind: t.kind, label: t.label, status: t.hash === n.code_hash ? "ok" : "stale" };
    } catch {
      return { ...n, kind: n.target.split(":")[0], label: target, status: "orphan" };
    }
  });
}

// Where a note would help most: busy folders, widely used components, and entry points (routes/screens).
export function missingNotes(db, g, limit = 15) {
  const have = new Set(db.prepare("SELECT target FROM notes").all().map((n) => n.target));
  const code = [...g.files.values()].filter((f) => f.lang === "ts" || f.lang === "dart");
  const dirs = new Map();
  for (const f of code) {
    const parts = f.path.split("/").slice(0, -1);
    for (let i = 1; i <= Math.min(parts.length, 3); i++) {
      const d = parts.slice(0, i).join("/");
      dirs.set(d, (dirs.get(d) ?? 0) + 1);
    }
  }
  const dirList = [...dirs].filter(([d, n]) => n >= 3 && !have.has(`dir:${d}`)).sort((a, b) => a[0].split("/").length - b[0].split("/").length || b[1] - a[1]);
  const comps = [...g.symbolById.values()]
    .filter((s) => (s.kind === "component" || s.kind === "widget" || s.kind === "hook" || s.kind === "function" || s.kind === "class") && s.exported && !have.has(`symbol:${s.file.path}#${s.name}`))
    .map((s) => ({ s, n: new Set(g.referencesTo(s).map((r) => r.file_id)).size }))
    .filter((x) => x.n >= 3)
    .sort((a, b) => b.n - a.n);
  const entries = [...new Map(g.routes.map((r) => [r.file_id, r])).values()].filter((r) => !have.has(`file:${r.file.path}`));
  return {
    dirs: dirList.slice(0, limit).map(([d, n]) => ({ target: `dir:${d}`, files: n })),
    symbols: comps.slice(0, limit).map(({ s, n }) => ({ target: `${s.file.path}#${s.name}`, usedInFiles: n })),
    entries: entries.slice(0, limit).map((r) => ({ target: r.file.path, route: `${r.kind} ${r.path}` })),
  };
}

// Notes that apply to a file: its own, its symbols', and every parent folder's (closest first).
export function notesFor(db, g, file) {
  const keys = [`file:${file.path}`];
  const parts = file.path.split("/").slice(0, -1);
  for (let i = parts.length; i >= 1; i--) keys.push(`dir:${parts.slice(0, i).join("/")}`);
  const byKey = new Map(allNotes(db, g).map((n) => [n.target, n]));
  return keys.map((k) => byKey.get(k)).filter(Boolean);
}
