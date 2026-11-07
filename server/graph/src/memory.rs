//! Project memory (port of `memory.mjs`): what was agreed and where work stands.
//!   - Project decisions: `docs/decisions.md` (committed) — indexed into `.wara/memory.db`
//!   - Tasks (handoff) and working notes: only in `.wara/memory.db` (personal, gitignored)
//!   - The brain ([`crate::brain`]) — overview and find cover it too
//!
//! Queries return only what matches, so cost does not grow with the project.

#![allow(clippy::type_complexity)]

use crate::brain::{self, BRAIN_DIR};
use crate::build::project_root;
use crate::files::{ensure_gitignored, WARA_DIR};
use crate::js::{self, js_trim};
use crate::re;
use crate::util::{self, obj, split_lines};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::time::Duration;

/// `DECISIONS_FILE`.
pub const DECISIONS_FILE: &str = "docs/decisions.md";

/// The index DDL, verbatim from `memory.mjs`.
pub const SCHEMA: &str = "
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
";

fn today() -> String {
    js::iso_now()[..10].to_string()
}

struct Db {
    conn: Connection,
    gitignore: Value,
}

fn open(root: &str, create: bool) -> Result<Option<Db>> {
    let file = util::join(&[root, WARA_DIR, "memory.db"]);
    if !create && !util::exists(&file) {
        return Ok(None);
    }
    let gitignore = if create {
        ensure_gitignored(std::path::Path::new(root))?
    } else {
        json!({ "added": false })
    }; // before .wara/ exists
    std::fs::create_dir_all(util::join(&[root, WARA_DIR]))?;
    let conn = Connection::open(&file)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(Some(Db { conn, gitignore }))
}

fn get_meta(db: &Connection, k: &str) -> Result<Option<String>> {
    Ok(db
        .query_row("SELECT value FROM meta WHERE key = ?", [k], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

fn set_meta(db: &Connection, k: &str, v: &str) -> Result<()> {
    db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)", params![k, v])?;
    Ok(())
}

// ---------- docs/decisions.md ----------

/// One entry of `docs/decisions.md` (`parseDecisions`).
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub id: String,
    pub date: Option<String>,
    pub title: String,
    pub tags: String,
    pub supersedes: String,
    pub body: String,
    pub line: i64,
}

impl Decision {
    /// `{ id, date, title, tags, supersedes, body, line }`.
    pub fn to_json(&self) -> Value {
        obj(vec![
            ("id", json!(self.id)),
            ("date", util::jstr(self.date.as_deref())),
            ("title", json!(self.title)),
            ("tags", json!(self.tags)),
            ("supersedes", json!(self.supersedes)),
            ("body", json!(self.body)),
            ("line", json!(self.line)),
        ])
    }
}

/// `parseDecisions(text)`: `## D-12 · 2026-10-06 · Title` (id and date optional, so hand-written
/// entries still count), then `Tags:` / `Supersedes:` fields and body lines.
pub fn parse_decisions(text: &str) -> Vec<Decision> {
    let mut out: Vec<(Decision, Vec<String>)> = Vec::new();
    let mut n = 0;
    for (i, line) in split_lines(text).into_iter().enumerate() {
        if let Some(h) = re!(r"^##\s+(?:(D-\d+)\s*[·|–-]\s*)?(?:(\d{4}-\d{2}-\d{2})\s*[·|–-]\s*)?(.+?)\s*$").exec_str(line) {
            if line.starts_with("## ") {
                n += 1;
                out.push((
                    Decision {
                        id: h[1].clone().unwrap_or_else(|| format!("H{n}")),
                        date: h[2].clone(),
                        title: h[3].clone().unwrap_or_default(),
                        tags: String::new(),
                        supersedes: String::new(),
                        body: String::new(),
                        line: i as i64 + 1,
                    },
                    Vec::new(),
                ));
                continue;
            }
        }
        let Some((cur, body)) = out.last_mut() else { continue };
        let tl = js_trim(line);
        let f = re!(r"^\**(Tags|Supersedes|Why|Decision)\**\s*:\**\s*(.*)$", "i").exec_str(tl);
        match &f {
            Some(f) if f[1].as_deref().is_some_and(|k| k.eq_ignore_ascii_case("tags")) => cur.tags = f[2].clone().unwrap_or_default().to_lowercase(),
            Some(f) if f[1].as_deref().is_some_and(|k| k.eq_ignore_ascii_case("supersedes")) => cur.supersedes = js_trim(f[2].as_deref().unwrap_or("")).to_string(),
            _ => {
                if !tl.is_empty() {
                    body.push(tl.to_string());
                }
            }
        }
    }
    out.into_iter()
        .map(|(mut d, body)| {
            d.body = body.join("\n");
            d
        })
        .collect()
}

// Re-index docs/decisions.md only when it changed (also catches edits by hand or from git pulls).
fn sync_decisions(db: &Connection, root: &str) -> Result<()> {
    let file = util::join(&[root, DECISIONS_FILE]);
    let st = std::fs::metadata(&file).ok();
    let stamp = match &st {
        Some(st) => format!("{}:{}", js::js_number(js::mtime_ms(st)), st.len()),
        None => "none".into(),
    };
    if get_meta(db, "decisions_stamp")?.as_deref() == Some(stamp.as_str()) {
        return Ok(());
    }
    db.execute_batch("DELETE FROM decisions")?;
    if st.is_some() {
        let mut put = db.prepare("INSERT OR REPLACE INTO decisions (id, date, title, tags, body, supersedes, line) VALUES (?, ?, ?, ?, ?, ?, ?)")?;
        for d in parse_decisions(&util::read_text(&file)?) {
            put.execute(params![d.id, d.date, d.title, d.tags, d.body, d.supersedes, d.line])?;
        }
    }
    set_meta(db, "decisions_stamp", &stamp)?;
    Ok(())
}

fn superseded_ids(db: &Connection) -> Result<HashSet<String>> {
    let mut st = db.prepare("SELECT supersedes FROM decisions WHERE supersedes != ''")?;
    let rows: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .iter()
        .flat_map(|r| r.split(|c: char| c == ',' || js::is_js_space(c as u32)).filter(|x| !x.is_empty()).map(String::from))
        .collect())
}

