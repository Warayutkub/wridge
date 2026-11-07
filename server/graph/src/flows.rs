//! Flows (port of `flows.mjs`): how the project behaves, step by step, as small markdown files in
//! `docs/flows/` (committed). The file keeps only what the code graph can't know; the code path is
//! traced live from the graph ([`trace`]); drafts for uncovered entry points are computed on demand.
//!
//! Flows are JSON values with the JS shape (`{ title, entry, steps, states, notes, extra }`), so the
//! web page's objects go through [`format_flow`] unchanged.

use crate::data::model::find_entity;
use crate::js::{self, cmp_utf16, json_parse, json_stringify_pretty};
use crate::query::Graph;
use crate::re;
use crate::routes_next::route_matches;
use crate::util::{self, arr, join_values, obj, s, t, trim_end};
use anyhow::{bail, Result};
use indexmap::{IndexMap, IndexSet};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// `FLOWS_DIR`.
pub const FLOWS_DIR: &str = "docs/flows";
const IGNORED_FILE: &str = ".wara/flows-ignored.json";

fn http_re() -> &'static crate::jsre::JsRegex {
    re!(r"^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)$")
}

/// `hashText(text)`: sha1 hex, first 16 digits.
pub fn hash_text(text: &str) -> String {
    util::sha1_hex(text, 16)
}

fn refs_in(text: &str) -> Vec<Value> {
    let js = crate::jsre::JsStr::new(text);
    re!(r"`([^`]+)`", "g").all(&js).iter().map(|m| Value::String(m.str(&js, 1).unwrap_or_default())).collect()
}

// ---------- file format ----------

/// `parseFlow(text)` → `{ title, entry, steps, states, notes, extra }`.
pub fn parse_flow(text: &str) -> Value {
    let mut title = String::new();
    let mut entry = String::new();
    let mut steps: Vec<Map<String, Value>> = Vec::new();
    let mut states: Vec<Value> = Vec::new();
    let mut extra: Vec<(String, Vec<Value>)> = Vec::new();
    let mut section = "head";
    let mut has_step = false;
    let mut notes: Vec<&str> = Vec::new();
    let cleaned = text.replace('\r', "");
    for raw in cleaned.split('\n') {
        let line = trim_end(raw);
        if let Some(m) = re!(r"^#\s+(?:flow:\s*)?(.+)$", "i").exec_str(line) {
            if title.is_empty() {
                title = js::js_trim(m[1].as_deref().unwrap_or("")).to_string();
                continue;
            }
        }
        if let Some(m) = re!(r"^##\s+(.+)$").exec_str(line) {
            let heading = js::js_trim(m[1].as_deref().unwrap_or("")).to_string();
            let name = heading.to_lowercase();
            section = if name.starts_with("step") {
                "steps"
            } else if name.starts_with("state") {
                "states"
            } else if name.starts_with("note") {
                "notes"
            } else {
                "extra"
            };
            if section == "extra" {
                extra.push((heading, Vec::new()));
            }
            has_step = false;
            continue;
        }
        if section == "head" {
            if let Some(m) = re!(r"^entry:\s*(.*)$", "i").exec_str(line) {
                entry = js::js_trim(m[1].as_deref().unwrap_or("")).to_string();
            }
            continue;
        }
        if section == "steps" {
            if let Some(m) = re!(r"^\s*(\d+[a-z]?)[.)]\s*\[([^\]]+)\]\s*(.*)$").exec_str(line) {
                let id = m[1].clone().unwrap_or_default();
                let actor = js::js_trim(m[2].as_deref().unwrap_or("")).to_string();
                let text_part = js::js_trim(m[3].as_deref().unwrap_or("")).to_string();
                let group = if re!(r"[a-z]$").test_str(&id) {
                    json!(re!(r"[a-z]$").replace_first_str(&id, ""))
                } else {
                    Value::Null
                };
                let mut st = Map::new();
                st.insert("id".into(), json!(id));
                st.insert("group".into(), group);
                st.insert("actor".into(), json!(re!(r"^⏳\s*").replace_first_str(&actor, "")));
                st.insert("text".into(), json!(text_part));
                st.insert("wait".into(), json!(actor.contains('⏳') || text_part.starts_with('⏳')));
                st.insert("refs".into(), Value::Array(refs_in(&text_part)));
                st.insert("branches".into(), json!([]));
                steps.push(st);
                has_step = true;
            } else if has_step {
                if let Some(m) = re!(r"^\s+-\s*(.+)$").exec_str(line) {
                    let body = m[1].clone().unwrap_or_default();
                    let branch = match re!(r"^(.*?)\s*→\s*(end|step\s*(\d+[a-z]?))\s*$", "i").exec_str(&body) {
                        Some(b) => {
                            let to = if b[2].as_deref().unwrap_or("").to_lowercase() == "end" {
                                json!("end")
                            } else {
                                b[3].clone().map(Value::String).unwrap_or(Value::Null)
                            };
                            obj(vec![("label", json!(js::js_trim(b[1].as_deref().unwrap_or("")))), ("to", to)])
                        }
                        None => obj(vec![("label", json!(js::js_trim(&body))), ("to", Value::Null)]),
                    };
                    if let Some(Value::Array(b)) = steps.last_mut().and_then(|s| s.get_mut("branches")) {
                        b.push(branch);
                    }
                }
            }
            continue;
        }
        if section == "states" {
            if let Some(m) = re!(r"^\s*-?\s*`?([\w.]+)`?\s*:\s*(.+)$").exec_str(line) {
                states.push(parse_states(m[1].as_deref().unwrap_or(""), m[2].as_deref().unwrap_or("")));
            }
            continue;
        }
        if section == "notes" {
            notes.push(raw);
            continue;
        }
        if let Some(x) = extra.last_mut() {
            x.1.push(json!(raw));
        }
    }
    obj(vec![
        ("title", json!(title)),
        ("entry", json!(entry)),
        ("steps", Value::Array(steps.into_iter().map(Value::Object).collect())),
        ("states", Value::Array(states)),
        ("notes", json!(js::js_trim(&notes.join("\n")))),
        (
            "extra",
            Value::Array(extra.into_iter().map(|(h, l)| obj(vec![("heading", json!(h)), ("lines", Value::Array(l))])).collect()),
        ),
    ])
}

