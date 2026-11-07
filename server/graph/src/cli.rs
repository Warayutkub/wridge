//! `wara code-graph …` and `wara memory …` without the server (ports of `cli.mjs` and
//! `memory-cli.mjs`). They return the text the JS prints with `console.log` (lines joined with
//! `\n`, no trailing newline); the caller prints it (`println!`) when it is not empty.

use crate::build::{open_graph, update_graph, UpdateOptions};
use crate::js::json_stringify;
use crate::memory;
use crate::notes;
use crate::query::{self, QueryOptions};
use crate::re;
use crate::util::s;
use anyhow::Result;
use std::collections::HashMap;

/// `CODE_GRAPH_USAGE`.
pub const CODE_GRAPH_USAGE: &str = r#"wara code-graph — ask a project's code graph (no server needed)

  node wara.mjs code-graph update [--dir D] [--force]     build/refresh <D>/.wara/code-graph.db
  node wara.mjs code-graph overview [--dir D]             files, routes, most used components, packages
  node wara.mjs code-graph find <text>                    symbols / files / routes containing text
  node wara.mjs code-graph file <path>                    what a file declares, imports, and who imports it
  node wara.mjs code-graph used_by <target>               every place that uses it
  node wara.mjs code-graph uses <target>                  what it depends on
  node wara.mjs code-graph impact <target> [--depth 3]    everything affected by changing it
  node wara.mjs code-graph route </url> [--depth 3]       route files + the component tree it renders (Flutter: route ScreenWidget)
  node wara.mjs code-graph design                         Figma components ↔ code components (sync first: node wara.mjs code_design_sync projectDir=…)
  node wara.mjs code-graph tokens [token]                 design tokens: usage, unused, raw colors, values that differ from Figma
  node wara.mjs code-graph notes                          AI-written notes: stale ones, and where a note is missing
  node wara.mjs code-graph data                           data model: tables, RLS, who reads/writes, drift from migrations
  node wara.mjs code-graph table <name>                   columns, indexes, relations, policies, queries using it
  node wara.mjs code-graph impact <table>.<column>        queries + functions + routes affected by a schema change
  node wara.mjs code-graph note <target> "text"           save a note (purpose/rules, 1-3 sentences); "" deletes

  target: Button | components/ui/button.tsx | components/ui/button.tsx#Button | /jobs/[id]
  --dir defaults to the current folder; --limit N caps list length"#;

/// `Number(x)` for an option value (NaN when not a number).
fn js_number_of(v: &str) -> f64 {
    let t = crate::js::js_trim(v);
    if t.is_empty() {
        0.0
    } else {
        t.parse::<f64>().unwrap_or(f64::NAN)
    }
}

