//! Tasks: one board per project that the person and the AI share (Figma "wridge Tasks · Prototype · Approved").
//! The board is a file, `<project>/docs/tasks/board.json`, so it travels with the code. The `tasks` MCP tool and
//! the window's `/api/tasks` both go through [`apply`]; every change is written at once (atomic rename).
//!
//! A task may carry a suggested change (where + old → new): the AI's findings wait in Review for the person's
//! Accept / Skip. Skipped tasks keep their reason so a later check doesn't raise them again.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail};
use serde_json::{json, Map, Value};

pub const STATUSES: &[&str] = &["todo", "doing", "review", "done", "skipped"];
const ACTIVITY_MAX: usize = 40;
static LOCK: Mutex<()> = Mutex::new(());

pub fn board_path(dir: &str) -> PathBuf {
    Path::new(dir).join("docs").join("tasks").join("board.json")
}

pub fn check_dir(dir: &str) -> anyhow::Result<()> {
    let p = Path::new(dir);
    if dir.trim().is_empty() || !p.is_absolute() {
        bail!("projectDir must be an absolute path");
    }
    if !p.is_dir() {
        bail!("projectDir is not a folder: {dir}");
    }
    Ok(())
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

pub fn load(dir: &str) -> anyhow::Result<Value> {
    let path = board_path(dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("{} is not valid JSON: {e}", path.display()))?;
            if !v.get("tasks").map(Value::is_array).unwrap_or(false) {
                bail!("{} has no tasks list", path.display());
            }
            Ok(v)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"version": 1, "next": 1, "tasks": []})),
        Err(e) => Err(e.into()),
    }
}

