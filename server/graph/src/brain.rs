//! The brain (port of `brain.mjs`): a project's notes as plain Markdown files in `<project>/brain/`,
//! linked with `[[…]]` like Obsidian. The files are the truth; `.wara/brain.db` is only an index
//! built from them (same schema and stamps as Node, so either server can read and update it).
//!
//! Links: `[[Note]]` another note · `[[code:path#Name]]` · `[[figma:12:34]]` · `[[roblox:Game.Path]]`
//! · `[[other-project:Note]]`. A front-matter key holding links is a named relation
//! (`boss_of: [[CH1]]`). Deleting moves a note to `brain/.trash/`; only the user empties the trash.

use crate::build::project_root;
use crate::files::{ensure_gitignored, WARA_DIR};
use crate::js::{self, js_trim, locale_compare, slice_utf16};
use crate::jsre::JsStr;
use crate::re;
use crate::util::{self, obj, split_lines};
use anyhow::{anyhow, bail, Result};
use indexmap::IndexSet;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::Duration;

/// `BRAIN_DIR`.
pub const BRAIN_DIR: &str = "brain";
/// `TRASH_DIR`.
pub const TRASH_DIR: &str = ".trash";
const EXTERNAL: [&str; 5] = ["code", "figma", "roblox", "url", "file"];

/// The index DDL, verbatim from `brain.mjs` (SQLite keeps the text in `sqlite_master`).
pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS notes (
  path TEXT PRIMARY KEY, name TEXT NOT NULL, title TEXT NOT NULL, aliases TEXT, tags TEXT, body TEXT,
  stamp TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS links (
  src TEXT NOT NULL, kind TEXT NOT NULL, target TEXT NOT NULL, rel TEXT, line INTEGER
);
CREATE INDEX IF NOT EXISTS links_src ON links(src);
CREATE INDEX IF NOT EXISTS links_target ON links(target);
";

fn lower(s: &str) -> String {
    s.to_lowercase()
}

// ---------- parsing ----------

/// `parseFrontMatter(text)` → `(data, body, offset)`: `key: value`, `key: [a, b]`, or `key:`
/// followed by `- item` lines. `data` values are strings or string arrays.
pub fn parse_front_matter(text: &str) -> (Map<String, Value>, String, usize) {
    let js = JsStr::new(text);
    let Some(m) = re!(r"^---\r?\n([\s\S]*?)\r?\n---\r?\n?").exec(&js) else {
        return (Map::new(), text.to_string(), 0);
    };
    let mut data = Map::new();
    let mut key: Option<String> = None;
    let inner = m.str(&js, 1).unwrap_or_default();
    for line in split_lines(&inner) {
        if let Some(item) = re!(r"^\s*-\s+(.*)$").exec_str(line) {
            if let Some(k) = &key {
                let v = js_trim(item[1].as_deref().unwrap_or("")).to_string();
                let mut list = match data.get(k) {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(a)) => a.clone(),
                    Some(other) => vec![other.clone()],
                };
                list.push(json!(v));
                data.insert(k.clone(), Value::Array(list));
                continue;
            }
        }
        let Some(kv) = re!(r"^([\w฀-๿-]+)\s*:\s*(.*)$").exec_str(line) else { continue };
        let k = kv[1].clone().unwrap_or_default();
        key = Some(k.clone());
        let v = js_trim(kv[2].as_deref().unwrap_or("")).to_string();
        let value = if v.is_empty() {
            json!([])
        } else if re!(r"^\[.*\]$").test_str(&v) && !re!(r"^\[\[").test_str(&v) {
            let inner = JsStr::new(&v).slice_js(1, -1).to_string();
            Value::Array(inner.split(',').map(|s| js_trim(s).to_string()).filter(|s| !s.is_empty()).map(Value::String).collect())
        } else {
            json!(v)
        };
        data.insert(k, value);
    }
    let body = js.slice_from(m.end).to_string();
    let offset = m.whole(&js).to_string().split('\n').count() - 1;
    (data, body, offset)
}

/// One `[[link]]` of a note.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub kind: String,
    pub target: String,
    pub rel: Option<String>,
    pub line: i64,
}

/// `linkTarget(raw)` → `(kind, target)`: a note of this project, another project's note, or
/// something outside the brain.
pub fn link_target(raw: &str) -> (String, String) {
    let t = js_trim(raw);
    if let Some(i) = t.find(':').filter(|i| *i > 0) {
        let prefix = lower(&t[..i]);
        if EXTERNAL.contains(&prefix.as_str()) {
            return (prefix, js_trim(&t[i + 1..]).to_string());
        }
        return ("project".into(), format!("{}:{}", js_trim(&t[..i]), js_trim(&t[i + 1..])));
    }
    ("note".into(), t.to_string())
}