/// `codeGraphCli(argv)`: `cwd` is `process.cwd()`. Returns the printed text.
pub fn code_graph_cli(argv: &[String], cwd: &str) -> Result<String> {
    let mut opts: HashMap<String, Option<String>> = HashMap::new();
    let mut force = false;
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a == "--force" {
            force = true;
        } else if let Some(k) = a.strip_prefix("--") {
            i += 1;
            opts.insert(k.to_string(), argv.get(i).cloned());
        } else {
            // Git Bash rewrites "/api/jobs/1" into "C:/Program Files/Git/api/jobs/1" before the program sees it.
            pos.push(
                re!(r"^[A-Za-z]:[\\/](?:[^\\/]+[\\/])*?Git(?:[\\/]usr)?(?=[\\/])", "i")
                    .replace_first_str(a, "")
                    .replace('\\', "/"),
            );
        }
        i += 1;
    }
    let Some(query) = pos.first().cloned().filter(|q| !q.is_empty() && q != "help") else {
        return Ok(CODE_GRAPH_USAGE.to_string());
    };
    let rest = &pos[1..];
    let dir_opt = match opts.get("dir") {
        Some(Some(d)) => d.clone(),
        Some(None) => "undefined".into(),
        None => cwd.to_string(),
    };
    let dir = crate::paths::resolve(&dir_opt);
    let mut out: Vec<String> = Vec::new();
    if query == "update" {
        let info = update_graph(&dir, UpdateOptions { force })?;
        let g = &info["gitignore"];
        if g.get("added") == Some(&serde_json::Value::Bool(true)) {
            out.push(format!("added {} to {}", s(g.get("pattern")), s(g.get("gitignore"))));
        }
        out.push(format!(
            "{}\n{} files · parsed {} · removed {} · unresolved imports {} · {} ms",
            s(info.get("db")),
            s(info.get("files")),
            s(info.get("parsed")),
            s(info.get("removed")),
            s(info.get("unresolvedImports")),
            s(info.get("ms"))
        ));
        for e in crate::util::arr(info.get("parseErrors")) {
            out.push(format!("parse error: {}", s(Some(e))));
        }
        return Ok(out.join("\n"));
    }
    let (db, info) = open_graph(&dir)?;
    if query == "note" {
        let g = query::graph(&db)?;
        let target = rest.first().cloned().unwrap_or_else(|| "undefined".into());
        let words = if rest.len() > 1 { rest[1..].join(" ") } else { String::new() };
        return Ok(json_stringify(&notes::write_note(&db, &g, &target, Some(&words))?));
    }
    let g = &info["gitignore"];
    if g.get("added") == Some(&serde_json::Value::Bool(true)) {
        out.push(format!("(added {} to {})", s(g.get("pattern")), s(g.get("gitignore"))));
    }
    let target = rest.join(" ");
    let num_opt = |k: &str| -> Option<f64> {
        match opts.get(k) {
            Some(Some(v)) if !v.is_empty() => Some(js_number_of(v)),
            _ => None,
        }
    };
    let o = QueryOptions {
        depth: num_opt("depth").filter(|d| d.is_finite()).map(|d| d as i64),
        limit: num_opt("limit").filter(|d| d.is_finite()).map(|d| d as i64).unwrap_or(40),
        root: Some(dir.clone()),
    };
    out.push(query::run_query(&db, &query, if target.is_empty() { None } else { Some(&target) }, &o)?);
    Ok(out.join("\n"))
}

/// `memoryCli(argv)`: `cwd` is `process.cwd()`; `read_stdin` is called only for `brief --stdin`.
/// `brief` (the SessionStart hook) never fails: any error gives an empty answer.
pub fn memory_cli(argv: &[String], cwd: &str, read_stdin: impl FnOnce() -> String) -> Result<String> {
    let mut opts: HashMap<String, Option<String>> = HashMap::new();
    let mut stdin = false;
    let mut pos: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--stdin" {
            stdin = true;
        } else if let Some(k) = argv[i].strip_prefix("--") {
            i += 1;
            opts.insert(k.to_string(), argv.get(i).cloned());
        } else {
            pos.push(&argv[i]);
        }
        i += 1;
    }
    let cmd = pos.first().copied().unwrap_or("");
    let rest = if pos.len() > 1 { &pos[1..] } else { &[][..] };
    let dir_opt = opts.get("dir").map(|d| d.clone().unwrap_or_else(|| "undefined".into()));

    if cmd == "brief" {
        // Hook mode: never fail loudly — a broken hook must not block the session.
        let run = || -> Result<String> {
            let mut c = dir_opt.clone().unwrap_or_else(|| cwd.to_string());
            if stdin {
                let text = read_stdin();
                let v = crate::js::json_parse(if text.is_empty() { "{}" } else { &text })?;
                if let Some(x) = v.get("cwd").filter(|x| !x.is_null()) {
                    c = s(Some(x));
                }
            }
            let Some(root) = memory::find_project_root(&c) else { return Ok(String::new()) };
            let text = memory::overview(&root, true)?;
            Ok(if text.is_empty() {
                String::new()
            } else {
                format!("{text}\n(Injected by the wridge project-memory hook after compaction/resume — this is the agreed state; continue from it.)")
            })
        };
        return Ok(run().unwrap_or_default());
    }

    let dir = crate::paths::resolve(&dir_opt.or_else(|| memory::find_project_root(cwd)).unwrap_or_else(|| cwd.to_string()));
    if cmd == "overview" {
        memory::overview(&dir, false)
    } else if cmd == "find" && !rest.is_empty() {
        memory::find(&dir, &rest.join(" "), false, 8)
    } else {
        Ok("node wara.mjs memory overview [--dir D] | find <topic> [--dir D] | brief [--stdin]".into())
    }
}
