//! Meaning written by AI on top of the structural graph (port of `notes.mjs`): what a folder, file
//! or symbol is for. Each note stores a hash of the code it describes; when that code changes the
//! note is reported as stale. Notes live in the `notes` table of `.wara/code-graph.db`.

use crate::js::{self, cmp_utf16};
use crate::query::{FileRow, Graph};
use crate::re;
use crate::util::{self, obj};
use anyhow::{bail, Result};
use indexmap::IndexMap;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashSet;

fn sha(s: &str) -> String {
    util::sha1_hex(s, 12)
}

/// `resolveNoteTarget(g, target)` → `{ key, kind, label, hash }`.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteTarget {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub hash: Option<String>,
}

impl NoteTarget {
    pub fn to_json(&self) -> Value {
        obj(vec![
            ("key", json!(self.key)),
            ("kind", json!(self.kind)),
            ("label", json!(self.label)),
            ("hash", util::jstr(self.hash.as_deref())),
        ])
    }
}

/// target: `"dir:app/create"` | `"app/create/page.tsx"` | `"path#Name"` | `"Name"`.
pub fn resolve_note_target(g: &Graph, target: &str) -> Result<NoteTarget> {
    let t = js::js_trim(target).replace('\\', "/");
    if t.starts_with("dir:") || t.ends_with('/') {
        let dir = re!(r"^dir:").replace_first_str(&t, "");
        let dir = re!(r"\/+$").replace_first_str(&dir, "");
        let dir = re!(r"^\.\/?").replace_first_str(&dir, "");
        let prefix = format!("{dir}/");
        let files: Vec<&FileRow> = g.files.iter().filter(|f| dir.is_empty() || f.path.starts_with(&prefix)).collect();
        if files.is_empty() {
            bail!("No files under \"{dir}\"");
        }
        return Ok(NoteTarget {
            key: format!("dir:{dir}"),
            kind: "dir".into(),
            label: format!("{}/", if dir.is_empty() { "(root)" } else { &dir }),
            hash: Some(dir_hash(&files)),
        });
    }
    let mut parts = t.split('#');
    let path_part = parts.next().unwrap_or("");
    let sym_part = parts.next();
    let file = g.file_by_path(path_part);
    if let Some(fi) = file {
        if sym_part.is_none_or(str::is_empty) {
            let f = &g.files[fi];
            return Ok(NoteTarget {
                key: format!("file:{}", f.path),
                kind: "file".into(),
                label: f.path.clone(),
                hash: Some(file_hash(g, f)),
            });
        }
    }
    let syms: Vec<usize> = match file {
        Some(fi) => {
            let sp = sym_part.unwrap_or("");
            g.symbols_of(g.files[fi].id)
                .iter()
                .copied()
                .filter(|&i| g.syms[i].name == sp || (sp == "default" && g.syms[i].is_default != 0))
                .collect()
        }
        None => (0..g.syms.len()).filter(|&i| g.syms[i].name == t && g.syms[i].kind != "alias").collect(),
    };
    if syms.is_empty() {
        bail!("Nothing named \"{t}\" — use a file path, path#Name, dir:<folder>, or a unique symbol name");
    }
    if syms.len() > 1 {
        let list: Vec<String> = syms.iter().map(|&i| format!("{}#{}", g.sym_path(i), g.syms[i].name)).collect();
        bail!("\"{t}\" is declared in {} places — use path#Name: {}", syms.len(), list.join(", "));
    }
    let si = syms[0];
    Ok(NoteTarget {
        key: format!("symbol:{}#{}", g.sym_path(si), g.syms[si].name),
        kind: "symbol".into(),
        label: format!("{} ({})", g.syms[si].name, g.sym_path(si)),
        hash: g.syms[si].hash.clone(),
    })
}

