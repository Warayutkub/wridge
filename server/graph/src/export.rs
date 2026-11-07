//! Whole-graph JSON for the web views (port of `export.mjs`): the code graph page ([`export_graph`])
//! and the data model page ([`export_data`]). The page computes used-by / impact itself from the edges.

#![allow(clippy::type_complexity)]

use crate::db::get_meta;
use crate::design::{component_links, design_snapshot, token_table};
use crate::js::json_parse;
use crate::notes::all_notes;
use crate::query::graph;
use crate::util::{self, obj};
use anyhow::Result;
use indexmap::IndexMap;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

fn json_col(v: Option<String>) -> Result<Value> {
    Ok(match v.filter(|x| !x.is_empty()) {
        Some(x) => json_parse(&x)?,
        None => Value::Null,
    })
}

fn name_of(root: &str) -> (String, String) {
    let r = root.replace('\\', "/");
    let name = r.split('/').next_back().unwrap_or("").to_string();
    (r, name)
}

/// `exportData(db, root)`: schema + who reads/writes it, for the "โครงสร้างข้อมูล" view (ER diagram
/// and 3D). `root` is used as given (only for `root`/`name`).
pub fn export_data(db: &Connection, root: &str) -> Result<Value> {
    let model = get_meta(db, "data_model")?.and_then(|t| json_parse(&t).ok()).filter(|v| !v.is_null());
    let mut st = db.prepare(
        "SELECT d.entity, d.op, d.form, d.columns, d.filters, d.line, f.path FROM data_refs d JOIN files f ON f.id = d.file_id WHERE d.op != 'ddl' ORDER BY f.path, d.line",
    )?;
    let rows: Vec<(String, String, String, Option<String>, Option<String>, Option<i64>, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut refs = Vec::new();
    for (entity, op, form, columns, filters, line, path) in rows {
        refs.push(obj(vec![
            ("entity", json!(entity)),
            ("op", json!(op)),
            ("form", json!(form)),
            ("columns", json_col(columns)?),
            ("filters", json_col(filters)?),
            ("line", util::jnum(line)),
            ("path", json!(path)),
        ]));
    }
    let (r, name) = name_of(root);
    let stores: Vec<Value> = util::arr(model.as_ref().and_then(|m| m.get("stores")))
        .iter()
        .map(|st| {
            let mut o = Map::new();
            if let Some(k) = st.get("kind") {
                o.insert("kind".into(), k.clone());
            }
            if let Some(l) = st.get("label") {
                o.insert("label".into(), l.clone());
            }
            o.insert("sources".into(), st.get("sources").filter(|v| !v.is_null()).cloned().unwrap_or(json!([])));
            o.insert(
                "unparsed".into(),
                match st.get("unparsed").filter(|v| !v.is_null()) {
                    Some(Value::Array(a)) => json!(a.len()),
                    Some(Value::String(x)) => json!(crate::js::utf16_len(x)),
                    _ => json!(0),
                },
            );
            o.insert("entities".into(), Value::Array(util::values(st.get("entities")).into_iter().cloned().collect()));
            Value::Object(o)
        })
        .collect();
    Ok(obj(vec![
        ("root", json!(r)),
        ("name", json!(name)),
        ("updatedAt", util::jstr(get_meta(db, "updated_at")?.as_deref())),
        ("stores", Value::Array(stores)),
        ("refs", Value::Array(refs)),
    ]))
}

/// `exportGraph(db, root)`: the whole graph for the "กราฟโค้ด" view — files, symbols, file and
/// symbol edges, packages, routes, design links, tokens and notes. `root` is used as given.
pub fn export_graph(db: &Connection, root: &str) -> Result<Value> {
    let g = graph(db)?;
    let files: Vec<Value> = g
        .files
        .iter()
        .map(|f| {
            let mut o = Map::new();
            o.insert("id".into(), json!(f.id));
            o.insert("path".into(), json!(f.path));
            o.insert("lang".into(), json!(f.lang));
            if let Some(d) = &f.directive {
                o.insert("directive".into(), json!(d));
            }
            if let Some(e) = &f.parse_error {
                o.insert("error".into(), json!(e));
            }
            o.insert(
                "routes".into(),
                json!(g.routes.iter().filter(|r| r.file_id == f.id).map(|r| format!("{} {}", r.kind, r.path)).collect::<Vec<_>>()),
            );
            Value::Object(o)
        })
        .collect();
    let mut symbols = Vec::new();
    for sy in g.syms.iter().filter(|s| s.kind != "alias") {
        let mut o = Map::new();
        o.insert("id".into(), json!(sy.id));
        o.insert("file".into(), json!(sy.file_id));
        o.insert("name".into(), json!(sy.name));
        o.insert("kind".into(), json!(sy.kind));
        o.insert("line".into(), util::jnum(sy.line));
        o.insert("end".into(), util::jnum(sy.end_line));
        o.insert("exported".into(), json!(if sy.exported != 0 { 1 } else { 0 }));
        o.insert("isDefault".into(), json!(if sy.is_default != 0 { 1 } else { 0 }));
        if let Some(p) = sy.props.as_deref().filter(|p| !p.is_empty()) {
            o.insert("props".into(), json_parse(p)?);
        }
        symbols.push(Value::Object(o));
    }

    // file → file (any import kind), weighted by how many names are imported
    let mut file_edges: IndexMap<(i64, i64), i64> = IndexMap::new();
    let mut externals: IndexMap<i64, util::Counter> = IndexMap::new();
    for (file_id, imps) in g.imports_by_file() {
        for &ii in imps {
            let i = &g.imports[ii];
            match i.target_file_id.filter(|x| *x != 0 && *x != file_id) {
                Some(tf) => *file_edges.entry((file_id, tf)).or_insert(0) += 1,
                None => {
                    if let Some(e) = i.external.as_deref().filter(|e| !e.is_empty()) {
                        externals.entry(file_id).or_default().add(e, 1);
                    }
                }
            }
        }
    }

    // symbol → symbol uses ([from, to, kind, line, count]); from = owner symbol id, or -fileId for module-level code
    let mut symbol_edges: IndexMap<String, (i64, i64, String, Option<i64>, i64)> = IndexMap::new();
    for (file_id, refs) in g.refs_by_file() {
        for &ri in refs {
            let r = &g.refs[ri];
            let b = g.binding_target(file_id, &r.local);
            let mut target = b.as_ref().and_then(|b| b.symbol);
            if let Some(b) = &b {
                if b.namespace {
                    if let Some(m) = r.member.as_deref().filter(|m| !m.is_empty()) {
                        target = b.file.and_then(|f| g.definition(g.files[f].id, Some(m.split('.').next().unwrap_or(""))));
                    }
                }
            }
            let Some(target) = target else { continue };
            let ts = &g.syms[target];
            if ts.kind == "alias" {
                continue;
            }
            let from = r.symbol_id.unwrap_or(-file_id);
            if from == ts.id {
                continue;
            }
            let k = format!("{from}>{}>{}", ts.id, r.kind);
            match symbol_edges.get_mut(&k) {
                Some(e) => e.4 += r.count,
                None => {
                    symbol_edges.insert(k, (from, ts.id, r.kind.clone(), r.line, r.count));
                }
            }
        }
    }

    // Object.fromEntries(externals): file ids are integer keys, so JS orders them ascending
    let mut ext_keys: Vec<i64> = externals.keys().copied().collect();
    ext_keys.sort_by_key(|k| if *k >= 0 { (0, *k) } else { (1, 0) });
    let mut ext_obj = Map::new();
    for k in ext_keys.iter().filter(|k| **k >= 0) {
        ext_obj.insert(k.to_string(), counter_obj(&externals[k]));
    }
    for k in externals.keys().filter(|k| **k < 0) {
        ext_obj.insert(k.to_string(), counter_obj(&externals[k]));
    }

    let (r, name) = name_of(root);
    let mut out = Map::new();
    out.insert("root".into(), json!(r));
    out.insert("name".into(), json!(name));
    out.insert("updatedAt".into(), util::jstr(get_meta(db, "updated_at")?.as_deref()));
    out.insert("files".into(), Value::Array(files));
    out.insert("symbols".into(), Value::Array(symbols));
    out.insert("fileEdges".into(), Value::Array(file_edges.iter().map(|((a, b), n)| json!([a, b, n])).collect()));
    out.insert(
        "symbolEdges".into(),
        Value::Array(symbol_edges.values().map(|(f, t, k, l, c)| json!([f, t, k, util::jnum(*l), c])).collect()),
    );
    out.insert("externals".into(), Value::Object(ext_obj));
    out.insert(
        "routes".into(),
        Value::Array(
            g.routes
                .iter()
                .map(|r| {
                    obj(vec![
                        ("file", json!(r.file_id)),
                        ("path", json!(r.path)),
                        ("kind", json!(r.kind)),
                        ("framework", json!(r.framework)),
                        ("root", util::jstr(r.root.as_deref())),
                    ])
                })
                .collect(),
        ),
    );
    if let Value::Object(d) = design_data(db, &g, root)? {
        out.extend(d);
    }
    out.insert(
        "notes".into(),
        Value::Array(
            all_notes(db, &g)?
                .iter()
                .map(|n| {
                    obj(vec![
                        ("target", json!(n.target)),
                        ("text", json!(n.text)),
                        ("status", json!(n.status)),
                        ("updatedAt", json!(n.updated_at)),
                    ])
                })
                .collect(),
        ),
    );
    Ok(Value::Object(out))
}

fn counter_obj(c: &util::Counter) -> Value {
    let mut o = Map::new();
    for (k, n) in c.entries() {
        o.insert(k, json!(n));
    }
    Value::Object(o)
}

// Figma links (linked in wara.components.json, or suggested by name) and the token table.
fn design_data(db: &Connection, g: &crate::query::Graph, root: &str) -> Result<Value> {
    let snap = design_snapshot(db);
    let links = component_links(g, root, snap.as_ref())?;
    let table = token_table(db, g, snap.as_ref())?;
    let mut link_list: Vec<Value> = Vec::new();
    for l in links.linked.iter().filter(|l| l.symbol.is_some()) {
        link_list.push(obj(vec![
            ("symbol", json!(g.syms[l.symbol.unwrap()].id)),
            ("figma", json!(l.figma_name)),
            (
                "figmaId",
                l.figma.as_ref().and_then(|f| f.get("id")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
            ),
            ("status", json!("linked")),
        ]));
    }
    for x in &links.suggested {
        let mut o = Map::new();
        o.insert("symbol".into(), json!(g.syms[x.symbol].id));
        if let Some(n) = x.figma.get("name") {
            o.insert("figma".into(), n.clone());
        }
        if let Some(n) = x.figma.get("id") {
            o.insert("figmaId".into(), n.clone());
        }
        o.insert("status".into(), json!("suggested"));
        link_list.push(Value::Object(o));
    }
    let broken: Vec<Value> = links
        .linked
        .iter()
        .filter(|l| match &l.classes {
            Some(c) => !c.missing.is_empty(),
            None => l.symbol.is_none(),
        })
        .map(|l| json!(l.figma_name))
        .collect();
    let class_links: Vec<Value> = links
        .linked
        .iter()
        .filter_map(|l| {
            let c = l.classes.as_ref()?;
            let mut o = Map::new();
            o.insert("figma".into(), json!(l.figma_name));
            o.insert(
                "figmaId".into(),
                l.figma.as_ref().and_then(|f| f.get("id")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
            );
            o.insert("element".into(), l.entry.get("element").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null));
            if let Some(cn) = l.entry.get("className") {
                o.insert("className".into(), cn.clone());
            }
            o.insert("variants".into(), l.entry.get("variants").filter(|v| !v.is_null()).cloned().unwrap_or(json!({})));
            o.insert("files".into(), json!(c.files));
            o.insert("missing".into(), json!(c.missing));
            Some(Value::Object(o))
        })
        .collect();
    let mut design = Map::new();
    design.insert(
        "file".into(),
        snap.as_ref().and_then(|x| x.get("file")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
    );
    design.insert(
        "fetchedAt".into(),
        snap.as_ref().and_then(|x| x.get("fetchedAt")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
    );
    design.insert("links".into(), Value::Array(link_list));
    design.insert("broken".into(), Value::Array(broken));
    design.insert("classLinks".into(), Value::Array(class_links));
    design.insert(
        "figmaOnly".into(),
        Value::Array(links.figma_only.iter().map(|c| c.get("name").cloned().unwrap_or(Value::Null)).collect()),
    );
    let tokens: Vec<Value> = table
        .rows
        .iter()
        .map(|r| {
            let mut o = Map::new();
            if r.name.starts_with("figma:") {
                if let Some(t) = r.figma.as_ref().and_then(|f| f.get("token")) {
                    o.insert("name".into(), t.clone());
                }
            } else {
                o.insert("name".into(), json!(r.name));
            }
            o.insert(
                "figma".into(),
                match &r.figma {
                    Some(f) => {
                        let mut x = Map::new();
                        if let Some(v) = f.get("token") {
                            x.insert("token".into(), v.clone());
                        }
                        if let Some(v) = f.get("value") {
                            x.insert("value".into(), v.clone());
                        }
                        Value::Object(x)
                    }
                    None => Value::Null,
                },
            );
            let first = r.defs.first().and_then(|d| d.value.clone());
            o.insert("value".into(), util::jstr(first.as_deref()));
            o.insert(
                "resolved".into(),
                match &r.resolved {
                    Some(x) if !x.is_empty() && Some(x) != first.as_ref() => json!(x),
                    _ => Value::Null,
                },
            );
            o.insert(
                "defined".into(),
                r.defs.first().map(|d| json!(format!("{}:{}", d.path, util::num(d.line)))).unwrap_or(Value::Null),
            );
            o.insert("count".into(), json!(r.count));
            o.insert("files".into(), json!(r.files.iter().collect::<Vec<_>>()));
            o.insert("mismatch".into(), json!(r.mismatch));
            Value::Object(o)
        })
        .collect();
    Ok(obj(vec![
        ("design", Value::Object(design)),
        ("tokens", Value::Array(tokens)),
        (
            "hardcoded",
            Value::Array(table.hard_files.iter().map(|f| obj(vec![("path", json!(f.path)), ("count", json!(f.n))])).collect()),
        ),
    ]))
}