fn parse_states(column: &str, text: &str) -> Value {
    let mut transitions: Vec<Value> = Vec::new();
    let mut vals: IndexSet<String> = IndexSet::new();
    for chain in text.split('·') {
        let mut note: Option<String> = None;
        let js_chain = crate::jsre::JsStr::new(chain);
        let body = re!(r"\(([^)]*)\)\s*$")
            .replace_first_with(&js_chain, |m| {
                note = Some(js::js_trim(&m.str(&js_chain, 1).unwrap_or_default()).to_string());
                crate::jsre::JsStr::default()
            })
            .to_string();
        let body = js::js_trim(&body).to_string();
        let parts: Vec<Vec<String>> = body
            .split('→')
            .map(|p| p.split('|').map(|v| js::js_trim(v).to_string()).filter(|v| !v.is_empty()).collect())
            .collect();
        for v in parts.iter().flatten() {
            vals.insert(v.clone());
        }
        for i in 0..parts.len().saturating_sub(1) {
            for from in &parts[i] {
                for to in &parts[i + 1] {
                    let n = if i == parts.len() - 2 {
                        note.clone().map(Value::String).unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    };
                    transitions.push(obj(vec![("from", json!(from)), ("to", json!(to)), ("note", n)]));
                }
            }
        }
    }
    obj(vec![
        ("column", json!(column)),
        ("values", json!(vals.into_iter().collect::<Vec<_>>())),
        ("transitions", Value::Array(transitions)),
    ])
}

/// `===` between JSON values as JS compares them (primitives by value; objects never equal here).
fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Object(_)), _) | (_, Some(Value::Object(_))) | (Some(Value::Array(_)), _) | (_, Some(Value::Array(_))) => false,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// A `Map` key for a JSON value (strings and numbers stay distinct, like JS Map keys).
fn map_key(v: Option<&Value>) -> String {
    match v {
        None => "u".into(),
        Some(Value::Number(n)) => format!("n{}", js::js_number(n.as_f64().unwrap_or(f64::NAN))),
        Some(x) => format!("v{}", js::json_stringify(x)),
    }
}

/// `formatFlow(flow)`: canonical markdown. Steps keep their order; ids are renumbered (5a/5b for a
/// parallel group). `flow` is the JSON the web page sends (or [`parse_flow`]'s result).
pub fn format_flow(flow: &Value) -> Result<String> {
    let Some(Value::Array(steps)) = flow.get("steps") else {
        bail!("flow.steps is not iterable")
    };
    let mut numbering: HashMap<String, String> = HashMap::new();
    let mut n = 0;
    let mut last_group: Option<Value> = None;
    let mut letter = 0u32;
    for st in steps {
        let group = match st.get("group") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.clone()),
        };
        let key = map_key(st.get("id"));
        if group.as_ref().is_some_and(|g| t(Some(g))) && strict_eq(group.as_ref(), last_group.as_ref()) {
            letter += 1;
            numbering.insert(key, format!("{n}{}", char::from_u32(97 + letter).unwrap_or('?')));
        } else if group.as_ref().is_some_and(|g| t(Some(g))) {
            n += 1;
            letter = 0;
            numbering.insert(key, format!("{n}a"));
        } else {
            n += 1;
            numbering.insert(key, n.to_string());
        }
        last_group = group;
    }
    let mut out: Vec<String> = vec![format!("# Flow: {}", if t(flow.get("title")) { s(flow.get("title")) } else { "Untitled".into() })];
    if t(flow.get("entry")) {
        out.push(format!("entry: {}", s(flow.get("entry"))));
    }
    out.push(String::new());
    out.push("## Steps".into());
    for st in steps {
        let num = numbering.get(&map_key(st.get("id"))).cloned().unwrap_or_else(|| "undefined".into());
        out.push(
            trim_end(&format!(
                "{num}. [{}{}] {}",
                if t(st.get("wait")) { "⏳ " } else { "" },
                if t(st.get("actor")) { s(st.get("actor")) } else { "server".into() },
                s(st.get("text"))
            ))
            .to_string(),
        );
        for b in arr(st.get("branches")) {
            let to = b.get("to");
            let to = if to.and_then(Value::as_str) == Some("end") {
                "→ end".to_string()
            } else if t(to) && numbering.contains_key(&map_key(to)) {
                format!("→ step {}", numbering[&map_key(to)])
            } else {
                String::new()
            };
            out.push(trim_end(&format!("   - {}{}", s(b.get("label")), if to.is_empty() { String::new() } else { format!(" {to}") })).to_string());
        }
    }
    let states = arr(flow.get("states"));
    if !states.is_empty() {
        out.push(String::new());
        out.push("## States".into());
        for st in states {
            let trans = arr(st.get("transitions"));
            let mut items: Vec<String> = trans
                .iter()
                .map(|x| {
                    format!(
                        "{} → {}{}",
                        s(x.get("from")),
                        s(x.get("to")),
                        if t(x.get("note")) { format!(" ({})", s(x.get("note"))) } else { String::new() }
                    )
                })
                .collect();
            for v in arr(st.get("values")) {
                if !trans.iter().any(|x| strict_eq(x.get("from"), Some(v)) || strict_eq(x.get("to"), Some(v))) {
                    items.push(if v.is_null() { String::new() } else { s(Some(v)) });
                }
            }
            out.push(format!("`{}`: {}", s(st.get("column")), items.join(" · ")));
        }
    }
    if let Some(Value::String(notes)) = flow.get("notes") {
        if !js::js_trim(notes).is_empty() {
            out.push(String::new());
            out.push("## Notes".into());
            out.push(js::js_trim(notes).to_string());
        }
    }
    for x in arr(flow.get("extra")) {
        out.push(String::new());
        out.push(format!("## {}", s(x.get("heading"))));
        for l in arr(x.get("lines")) {
            out.push(if l.is_null() { String::new() } else { s(Some(l)) });
        }
    }
    Ok(format!("{}\n", trim_end(&out.join("\n"))))
}

