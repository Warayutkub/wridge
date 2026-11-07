//! The server's side of the code graph (legacy-node/codegraph/** → the wridge-graph crate): the projects
//! list (`code-projects.json`, at `Paths::projects_file`), the checks index.mjs runs before every graph
//! route, and the JS argument handling around the crate's typed functions (Node's TypeErrors for values
//! that are not strings, `String(x)` for the rest).
//!
//! SQLite and parsing are blocking: everything here runs through [`blocking`], off the async workers and one
//! call at a time, as on Node's single thread (two graph updates of one project at once would both insert the
//! changed files; a list read could meet a half-written `code-projects.json`). Other requests keep going.
//! Results are written with [`stringify`] (`wridge_graph::js::json_stringify`: JS number format,
//! integer-like keys first).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};
use wridge_graph as wg;

use crate::jsjson::{as_f64, js_string, number_to_string, truthy};

/// One wridge-graph call at a time (see the module doc).
static GRAPH: Mutex<()> = Mutex::new(());

/// Runs blocking graph work off the async workers, one call at a time. Never nest it (the lock is not
/// reentrant); awaits between calls (the plugin) are outside it, as Node runs other requests then.
pub async fn blocking<T, F>(f: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let _one_at_a_time = GRAPH.lock().unwrap_or_else(|e| e.into_inner());
        f()
    })
    .await
    .map_err(|e| anyhow!("{e}"))?
}

/// [`is_known`] through [`blocking`].
pub async fn known(file: &Path, dir: &Value) -> anyhow::Result<bool> {
    let (file, dir): (PathBuf, Value) = (file.to_path_buf(), dir.clone());
    blocking(move || is_known(&file, &dir)).await
}

/// `JSON.stringify(value)` for crate results.
pub fn stringify(v: &Value) -> String {
    wg::js::json_stringify(v)
}

/// How Node's ERR_INVALID_ARG_TYPE names a received value (`None` = undefined).
pub fn received(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => format!("type boolean ({b})"),
        Some(Value::Number(n)) => format!("type number ({})", number_to_string(as_f64(n))),
        Some(Value::String(s)) => {
            // Node shows at most 25 characters of a string
            let shown: String = if s.chars().count() > 28 { format!("{}...", s.chars().take(25).collect::<String>()) } else { s.clone() };
            format!("type string ('{shown}')")
        }
        Some(Value::Array(_)) => "an instance of Array".into(),
        Some(Value::Object(_)) => "an instance of Object".into(),
    }
}

/// `The "<arg>" argument must be of type string. Received …` — what `path.join` ("path") and
/// `path.resolve` ("paths[i]") throw for a value that is not a string.
pub fn path_arg_error(arg: &str, v: Option<&Value>) -> anyhow::Error {
    anyhow!("The \"{arg}\" argument must be of type string. Received {}", received(v))
}

/// A value `path.join` / `path.resolve` takes as it is: a string, else Node's TypeError.
pub fn path_string<'a>(arg: &str, v: Option<&'a Value>) -> anyhow::Result<&'a str> {
    match v {
        Some(Value::String(s)) => Ok(s),
        other => Err(path_arg_error(arg, other)),
    }
}

/// `projectDir` as `projectRoot()` / `updateGraph()` receive it: a string, or the error JS throws first
/// (`!projectDir` → "must be an absolute path"; `isAbsolute(5)` → Node's TypeError).
pub fn project_dir_arg(v: Option<&Value>) -> anyhow::Result<String> {
    match v {
        Some(Value::String(s)) => Ok(s.clone()),
        other if !truthy(other) => Err(anyhow!("projectDir must be an absolute path, got \"{}\"", js_string(other))),
        other => Err(path_arg_error("path", other)),
    }
}

/// `projectRoot(dir)` as a string (resolved, backslashes on Windows).
pub fn project_root(dir: &str) -> anyhow::Result<String> {
    Ok(wg::project_root(dir)?.to_string_lossy().into_owned())
}

/// `root.replaceAll("\\", "/").split("/").pop()`
pub fn base_name(root: &str) -> String {
    root.replace('\\', "/").rsplit('/').next().unwrap_or("").to_string()
}