fn link_re() -> &'static crate::jsre::JsRegex {
    re!(r"\[\[([^\]|#]+)(?:#([^\]|]*))?(?:\|[^\]]*)?\]\]", "g")
}

// [[x#part]]: for a note, #part is a heading (ignored); for code it names the symbol (code:path#Name).
fn links_in(text: &str) -> Vec<(String, String)> {
    let js = JsStr::new(text);
    link_re()
        .all(&js)
        .iter()
        .map(|m| {
            let (kind, target) = link_target(&m.str(&js, 1).unwrap_or_default());
            match m.str(&js, 2).filter(|p| !p.is_empty()) {
                Some(part) if kind == "code" => (kind, format!("{target}#{}", js_trim(&part))),
                _ => (kind, target),
            }
        })
        .collect()
}

/// A parsed note (`parseNote(text, path)`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedNote {
    pub path: String,
    pub name: String,
    pub title: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub body: String,
    pub links: Vec<Link>,
}

impl ParsedNote {
    pub fn to_json(&self) -> Value {
        obj(vec![
            ("path", json!(self.path)),
            ("name", json!(self.name)),
            ("title", json!(self.title)),
            ("aliases", json!(self.aliases)),
            ("tags", json!(self.tags)),
            ("body", json!(self.body)),
            (
                "links",
                Value::Array(
                    self.links
                        .iter()
                        .map(|l| {
                            obj(vec![
                                ("kind", json!(l.kind)),
                                ("target", json!(l.target)),
                                ("rel", util::jstr(l.rel.as_deref())),
                                ("line", json!(l.line)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

fn as_list(v: Option<&Value>) -> Vec<String> {
    match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(|x| util::s(Some(x))).collect(),
        Some(x) => vec![util::s(Some(x))],
    }
}

/// `parseNote(text, path)`.
pub fn parse_note(text: &str, path: &str) -> ParsedNote {
    let (data, body, offset) = parse_front_matter(text);
    let name = re!(r"\.md$", "i").replace_first_str(path, "").split('/').next_back().unwrap_or("").to_string();
    let heading = re!(r"^#\s+(.+)$", "m").exec_str(&body);
    let mut tags: IndexSet<String> = IndexSet::new();
    for tg in as_list(data.get("tags")) {
        let t = re!(r"^#").replace_first_str(&lower(&tg), "");
        if !t.is_empty() {
            tags.insert(t);
        }
    }
    let mut links = Vec::new();
    for (i, line) in split_lines(&body).into_iter().enumerate() {
        if !re!(r"^#{1,6}\s").test_str(line) {
            let js = JsStr::new(line);
            for m in re!(r"(?:^|\s)#([\p{L}\p{M}\p{N}_/-]+)", "gu").all(&js) {
                tags.insert(lower(&m.str(&js, 1).unwrap_or_default()));
            }
        }
        for (kind, target) in links_in(line) {
            links.push(Link {
                kind,
                target,
                rel: None,
                line: (offset + i + 1) as i64,
            });
        }
    }
    for key in js::js_keys(&data) {
        if key == "tags" || key == "aliases" {
            continue;
        }
        for v in as_list(data.get(key.as_str())) {
            for (kind, target) in links_in(&v) {
                links.push(Link {
                    kind,
                    target,
                    rel: Some(key.clone()),
                    line: 1,
                });
            }
        }
    }
    ParsedNote {
        path: path.to_string(),
        title: heading.and_then(|h| h[1].clone()).map(|h| js_trim(&h).to_string()).unwrap_or_else(|| name.clone()),
        name,
        aliases: as_list(data.get("aliases")),
        tags: tags.into_iter().collect(),
        body,
        links,
    }
}

// ---------- index ----------

fn walk(dir: &str, prefix: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // .trash, .obsidian
        }
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk(&util::join(&[dir, &name]), &rel, out);
        } else if re!(r"\.md$", "i").test_str(&name) {
            out.push(rel);
        }
    }
}

fn walk_dir(base: &str) -> Vec<String> {
    let mut out = Vec::new();
    if util::exists(base) {
        walk(base, "", &mut out);
    }
    out
}

fn open_index(root: &str) -> Result<Connection> {
    ensure_gitignored(std::path::Path::new(root))?; // before .wara/ exists
    std::fs::create_dir_all(util::join(&[root, WARA_DIR]))?;
    let db = Connection::open(util::join(&[root, WARA_DIR, "brain.db"]))?;
    db.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    db.busy_timeout(Duration::from_millis(5000))?;
    db.execute_batch("PRAGMA foreign_keys = ON;")?;
    db.execute_batch(SCHEMA)?;
    Ok(db)
}

// Re-reads only notes whose file changed (edits by hand, in Obsidian, or from git all count).
fn sync(db: &Connection, root: &str) -> Result<()> {
    let base = util::join(&[root, BRAIN_DIR]);
    let files = walk_dir(&base);
    let known: Vec<(String, String)> = {
        let mut st = db.prepare("SELECT path, stamp FROM notes")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let known_map: HashMap<&str, &str> = known.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
    let drop = |p: &str| -> Result<()> {
        db.execute("DELETE FROM notes WHERE path = ?", [p])?;
        db.execute("DELETE FROM links WHERE src = ?", [p])?;
        Ok(())
    };
    for p in &files {
        let full = util::join(&[&base, p]);
        let st = std::fs::metadata(&full).map_err(|e| util::fs_error(&e, "stat", &full))?;
        let stamp = format!("{}:{}", js::js_number(js::mtime_ms(&st)), st.len());
        if known_map.get(p.as_str()) == Some(&stamp.as_str()) {
            continue;
        }
        let n = parse_note(&util::read_text(&full)?, p);
        drop(p)?;
        db.execute(
            "INSERT OR REPLACE INTO notes (path, name, title, aliases, tags, body, stamp) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![p, n.name, n.title, n.aliases.join("\n"), n.tags.join(","), n.body, stamp],
        )?;
        let mut link = db.prepare_cached("INSERT INTO links (src, kind, target, rel, line) VALUES (?, ?, ?, ?, ?)")?;
        for l in &n.links {
            link.execute(params![p, l.kind, l.target, l.rel, l.line])?;
        }
    }
    let present: std::collections::HashSet<&str> = files.iter().map(String::as_str).collect();
    for (p, _) in &known {
        if !present.contains(p.as_str()) {
            drop(p)?;
        }
    }
    Ok(())
}

fn root_of(project_dir: &str) -> Result<String> {
    Ok(project_root(project_dir)?.to_string_lossy().into_owned())
}

fn with_brain<T>(project_dir: &str, f: impl FnOnce(&Connection, &str) -> Result<T>) -> Result<T> {
    let root = root_of(project_dir)?;
    let db = open_index(&root)?;
    sync(&db, &root)?;
    f(&db, &root)
}

/// A row of the `notes` index (the columns `resolve` reads).
#[derive(Clone, Debug)]
struct Row {
    path: String,
    name: String,
    title: String,
    aliases: String,
    tags: String,
    body: String,
}

fn rows(db: &Connection) -> Result<Vec<Row>> {
    let mut st = db.prepare("SELECT path, name, title, aliases, tags, body FROM notes")?;
    let rows = st.query_map([], |r| {
        Ok(Row {
            path: r.get(0)?,
            name: r.get(1)?,
            title: r.get(2)?,
            aliases: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            tags: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            body: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// A note name (file name, title, alias or path) → its row.
fn resolve_in<'r>(rows: &'r [Row], name: &str) -> Option<&'r Row> {
    let n = lower(&re!(r"\.md$", "i").replace_first_str(js_trim(name), ""));
    rows.iter()
        .find(|r| lower(&re!(r"\.md$", "i").replace_first_str(&r.path, "")) == n)
        .or_else(|| rows.iter().find(|r| lower(&r.name) == n))
        .or_else(|| rows.iter().find(|r| lower(&r.title) == n))
        .or_else(|| rows.iter().find(|r| r.aliases.split('\n').any(|a| lower(a) == n)))
}

fn resolve(db: &Connection, name: &str) -> Result<Option<Row>> {
    Ok(resolve_in(&rows(db)?, name).cloned())
}

fn label(title: &str, name: &str) -> String {
    if lower(title) == lower(name) {
        title.to_string()
    } else {
        format!("{title} ({name})")
    }
}

// ---------- reading ----------

/// `brainSummary(projectDir)` → `{ count, tags: [[tag, n]…12], broken }`, or `None` without `brain/`.
pub fn brain_summary(project_dir: &str) -> Result<Option<Value>> {
    let root = root_of(project_dir)?;
    if !util::exists(&util::join(&[&root, BRAIN_DIR])) {
        return Ok(None);
    }
    with_brain(&root, |db, root| {
        let count: i64 = db.query_row("SELECT COUNT(*) AS n FROM notes", [], |r| r.get(0))?;
        let mut tag_count = util::Counter::default();
        {
            let mut st = db.prepare("SELECT tags FROM notes")?;
            let tags = st.query_map([], |r| r.get::<_, Option<String>>(0))?;
            for t in tags {
                for tag in t?.unwrap_or_default().split(',').filter(|x| !x.is_empty()) {
                    tag_count.add(tag, 1);
                }
            }
        }
        let code_broken = with_code_graph(root, |gdb| -> Result<usize> {
            let mut st = db.prepare("SELECT target FROM links WHERE kind = 'code'")?;
            let targets: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            let mut n = 0;
            for t in targets {
                if !code_exists(root, &t, gdb)? {
                    n += 1;
                }
            }
            Ok(n)
        })?;
        let broken = broken_links(db)?.len() + code_broken;
        let tags: Vec<Value> = tag_count.sorted_desc().into_iter().take(12).map(|(t, n)| json!([t, n])).collect();
        Ok(Some(obj(vec![("count", json!(count)), ("tags", Value::Array(tags)), ("broken", json!(broken))])))
    })
}

// (src, target, line) of note links whose target does not resolve.
fn broken_links(db: &Connection) -> Result<Vec<(String, String, Option<i64>)>> {
    let all = rows(db)?;
    let mut st = db.prepare("SELECT src, target, line FROM links WHERE kind = 'note'")?;
    let links: Vec<(String, String, Option<i64>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(links.into_iter().filter(|l| resolve_in(&all, &l.1).is_none()).collect())
}

/// `findNotes(projectDir, query, { limit = 8 })` → `[{ note, path, tags, line }]`: one line per note.
pub fn find_notes(project_dir: &str, query: &str, limit: usize) -> Result<Vec<Value>> {
    if !util::exists(&util::join(&[&root_of(project_dir)?, BRAIN_DIR])) {
        return Ok(Vec::new()); // no brain: nothing to search, and no index made
    }
    with_brain(project_dir, |db, _| {
        let q = js_trim(&lower(query)).to_string();
        let words = util::words(&q);
        let mut scored: Vec<(Row, i64, String)> = Vec::new();
        for r in rows(db)? {
            let head = lower(&format!("{} {} {}", r.title, r.name, r.aliases));
            let tags = lower(&r.tags);
            let body = lower(&r.body);
            let mut sc = if format!("{head} {tags} {body}").contains(&q) { 5 } else { 0 };
            for w in &words {
                sc += if head.contains(w) { 3 } else { 0 } + if tags.contains(w) { 2 } else { 0 } + if body.contains(w) { 1 } else { 0 };
            }
            // the best matching line, so the answer stays short
            let mut best: Option<(&str, usize)> = None;
            for l in split_lines(&r.body) {
                if js_trim(l).is_empty() || re!(r"^#\s").test_str(l) {
                    continue;
                }
                let ll = lower(l);
                let c = words.iter().filter(|w| ll.contains(**w)).count();
                if best.is_none_or(|b| c > b.1) {
                    best = Some((l, c));
                }
            }
            let line = match best {
                Some((l, c)) if c > 0 => slice_utf16(js_trim(l), 0, 160),
                _ => String::new(),
            };
            scored.push((r, sc, line));
        }
        let mut hits: Vec<(Row, i64, String)> = scored.into_iter().filter(|x| x.1 > 0).collect();
        hits.sort_by_key(|x| std::cmp::Reverse(x.1));
        Ok(hits
            .into_iter()
            .take(limit)
            .map(|(r, _, line)| {
                obj(vec![
                    ("note", json!(label(&r.title, &r.name))),
                    ("path", json!(format!("{BRAIN_DIR}/{}", r.path))),
                    ("tags", json!(if r.tags.is_empty() { Vec::new() } else { r.tags.split(',').collect::<Vec<_>>() })),
                    ("line", json!(line)),
                ])
            })
            .collect())
    })
}

/// `readNote(projectDir, name)` → `{ note, path, text, links, backlinks, backlinkPaths }`.
pub fn read_note(project_dir: &str, name: &str) -> Result<Value> {
    with_brain(project_dir, |db, root| {
        let all = rows(db)?;
        let Some(r) = resolve_in(&all, name).cloned() else {
            bail!("No note \"{name}\" in {BRAIN_DIR}/ — project_memory find lists what exists");
        };
        let mut st = db.prepare("SELECT kind, target, rel FROM links WHERE src = ?")?;
        let links: Vec<(String, String, Option<String>)> = st.query_map([&r.path], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (kind, target, rel) in links {
            let mut o = Map::new();
            o.insert("kind".into(), json!(kind));
            o.insert("target".into(), json!(target));
            o.insert("rel".into(), util::jstr(rel.as_deref()));
            if kind == "code" {
                if with_code_graph(root, |gdb| Ok(!code_exists(root, &target, gdb)?))? {
                    o.insert("missing".into(), json!(true));
                }
            } else if kind == "note" {
                match resolve_in(&all, &target) {
                    Some(hit) => {
                        o.insert("target".into(), json!(hit.name));
                    }
                    None => {
                        o.insert("missing".into(), json!(true));
                    }
                }
            }
            out.push(Value::Object(o));
        }
        let mut keys: std::collections::HashSet<String> = [r.name.clone(), r.title.clone(), re!(r"\.md$", "i").replace_first_str(&r.path, "")]
            .iter()
            .map(|k| lower(k))
            .collect();
        for a in r.aliases.split('\n').filter(|a| !a.is_empty()) {
            keys.insert(lower(a));
        }
        let mut st = db.prepare("SELECT src, target FROM links WHERE kind = 'note'")?;
        let note_links: Vec<(String, String)> = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut back_paths: IndexSet<String> = IndexSet::new();
        for (src, target) in note_links {
            if keys.contains(&lower(&target)) && src != r.path {
                back_paths.insert(src);
            }
        }
        let mut back_rows: Vec<(String, String, String)> = Vec::new();
        for p in back_paths {
            let row: Option<(String, String, String)> = db
                .query_row("SELECT path, name, title FROM notes WHERE path = ?", [&p], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))
                .optional()?;
            if let Some(row) = row {
                back_rows.push(row);
            }
        }
        let text = util::read_text(&util::join(&[root, BRAIN_DIR, &r.path]))?;
        Ok(obj(vec![
            ("note", json!(label(&r.title, &r.name))),
            ("path", json!(format!("{BRAIN_DIR}/{}", r.path))),
            ("text", json!(text)),
            ("links", Value::Array(out)),
            ("backlinks", json!(back_rows.iter().map(|b| label(&b.2, &b.1)).collect::<Vec<_>>())),
            ("backlinkPaths", json!(back_rows.iter().map(|b| b.0.clone()).collect::<Vec<_>>())),
        ]))
    })
}

// the first line of text, links shown as their words
fn link_words(line: &str) -> String {
    let js = JsStr::new(line);
    re!(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]", "g")
        .replace_all_with(&js, |m| m.get(&js, 2).or_else(|| m.get(&js, 1)).unwrap_or_default())
        .to_string()
}

/// `brainGraph(projectDir)` → `{ nodes, edges, trash }`: everything for the web page's graph.
pub fn brain_graph(project_dir: &str) -> Result<Value> {
    let out = with_brain(project_dir, |db, root| {
        with_code_graph(root, |gdb| {
            let mut st = db.prepare("SELECT path, name, title, tags, body FROM notes ORDER BY path")?;
            let notes: Vec<(String, String, String, String, String)> = st
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let first_line = |body: &str| -> String {
                let l = split_lines(body).into_iter().map(js_trim).find(|l| !l.is_empty() && !l.starts_with('#'));
                match l {
                    Some(l) => {
                        let w = link_words(l);
                        slice_utf16(&re!(r"\*\*|`", "g").replace_all_str(&w, ""), 0, 140)
                    }
                    None => String::new(),
                }
            };
            let mut nodes: Vec<Value> = notes
                .iter()
                .map(|(path, name, title, tags, body)| {
                    obj(vec![
                        ("id", json!(format!("note:{path}"))),
                        ("kind", json!("note")),
                        ("label", json!(title)),
                        ("name", json!(name)),
                        ("path", json!(path)),
                        ("tags", json!(if tags.is_empty() { Vec::new() } else { tags.split(',').collect::<Vec<_>>() })),
                        ("line", json!(first_line(body))),
                    ])
                })
                .collect();
            let mut seen: std::collections::HashSet<String> = notes.iter().map(|n| format!("note:{}", n.0)).collect();
            let all = rows(db)?;
            let mut edges = Vec::new();
            let mut st = db.prepare("SELECT src, kind, target, rel FROM links")?;
            let links: Vec<(String, String, String, Option<String>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
            for (src, kind, target, rel) in links {
                let to;
                if kind == "note" {
                    match resolve_in(&all, &target) {
                        Some(hit) => to = format!("note:{}", hit.path),
                        None => {
                            to = format!("missing:{target}");
                            if seen.insert(to.clone()) {
                                nodes.push(obj(vec![("id", json!(to)), ("kind", json!("missing")), ("label", json!(target))]));
                            }
                        }
                    }
                } else {
                    to = format!("{kind}:{target}");
                    if seen.insert(to.clone()) {
                        let mut o = Map::new();
                        o.insert("id".into(), json!(to));
                        o.insert("kind".into(), json!(kind));
                        o.insert("label".into(), json!(target));
                        if kind == "code" && !code_exists(root, &target, gdb)? {
                            o.insert("missing".into(), json!(true));
                        }
                        nodes.push(Value::Object(o));
                    }
                }
                edges.push(obj(vec![("from", json!(format!("note:{src}"))), ("to", json!(to)), ("rel", util::jstr(rel.as_deref()))]));
            }
            Ok((nodes, edges))
        })
    })?;
    Ok(obj(vec![
        ("nodes", Value::Array(out.0)),
        ("edges", Value::Array(out.1)),
        ("trash", list_trash(project_dir)?),
    ]))
}

// ---------- writing ----------

fn safe_name(name: &str) -> String {
    let n = re!(r"\.md$", "i").replace_first_str(js_trim(name), "");
    let n = re!(r#"[<>:"|?*\\]+"#, "g").replace_all_str(&n, "-");
    let n = re!(r"^\/+|\/+$", "g").replace_all_str(&n, "");
    re!(r"\.\.+", "g").replace_all_str(&n, ".")
}

/// Options of [`write_note`] (`{ text, mode = "create", tags, keepOld = true }`).
#[derive(Clone, Debug)]
pub struct WriteOptions {
    /// The note text (`String(text ?? "")` in JS).
    pub text: Option<String>,
    /// `create` (fails if it exists) | `replace` | `append`; other values overwrite without trash.
    pub mode: String,
    pub tags: Option<Vec<String>>,
    /// replace: copy the old text to the trash first (the web editor saves in place: `false`).
    pub keep_old: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions {
            text: None,
            mode: "create".into(),
            tags: None,
            keep_old: true,
        }
    }
}

/// `writeNote(projectDir, name, { text, mode, tags, keepOld })` →
/// `{ saved, mode, linksToMissingNotes? }`.
pub fn write_note(project_dir: &str, name: &str, opts: &WriteOptions) -> Result<Value> {
    let root = root_of(project_dir)?;
    let text = opts.text.clone().unwrap_or_default();
    if js_trim(&text).is_empty() {
        bail!("write needs text");
    }
    let existing = with_brain(&root, |db, _| resolve(db, name))?;
    let (rel, full) = match &existing {
        Some(e) => (e.path.clone(), util::join(&[&root, BRAIN_DIR, &e.path])),
        None => {
            let rel = format!("{}.md", safe_name(name));
            if safe_name(name).is_empty() {
                bail!("A note needs a name");
            }
            let full = util::join(&[&root, BRAIN_DIR, &rel]);
            (rel, full)
        }
    };
    if let Some(e) = &existing {
        if opts.mode == "create" {
            bail!("\"{}\" already exists ({BRAIN_DIR}/{rel}) — use mode append or replace", label(&e.title, &e.name));
        }
    }
    if let Some(parent) = std::path::Path::new(&full).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut body = js_trim(&text).to_string();
    if opts.mode == "append" && existing.is_some() {
        let old = util::read_text(&full)?;
        std::fs::write(&full, format!("{}\n\n{body}\n", util::trim_end(&old)))?;
    } else {
        if opts.mode == "replace" && existing.is_some() && opts.keep_old {
            to_trash(&root, &rel, true)?;
        }
        if !re!(r"^---\r?\n").test_str(&body) && !re!(r"^#\s", "m").test_str(&body) {
            body = format!("# {}\n\n{body}", safe_name(name).split('/').next_back().unwrap_or(""));
        }
        if let Some(tags) = opts.tags.as_ref().filter(|t| !t.is_empty()) {
            if !re!(r"^---\r?\n").test_str(&body) {
                body = format!("---\ntags: [{}]\n---\n{body}", tags.join(", "));
            }
        }
        std::fs::write(&full, format!("{body}\n"))?;
    }
    with_brain(&root, |db, _| {
        let r: Option<String> = db.query_row("SELECT path FROM notes WHERE path = ?", [&rel], |x| x.get(0)).optional()?;
        let Some(path) = r else { bail!("Cannot read properties of undefined (reading 'path')") };
        let broken: Vec<String> = broken_links(db)?.into_iter().filter(|l| l.0 == rel).map(|l| l.1).collect();
        let mut o = Map::new();
        o.insert("saved".into(), json!(format!("{BRAIN_DIR}/{path}")));
        o.insert("mode".into(), json!(if existing.is_some() { opts.mode.as_str() } else { "create" }));
        if !broken.is_empty() {
            o.insert("linksToMissingNotes".into(), json!(broken));
        }
        Ok(Value::Object(o))
    })
}

// Into brain/.trash/ keeping the folder path; the file's time becomes "when it was deleted".
fn to_trash(root: &str, rel: &str, copy: bool) -> Result<String> {
    let from = util::join(&[root, BRAIN_DIR, rel]);
    let trash = util::join(&[root, BRAIN_DIR, TRASH_DIR]);
    let mut to = util::join(&[root, BRAIN_DIR, TRASH_DIR, rel]);
    let mut n = 2;
    while util::exists(&to) {
        to = util::join(&[root, BRAIN_DIR, TRASH_DIR, &re!(r"\.md$", "i").replace_first_str(rel, &format!(" ({n}).md"))]);
        n += 1;
    }
    if let Some(parent) = std::path::Path::new(&to).parent() {
        std::fs::create_dir_all(parent)?;
    }
    if copy {
        std::fs::write(&to, std::fs::read(&from).map_err(|e| util::fs_error(&e, "open", &from))?)?;
    } else {
        std::fs::rename(&from, &to).map_err(|e| util::fs_error(&e, "rename", &from))?;
    }
    util::touch_now(&to)?;
    Ok(util::slash(&crate::paths::relative(&trash, &to)))
}

/// `trashNote(projectDir, name)` → `{ trashed, restore, brokenLinksFrom? }`.
pub fn trash_note(project_dir: &str, name: &str) -> Result<Value> {
    let root = root_of(project_dir)?;
    let Some(r) = with_brain(&root, |db, _| resolve(db, name))? else {
        bail!("No note \"{name}\"")
    };
    let backlinks = read_note(&root, &r.path)?.get("backlinks").cloned().unwrap_or(json!([]));
    let at = to_trash(&root, &r.path, false)?;
    let mut o = Map::new();
    o.insert("trashed".into(), json!(format!("{BRAIN_DIR}/{}", r.path)));
    o.insert("restore".into(), json!(at));
    if backlinks.as_array().is_some_and(|b| !b.is_empty()) {
        o.insert("brokenLinksFrom".into(), backlinks);
    }
    Ok(Value::Object(o))
}

/// `listTrash(projectDir)` → `[{ path, deletedAt }]`, newest first.
pub fn list_trash(project_dir: &str) -> Result<Value> {
    let root = root_of(project_dir)?;
    let base = util::join(&[&root, BRAIN_DIR, TRASH_DIR]);
    let mut items: Vec<(String, String)> = Vec::new();
    for p in walk_dir(&base) {
        let full = util::join(&[&base, &p]);
        let st = std::fs::metadata(&full).map_err(|e| util::fs_error(&e, "stat", &full))?;
        items.push((p, util::mtime_iso(&st)));
    }
    items.sort_by(|a, b| locale_compare(&b.1, &a.1));
    Ok(Value::Array(
        items.into_iter().map(|(p, d)| obj(vec![("path", json!(p)), ("deletedAt", json!(d))])).collect(),
    ))
}

/// `restoreNote(projectDir, path)`: back to where it was (`x (2).md` → `x.md`); refuses to overwrite.
pub fn restore_note(project_dir: &str, path: &str) -> Result<Value> {
    let root = root_of(project_dir)?;
    let trash = util::join(&[&root, BRAIN_DIR, TRASH_DIR]);
    let from = util::join(&[&root, BRAIN_DIR, TRASH_DIR, path]);
    if !util::exists(&from) || util::slash(&crate::paths::relative(&trash, &from)).starts_with("..") {
        bail!("Not in the trash: {path}");
    }
    let rel = re!(r" \(\d+\)\.md$", "i").replace_first_str(path, ".md");
    let to = util::join(&[&root, BRAIN_DIR, &rel]);
    if util::exists(&to) {
        bail!("{BRAIN_DIR}/{rel} exists again — rename or trash it first");
    }
    if let Some(parent) = std::path::Path::new(&to).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&from, &to).map_err(|e| util::fs_error(&e, "rename", &from))?;
    Ok(obj(vec![("restored", json!(format!("{BRAIN_DIR}/{rel}")))]))
}

/// `emptyTrash(projectDir, path)`: the web page's "empty trash" button only (the user's own click);
/// no AI tool may call this. `path` (as listed by [`list_trash`]) removes one file, `None`/`""` all.
pub fn empty_trash(project_dir: &str, path: Option<&str>) -> Result<Value> {
    let root = root_of(project_dir)?;
    let base = util::join(&[&root, BRAIN_DIR, TRASH_DIR]);
    if let Some(p) = path.filter(|p| !p.is_empty()) {
        let file = util::join(&[&base, p]);
        if util::slash(&crate::paths::relative(&base, &file)).starts_with("..") || !util::exists(&file) {
            bail!("Not in the trash: {p}");
        }
        std::fs::remove_file(&file).map_err(|e| anyhow!("{e}"))?;
        return Ok(obj(vec![("removed", json!(1))]));
    }
    let n = list_trash(&root)?.as_array().map_or(0, Vec::len);
    if util::exists(&base) {
        std::fs::remove_dir_all(&base)?;
    }
    Ok(obj(vec![("removed", json!(n))]))
}

// ---------- links into the code graph ----------

fn norm_code(t: &str) -> String {
    re!(r"^\.\/").replace_first_str(&js_trim(t).replace('\\', "/"), "")
}

// Is a [[code:…]] target still there? path (on disk), path#Name (the graph knows the symbol, when it
// parsed the file), or a bare Name (some symbol has it). Unparsed languages are checked on disk only.
fn code_exists(root: &str, target: &str, gdb: Option<&Connection>) -> Result<bool> {
    let t = norm_code(target);
    let mut parts = t.split('#');
    let p = parts.next().unwrap_or("");
    let name = parts.next().filter(|n| !n.is_empty());
    if p.is_empty() {
        return Ok(true);
    }
    if !re!(r"[/.]").test_str(p) && name.is_none() {
        return Ok(match gdb {
            Some(g) => g.query_row("SELECT 1 FROM symbols WHERE name = ? LIMIT 1", [p], |_| Ok(())).optional()?.is_some(),
            None => true,
        });
    }
    if !util::exists(&util::join(&[root, p])) {
        return Ok(false);
    }
    if let (Some(name), Some(g)) = (name, gdb) {
        let f: Option<i64> = g.query_row("SELECT id FROM files WHERE path = ?", [p], |r| r.get(0)).optional()?;
        if let Some(fid) = f {
            let n: i64 = g.query_row("SELECT COUNT(*) AS n FROM symbols WHERE file_id = ?", [fid], |r| r.get(0))?;
            if n != 0 {
                return Ok(g
                    .query_row("SELECT 1 FROM symbols WHERE file_id = ? AND name = ?", params![fid, name], |_| Ok(()))
                    .optional()?
                    .is_some());
            }
        }
    }
    Ok(true)
}

fn with_code_graph<T>(root: &str, f: impl FnOnce(Option<&Connection>) -> Result<T>) -> Result<T> {
    let file = util::join(&[root, WARA_DIR, "code-graph.db"]);
    let gdb = if util::exists(&file) {
        Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()
    } else {
        None
    };
    f(gdb.as_ref())
}

/// `codeLinkMissing(projectDir, target)`: is a `[[code:…]]` target gone?
pub fn code_link_missing(project_dir: &str, target: &str) -> Result<bool> {
    let root = root_of(project_dir)?;
    with_code_graph(&root, |gdb| Ok(!code_exists(&root, target, gdb)?))
}

/// `notesAbout(projectDir, { path, name })` → `[{ note, line }]`: notes whose `[[code:…]]` links
/// point at this file or symbol (the signpost `code_graph` adds to its answers).
pub fn notes_about(project_dir: &str, path: Option<&str>, name: Option<&str>) -> Result<Vec<Value>> {
    let root = root_of(project_dir)?;
    if !util::exists(&util::join(&[&root, BRAIN_DIR])) {
        return Ok(Vec::new());
    }
    with_brain(&root, |db, _| {
        let p = path.filter(|p| !p.is_empty()).map(norm_code);
        let name = name.filter(|n| !n.is_empty());
        let mut hits: IndexSet<String> = IndexSet::new();
        let mut st = db.prepare("SELECT src, target FROM links WHERE kind = 'code'")?;
        let links: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (src, target) in links {
            let t = norm_code(&target);
            let mut parts = t.split('#');
            let tp = parts.next().unwrap_or("");
            let tn = parts.next();
            let by_path = p.as_deref().is_some_and(|p| tp == p && (tn.is_none_or(str::is_empty) || name.is_none() || tn == name));
            let by_name = name.is_some_and(|n| t == n || (tn == Some(n) && p.as_deref().is_none_or(|p| tp == p)));
            if by_path || by_name {
                hits.insert(src);
            }
        }
        let mut out = Vec::new();
        for src in hits {
            let row: Option<(String, String, String, Option<String>)> = db
                .query_row("SELECT path, name, title, body FROM notes WHERE path = ?", [&src], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?;
            let Some((_, nm, title, body)) = row else { continue };
            let body = body.unwrap_or_default();
            let line = split_lines(&body)
                .into_iter()
                .map(js_trim)
                .find(|x| !x.is_empty() && !x.starts_with('#') && *x != "---")
                .map(|x| slice_utf16(&link_words(x), 0, 120))
                .unwrap_or_default();
            out.push(obj(vec![("note", json!(label(&title, &nm))), ("line", json!(line))]));
        }
        Ok(out)
    })
}

/// `codeLinks(projectDir)` → `[{ target, note, notePath }]`: every `[[code:…]]` link with its note.
pub fn code_links(project_dir: &str) -> Result<Vec<Value>> {
    let root = root_of(project_dir)?;
    if !util::exists(&util::join(&[&root, BRAIN_DIR])) {
        return Ok(Vec::new());
    }
    with_brain(&root, |db, _| {
        let mut st = db.prepare("SELECT l.target, n.path, n.name, n.title FROM links l JOIN notes n ON n.path = l.src WHERE l.kind = 'code'")?;
        let rows: Vec<(String, String, String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows
            .into_iter()
            .map(|(target, path, name, title)| {
                obj(vec![
                    ("target", json!(norm_code(&target))),
                    ("note", json!(label(&title, &name))),
                    ("notePath", json!(re!(r"\.md$", "i").replace_first_str(&path, ""))),
                ])
            })
            .collect())
    })
}