// ---------- files ----------

fn safe_file(name: &str) -> bool {
    re!(r"^[a-z0-9][a-z0-9._-]*\.md$", "i").test_str(name)
}

/// `slugify(name)`: lower case, NFKD, runs of anything but a-z0-9 → "-", at most 60 characters.
pub fn slugify(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let lower: String = name.to_lowercase().nfkd().collect();
    let d = re!(r"[^a-z0-9]+", "g").replace_all_str(&lower, "-");
    let d = re!(r"^-+|-+$", "g").replace_all_str(&d, "");
    let d: String = d.chars().take(60).collect();
    if d.is_empty() {
        format!("flow-{}", to_base36(util::now_ms() as u64))
    } else {
        d
    }
}

fn to_base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// `readFlowFiles(root)`: every `docs/flows/*.md` (sorted), as `{ file, hash, ...parseFlow(text) }`.
pub fn read_flow_files(root: &str) -> Result<Vec<Value>> {
    let dir = util::join(&[root, FLOWS_DIR]);
    if !util::exists(&dir) {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| safe_file(n))
        .collect();
    names.sort_by(|a, b| cmp_utf16(a, b));
    let mut out = Vec::new();
    for file in names {
        let text = util::read_text(&util::join(&[&dir, &file]))?;
        let mut o = Map::new();
        o.insert("file".into(), json!(format!("{FLOWS_DIR}/{file}")));
        o.insert("hash".into(), json!(hash_text(&text)));
        if let Value::Object(p) = parse_flow(&text) {
            o.extend(p);
        }
        out.push(Value::Object(o));
    }
    Ok(out)
}

/// The flow file changed since it was opened (`FlowConflict`): `current` is its text now
/// (`None` when a new file was expected but one exists... or the reverse). The HTTP route answers 409.
#[derive(Debug, Clone)]
pub struct FlowConflict {
    pub current: Option<String>,
}

impl std::fmt::Display for FlowConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The flow file changed since it was opened")
    }
}

impl std::error::Error for FlowConflict {}

/// `saveFlowFile(root, name, text, baseHash)`: writes `docs/flows/<name>.md`. `base_hash` is the
/// hash when the file was opened (`Value::Null` = must not exist yet). A conflict is an
/// `anyhow::Error` wrapping [`FlowConflict`] (`err.downcast_ref::<FlowConflict>()`).
pub fn save_flow_file(root: &str, name: &str, text: &str, base_hash: &Value) -> Result<Value> {
    let file = re!(r"^docs\/flows\/").replace_first_str(name, "");
    if !safe_file(&file) {
        bail!("Invalid flow file name \"{name}\"");
    }
    let dir = util::join(&[root, FLOWS_DIR]);
    let path = util::join(&[&dir, &file]);
    if util::exists(&path) {
        let current = util::read_text(&path)?;
        if base_hash.as_str() != Some(hash_text(&current).as_str()) {
            return Err(FlowConflict { current: Some(current) }.into());
        }
    } else if t(Some(base_hash)) {
        return Err(FlowConflict { current: None }.into());
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, text)?;
    Ok(obj(vec![("file", json!(format!("{FLOWS_DIR}/{file}"))), ("hash", json!(hash_text(text)))]))
}

/// `readIgnored(root)`: draft ids the user dismissed (`.wara/flows-ignored.json`).
pub fn read_ignored(root: &str) -> IndexSet<String> {
    let Ok(text) = js::read_utf8(util::join(&[root, IGNORED_FILE])) else {
        return IndexSet::new();
    };
    match json_parse(&text) {
        Ok(Value::Array(a)) => a.iter().map(|v| s(Some(v))).collect(),
        Ok(Value::String(st)) => st.chars().map(String::from).collect(),
        _ => IndexSet::new(),
    }
}

/// `ignoreDraft(root, id)`.
pub fn ignore_draft(root: &str, id: &str) -> Result<()> {
    let mut set = read_ignored(root);
    set.insert(id.to_string());
    std::fs::create_dir_all(util::join(&[root, ".wara"]))?;
    let list = Value::Array(set.into_iter().map(Value::String).collect());
    std::fs::write(util::join(&[root, IGNORED_FILE]), json_stringify_pretty(&list, 1))?;
    Ok(())
}

