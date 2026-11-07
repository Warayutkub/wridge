//! Links the code graph to the Figma design (port of `design.mjs`): components
//! (`wara.components.json` + name matching) and tokens. A snapshot of the Figma side is stored in
//! the graph (meta `design`) so links work while Figma is closed.

use crate::build::create_dart_resolver;
use crate::db::{get_meta, set_meta};
use crate::js::{self, json_parse, json_stringify};
use crate::jsre::{JsRegex, JsStr};
use crate::query::{FileRow, Graph};
use crate::re;
use crate::resolve_ts::create_resolver;
use crate::styles::{norm_color, tw_to_vars};
use crate::util::{self, s, t};
use anyhow::{anyhow, bail, Result};
use indexmap::{IndexMap, IndexSet};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use wridge_figma::codegen::{format_tokens, write_component_map, COMPONENT_MAP_FILE};

// ---------- snapshot from Figma (needs the plugin) ----------

/// `syncDesign(db, send)` with the three plugin answers already fetched: `get_document_info {}`,
/// `get_components {}` and `get_tokens { source: "auto" }` (120 s timeout), in that order. Stores
/// the snapshot (meta `design`) → `{ file, components, tokens }`.
pub fn sync_design_from(db: &Connection, info: &Value, components: &Value, data: &Value) -> Result<Value> {
    let css = format_tokens(data, "tailwind").get("map").cloned().unwrap_or(Value::Null);
    let dart = format_tokens(data, "dart").get("map").cloned().unwrap_or(Value::Null);
    let default_mode: HashMap<String, Value> = util::arr(data.get("collections"))
        .iter()
        .map(|c| (s(c.get("name")), c.get("defaultMode").cloned().unwrap_or(Value::Null)))
        .collect();
    let mut tokens: Vec<Value> = Vec::new();
    let token = |tk: Option<&Value>, ty: Option<Value>, value: String| -> Value {
        let key = s(tk);
        let mut o = Map::new();
        if let Some(v) = tk {
            o.insert("token".into(), v.clone());
        }
        if let Some(ty) = ty {
            o.insert("type".into(), ty);
        }
        o.insert("value".into(), json!(value));
        if let Some(v) = css.get(&key) {
            o.insert("css".into(), v.clone());
        }
        if let Some(v) = dart.get(&key) {
            o.insert("dart".into(), v.clone());
        }
        Value::Object(o)
    };
    for v in util::arr(data.get("variables")) {
        if v.get("type").and_then(Value::as_str) == Some("BOOLEAN") {
            continue;
        }
        let values = v.get("values");
        let mode = default_mode.get(&s(v.get("collection")));
        let mode_key = match mode {
            None => "undefined".to_string(),
            Some(m) => s(Some(m)),
        };
        let raw = values
            .and_then(|vs| vs.get(&mode_key))
            .filter(|x| !x.is_null())
            .or_else(|| util::values(values).into_iter().next());
        let value = match raw {
            None => "undefined".to_string(),
            Some(Value::Object(o)) => format!("→ {}", s(o.get("alias"))),
            Some(Value::Array(_)) | Some(Value::Null) => "→ undefined".to_string(),
            Some(x) => s(Some(x)),
        };
        tokens.push(token(v.get("token"), v.get("type").cloned(), value));
    }
    for st in util::arr(data.get("paintStyles")) {
        if let Some(Value::String(val)) = st.get("value") {
            tokens.push(token(st.get("token"), Some(json!("COLOR")), val.clone()));
        }
    }
    for st in util::arr(data.get("textStyles")) {
        tokens.push(token(
            st.get("token"),
            Some(json!("TEXT")),
            format!("{} {} {}px", s(st.get("family")), s(st.get("style")), s(st.get("size"))),
        ));
    }
    for st in util::arr(data.get("effectStyles")) {
        let n = match st.get("shadows") {
            None | Some(Value::Null) => "0".to_string(),
            v => match v.and_then(|x| x.as_array()) {
                Some(a) => a.len().to_string(),
                None => util::len_of(v).to_string(),
            },
        };
        tokens.push(token(st.get("token"), Some(json!("SHADOW")), format!("{n} shadow(s)")));
    }
    let Some(list) = components.as_array() else {
        bail!("components.filter is not a function")
    };
    let comps: Vec<Value> = list
        .iter()
        .filter(|c| matches!(c.get("type").and_then(Value::as_str), Some("COMPONENT") | Some("COMPONENT_SET")))
        .map(|c| {
            let mut o = Map::new();
            for k in ["name", "id", "type"] {
                if let Some(v) = c.get(k) {
                    o.insert(k.into(), v.clone());
                }
            }
            Value::Object(o)
        })
        .collect();
    let mut snap = Map::new();
    if let Some(f) = info.get("fileName") {
        snap.insert("file".into(), f.clone());
    }
    snap.insert("fetchedAt".into(), json!(js::iso_now()));
    let ncomp = comps.len();
    let ntok = tokens.len();
    snap.insert("components".into(), Value::Array(comps));
    snap.insert("tokens".into(), Value::Array(tokens));
    set_meta(db, "design", &json_stringify(&Value::Object(snap)))?;
    let mut out = Map::new();
    if let Some(f) = info.get("fileName") {
        out.insert("file".into(), f.clone());
    }
    out.insert("components".into(), json!(ncomp));
    out.insert("tokens".into(), json!(ntok));
    Ok(Value::Object(out))
}

