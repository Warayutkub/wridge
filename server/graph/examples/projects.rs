//! `cargo run --example projects -- <code-projects.json> <add|remove|known|list> [dir]`
//! Runs one `projects.rs` call and prints its result as JSON (the Rust side of `parity/projects.mjs`).
use std::path::Path;
use wridge_graph::js::json_stringify;
use wridge_graph::projects;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = Path::new(&a[0]);
    let dir = a.get(2).map(String::as_str).unwrap_or("");
    let out = match a[1].as_str() {
        "add" => projects::add_project(file, dir).unwrap(),
        "remove" => {
            projects::remove_project(file, dir).unwrap();
            serde_json::Value::Null
        }
        "known" => serde_json::Value::Bool(projects::is_known_project(file, dir)),
        _ => projects::list_projects(file),
    };
    println!("{}", json_stringify(&out));
}