// ---------- checking refs against the graph ----------

/// `checkRef(g, ref)` → `{ ref, ok, kind, where? }` (lists: `{ ref, ok, kind: "list", parts }`).
pub fn check_ref(g: &Graph, rf: &str) -> Value {
    let js_ref = crate::jsre::JsStr::new(rf);
    let mut parts: Vec<String> = Vec::new();
    {
        // ref.split(/\s*,\s*/)
        let mut last = 0;
        for m in re!(r"\s*,\s*", "g").all(&js_ref) {
            if m.end == m.index && m.index == last {
                continue;
            }
            parts.push(js_ref.slice(last, m.index).to_string());
            last = m.end;
        }
        parts.push(js_ref.slice_from(last).to_string());
        parts.retain(|p| !p.is_empty());
    }
    if parts.len() > 1 {
        let each: Vec<Value> = parts.iter().map(|p| check_ref(g, p)).collect();
        let ok = each.iter().all(|x| x.get("ok") == Some(&Value::Bool(true)));
        return obj(vec![("ref", json!(rf)), ("ok", json!(ok)), ("kind", json!("list")), ("parts", Value::Array(each))]);
    }
    let r = js::js_trim(&re!(r":\d+$").replace_first_str(rf, "")).to_string();
    let with_where = |ok: bool, kind: &str, wh: Option<String>| -> Value {
        let mut o = Map::new();
        o.insert("ref".into(), json!(rf));
        o.insert("ok".into(), json!(ok));
        o.insert("kind".into(), json!(kind));
        if let Some(w) = wh {
            o.insert("where".into(), json!(w));
        }
        Value::Object(o)
    };
    if let Some(m) = re!(r"^(?:(GET|POST|PUT|PATCH|DELETE)\s+|webhook\s+|cron\s+)?(\/\S*)$", "i").exec_str(&r) {
        let p = m[2].clone().unwrap_or_default();
        let hits: Vec<usize> = (0..g.routes.len()).filter(|&i| g.routes[i].path == p || route_matches(&g.routes[i].path, &p)).collect();
        let method = m[1].as_ref().map(|x| x.to_uppercase());
        let ok = !hits.is_empty()
            && (method.is_none()
                || hits
                    .iter()
                    .any(|&h| g.routes[h].kind != "route" || g.symbols_of(g.routes[h].file_id).iter().any(|&si| Some(&g.syms[si].name) == method.as_ref())));
        let wh = hits.first().and_then(|&h| g.file(g.routes[h].file_id)).map(|f| f.path.clone());
        return with_where(ok, "route", wh);
    }
    if re!(r"[/\\]").test_str(&r) && re!(r"\.\w+$").test_str(&r) {
        let f = g.file_by_path(&r);
        return with_where(f.is_some(), "file", f.map(|i| g.files[i].path.clone()));
    }
    let mut tc = r.split('.');
    let table = tc.next().unwrap_or("");
    let column = tc.next().filter(|c| !c.is_empty());
    if let Some((_, ent)) = g.data_model().and_then(|dm| find_entity(dm, table)) {
        let ok = match column {
            None => true,
            Some(c) => arr(ent.get("columns"))
                .iter()
                .any(|x| x.get("name").and_then(Value::as_str) == Some(c) || x.get("db").and_then(Value::as_str) == Some(c)),
        };
        return with_where(ok, if column.is_some() { "column" } else { "table" }, Some(s(ent.get("name"))));
    }
    if let Some(si) = (0..g.syms.len()).find(|&i| g.syms[i].name == r && g.syms[i].kind != "alias") {
        return with_where(true, "symbol", Some(g.where_(si)));
    }
    with_where(false, if re!(r"^[a-z_][\w]*$", "i").test_str(&r) { "unknown" } else { "text" }, None)
}

/// `checkFlow(g, flow)` → `{ steps: [{ id, refs }], stale: [{ step, ref }] }`.
pub fn check_flow(g: &Graph, flow: &Value) -> Value {
    let mut steps = Vec::new();
    let mut stale = Vec::new();
    for st in arr(flow.get("steps")) {
        let refs: Vec<Value> = arr(st.get("refs")).iter().map(|r| check_ref(g, &s(Some(r)))).collect();
        for r in &refs {
            if r.get("ok") != Some(&Value::Bool(true)) && r.get("kind").and_then(Value::as_str) != Some("text") {
                stale.push(obj(vec![
                    ("step", st.get("id").cloned().unwrap_or(Value::Null)),
                    ("ref", r.get("ref").cloned().unwrap_or(Value::Null)),
                ]));
            }
        }
        steps.push(obj(vec![("id", st.get("id").cloned().unwrap_or(Value::Null)), ("refs", Value::Array(refs))]));
    }
    obj(vec![("steps", Value::Array(steps)), ("stale", Value::Array(stale))])
}

// ---------- trace: what the code really does from an entry point ----------

struct Start {
    routes: Vec<usize>,
    syms: Vec<usize>,
    error: Option<String>,
}