/// `syncDesign(db, send)`: `send(command, params, timeoutMs)` asks the Figma plugin.
pub fn sync_design(db: &Connection, mut send: impl FnMut(&str, Value, Option<u64>) -> Result<Value>) -> Result<Value> {
    let info = send("get_document_info", json!({}), None)?;
    let components = send("get_components", json!({}), None)?;
    let data = send("get_tokens", json!({ "source": "auto" }), Some(120_000))?;
    sync_design_from(db, &info, &components, &data)
}

/// `designSnapshot(db)`: the stored Figma snapshot, or `None`.
pub fn design_snapshot(db: &Connection) -> Option<Value> {
    let text = get_meta(db, "design").ok().flatten()?;
    json_parse(&text).ok().filter(|v| !v.is_null())
}

// ---------- components ----------

fn norm(st: &str) -> String {
    re!(r"[^a-z0-9]", "g").replace_all_str(&st.to_lowercase(), "")
}

fn read_map(root: &str) -> Value {
    js::read_utf8(util::join(&[root, COMPONENT_MAP_FILE]))
        .ok()
        .and_then(|t| json_parse(&t).ok())
        .unwrap_or_else(|| json!({}))
}

/// CSS classes of a class mapping: where they are defined, which are missing, how many files use them.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassInfo {
    pub defined: Vec<String>,
    pub missing: Vec<String>,
    pub files: usize,
}

/// An entry of `wara.components.json` with what it points at.
#[derive(Clone, Debug)]
pub struct Linked {
    pub figma_name: String,
    /// The Figma component of the snapshot (`{ name, id, type }`).
    pub figma: Option<Value>,
    pub entry: Value,
    /// Index into [`Graph::syms`].
    pub symbol: Option<usize>,
    pub classes: Option<ClassInfo>,
}

/// A Figma component paired with a code component by name.
#[derive(Clone, Debug)]
pub struct Suggested {
    pub figma: Value,
    pub symbol: usize,
    pub exact: bool,
}

/// `componentLinks(g, root, snapshot)`.
#[derive(Clone, Debug, Default)]
pub struct ComponentLinks {
    pub linked: Vec<Linked>,
    pub suggested: Vec<Suggested>,
    pub figma_only: Vec<Value>,
    pub code_only: Vec<usize>,
}

fn figma_of<'v>(snapshot: Option<&'v Value>, figma_name: &str) -> Option<&'v Value> {
    util::arr(snapshot.and_then(|sn| sn.get("components"))).iter().find(|c| {
        let n = s(c.get("name"));
        n == figma_name || n.split('/').next() == Some(figma_name)
    })
}