/// Keeps a wridge folder (docs/tasks, docs/reports) out of git without touching the
/// project's own .gitignore: a `.gitignore` of `*` inside the folder ignores everything
/// in it, itself included. Written only when missing, so a project that wants to share
/// the folder empties the file (deleting it would only bring it back on the next save).
pub fn keep_out_of_git(folder: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(folder)?;
    let f = folder.join(".gitignore");
    if !f.exists() {
        std::fs::write(&f, "# made by wridge: keeps this folder out of git. To share it, empty this file (don't delete it).
*
")?;
    }
    Ok(())
}

pub fn save(dir: &str, board: &Value) -> anyhow::Result<()> {
    let path = board_path(dir);
    keep_out_of_git(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(board)? + "\n")?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// The file's change time (ms), so the window can skip a reload when nothing changed.
pub fn stamp(dir: &str) -> u64 {
    std::fs::metadata(board_path(dir))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn s(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").trim().to_string()
}
fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}
fn status_of(v: &str) -> anyhow::Result<String> {
    let v = v.trim().to_lowercase().replace(['-', ' '], "");
    let v = match v.as_str() {
        "todo" | "to" => "todo",
        "doing" | "inprogress" | "progress" => "doing",
        "review" | "inreview" => "review",
        "done" | "closed" => "done",
        "skipped" | "skip" | "wontfix" => "skipped",
        _ => bail!("status must be one of todo, doing, review, done, skipped"),
    };
    Ok(v.to_string())
}

fn subtasks(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| match x {
                    Value::String(t) if !t.trim().is_empty() => Some(json!({"text": t.trim(), "done": false})),
                    Value::Object(o) => {
                        let t = s(o.get("text"));
                        (!t.is_empty()).then(|| json!({"text": t, "done": o.get("done").and_then(Value::as_bool).unwrap_or(false)}))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn change(v: Option<&Value>) -> Option<Value> {
    let o = v?.as_object()?;
    let (old, new) = (o.get("old").and_then(Value::as_str)?, o.get("new").and_then(Value::as_str)?);
    Some(json!({"where": s(o.get("where")), "old": old, "new": new}))
}

fn log(task: &mut Value, who: &str, text: &str) {
    let at = now_iso();
    let act = task.as_object_mut().unwrap().entry("activity").or_insert_with(|| json!([]));
    if let Some(a) = act.as_array_mut() {
        a.push(json!({"at": at, "who": who, "text": text}));
        if a.len() > ACTIVITY_MAX {
            let cut = a.len() - ACTIVITY_MAX;
            a.drain(0..cut);
        }
    }
    task["updated"] = json!(at);
}

/// One task from what the AI or the window sends.
fn new_task(o: &Map<String, Value>, id: String, who: &str) -> anyhow::Result<Value> {
    let title = s(o.get("title"));
    if title.is_empty() {
        bail!("every task needs a title");
    }
    let ch = change(o.get("change"));
    let status = match o.get("status").and_then(Value::as_str) {
        Some(v) => status_of(v)?,
        None if ch.is_some() => "review".into(),
        None => "todo".into(),
    };
    let at = now_iso();
    let w = s(o.get("who"));
    let owner = if w.is_empty() { who.to_string() } else { w };
    let mut t = json!({
        "id": id, "title": title, "status": status, "description": s(o.get("description")),
        "labels": strs(o.get("labels")), "subtasks": subtasks(o.get("subtasks")), "links": strs(o.get("links")),
        "due": s(o.get("due")), "who": owner,
        "from": s(o.get("from")), "by": who, "created": at, "updated": at, "activity": [],
    });
    if let Some(c) = ch {
        t["change"] = c;
    }
    let suggested = status == "review" && t.get("change").is_some();
    log(&mut t, who, if suggested { "suggested this" } else { "added this" });
    Ok(t)
}

fn find<'a>(board: &'a mut Value, id: &str) -> anyhow::Result<&'a mut Value> {
    let id = id.trim().to_uppercase();
    let id = if id.starts_with("T-") { id } else { format!("T-{id}") };
    board["tasks"].as_array_mut().unwrap().iter_mut().find(|t| t["id"].as_str() == Some(id.as_str())).ok_or_else(|| anyhow!("no task {id} — tasks list shows the ids"))
}

fn progress(t: &Value) -> String {
    let subs = t["subtasks"].as_array().map(|a| a.len()).unwrap_or(0);
    if subs == 0 {
        return String::new();
    }
    let done = t["subtasks"].as_array().unwrap().iter().filter(|x| x["done"].as_bool() == Some(true)).count();
    format!(" · {done}/{subs}")
}

/// One line per task (what the AI reads: short, no JSON).
pub fn line(t: &Value) -> String {
    let mut out = format!("{} [{}] {}{}", s(t.get("id")), s(t.get("status")), s(t.get("title")), progress(t));
    let labels = strs(t.get("labels"));
    if !labels.is_empty() {
        out += &format!(" · {}", labels.join(", "));
    }
    for (k, pre) in [("due", "due "), ("from", "from "), ("who", "@")] {
        let v = s(t.get(k));
        if !v.is_empty() {
            out += &format!(" · {pre}{v}");
        }
    }
    if let Some(c) = t.get("change") {
        let w = s(c.get("where"));
        out += &format!(" · change{}", if w.is_empty() { String::new() } else { format!(" at {w}") });
    }
    out
}

/// The whole task as text (get).
pub fn detail(t: &Value) -> String {
    let mut out = vec![line(t)];
    let d = s(t.get("description"));
    if !d.is_empty() {
        out.push(format!("description:\n{d}"));
    }
    if let Some(a) = t["subtasks"].as_array().filter(|a| !a.is_empty()) {
        out.push("subtasks:".into());
        for (i, x) in a.iter().enumerate() {
            out.push(format!("  {}. [{}] {}", i + 1, if x["done"].as_bool() == Some(true) { "x" } else { " " }, s(x.get("text"))));
        }
    }
    let links = strs(t.get("links"));
    if !links.is_empty() {
        out.push(format!("links: {}", links.join(" · ")));
    }
    if let Some(c) = t.get("change") {
        out.push(format!("change{}:\n- {}\n+ {}", { let w = s(c.get("where")); if w.is_empty() { w } else { format!(" at {w}") } }, s(c.get("old")), s(c.get("new"))));
    }
    let reason = s(t.get("skipReason"));
    if !reason.is_empty() {
        out.push(format!("skipped because: {reason}"));
    }
    if let Some(a) = t["activity"].as_array().filter(|a| !a.is_empty()) {
        out.push("activity:".into());
        for x in a.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev() {
            out.push(format!("  {} {}: {}", s(x.get("at")).get(5..16).unwrap_or(""), s(x.get("who")), s(x.get("text"))));
        }
    }
    out.join("\n")
}

/// Tasks matching a filter (status / label / from / query); skipped ones only when asked for.
pub fn select<'a>(board: &'a Value, args: &Value) -> anyhow::Result<Vec<&'a Value>> {
    let status = match args.get("status").and_then(Value::as_str) {
        Some("all") => Some("all".to_string()),
        Some(v) => Some(status_of(v)?),
        None => None,
    };
    let label = s(args.get("label")).to_lowercase();
    let from = s(args.get("from")).to_lowercase();
    let query = s(args.get("query")).to_lowercase();
    let ids: Vec<String> = strs(args.get("ids")).into_iter().map(|i| i.to_uppercase()).collect();
    Ok(board["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| {
            let st = s(t.get("status"));
            let ok_status = match &status {
                Some(x) if x == "all" => true,
                Some(x) => &st == x,
                None => st != "skipped",
            };
            ok_status
                && (label.is_empty() || strs(t.get("labels")).iter().any(|l| l.to_lowercase() == label))
                && (from.is_empty() || s(t.get("from")).to_lowercase().contains(&from))
                && (query.is_empty() || format!("{} {}", s(t.get("title")), s(t.get("description"))).to_lowercase().contains(&query))
                && (ids.is_empty() || ids.contains(&s(t.get("id"))))
        })
        .collect())
}