fn start_symbols(g: &Graph, target: &str) -> Start {
    let tt = re!(r":\d+$").replace_first_str(js::js_trim(target), "");
    let m = re!(r"^(GET|POST|PUT|PATCH|DELETE)\s+(\/\S*)$", "i").exec_str(&tt);
    let path = m.as_ref().map(|m| m[2].clone().unwrap_or_default()).unwrap_or_else(|| tt.clone());
    if path.starts_with('/') {
        let routes: Vec<usize> = (0..g.routes.len())
            .filter(|&i| g.routes[i].kind != "layout" && (g.routes[i].path == path || route_matches(&g.routes[i].path, &path)))
            .collect();
        let mut out = Vec::new();
        for &ri in &routes {
            let r = &g.routes[ri];
            let syms: Vec<usize> = g.symbols_of(r.file_id).iter().copied().filter(|&i| g.syms[i].kind != "alias").collect();
            if r.kind == "route" {
                let want = m.as_ref().map(|m| m[1].clone().unwrap_or_default().to_uppercase());
                out.extend(
                    syms.into_iter()
                        .filter(|&i| g.syms[i].exported != 0 && http_re().test_str(&g.syms[i].name) && want.as_ref().is_none_or(|w| g.syms[i].name == *w)),
                );
            } else {
                out.extend(syms.into_iter().filter(|&i| g.syms[i].is_default != 0).take(1));
            }
        }
        return Start { routes, syms: out, error: None };
    }
    let hit = g.lookup(&tt, false);
    if let Some(sy) = hit.symbols {
        return Start {
            routes: Vec::new(),
            syms: sy,
            error: None,
        };
    }
    if let Some(f) = hit.file {
        let syms = g
            .symbols_of(g.files[f].id)
            .iter()
            .copied()
            .filter(|&i| g.syms[i].exported != 0 && g.syms[i].kind != "alias")
            .collect();
        return Start {
            routes: Vec::new(),
            syms,
            error: None,
        };
    }
    Start {
        routes: Vec::new(),
        syms: Vec::new(),
        error: hit.error,
    }
}

/// `trace(g, target, { depth = 3, max = 40 })`: calls (project functions, depth-limited), table
/// reads/writes and packages reached from the target.
pub fn trace(g: &Graph, target: &str, depth: i64, max: usize) -> Result<Value> {
    let start = start_symbols(g, target);
    if start.syms.is_empty() {
        return Ok(obj(vec![
            ("target", json!(target)),
            ("error", json!(start.error.unwrap_or_else(|| format!("Nothing to trace for \"{target}\"")))),
            ("calls", json!([])),
            ("data", json!([])),
            ("packages", json!([])),
        ]));
    }
    let mut seen: HashSet<usize> = start.syms.iter().copied().collect();
    let mut calls: Vec<Value> = Vec::new();
    let mut packages: IndexSet<String> = IndexSet::new();
    let mut frontier: Vec<(usize, i64)> = start.syms.iter().map(|&s| (s, 0)).collect();
    while !frontier.is_empty() && calls.len() < max {
        let mut next = Vec::new();
        for &(sym, level) in &frontier {
            let sy = &g.syms[sym];
            for &ri in g.refs_of(sy.file_id) {
                let r = &g.refs[ri];
                if r.symbol_id != Some(sy.id) || r.kind == "type" {
                    continue;
                }
                let b = g.binding_target(sy.file_id, &r.local);
                if let Some(e) = b.as_ref().and_then(|b| b.external.clone()) {
                    packages.insert(e);
                    continue;
                }
                let Some(s2) = b.and_then(|b| b.symbol) else { continue };
                if seen.contains(&s2) || !["function", "hook", "class"].contains(&g.syms[s2].kind.as_str()) {
                    continue;
                }
                seen.insert(s2);
                calls.push(obj(vec![
                    ("from", json!(sy.name)),
                    ("name", json!(g.syms[s2].name)),
                    ("kind", json!(g.syms[s2].kind)),
                    ("file", json!(g.sym_path(s2))),
                    ("line", util::jnum(g.syms[s2].line)),
                    ("level", json!(level + 1)),
                ]));
                if level + 1 < depth {
                    next.push((s2, level + 1));
                }
            }
        }
        frontier = next;
    }
    let start_files: HashSet<i64> = start.syms.iter().map(|&s| g.syms[s].file_id).collect();
    let mut data = Vec::new();
    let mut st = g.db.prepare("SELECT * FROM data_refs WHERE op != 'ddl'")?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>("file_id")?,
            r.get::<_, String>("entity")?,
            r.get::<_, String>("op")?,
            r.get::<_, Option<String>>("columns")?,
            r.get::<_, Option<i64>>("line")?,
        ))
    })?;
    for row in rows {
        let (file_id, entity, op, columns, line) = row?;
        let owner = g.owner_at(file_id, Some(line.unwrap_or(0)));
        let inside = match owner {
            Some(o) => seen.contains(&o),
            None => start_files.contains(&file_id),
        };
        if !inside {
            continue;
        }
        let mut o = Map::new();
        o.insert("entity".into(), json!(entity));
        o.insert("op".into(), json!(op));
        o.insert(
            "columns".into(),
            match columns.filter(|c| !c.is_empty()) {
                Some(c) => json_parse(&c)?,
                None => Value::Null,
            },
        );
        if let Some(f) = g.file(file_id) {
            o.insert("file".into(), json!(f.path));
        }
        o.insert("line".into(), util::jnum(line));
        o.insert("by".into(), owner.map(|o| json!(g.syms[o].name)).unwrap_or(Value::Null));
        data.push(Value::Object(o));
    }
    Ok(obj(vec![
        ("target", json!(target)),
        (
            "start",
            Value::Array(
                start
                    .syms
                    .iter()
                    .map(|&s| obj(vec![("name", json!(g.syms[s].name)), ("file", json!(g.sym_path(s))), ("line", util::jnum(g.syms[s].line))]))
                    .collect(),
            ),
        ),
        (
            "routes",
            Value::Array(start.routes.iter().map(|&r| json!(format!("{} {}", g.routes[r].kind, g.routes[r].path))).collect()),
        ),
        ("calls", Value::Array(calls)),
        ("data", Value::Array(data)),
        ("packages", json!(packages.into_iter().collect::<Vec<_>>())),
    ]))
}

