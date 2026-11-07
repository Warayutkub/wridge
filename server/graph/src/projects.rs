//! Projects added in the "กราฟโค้ด" page (port of `projects.mjs`). The web view may only read
//! graphs of folders listed here. The list lives in a JSON file (`code-projects.json`), passed in
//! as `file` — the Node server keeps it next to `index.mjs`.

use crate::js::{iso_now, json_stringify_pretty};
use crate::paths;
use serde_json::{json, Value};
use std::path::Path;

fn key(dir: &str) -> String {
    paths::resolve(dir).replace('\\', "/").to_lowercase()
}

fn entry_dir(p: &Value) -> String {
    match p.get("dir") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "undefined".into(),
        Some(v) => crate::js::json_stringify(v),
    }
}

/// `listProjects()`: the parsed file, or `[]` when it is missing or not JSON.
pub fn list_projects(file: &Path) -> Value {
    crate::js::read_utf8(file).ok().and_then(|t| crate::js::json_parse(&t).ok()).unwrap_or_else(|| json!([]))
}

fn list_array(file: &Path) -> Vec<Value> {
    match list_projects(file) {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

fn write(file: &Path, list: &Value) -> anyhow::Result<()> {
    std::fs::write(file, format!("{}\n", json_stringify_pretty(list, 2)))?;
    Ok(())
}

/// `addProject(dir)`: puts the folder first (replacing an older entry for it) and returns the entry.
pub fn add_project(file: &Path, dir: &str) -> anyhow::Result<Value> {
    let k = key(dir);
    let list: Vec<Value> = list_array(file).into_iter().filter(|p| key(&entry_dir(p)) != k).collect();
    let entry = json!({ "dir": paths::resolve(dir).replace('\\', "/"), "addedAt": iso_now() });
    let mut all = vec![entry.clone()];
    all.extend(list);
    write(file, &Value::Array(all))?;
    Ok(entry)
}

/// `removeProject(dir)`.
pub fn remove_project(file: &Path, dir: &str) -> anyhow::Result<()> {
    let k = key(dir);
    let list: Vec<Value> = list_array(file).into_iter().filter(|p| key(&entry_dir(p)) != k).collect();
    write(file, &Value::Array(list))
}

/// `isKnownProject(dir)`: case-insensitive, `\` and `/` alike.
pub fn is_known_project(file: &Path, dir: &str) -> bool {
    let k = key(dir);
    list_array(file).iter().any(|p| key(&entry_dir(p)) == k)
}