fn next_decision_id(db: &Connection) -> Result<String> {
    let mut st = db.prepare("SELECT id FROM decisions WHERE id LIKE 'D-%'")?;
    let ids: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut max = 0f64;
    for id in ids {
        let n = js_to_number(&id[2.min(id.len())..]);
        max = if n.is_nan() || max.is_nan() { f64::NAN } else { max.max(n) };
    }
    Ok(format!("D-{}", js::js_number(max + 1.0)))
}

/// `Number(s)` for a string.
fn js_to_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

// ---------- queries ----------

#[derive(Clone, Debug)]
struct DecisionRow {
    id: String,
    date: Option<String>,
    title: String,
    tags: Option<String>,
    body: Option<String>,
    supersedes: Option<String>,
}

fn decisions(db: &Connection, sql: &str) -> Result<Vec<DecisionRow>> {
    let mut st = db.prepare(sql)?;
    let rows = st.query_map([], |r| {
        Ok(DecisionRow {
            id: r.get("id")?,
            date: r.get("date")?,
            title: r.get("title")?,
            tags: r.get("tags")?,
            body: r.get("body")?,
            supersedes: r.get("supersedes")?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn fmt_decision(d: &DecisionRow, full: bool) -> String {
    let tags = d.tags.as_deref().unwrap_or("");
    let body = d.body.as_deref().unwrap_or("");
    let sup = d.supersedes.as_deref().unwrap_or("");
    format!(
        "{} · {} · {}{}{}{}",
        d.id,
        d.date.as_deref().unwrap_or("—"),
        d.title,
        if tags.is_empty() { String::new() } else { format!(" [{tags}]") },
        if full && !body.is_empty() {
            format!("\n  {}", body.replace('\n', "\n  "))
        } else {
            String::new()
        },
        if sup.is_empty() { String::new() } else { format!("\n  (replaces {sup})") }
    )
}

#[derive(Clone, Debug)]
struct TaskRow {
    id: i64,
    name: String,
    stage: Option<String>,
    done: Option<String>,
    next: Option<String>,
    questions: Option<String>,
    status: String,
    updated_at: Option<String>,
}

impl TaskRow {
    fn to_json(&self) -> Value {
        obj(vec![
            ("id", json!(self.id)),
            ("name", json!(self.name)),
            ("stage", util::jstr(self.stage.as_deref())),
            ("done", util::jstr(self.done.as_deref())),
            ("next", util::jstr(self.next.as_deref())),
            ("questions", util::jstr(self.questions.as_deref())),
            ("status", json!(self.status)),
            ("updated_at", util::jstr(self.updated_at.as_deref())),
        ])
    }
}

fn task_rows(db: &Connection, sql: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<TaskRow>> {
    let mut st = db.prepare(sql)?;
    let rows = st.query_map(p, |r| {
        Ok(TaskRow {
            id: r.get("id")?,
            name: r.get("name")?,
            stage: r.get("stage")?,
            done: r.get("done")?,
            next: r.get("next")?,
            questions: r.get("questions")?,
            status: r.get("status")?,
            updated_at: r.get("updated_at")?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn nonempty(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|x| !x.is_empty())
}

fn task_block(db: &Connection, t: &TaskRow) -> Result<String> {
    let mut st = db.prepare("SELECT id, text FROM notes WHERE task_id = ? AND archived = 0 ORDER BY id")?;
    let notes: Vec<(i64, String)> = st.query_map([t.id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let updated = match &t.updated_at {
        Some(u) => js::slice_utf16(u, 0, 16).replacen('T', " ", 1),
        None => "null".into(),
    };
    let mut lines = vec![format!(
        "### {}{} (updated {updated})",
        t.name,
        nonempty(&t.stage).map(|s| format!(" — {s}")).unwrap_or_default()
    )];
    if let Some(d) = nonempty(&t.done) {
        lines.push(format!("Done: {d}"));
    }
    if let Some(n) = nonempty(&t.next) {
        lines.push(format!("Next: {n}"));
    }
    if let Some(q) = nonempty(&t.questions) {
        lines.push(format!("Waiting on the user: {q}"));
    }
    if !notes.is_empty() {
        lines.push(format!(
            "Agreed while working:\n{}",
            notes.iter().map(|(id, text)| format!("- (n{id}) {text}")).collect::<Vec<_>>().join("\n")
        ));
    }
    Ok(lines.join("\n"))
}

/// `overview(projectDir, { brief })`: open tasks, decision titles, brain tags. `brief` (the
/// SessionStart hook) answers `""` when there is nothing to say and creates nothing for a folder
/// without memory.
pub fn overview(project_dir: &str, brief: bool) -> Result<String> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let create = !brief || util::exists(&util::join(&[&root, DECISIONS_FILE])) || util::exists(&util::join(&[&root, BRAIN_DIR]));
    let Some(db) = open(&root, create)? else { return Ok(String::new()) };
    let db = &db.conn;
    sync_decisions(db, &root)?;
    let tasks = task_rows(db, "SELECT * FROM tasks WHERE status = 'open' ORDER BY updated_at DESC", &[])?;
    let old = superseded_ids(db)?;
    let decisions: Vec<DecisionRow> = decisions(db, "SELECT * FROM decisions ORDER BY line")?
        .into_iter()
        .filter(|d| !old.contains(&d.id))
        .collect();
    let brain = brain::brain_summary(&root)?;
    let brain_count = brain.as_ref().and_then(|b| b.get("count")).and_then(Value::as_i64).unwrap_or(0);
    if brief && tasks.is_empty() && decisions.is_empty() && brain_count == 0 {
        return Ok(String::new());
    }
    let mut lines = vec![format!("# Project memory — {}", root.replace('\\', "/"))];
    lines.push(String::new());
    lines.push(if tasks.is_empty() {
        "## Open tasks: none".into()
    } else {
        format!("## Open tasks ({})", tasks.len())
    });
    for t in tasks.iter().take(5) {
        lines.push(task_block(db, t)?);
    }
    let mut tag_count = util::Counter::default();
    for d in &decisions {
        for tag in d
            .tags
            .as_deref()
            .unwrap_or("")
            .split(|c: char| c == ',' || js::is_js_space(c as u32))
            .filter(|x| !x.is_empty())
        {
            tag_count.add(tag, 1);
        }
    }
    lines.push(String::new());
    lines.push(format!("## Project decisions ({} in {DECISIONS_FILE})", decisions.len()));
    if decisions.len() <= 30 {
        lines.extend(decisions.iter().map(|d| fmt_decision(d, false)));
    } else {
        lines.push(format!(
            "By tag: {}",
            tag_count.sorted_desc().iter().map(|(t, c)| format!("{t} {c}")).collect::<Vec<_>>().join(" · ")
        ));
        lines.push("Latest:".into());
        lines.extend(decisions[decisions.len() - 8..].iter().map(|d| fmt_decision(d, false)));
    }
    if let Some(b) = &brain {
        lines.push(String::new());
        lines.push(format!("## Brain ({} notes in {BRAIN_DIR}/)", util::s(b.get("count"))));
        let tags = util::arr(b.get("tags"));
        if !tags.is_empty() {
            lines.push(format!(
                "Tags: {}",
                tags.iter().map(|x| format!("{} {}", util::s(x.get(0)), util::s(x.get(1)))).collect::<Vec<_>>().join(" · ")
            ));
        }
        if util::t(b.get("broken")) {
            lines.push(format!("{} link(s) point at notes that don't exist.", util::s(b.get("broken"))));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Details: project_memory find <topic> (never open {DECISIONS_FILE} whole). Record agreements as they happen."
    ));
    Ok(lines.join("\n"))
}

/// `find(projectDir, query, { includeOld, limit = 8 })`: matching decisions, working notes and
/// brain notes, one line each.
pub fn find(project_dir: &str, query: &str, include_old: bool, limit: usize) -> Result<String> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let db = open(&root, true)?.unwrap();
    let db = &db.conn;
    sync_decisions(db, &root)?;
    let q = js_trim(&query.to_lowercase()).to_string();
    let words = util::words(&q);
    let score = |title: &str, tags: Option<&str>, body: Option<&str>| -> i64 {
        let t = title.to_lowercase();
        let g = tags.unwrap_or("").to_lowercase();
        let b = body.unwrap_or("").to_lowercase();
        let mut s = if format!("{t} {g} {b}").contains(&q) { 5 } else { 0 };
        for w in &words {
            s += if t.contains(w) { 3 } else { 0 } + if g.contains(w) { 2 } else { 0 } + if b.contains(w) { 1 } else { 0 };
        }
        s
    };
    let old = superseded_ids(db)?;
    let mut ds: Vec<(DecisionRow, i64)> = decisions(db, "SELECT * FROM decisions")?
        .into_iter()
        .filter(|d| include_old || !old.contains(&d.id))
        .map(|d| {
            let s = score(&d.title, d.tags.as_deref(), d.body.as_deref());
            (d, s)
        })
        .filter(|x| x.1 > 0)
        .collect();
    ds.sort_by_key(|x| std::cmp::Reverse(x.1));
    ds.truncate(limit);
    let mut st = db.prepare("SELECT n.*, t.name AS task FROM notes n LEFT JOIN tasks t ON t.id = n.task_id WHERE n.archived = 0 OR ?")?;
    let notes: Vec<(i64, String, Option<String>, i64, Option<String>)> = st
        .query_map([if include_old { 1 } else { 0 }], |r| {
            Ok((r.get("id")?, r.get("text")?, r.get("tags")?, r.get("archived")?, r.get("task")?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut ns: Vec<((i64, String, Option<String>, i64, Option<String>), i64)> = notes
        .into_iter()
        .map(|n| {
            let s = score(&n.1, n.2.as_deref(), n.4.as_deref());
            (n, s)
        })
        .filter(|x| x.1 > 0)
        .collect();
    ns.sort_by_key(|x| std::cmp::Reverse(x.1));
    ns.truncate(limit);
    let mut lines: Vec<String> = Vec::new();
    if !ds.is_empty() {
        lines.push(format!("## Decisions ({DECISIONS_FILE})"));
        for (d, _) in &ds {
            lines.push(format!("{}{}", fmt_decision(d, true), if old.contains(&d.id) { "\n  [replaced]" } else { "" }));
        }
    }
    if !ns.is_empty() {
        lines.push(String::new());
        lines.push("## Working notes".into());
        for ((id, text, _, archived, task), _) in &ns {
            lines.push(format!(
                "- (n{id}) [{}{}] {text}",
                task.as_deref().unwrap_or("no task"),
                if *archived != 0 { ", closed" } else { "" }
            ));
        }
    }
    let brain = if util::exists(&util::join(&[&root, BRAIN_DIR])) {
        brain::find_notes(&root, query, limit)?
    } else {
        Vec::new()
    };
    if !brain.is_empty() {
        lines.push(String::new());
        lines.push("## Brain notes (project_memory read <note> for the whole note)".into());
        for b in &brain {
            let tags = util::arr(b.get("tags"));
            let line = util::s(b.get("line"));
            lines.push(format!(
                "- {}{}{}",
                util::s(b.get("note")),
                if tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", util::join_values(tags, ", "))
                },
                if line.is_empty() { String::new() } else { format!(" — {line}") }
            ));
        }
    }
    Ok(if lines.is_empty() {
        format!("Nothing recorded about \"{query}\".")
    } else {
        lines.join("\n")
    })
}

// ---------- writes ----------

/// Arguments of [`decide`].
#[derive(Clone, Debug, Default)]
pub struct DecideArgs {
    pub title: String,
    pub decision: String,
    pub why: Option<String>,
    pub tags: Option<Vec<String>>,
    pub supersedes: Option<String>,
}

/// `decide(projectDir, { title, decision, why, tags, supersedes })` → `{ id, file, gitignore? }`:
/// appends an entry to `docs/decisions.md` (creating it with its header).
pub fn decide(project_dir: &str, a: &DecideArgs) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let db = open(&root, true)?.unwrap();
    sync_decisions(&db.conn, &root)?;
    let id = next_decision_id(&db.conn)?;
    let file = util::join(&[&root, DECISIONS_FILE]);
    let created = !util::exists(&file);
    if created {
        if let Some(parent) = std::path::Path::new(&file).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file, "# Project decisions\n\nAgreed choices and rules for this project, newest last. Each entry: `## D-n · date · title`, then Tags / Decision / Why (and Supersedes when it replaces an older one). Edit freely; the AI reads it through wridge `project_memory`.\n")?;
    }
    let mut entry: Vec<String> = vec![String::new(), format!("## {id} · {} · {}", today(), js_trim(&a.title))];
    if let Some(tags) = a.tags.as_ref().filter(|t| !t.is_empty()) {
        entry.push(format!("Tags: {}", tags.join(", ").to_lowercase()));
    }
    entry.push(format!("Decision: {}", js_trim(&a.decision)));
    if let Some(w) = a.why.as_deref().filter(|w| !w.is_empty()) {
        entry.push(format!("Why: {}", js_trim(w)));
    }
    if let Some(s) = a.supersedes.as_deref().filter(|s| !s.is_empty()) {
        entry.push(format!("Supersedes: {s}"));
    }
    let entry = entry.join("\n");
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).create(true).open(&file)?;
        f.write_all(format!("\n{}\n", js::js_trim_start(&entry)).as_bytes())?;
    }
    sync_decisions(&db.conn, &root)?;
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert(
        "file".into(),
        json!(format!("{DECISIONS_FILE}{}", if created { " (created — commit it with the work)" } else { "" })),
    );
    if db.gitignore.get("added") == Some(&Value::Bool(true)) {
        o.insert("gitignore".into(), json!(format!("added {} to .gitignore", util::s(db.gitignore.get("pattern")))));
    }
    Ok(Value::Object(o))
}

fn task_id(db: &Connection, name: &str, create: bool) -> Result<i64> {
    if let Some(id) = db.query_row("SELECT id FROM tasks WHERE name = ?", [name], |r| r.get::<_, i64>(0)).optional()? {
        return Ok(id);
    }
    if !create {
        bail!("No task \"{name}\" — start it with action \"handoff\"");
    }
    db.execute("INSERT INTO tasks (name, updated_at) VALUES (?, ?)", params![name, js::iso_now()])?;
    Ok(db.last_insert_rowid())
}

/// `note(projectDir, { task, text, tags })` → `{ note: "n<id>", task }`: a working note (`.wara`).
pub fn note(project_dir: &str, task: Option<&str>, text: &str, tags: Option<&[String]>) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let db = open(&root, true)?.unwrap();
    let db = &db.conn;
    let id = match task.filter(|t| !t.is_empty()) {
        Some(t) => Some(task_id(db, t, true)?),
        None => None,
    };
    db.execute(
        "INSERT INTO notes (task_id, text, tags, created_at) VALUES (?, ?, ?, ?)",
        params![id, js_trim(text), tags.unwrap_or(&[]).join(", "), js::iso_now()],
    )?;
    Ok(obj(vec![("note", json!(format!("n{}", db.last_insert_rowid()))), ("task", util::jstr(task))]))
}

/// Arguments of [`handoff`] (`None` keeps the stored value).
#[derive(Clone, Debug, Default)]
pub struct HandoffArgs {
    pub task: String,
    pub stage: Option<String>,
    pub done: Option<String>,
    pub next: Option<String>,
    pub questions: Option<String>,
    pub close: bool,
}

/// `handoff(projectDir, { task, stage, done, next, questions, close })`: create/update a task's
/// state (closing archives its notes) → the task block text.
pub fn handoff(project_dir: &str, a: &HandoffArgs) -> Result<String> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let db = open(&root, true)?.unwrap();
    let db = &db.conn;
    let id = task_id(db, &a.task, !a.close)?;
    let prev = task_rows(db, "SELECT * FROM tasks WHERE id = ?", &[&id])?.into_iter().next().unwrap();
    db.execute(
        "INSERT INTO task_history (task_id, snapshot, at) VALUES (?, ?, ?)",
        params![id, js::json_stringify(&prev.to_json()), js::iso_now()],
    )?;
    let v = |val: &Option<String>, old: &Option<String>| val.clone().or_else(|| old.clone());
    db.execute(
        "UPDATE tasks SET stage = ?, done = ?, next = ?, questions = ?, status = ?, updated_at = ? WHERE id = ?",
        params![
            v(&a.stage, &prev.stage),
            v(&a.done, &prev.done),
            v(&a.next, &prev.next),
            v(&a.questions, &prev.questions),
            if a.close { "closed" } else { "open" },
            js::iso_now(),
            id
        ],
    )?;
    if a.close {
        db.execute("UPDATE notes SET archived = 1 WHERE task_id = ?", [id])?;
    }
    let t = task_rows(db, "SELECT * FROM tasks WHERE id = ?", &[&id])?.into_iter().next().unwrap();
    if a.close {
        Ok(format!("Closed \"{}\". Its working notes are archived (find … includeOld shows them).", a.task))
    } else {
        task_block(db, &t)
    }
}

// ---------- hook entry: find the project from a working directory ----------

/// `findProjectRoot(cwd)`: the nearest folder (upwards) with `.wara/memory.db`, `docs/decisions.md`
/// or `brain/`.
pub fn find_project_root(cwd: &str) -> Option<String> {
    let mut dir = crate::paths::resolve(cwd);
    loop {
        if util::exists(&util::join(&[&dir, WARA_DIR, "memory.db"])) || util::exists(&util::join(&[&dir, DECISIONS_FILE])) || util::exists(&util::join(&[&dir, BRAIN_DIR])) {
            return Some(dir);
        }
        let parent = win_dirname(&dir);
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

// path.dirname for a resolved path
fn win_dirname(p: &str) -> String {
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let t = p.trim_end_matches(sep);
    match t.rfind(sep) {
        None => p.to_string(),
        Some(i) => {
            let head = &t[..i];
            if head.is_empty() {
                sep.to_string()
            } else if cfg!(windows) && head.len() == 2 && head.ends_with(':') {
                format!("{head}{sep}")
            } else {
                head.to_string()
            }
        }
    }
}