/// `statusValues(g, root, tableNames)`: values a status-like column is set to near the code that
/// writes it (`status: 'paid'`, `status = 'paid'`).
pub fn status_values(g: &Graph, root: &str, table_names: &IndexSet<String>) -> Result<Vec<Value>> {
    let mut found: IndexMap<String, IndexSet<String>> = IndexMap::new();
    let mut st = g.db.prepare("SELECT * FROM data_refs WHERE op = 'write'")?;
    let rows: Vec<(i64, String, Option<i64>)> = st
        .query_map([], |r| Ok((r.get("file_id")?, r.get("entity")?, r.get("line")?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, e, _)| table_names.contains(e))
        .collect();
    let mut cache: HashMap<String, Vec<String>> = HashMap::new();
    for (file_id, entity, line) in rows {
        let Some(f) = g.file(file_id) else { continue };
        let lines = cache.entry(f.path.clone()).or_insert_with(|| {
            js::read_utf8(util::join(&[root, &f.path]))
                .map(|t| t.split('\n').map(String::from).collect())
                .unwrap_or_default()
        });
        let l = line.unwrap_or(1);
        let from = (l - 3).max(0) as usize;
        let to = ((l + 8).max(0) as usize).min(lines.len());
        if from >= to {
            continue;
        }
        for text in &lines[from..to] {
            let js_line = crate::jsre::JsStr::new(text);
            for m in re!(r#"\b(\w*status\w*|state)\b["']?\s*(?::|=)\s*['"]([a-z][\w-]{1,30})['"]"#, "gi").all(&js_line) {
                let key = format!("{entity}.{}", m.str(&js_line, 1).unwrap_or_default());
                found.entry(key).or_default().insert(m.str(&js_line, 2).unwrap_or_default());
            }
        }
    }
    Ok(found
        .into_iter()
        .map(|(column, vals)| {
            let mut v: Vec<String> = vals.into_iter().collect();
            v.sort_by(|a, b| cmp_utf16(a, b));
            obj(vec![("column", json!(column)), ("values", json!(v)), ("transitions", json!([]))])
        })
        .collect())
}

// ---------- drafts: entry points no flow covers yet ----------

fn actor_for(path: &str) -> &'static str {
    if re!(r"\/cron\b|\/jobs?\b").test_str(path) {
        "cron"
    } else if re!(r"webhook|callback").test_str(path) {
        "webhook"
    } else {
        "web"
    }
}

fn step(id: String, actor: &str, text: String, wait: bool, refs: Vec<String>) -> Value {
    obj(vec![
        ("id", json!(id)),
        ("actor", json!(actor)),
        ("text", json!(text)),
        ("wait", json!(wait)),
        ("refs", json!(refs)),
        ("branches", json!([])),
    ])
}