// Structure of a file (what it declares and imports), not its formatting.
fn file_hash(g: &Graph, file: &FileRow) -> String {
    let syms: Vec<String> = g
        .symbols_of(file.id)
        .iter()
        .map(|&i| format!("{}:{}", g.syms[i].name, g.syms[i].hash.as_deref().unwrap_or("null")))
        .collect();
    let imps: Vec<&str> = g.imports_of(file.id).iter().map(|&i| g.imports[i].source.as_str()).collect();
    let size = if file.lang == "ts" || file.lang == "dart" {
        String::new()
    } else {
        util::num(file.size)
    };
    sha(&format!("{}#{}#{size}", syms.join("|"), imps.join("|")))
}

// A folder note goes stale when files are added, removed or renamed in it.
fn dir_hash(files: &[&FileRow]) -> String {
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_by(|a, b| cmp_utf16(a, b));
    sha(&paths.join("|"))
}

/// `writeNote(db, g, target, text)`: saves (or, for empty text, deletes) a note →
/// `{ saved, label }` / `{ removed }`.
pub fn write_note(db: &Connection, g: &Graph, target: &str, text: Option<&str>) -> Result<Value> {
    let t = resolve_note_target(g, target)?;
    let text = text.map(js::js_trim).unwrap_or("");
    if text.is_empty() {
        db.execute("DELETE FROM notes WHERE target = ?", [&t.key])?;
        return Ok(obj(vec![("removed", json!(t.key))]));
    }
    db.execute(
        "INSERT OR REPLACE INTO notes (target, text, code_hash, updated_at) VALUES (?, ?, ?, ?)",
        params![t.key, text, t.hash, js::iso_now()],
    )?;
    Ok(obj(vec![("saved", json!(t.key)), ("label", json!(t.label))]))
}

/// One note with its current status: `ok` | `stale` (code changed) | `orphan` (target gone).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteInfo {
    pub target: String,
    pub text: String,
    pub code_hash: Option<String>,
    pub updated_at: String,
    pub kind: String,
    pub label: String,
    pub status: String,
}

impl NoteInfo {
    /// `{ target, text, code_hash, updated_at, kind, label, status }`.
    pub fn to_json(&self) -> Value {
        obj(vec![
            ("target", json!(self.target)),
            ("text", json!(self.text)),
            ("code_hash", util::jstr(self.code_hash.as_deref())),
            ("updated_at", json!(self.updated_at)),
            ("kind", json!(self.kind)),
            ("label", json!(self.label)),
            ("status", json!(self.status)),
        ])
    }
}