fn list(file: &Path) -> Vec<Value> {
    match wg::projects::list_projects(file) {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

/// `isKnownProject(dir)` for a `dir` from the query or a JSON body: `key(dir)` (`path.resolve`) throws for
/// a value that is not a string, once the list has an entry to compare with.
pub fn is_known(file: &Path, dir: &Value) -> anyhow::Result<bool> {
    if list(file).is_empty() {
        return Ok(false);
    }
    match dir {
        Value::String(s) => Ok(wg::projects::is_known_project(file, s)),
        other => Err(path_arg_error("paths[0]", Some(other))),
    }
}

/// `removeProject(dir)`: rewrites the list without that folder (`key(dir)` throws like [`is_known`]).
pub fn remove(file: &Path, dir: Option<&Value>) -> anyhow::Result<()> {
    let d = match dir {
        Some(Value::String(s)) => s.clone(),
        other if !list(file).is_empty() => return Err(path_arg_error("paths[0]", other)),
        _ => String::new(),
    };
    wg::projects::remove_project(file, &d)
}

/// `codeFileCount(dir)`: parsed source files in the project's graph (0 for a book or a game the graph
/// cannot read, and on any error), read-only.
pub fn code_file_count(dir: &str) -> i64 {
    let file = wg::graph_file(Path::new(dir));
    if !file.exists() {
        return 0;
    }
    let Ok(db) = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else {
        return 0;
    };
    db.query_row("SELECT COUNT(*) AS n FROM files WHERE lang IN ('ts', 'dart')", [], |r| r.get::<_, i64>(0)).unwrap_or(0)
}

/// GET /api/code/projects: `listProjects().map((p) => ({ ...p, exists, hasGraph, codeFiles }))`.
pub fn projects_view(file: &Path) -> anyhow::Result<Value> {
    let Value::Array(items) = wg::projects::list_projects(file) else {
        return Err(anyhow!("listProjects(...).map is not a function"));
    };
    let mut out = Vec::with_capacity(items.len());
    for p in items {
        let dir = match &p {
            Value::Object(o) => o.get("dir").cloned(),
            Value::Null => return Err(anyhow!("Cannot read properties of null (reading 'dir')")),
            _ => None,
        };
        let d = path_string("path", dir.as_ref())?.to_string();
        let mut o = match p {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        o.insert("exists".into(), Value::Bool(!d.is_empty() && Path::new(&d).exists()));
        o.insert("hasGraph".into(), Value::Bool(wg::graph_file(Path::new(&d)).exists()));
        o.insert("codeFiles".into(), Value::from(code_file_count(&d)));
        out.push(Value::Object(o));
    }
    Ok(Value::Array(out))
}

/// `{ ...a, key: value }` for a crate result (an object).
pub fn with(mut a: Value, key: &str, value: Value) -> Value {
    if let Value::Object(o) = &mut a {
        o.insert(key.into(), value);
    }
    a
}

/// `{ name, ...b, update }` (GET /api/flows).
pub fn named(name: String, b: Value, update: Value) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), Value::String(name));
    if let Value::Object(m) = b {
        for (k, v) in m {
            o.insert(k, v);
        }
    }
    o.insert("update".into(), update);
    Value::Object(o)
}

/// The `code_graph` / `code-graph` head line when opening the graph added `.wara/` to `.gitignore`.
pub fn gitignore_notice(info: &Value) -> String {
    let g = &info["gitignore"];
    if truthy(g.get("added")) {
        format!("(added {} to {})\n", js_string(g.get("pattern")), js_string(g.get("gitignore")))
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_type_errors() {
        assert_eq!(
            path_arg_error("paths[0]", Some(&json!(5))).to_string(),
            "The \"paths[0]\" argument must be of type string. Received type number (5)"
        );
        assert_eq!(path_arg_error("path", None).to_string(), "The \"path\" argument must be of type string. Received undefined");
        assert_eq!(path_arg_error("path", Some(&json!([1]))).to_string(), "The \"path\" argument must be of type string. Received an instance of Array");
        assert_eq!(project_dir_arg(None).unwrap_err().to_string(), "projectDir must be an absolute path, got \"undefined\"");
        assert_eq!(project_dir_arg(Some(&json!(""))).unwrap(), "");
        assert_eq!(project_dir_arg(Some(&json!(0))).unwrap_err().to_string(), "projectDir must be an absolute path, got \"0\"");
        assert_eq!(
            project_dir_arg(Some(&json!({}))).unwrap_err().to_string(),
            "The \"path\" argument must be of type string. Received an instance of Object"
        );
        assert_eq!(base_name("C:\\Users\\me\\proj"), "proj");
    }

    #[test]
    fn projects_list_and_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("code-projects.json");
        assert_eq!(projects_view(&file).unwrap(), json!([]));
        assert!(!is_known(&file, &json!(5)).unwrap());
        remove(&file, None).unwrap(); // empty list: no key(dir), writes []
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]\n");
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().into_owned();
        wg::projects::add_project(&file, &d).unwrap();
        assert!(is_known(&file, &json!(d.to_uppercase())).unwrap());
        assert_eq!(
            is_known(&file, &json!(true)).unwrap_err().to_string(),
            "The \"paths[0]\" argument must be of type string. Received type boolean (true)"
        );
        let view = projects_view(&file).unwrap();
        assert_eq!(view[0]["exists"], json!(true));
        assert_eq!(view[0]["hasGraph"], json!(false));
        assert_eq!(view[0]["codeFiles"], json!(0));
        assert!(remove(&file, Some(&json!(null))).is_err());
        remove(&file, Some(&json!(d))).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]\n");
    }
}