/// `draftFlows(g, root, flows)`: one draft per API handler no flow covers yet.
pub fn draft_flows(g: &Graph, root: &str, flows: &[Value]) -> Result<Vec<Value>> {
    let mut covered: HashSet<String> = HashSet::new();
    for f in flows {
        for st in arr(f.get("steps")) {
            for r in arr(st.get("refs")) {
                covered.insert(re!(r"^(GET|POST|PUT|PATCH|DELETE)\s+", "i").replace_first_str(&s(Some(r)), ""));
            }
        }
    }
    let ignored = read_ignored(root);
    let mut drafts = Vec::new();
    // Plumbing (auth checks, response helpers, the db wrapper) is called by most handlers: leave it out.
    let route_rows: Vec<usize> = (0..g.routes.len()).filter(|&i| g.routes[i].kind == "route").collect();
    let handlers_of = |file_id: i64| -> Vec<usize> {
        g.symbols_of(file_id)
            .iter()
            .copied()
            .filter(|&i| g.syms[i].exported != 0 && http_re().test_str(&g.syms[i].name))
            .collect()
    };
    let handlers: Vec<usize> = route_rows.iter().flat_map(|&r| handlers_of(g.routes[r].file_id)).collect();
    let mut fan_in: IndexMap<usize, usize> = IndexMap::new();
    for &h in &handlers {
        let hs = &g.syms[h];
        let mut callees: IndexSet<usize> = IndexSet::new();
        for &ri in g.refs_of(hs.file_id) {
            if g.refs[ri].symbol_id != Some(hs.id) {
                continue;
            }
            if let Some(si) = g.binding_target(hs.file_id, &g.refs[ri].local).and_then(|b| b.symbol) {
                callees.insert(si);
            }
        }
        for id in callees {
            *fan_in.entry(id).or_insert(0) += 1;
        }
    }
    let threshold = 5f64.max(handlers.len() as f64 * 0.1);
    let plumbing = |name: &str| fan_in.iter().any(|(id, n)| *n as f64 >= threshold && g.syms[*id].name == name);
    for &ri in &route_rows {
        let r = &g.routes[ri];
        for h in handlers_of(r.file_id) {
            let id = format!("{} {}", g.syms[h].name, r.path);
            if ignored.contains(&id) || covered.contains(&r.path) || covered.contains(&id) {
                continue;
            }
            let tr = trace(g, &id, 2, 12)?;
            let mut steps: Vec<Value> = Vec::new();
            let actor = actor_for(&r.path);
            steps.push(step("1".into(), actor, format!("`{id}`"), actor != "web", vec![id.clone()]));
            for c in arr(tr.get("calls"))
                .iter()
                .filter(|c| c.get("level").and_then(Value::as_i64) == Some(1) && !plumbing(&s(c.get("name"))))
                .take(4)
            {
                let name = s(c.get("name"));
                steps.push(step((steps.len() + 1).to_string(), "server", format!("`{name}`"), false, vec![name]));
            }
            let data = arr(tr.get("data"));
            let mut writes: IndexSet<String> = IndexSet::new();
            let mut reads: IndexSet<String> = IndexSet::new();
            for d in data {
                match d.get("op").and_then(Value::as_str) {
                    Some("write") => {
                        writes.insert(s(d.get("entity")));
                    }
                    Some("read") => {
                        reads.insert(s(d.get("entity")));
                    }
                    _ => {}
                }
            }
            let reads: Vec<String> = reads.into_iter().filter(|e| !writes.contains(e)).collect();
            if !writes.is_empty() {
                let w = writes.iter().cloned().collect::<Vec<_>>().join(", ");
                steps.push(step((steps.len() + 1).to_string(), "db", format!("writes `{w}`"), false, vec![w]));
            } else if !reads.is_empty() {
                let rd = reads.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
                steps.push(step((steps.len() + 1).to_string(), "db", format!("reads `{rd}`"), false, vec![rd]));
            }
            for p in arr(tr.get("packages"))
                .iter()
                .map(|p| s(Some(p)))
                .filter(|p| !re!(r"^(next|react|zod|@?types?)\b").test_str(p))
                .take(2)
            {
                steps.push(step((steps.len() + 1).to_string(), &p, format!("calls {p}"), false, vec![]));
            }
            drafts.push(obj(vec![
                ("id", json!(id)),
                ("title", json!(id)),
                ("file", Value::Null),
                ("draft", json!(true)),
                ("entry", json!(format!("{actor} {id}"))),
                ("steps", Value::Array(steps)),
                ("states", Value::Array(status_values(g, root, &writes)?)),
                ("notes", json!("")),
            ]));
        }
    }
    Ok(drafts)
}

/// `flowsOverview(g, root)` → `{ flows: [{ ...file, check }], drafts }` (what the flows page shows).
pub fn flows_overview(g: &Graph, root: &str) -> Result<Value> {
    let flows: Vec<Value> = read_flow_files(root)?
        .into_iter()
        .map(|mut f| {
            let check = check_flow(g, &f);
            if let Value::Object(o) = &mut f {
                o.insert("check".into(), check);
            }
            f
        })
        .collect();
    let drafts = draft_flows(g, root, &flows)?;
    Ok(obj(vec![("flows", Value::Array(flows)), ("drafts", Value::Array(drafts))]))
}

// ---------- text for the AI (code_graph flows / trace) ----------