/// `componentLinks(g, root, snapshot)`: Figma component ↔ code component/widget. `linked` come from
/// `wara.components.json`, `suggested` from names.
pub fn component_links(g: &Graph, root: &str, snapshot: Option<&Value>) -> Result<ComponentLinks> {
    let map = read_map(root);
    let paths: Vec<String> = g.files.iter().map(|f| f.path.clone()).collect();
    let resolve_ts = create_resolver(Path::new(root), &paths);
    let resolve_dart = create_dart_resolver(Path::new(root), &paths);
    let code_components: Vec<usize> = (0..g.syms.len())
        .filter(|&i| (g.syms[i].kind == "component" || g.syms[i].kind == "widget") && g.syms[i].exported != 0)
        .collect();

    let entries = util::entries(Some(&map));
    let mut out = ComponentLinks::default();
    let mut linked_symbols: IndexSet<usize> = IndexSet::new();
    let classes = class_index(g, root, &entries.iter().map(|(_, v)| *v).collect::<Vec<_>>())?;
    for (figma_name, entry) in &entries {
        if t(entry.get("className")) {
            let mut all = vec![entry.get("className").cloned().unwrap_or(Value::Null)];
            all.extend(util::values(entry.get("variants")).into_iter().cloned());
            let joined = util::join_values(&all, " ");
            let mut names: IndexSet<String> = IndexSet::new();
            for n in util::words(&joined) {
                names.insert(n.to_string());
            }
            let class_name = s(entry.get("className"));
            out.linked.push(Linked {
                figma_name: figma_name.to_string(),
                figma: figma_of(snapshot, figma_name).cloned(),
                entry: (*entry).clone(),
                symbol: None,
                classes: Some(ClassInfo {
                    defined: names
                        .iter()
                        .filter(|n| classes.defs.contains_key(*n))
                        .map(|n| format!(".{n} {}", classes.defs[n]))
                        .collect(),
                    missing: names.iter().filter(|n| !classes.defs.contains_key(*n)).cloned().collect(),
                    files: classes.used_in(&class_name.split(|c: char| js::is_js_space(c as u32)).collect::<Vec<_>>())?,
                }),
            });
            continue;
        }
        let spec = match entry.get("import") {
            None | Some(Value::Null) => String::new(),
            v => s(v),
        };
        let r = if spec.starts_with("package:") || spec.ends_with(".dart") {
            resolve_dart.resolve("lib/main.dart", &spec)
        } else {
            resolve_ts.resolve("index.ts", &spec)?
        };
        let file = r.file.as_ref().and_then(|f| g.file_by_path_exact(f));
        let component = entry.get("component").and_then(Value::as_str);
        let symbol = match file {
            Some(fi) => match component {
                Some(c) => g.definition(g.files[fi].id, Some(c)),
                None => None,
            },
            None => code_components.iter().copied().find(|&i| Some(g.syms[i].name.as_str()) == component),
        };
        out.linked.push(Linked {
            figma_name: figma_name.to_string(),
            figma: figma_of(snapshot, figma_name).cloned(),
            entry: (*entry).clone(),
            symbol,
            classes: None,
        });
        if let Some(si) = symbol {
            linked_symbols.insert(si);
        }
    }

    let mapped: std::collections::HashSet<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    for c in util::arr(snapshot.and_then(|sn| sn.get("components"))) {
        let name = s(c.get("name"));
        if mapped.contains(name.as_str()) || mapped.contains(name.split('/').next().unwrap_or("")) {
            continue;
        }
        let parts: Vec<String> = name
            .split('/')
            .map(|p| js::js_trim(p.split('=').next_back().unwrap_or("")).to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let mut rev = parts.clone();
        rev.reverse();
        let candidates: Vec<String> = [Some(parts.join("")), Some(rev.join("")), parts.first().cloned(), parts.last().cloned()]
            .into_iter()
            .map(|x| norm(&x.unwrap_or_else(|| "undefined".into())))
            .collect();
        let mut best: Option<(usize, usize)> = None;
        for (rank, cand) in candidates.iter().enumerate() {
            let hit = code_components
                .iter()
                .copied()
                .find(|&i| norm(&g.syms[i].name) == *cand || norm(&re!(r"(Widget|Component|View)$").replace_first_str(&g.syms[i].name, "")) == *cand);
            if let Some(h) = hit {
                best = Some((h, rank));
                break;
            }
        }
        match best {
            Some((symbol, rank)) => out.suggested.push(Suggested {
                figma: c.clone(),
                symbol,
                exact: rank < 2,
            }),
            None => out.figma_only.push(c.clone()),
        }
    }
    let suggested_ids: std::collections::HashSet<usize> = out.suggested.iter().map(|x| x.symbol).collect();
    out.code_only = code_components.into_iter().filter(|i| !linked_symbols.contains(i) && !suggested_ids.contains(i)).collect();
    Ok(out)
}

struct ClassIndex {
    defs: IndexMap<String, String>,
    sources: Vec<String>,
}

impl ClassIndex {
    fn used_in(&self, names: &[&str]) -> Result<usize> {
        let mut res: Vec<JsRegex> = Vec::new();
        for n in names.iter().filter(|n| !n.is_empty()) {
            let pat = format!("(^|[\\s\"'`{{]){}(?=[\\s\"'`}}]|$)", n.replace('-', "\\-"));
            res.push(JsRegex::try_new(&pat, "m").map_err(|e| anyhow!("Invalid regular expression: /{pat}/m: {e}"))?);
        }
        Ok(self
            .sources
            .iter()
            .filter(|text| {
                let js = JsStr::new(text);
                res.iter().all(|re| re.test(&js))
            })
            .count())
    }
}

fn read_file_safe(p: &str) -> String {
    js::read_utf8(p).unwrap_or_default()
}

// Where CSS classes of class mappings are defined (stylesheets) and which source files use them.
fn class_index(g: &Graph, root: &str, entries: &[&Value]) -> Result<ClassIndex> {
    let mut ix = ClassIndex {
        defs: IndexMap::new(),
        sources: Vec::new(),
    };
    if !entries.iter().any(|e| t(e.get("className"))) {
        return Ok(ix);
    }
    for f in &g.files {
        let ext = f.path.split('.').next_back().unwrap_or("");
        if ["css", "scss", "sass", "less"].contains(&ext) {
            let text = read_file_safe(&util::join(&[root, &f.path]));
            for (i, line) in text.split('\n').enumerate() {
                let js = JsStr::new(line);
                let decl = re!(r"[{,]|^\s*\.").test(&js);
                for m in re!(r"\.(-?[A-Za-z_][\w-]*)", "g").all(&js) {
                    let name = m.str(&js, 1).unwrap_or_default();
                    if !ix.defs.contains_key(&name) && decl {
                        ix.defs.insert(name, format!("{}:{}", f.path, i + 1));
                    }
                }
            }
        } else if ["tsx", "jsx", "ts", "js", "html", "vue", "svelte", "php"].contains(&ext) {
            ix.sources.push(read_file_safe(&util::join(&[root, &f.path])));
        }
    }
    Ok(ix)
}

/// `importPathFor(g, root, file)`: the module path people already use for a file
/// (`@/components/ui/button`, `package:app/widgets/card.dart`).
pub fn import_path_for(g: &Graph, root: &str, file: &FileRow) -> String {
    let mut counts: IndexMap<&str, i64> = IndexMap::new();
    for i in g.imports_flat() {
        if i.target_file_id != Some(file.id) || i.source.starts_with('.') {
            continue;
        }
        *counts.entry(i.source.as_str()).or_insert(0) += 1;
    }
    let mut c: Vec<(&str, i64)> = counts.into_iter().collect();
    c.sort_by_key(|x| std::cmp::Reverse(x.1));
    if let Some((best, _)) = c.first() {
        return best.to_string();
    }
    if file.lang == "dart" {
        let pubspec = read_file_safe(&util::join(&[root, "pubspec.yaml"]));
        let pkg = re!(r"^name:\s*([A-Za-z0-9_]+)", "m").exec_str(&pubspec).and_then(|m| m[1].clone());
        return match pkg {
            Some(p) if file.path.starts_with("lib/") => format!("package:{p}/{}", &file.path[4..]),
            _ => file.path.clone(),
        };
    }
    let p = re!(r"\.(tsx?|jsx?|mjs)$").replace_first_str(&file.path, "");
    format!("./{}", re!(r"\/index$").replace_first_str(&p, ""))
}

/// `confirmLink(g, root, figmaName, symbolId)`: writes the Figma ↔ code pair to
/// `wara.components.json` (the file map_component uses) → the entry.
pub fn confirm_link(g: &Graph, root: &str, figma_name: &str, symbol_id: f64) -> Result<Value> {
    let si = if symbol_id.fract() == 0.0 { g.sym_index(symbol_id as i64) } else { None };
    let Some(si) = si else { bail!("Unknown symbol {}", js::js_number(symbol_id)) };
    let mut map = match read_map(root) {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    let mut entry = match map.get(figma_name) {
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    };
    let file = g
        .file(g.syms[si].file_id)
        .cloned()
        .ok_or_else(|| anyhow!("Cannot read properties of undefined (reading 'id')"))?;
    entry.insert("import".into(), json!(import_path_for(g, root, &file)));
    entry.insert("component".into(), json!(g.syms[si].name));
    map.insert(figma_name.to_string(), Value::Object(entry.clone()));
    write_component_map(root, &Value::Object(map))?;
    Ok(Value::Object(entry))
}

// ---------- tokens ----------

/// A `style_defs` row with its file path.
#[derive(Clone, Debug)]
pub struct StyleDefRow {
    pub file_id: i64,
    pub name: String,
    pub value: Option<String>,
    pub line: Option<i64>,
    pub path: String,
}

/// A `style_refs` row with its file path.
#[derive(Clone, Debug)]
pub struct StyleRefRow {
    pub file_id: i64,
    pub form: String,
    pub name: String,
    pub line: Option<i64>,
    pub count: i64,
    pub path: String,
}

/// One token: its code definitions, its Figma side, and where it is used.
#[derive(Clone, Debug)]
pub struct TokenRow {
    pub name: String,
    pub defs: Vec<StyleDefRow>,
    pub figma: Option<Value>,
    pub uses: Vec<StyleRefRow>,
    pub files: IndexSet<String>,
    pub count: i64,
    /// The first definition's value with `var(--x)` followed (up to 5 hops).
    pub resolved: Option<String>,
    pub mismatch: bool,
}

/// A raw color in code (`hardcoded`): `{ name, n, files }`.
#[derive(Clone, Debug)]
pub struct Hardcoded {
    pub name: String,
    pub n: i64,
    pub files: i64,
}

/// Raw colors per file (`hardFiles`): `{ path, n }`.
#[derive(Clone, Debug)]
pub struct HardFile {
    pub path: String,
    pub n: i64,
}

/// `tokenTable(db, g, snapshot)`.
#[derive(Clone, Debug)]
pub struct TokenTable {
    pub rows: Vec<TokenRow>,
    pub hardcoded: Vec<Hardcoded>,
    pub hard_files: Vec<HardFile>,
}

/// `tokenTable(db, g, snapshot)`: every token from the code (definitions) and/or Figma, with usage
/// counts and mismatches.
pub fn token_table(db: &Connection, _g: &Graph, snapshot: Option<&Value>) -> Result<TokenTable> {
    let defs: Vec<StyleDefRow> = {
        let mut st = db.prepare("SELECT d.*, f.path FROM style_defs d JOIN files f ON f.id = d.file_id")?;
        let rows = st.query_map([], |r| {
            Ok(StyleDefRow {
                file_id: r.get("file_id")?,
                name: r.get("name")?,
                value: r.get("value")?,
                line: r.get("line")?,
                path: r.get("path")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let uses: Vec<StyleRefRow> = {
        let mut st = db.prepare("SELECT r.*, f.path FROM style_refs r JOIN files f ON f.id = r.file_id WHERE r.form != 'hex'")?;
        let rows = st.query_map([], |r| {
            Ok(StyleRefRow {
                file_id: r.get("file_id")?,
                form: r.get("form")?,
                name: r.get("name")?,
                line: r.get("line")?,
                count: r.get("count")?,
                path: r.get("path")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut rows: IndexMap<String, TokenRow> = IndexMap::new();
    fn row<'m>(rows: &'m mut IndexMap<String, TokenRow>, name: &str) -> &'m mut TokenRow {
        rows.entry(name.to_string()).or_insert_with(|| TokenRow {
            name: name.to_string(),
            defs: Vec::new(),
            figma: None,
            uses: Vec::new(),
            files: IndexSet::new(),
            count: 0,
            resolved: None,
            mismatch: false,
        })
    }
    for d in defs {
        let n = d.name.clone();
        row(&mut rows, &n).defs.push(d);
    }
    for tk in util::arr(snapshot.and_then(|sn| sn.get("tokens"))) {
        let names: Vec<String> = [tk.get("css"), tk.get("dart")].into_iter().filter(|x| t(*x)).map(|x| s(x)).collect();
        let existing = names.iter().find(|n| rows.contains_key(*n)).cloned();
        let key = existing.or_else(|| names.first().cloned()).unwrap_or_else(|| format!("figma:{}", s(tk.get("token"))));
        row(&mut rows, &key).figma = Some(tk.clone());
    }
    // Usages: var(--x) and AppColors.x by name; Tailwind utilities through the variables they read.
    for u in uses {
        let targets = if u.form == "tw" { tw_to_vars(&u.name) } else { vec![u.name.clone()] };
        let Some(name) = targets.into_iter().find(|x| rows.contains_key(x)) else { continue };
        let r = rows.get_mut(&name).unwrap();
        r.files.insert(u.path.clone());
        r.count += u.count;
        r.uses.push(u);
    }
    // var(--primary) → the value --primary holds (follow up to 5 hops), so aliases can be compared with Figma.
    let first_values: HashMap<String, Option<String>> = rows.iter().filter_map(|(k, r)| r.defs.first().map(|d| (k.clone(), d.value.clone()))).collect();
    fn resolve_value(v: Option<String>, hops: u32, first: &HashMap<String, Option<String>>) -> Option<String> {
        let text = v.clone().unwrap_or_default();
        let Some(m) = re!(r"^var\(\s*(--[\w-]+)\s*\)$").exec_str(js::js_trim(&text)) else {
            return v;
        };
        if hops > 5 {
            return v;
        }
        let next = first.get(m[1].as_deref().unwrap_or("")).cloned().flatten().or(v);
        resolve_value(next, hops + 1, first)
    }
    for r in rows.values_mut() {
        r.resolved = r.defs.first().and_then(|d| resolve_value(d.value.clone(), 0, &first_values));
        let code = norm_color(r.resolved.as_deref());
        let fv = r.figma.as_ref().and_then(|f| f.get("value"));
        let figma_text = match fv {
            None | Some(Value::Null) => None,
            v if !t(v) => None,
            v => Some(s(v)),
        };
        let figma = norm_color(figma_text.as_deref());
        r.mismatch = matches!((&code, &figma), (Some(c), Some(f)) if c != f);
    }
    let hardcoded: Vec<Hardcoded> = {
        let mut st = db.prepare("SELECT r.name, sum(r.count) AS n, count(DISTINCT r.file_id) AS files FROM style_refs r WHERE r.form = 'hex' GROUP BY r.name ORDER BY n DESC")?;
        let rows = st.query_map([], |r| {
            Ok(Hardcoded {
                name: r.get(0)?,
                n: r.get(1)?,
                files: r.get(2)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let hard_files: Vec<HardFile> = {
        let mut st = db.prepare("SELECT f.path, sum(r.count) AS n FROM style_refs r JOIN files f ON f.id = r.file_id WHERE r.form = 'hex' GROUP BY f.path ORDER BY n DESC")?;
        let rows = st.query_map([], |r| Ok(HardFile { path: r.get(0)?, n: r.get(1)? }))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    Ok(TokenTable {
        rows: rows.into_values().collect(),
        hardcoded,
        hard_files,
    })
}