/// `allNotes(db, g)`: every note (by target) with its status.
pub fn all_notes(db: &Connection, g: &Graph) -> Result<Vec<NoteInfo>> {
    let mut st = db.prepare("SELECT * FROM notes ORDER BY target")?;
    let rows: Vec<(String, String, Option<String>, String)> = st
        .query_map([], |r| Ok((r.get("target")?, r.get("text")?, r.get("code_hash")?, r.get("updated_at")?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .map(|(target, text, code_hash, updated_at)| {
            let stripped = re!(r"^(file|symbol):").replace_first_str(&target, "");
            let lookup = if target.starts_with("dir:") { target.as_str() } else { stripped.as_str() };
            match resolve_note_target(g, lookup) {
                Ok(t) => NoteInfo {
                    status: if t.hash == code_hash { "ok" } else { "stale" }.into(),
                    kind: t.kind,
                    label: t.label,
                    target,
                    text,
                    code_hash,
                    updated_at,
                },
                Err(_) => NoteInfo {
                    kind: target.split(':').next().unwrap_or("").to_string(),
                    label: stripped,
                    status: "orphan".into(),
                    target,
                    text,
                    code_hash,
                    updated_at,
                },
            }
        })
        .collect())
}

/// `missingNotes(db, g, limit = 15)`: where a note would help most →
/// `{ dirs: [{ target, files }], symbols: [{ target, usedInFiles }], entries: [{ target, route }] }`.
pub fn missing_notes(db: &Connection, g: &Graph, limit: i64) -> Result<Value> {
    let have: HashSet<String> = {
        let mut st = db.prepare("SELECT target FROM notes")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut dirs: IndexMap<String, i64> = IndexMap::new();
    for f in g.files.iter().filter(|f| f.lang == "ts" || f.lang == "dart") {
        let parts: Vec<&str> = f.path.split('/').collect();
        let parts = &parts[..parts.len() - 1];
        for i in 1..=parts.len().min(3) {
            *dirs.entry(parts[..i].join("/")).or_insert(0) += 1;
        }
    }
    let mut dir_list: Vec<(String, i64)> = dirs.into_iter().filter(|(d, n)| *n >= 3 && !have.contains(&format!("dir:{d}"))).collect();
    dir_list.sort_by(|a, b| a.0.split('/').count().cmp(&b.0.split('/').count()).then(b.1.cmp(&a.1)));
    let mut comps: Vec<(usize, usize)> = (0..g.syms.len())
        .filter(|&i| {
            let s = &g.syms[i];
            ["component", "widget", "hook", "function", "class"].contains(&s.kind.as_str()) && s.exported != 0 && !have.contains(&format!("symbol:{}#{}", g.sym_path(i), s.name))
        })
        .map(|i| (i, g.references_to(i).iter().map(|r| g.refs[r.r].file_id).collect::<HashSet<_>>().len()))
        .filter(|x| x.1 >= 3)
        .collect();
    comps.sort_by_key(|x| std::cmp::Reverse(x.1));
    // new Map(routes.map((r) => [r.file_id, r])): first position, last route per file
    let mut by_file: IndexMap<i64, usize> = IndexMap::new();
    for (i, r) in g.routes.iter().enumerate() {
        by_file.insert(r.file_id, i);
    }
    let entries: Vec<usize> = by_file
        .values()
        .copied()
        .filter(|&i| !have.contains(&format!("file:{}", g.path_of(g.routes[i].file_id))))
        .collect();
    let lim = limit.max(0) as usize;
    Ok(obj(vec![
        (
            "dirs",
            Value::Array(
                dir_list
                    .iter()
                    .take(lim)
                    .map(|(d, n)| obj(vec![("target", json!(format!("dir:{d}"))), ("files", json!(n))]))
                    .collect(),
            ),
        ),
        (
            "symbols",
            Value::Array(
                comps
                    .iter()
                    .take(lim)
                    .map(|(i, n)| obj(vec![("target", json!(format!("{}#{}", g.sym_path(*i), g.syms[*i].name))), ("usedInFiles", json!(n))]))
                    .collect(),
            ),
        ),
        (
            "entries",
            Value::Array(
                entries
                    .iter()
                    .take(lim)
                    .map(|&i| {
                        let r = &g.routes[i];
                        obj(vec![("target", json!(g.path_of(r.file_id))), ("route", json!(format!("{} {}", r.kind, r.path)))])
                    })
                    .collect(),
            ),
        ),
    ]))
}

/// `notesFor(db, g, file)`: notes that apply to a file — its own and every parent folder's
/// (closest first).
pub fn notes_for(db: &Connection, g: &Graph, file: &FileRow) -> Result<Vec<NoteInfo>> {
    let mut keys = vec![format!("file:{}", file.path)];
    let parts: Vec<&str> = file.path.split('/').collect();
    let parts = &parts[..parts.len() - 1];
    for i in (1..=parts.len()).rev() {
        keys.push(format!("dir:{}", parts[..i].join("/")));
    }
    let all = all_notes(db, g)?;
    let mut by_key: std::collections::HashMap<String, NoteInfo> = std::collections::HashMap::new();
    for n in all {
        by_key.insert(n.target.clone(), n);
    }
    Ok(keys.iter().filter_map(|k| by_key.get(k).cloned()).collect())
}