/// `flowsText(g, root, target, limit)`: the `code_graph flows [name]` answer.
pub fn flows_text(g: &Graph, root: &str, target: Option<&str>, limit: i64) -> Result<String> {
    let ov = flows_overview(g, root)?;
    let flows = arr(ov.get("flows"));
    let drafts = arr(ov.get("drafts"));
    let target = target.filter(|x| !x.is_empty());
    let Some(target) = target else {
        if flows.is_empty() && drafts.is_empty() {
            return Ok(format!("No flows yet. Flows live in {FLOWS_DIR}/<name>.md (load the system-design skill for the format)."));
        }
        let mut lines = vec![format!("# Flows ({}) — {FLOWS_DIR}/", flows.len())];
        for f in flows {
            let stale = arr(f.get("check").and_then(|c| c.get("stale"))).len();
            lines.push(format!(
                "{} · {} · {} steps{}",
                s(f.get("title")),
                s(f.get("file")),
                arr(f.get("steps")).len(),
                if stale > 0 {
                    format!(" · ⚠ {stale} step(s) point at code that no longer exists")
                } else {
                    String::new()
                }
            ));
        }
        if !drafts.is_empty() {
            lines.push(String::new());
            lines.push(format!("# Entry points no flow covers yet ({}) — drafts, not saved", drafts.len()));
            for d in drafts.iter().take(limit.max(0) as usize) {
                let st = arr(d.get("steps"));
                lines.push(format!(
                    "{}{}",
                    s(d.get("title")),
                    if st.len() > 1 {
                        format!(" → {}", st[1..].iter().map(|x| s(x.get("text")).replace('`', "")).collect::<Vec<_>>().join(" → "))
                    } else {
                        String::new()
                    }
                ));
            }
            if drafts.len() as i64 > limit {
                lines.push(format!("… {} more", drafts.len() as i64 - limit));
            }
        }
        lines.push(String::new());
        lines.push("Read one: code_graph flows <name or file>. Trace live code: code_graph trace <route|symbol>.".into());
        return Ok(lines.join("\n"));
    };
    let q = target.to_lowercase();
    let lc = |v: Option<&Value>| s(v).to_lowercase();
    let f = flows
        .iter()
        .find(|x| lc(x.get("file")).ends_with(&format!("{q}.md")) || lc(x.get("file")) == q || lc(x.get("title")) == q)
        .or_else(|| flows.iter().find(|x| lc(x.get("title")).contains(&q) || lc(x.get("file")).contains(&q)));
    let Some(f) = f else {
        if let Some(d) = drafts.iter().find(|x| lc(x.get("id")).contains(&q)) {
            let mut lines = vec![format!("Draft (not saved): {}", s(d.get("title")))];
            for st in arr(d.get("steps")) {
                lines.push(format!(
                    "{}. [{}{}] {}",
                    s(st.get("id")),
                    if t(st.get("wait")) { "⏳ " } else { "" },
                    s(st.get("actor")),
                    s(st.get("text"))
                ));
            }
            for st in arr(d.get("states")) {
                lines.push(format!(
                    "states `{}`: {} (values found in code; moves unknown)",
                    s(st.get("column")),
                    join_values(arr(st.get("values")), ", ")
                ));
            }
            return Ok(lines.join("\n"));
        }
        return Ok(format!(
            "No flow matches \"{target}\". {}",
            if flows.is_empty() {
                "No flows yet.".to_string()
            } else {
                format!("Flows: {}", flows.iter().map(|x| s(x.get("title"))).collect::<Vec<_>>().join(", "))
            }
        ));
    };
    let stale_by: HashSet<String> = arr(f.get("check").and_then(|c| c.get("stale")))
        .iter()
        .map(|x| format!("{}|{}", s(x.get("step")), s(x.get("ref"))))
        .collect();
    let mut lines = vec![format!(
        "# {} — {}{}",
        s(f.get("title")),
        s(f.get("file")),
        if t(f.get("entry")) {
            format!(" · entry: {}", s(f.get("entry")))
        } else {
            String::new()
        }
    )];
    for st in arr(f.get("steps")) {
        let id = s(st.get("id"));
        let bad: Vec<String> = arr(st.get("refs")).iter().map(|r| s(Some(r))).filter(|r| stale_by.contains(&format!("{id}|{r}"))).collect();
        lines.push(format!(
            "{id}. [{}{}] {}{}",
            if t(st.get("wait")) { "⏳ " } else { "" },
            s(st.get("actor")),
            s(st.get("text")),
            if bad.is_empty() {
                String::new()
            } else {
                format!("  ⚠ not in code: {}", bad.join(", "))
            }
        ));
        for b in arr(st.get("branches")) {
            let to = b.get("to");
            lines.push(format!(
                "   - {}{}",
                s(b.get("label")),
                if t(to) {
                    format!(
                        " → {}",
                        if to.and_then(Value::as_str) == Some("end") {
                            "end".to_string()
                        } else {
                            format!("step {}", s(to))
                        }
                    )
                } else {
                    String::new()
                }
            ));
        }
    }
    for st in arr(f.get("states")) {
        let trans: Vec<String> = arr(st.get("transitions"))
            .iter()
            .map(|x| {
                format!(
                    "{}→{}{}",
                    s(x.get("from")),
                    s(x.get("to")),
                    if t(x.get("note")) { format!(" ({})", s(x.get("note"))) } else { String::new() }
                )
            })
            .collect();
        let body = if trans.is_empty() { join_values(arr(st.get("values")), ", ") } else { trans.join(", ") };
        lines.push(format!("states `{}`: {body}", s(st.get("column"))));
    }
    if t(f.get("notes")) {
        lines.push(String::new());
        lines.push(s(f.get("notes")));
    }
    Ok(lines.join("\n"))
}

/// `traceText(g, target)`: the `code_graph trace` answer.
pub fn trace_text(g: &Graph, target: &str) -> Result<String> {
    let tr = trace(g, target, 3, 40)?;
    if let Some(e) = tr.get("error") {
        return Ok(s(Some(e)));
    }
    let routes = arr(tr.get("routes"));
    let mut lines = vec![format!(
        "Trace {target}{} — from {}",
        if routes.is_empty() {
            String::new()
        } else {
            format!(" ({})", join_values(routes, ", "))
        },
        arr(tr.get("start"))
            .iter()
            .map(|x| format!("{} {}:{}", s(x.get("name")), s(x.get("file")), s(x.get("line"))))
            .collect::<Vec<_>>()
            .join(", ")
    )];
    let calls = arr(tr.get("calls"));
    let data = arr(tr.get("data"));
    let packages = arr(tr.get("packages"));
    if !calls.is_empty() {
        lines.push("## Calls".into());
        for c in calls {
            let level = c.get("level").and_then(Value::as_i64).unwrap_or(1);
            lines.push(format!(
                "{}{} → {} [{}] {}:{}",
                "  ".repeat((level - 1).max(0) as usize),
                s(c.get("from")),
                s(c.get("name")),
                s(c.get("kind")),
                s(c.get("file")),
                s(c.get("line"))
            ));
        }
    }
    if !data.is_empty() {
        lines.push("## Data".into());
        for d in data {
            lines.push(format!(
                "{} {}{} {}:{}{}",
                s(d.get("op")),
                s(d.get("entity")),
                if t(d.get("columns")) {
                    format!(" ({})", join_values(arr(d.get("columns")), ", "))
                } else {
                    String::new()
                },
                s(d.get("file")),
                s(d.get("line")),
                if t(d.get("by")) { format!(" in {}", s(d.get("by"))) } else { String::new() }
            ));
        }
    }
    if !packages.is_empty() {
        lines.push(format!("## Packages: {}", join_values(packages, ", ")));
    }
    if calls.is_empty() && data.is_empty() {
        lines.push("No project calls or data access found from here.".into());
    }
    Ok(lines.join("\n"))
}