/// Runs one action. `who` is "AI" (the tool) or "you" (the window). Returns (a short answer for the AI, the board).
pub fn apply(dir: &str, args: &Value, who: &str) -> anyhow::Result<(String, Value)> {
    check_dir(dir)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut board = load(dir)?;
    let action = s(args.get("action"));
    let id = s(args.get("id"));
    let need_id = || -> anyhow::Result<()> {
        if id.is_empty() {
            bail!("{action} needs id (e.g. \"T-3\")");
        }
        Ok(())
    };
    let msg = match action.as_str() {
        "list" => {
            let list = select(&board, args)?;
            let all = board["tasks"].as_array().unwrap();
            let count = |st: &str| all.iter().filter(|t| t["status"].as_str() == Some(st)).count();
            let head = format!(
                "{} · todo {} · doing {} · review {} · done {}{}",
                board_path(dir).display().to_string().replace('\\', "/"),
                count("todo"),
                count("doing"),
                count("review"),
                count("done"),
                match count("skipped") { 0 => String::new(), n => format!(" · skipped {n}") }
            );
            if list.is_empty() {
                return Ok((format!("{head}\n(no matching tasks)"), board));
            }
            return Ok((format!("{head}\n{}", list.iter().map(|t| line(t)).collect::<Vec<_>>().join("\n")), board));
        }
        "get" => {
            need_id()?;
            let t = find(&mut board, &id)?.clone();
            return Ok((detail(&t), board));
        }
        "add" => {
            let items = args.get("tasks").and_then(Value::as_array).cloned().unwrap_or_default();
            if items.is_empty() {
                bail!("add needs tasks: [{{title, …}}]");
            }
            let mut made = vec![];
            let mut next = board["next"].as_u64().unwrap_or(1);
            for it in &items {
                let o = it.as_object().ok_or_else(|| anyhow!("each task is an object with a title"))?;
                let t = new_task(o, format!("T-{next}"), who)?;
                next += 1;
                made.push(s(t.get("id")));
                board["tasks"].as_array_mut().unwrap().push(t);
            }
            board["next"] = json!(next);
            format!("added {}", made.join(", "))
        }
        "update" => {
            need_id()?;
            let set = args.get("set").and_then(Value::as_object).cloned().ok_or_else(|| anyhow!("update needs set: {{title?, description?, labels?, …}}"))?;
            let t = find(&mut board, &id)?;
            let mut changed = vec![];
            for (k, v) in &set {
                match k.as_str() {
                    "title" | "description" | "due" | "who" | "from" => {
                        t[k] = json!(v.as_str().unwrap_or("").trim());
                    }
                    "labels" | "links" => t[k] = json!(strs(Some(v))),
                    "subtasks" => t[k] = json!(subtasks(Some(v))),
                    "change" => match change(Some(v)) {
                        Some(c) => t["change"] = c,
                        None => {
                            t.as_object_mut().unwrap().remove("change");
                        }
                    },
                    "status" => t["status"] = json!(status_of(v.as_str().unwrap_or(""))?),
                    _ => bail!("can't set {k} — title, description, status, labels, subtasks, links, due, who, from, change"),
                }
                changed.push(k.clone());
            }
            log(t, who, &format!("changed {}", changed.join(", ")));
            format!("{} updated", s(t.get("id")))
        }
        "move" => {
            need_id()?;
            let st = status_of(&s(args.get("status")))?;
            let index = args.get("index").and_then(Value::as_u64);
            let tid = {
                let t = find(&mut board, &id)?;
                let from = s(t.get("status"));
                t["status"] = json!(st);
                if st != "skipped" {
                    t.as_object_mut().unwrap().remove("skipReason");
                }
                if from != st {
                    log(t, who, &format!("moved this to {st}"));
                }
                s(t.get("id"))
            };
            // place it at `index` among its new column (the window's drag)
            if let Some(i) = index {
                let tasks = board["tasks"].as_array_mut().unwrap();
                let pos = tasks.iter().position(|t| t["id"].as_str() == Some(tid.as_str())).unwrap();
                let task = tasks.remove(pos);
                let col: Vec<usize> = tasks.iter().enumerate().filter(|(_, t)| t["status"].as_str() == Some(st.as_str())).map(|(k, _)| k).collect();
                let at = col.get(i as usize).copied().unwrap_or_else(|| col.last().map(|k| k + 1).unwrap_or(tasks.len()));
                tasks.insert(at, task);
            }
            format!("{tid} → {st}")
        }
        "tick" => {
            need_id()?;
            let n = args.get("subtask").and_then(Value::as_u64).ok_or_else(|| anyhow!("tick needs subtask (its number, from 1)"))? as usize;
            let done = args.get("done").and_then(Value::as_bool).unwrap_or(true);
            let t = find(&mut board, &id)?;
            let text = {
                let subs = t["subtasks"].as_array_mut().ok_or_else(|| anyhow!("{id} has no subtasks"))?;
                let len = subs.len();
                let x = subs.get_mut(n.wrapping_sub(1)).ok_or_else(|| anyhow!("{id} has {len} subtasks"))?;
                x["done"] = json!(done);
                s(x.get("text"))
            };
            log(t, who, &format!("{} “{text}”", if done { "ticked" } else { "unticked" }));
            format!("{}{}", s(t.get("id")), progress(t))
        }
        "note" => {
            need_id()?;
            let text = s(args.get("text"));
            if text.is_empty() {
                bail!("note needs text");
            }
            let t = find(&mut board, &id)?;
            log(t, who, &format!("“{text}”"));
            format!("noted on {}", s(t.get("id")))
        }
        "accept" => {
            need_id()?;
            let t = find(&mut board, &id)?;
            t["status"] = json!("doing");
            log(t, who, "accepted the change");
            format!("{} accepted → doing (make the change, then move it to done)", s(t.get("id")))
        }
        "skip" => {
            need_id()?;
            let reason = s(args.get("text"));
            let t = find(&mut board, &id)?;
            t["status"] = json!("skipped");
            t["skipReason"] = json!(reason);
            log(t, who, &if reason.is_empty() { "skipped this".to_string() } else { format!("skipped: {reason}") });
            format!("{} skipped", s(t.get("id")))
        }
        "remove" => {
            need_id()?;
            let tid = s(find(&mut board, &id)?.get("id"));
            board["tasks"].as_array_mut().unwrap().retain(|t| t["id"].as_str() != Some(tid.as_str()));
            format!("{tid} removed")
        }
        _ => bail!("unknown action {action}"),
    };
    save(dir, &board)?;
    Ok((msg, board))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }
    fn run(d: &str, args: Value) -> String {
        apply(d, &args, "AI").unwrap().0
    }

    #[test]
    fn add_list_move_tick_and_skip() {
        let td = dir();
        let d = td.path().to_str().unwrap();
        assert!(run(d, json!({"action": "list"})).contains("(no matching tasks)"));
        let r = run(d, json!({"action": "add", "tasks": [
            {"title": "Sign in page", "subtasks": ["design", "api", "test"], "labels": ["UI"]},
            {"title": "SlipOK has no timeout", "change": {"where": "slip.ts:88", "old": "fetch(url)", "new": "fetch(url, {signal})"}, "from": "AI check · 7 Nov"}
        ]}));
        assert_eq!(r, "added T-1, T-2");
        let l = run(d, json!({"action": "list"}));
        assert!(l.contains("T-1 [todo] Sign in page · 0/3 · UI"), "{l}");
        assert!(l.contains("T-2 [review] SlipOK has no timeout · from AI check · 7 Nov"), "{l}");
        assert_eq!(run(d, json!({"action": "tick", "id": "1", "subtask": 2})), "T-1 · 1/3");
        assert_eq!(run(d, json!({"action": "move", "id": "T-1", "status": "doing"})), "T-1 → doing");
        assert!(run(d, json!({"action": "skip", "id": "T-2", "text": "fetch has a global timeout"})).contains("skipped"));
        // skipped tasks stay out of the default list but keep their reason
        assert!(!run(d, json!({"action": "list"})).contains("T-2"));
        assert!(run(d, json!({"action": "get", "id": "T-2"})).contains("skipped because: fetch has a global timeout"));
        assert!(run(d, json!({"action": "list", "from": "ai check"})).contains("no matching"));
        assert!(run(d, json!({"action": "list", "status": "all", "from": "ai check"})).contains("T-2"));
        // a file the person can read, next to the code
        let file = std::fs::read_to_string(board_path(d)).unwrap();
        assert!(file.contains("\"next\": 3"));
    }

    #[test]
    fn update_note_accept_remove_and_errors() {
        let td = dir();
        let d = td.path().to_str().unwrap();
        run(d, json!({"action": "add", "tasks": [{"title": "a"}, {"title": "b", "change": {"old": "x", "new": "y"}}]}));
        assert_eq!(run(d, json!({"action": "update", "id": "T-1", "set": {"description": "more", "due": "10 Nov", "labels": ["Ops"]}})), "T-1 updated");
        assert_eq!(run(d, json!({"action": "note", "id": "T-1", "text": "use grey"})), "noted on T-1");
        let g = run(d, json!({"action": "get", "id": "T-1"}));
        assert!(g.contains("description:\nmore") && g.contains("“use grey”") && g.contains("due 10 Nov"), "{g}");
        assert!(run(d, json!({"action": "accept", "id": "T-2"})).contains("doing"));
        assert_eq!(run(d, json!({"action": "remove", "id": "T-1"})), "T-1 removed");
        let e = |a: Value| apply(d, &a, "AI").unwrap_err().to_string();
        assert!(e(json!({"action": "get", "id": "T-9"})).contains("no task T-9"));
        assert!(e(json!({"action": "move", "id": "T-2", "status": "later"})).contains("status must be"));
        assert!(e(json!({"action": "add", "tasks": [{"description": "no title"}]})).contains("title"));
        assert!(apply("relative/dir", &json!({"action": "list"}), "AI").unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn move_places_the_task_in_its_column() {
        let td = dir();
        let d = td.path().to_str().unwrap();
        run(d, json!({"action": "add", "tasks": [{"title": "a"}, {"title": "b"}, {"title": "c"}]}));
        run(d, json!({"action": "move", "id": "T-3", "status": "todo", "index": 0}));
        let b = load(d).unwrap();
        let order: Vec<&str> = b["tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(order, ["T-3", "T-1", "T-2"]);
    }

    #[test]
    fn folder_stays_out_of_git() {
        let d = std::env::temp_dir().join(format!("wr-gi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        save(d.to_str().unwrap(), &json!({"version": 1, "next": 1, "tasks": []})).unwrap();
        let gi = d.join("docs").join("tasks").join(".gitignore");
        assert!(std::fs::read_to_string(&gi).unwrap().lines().any(|l| l == "*"));
        // an emptied rule is the project's choice: left alone
        std::fs::write(&gi, "").unwrap();
        save(d.to_str().unwrap(), &json!({"version": 1, "next": 1, "tasks": []})).unwrap();
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), "");
        let _ = std::fs::remove_dir_all(&d);
    }
}
