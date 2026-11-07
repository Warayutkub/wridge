//! Questions the AI asks the code graph (port of `query.mjs`). Answers are compact plain text.
//!
//! [`graph`] loads `.wara/code-graph.db` into memory with the same derived views as the JS
//! `graph(db)` (definitions through re-exports and barrels, import bindings, references, routes,
//! the widget/component kind fixes) and [`run_query`] answers one of [`QUERIES`].

#![allow(clippy::type_complexity, clippy::too_many_arguments)]

use crate::brain;
use crate::data::model::find_entity;
use crate::design;
use crate::flows;
use crate::js::{self, json_parse, locale_compare, slice_utf16};
use crate::notes::{self, NoteInfo};
use crate::re;
use crate::routes_next::route_matches;
use crate::util::{self, arr, join_values, len_of, num, pad_end, s, t, values, Counter};
use anyhow::{bail, Result};
use indexmap::IndexMap;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};

/// `QUERIES`: what `code_graph` accepts.
pub const QUERIES: [&str; 14] = [
    "overview", "find", "file", "used_by", "uses", "impact", "route", "design", "tokens", "notes", "data", "table", "flows", "trace",
];

// ---------- rows ----------

/// A row of `files`.
#[derive(Clone, Debug)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub lang: String,
    pub mtime: Option<f64>,
    pub size: Option<i64>,
    pub directive: Option<String>,
    pub parse_error: Option<String>,
}

/// A row of `symbols` (its `kind` as `graph()` leaves it: widget/component fixes applied).
#[derive(Clone, Debug)]
pub struct Sym {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    pub kind: String,
    pub exported: i64,
    pub is_default: i64,
    pub line: Option<i64>,
    pub end_line: Option<i64>,
    pub props: Option<String>,
    pub hash: Option<String>,
}

/// A row of `imports`.
#[derive(Clone, Debug)]
pub struct ImportRow {
    pub id: i64,
    pub file_id: i64,
    pub kind: String,
    pub source: String,
    pub imported: Option<String>,
    pub local: Option<String>,
    pub type_only: i64,
    pub line: Option<i64>,
    pub target_file_id: Option<i64>,
    pub external: Option<String>,
}

/// A row of `refs`.
#[derive(Clone, Debug)]
pub struct RefRow {
    pub id: i64,
    pub file_id: i64,
    pub symbol_id: Option<i64>,
    pub local: String,
    pub member: Option<String>,
    pub kind: String,
    pub line: Option<i64>,
    pub count: i64,
}

/// A row of `routes`.
#[derive(Clone, Debug)]
pub struct RouteRow {
    pub id: i64,
    pub file_id: i64,
    pub framework: String,
    pub path: String,
    pub kind: String,
    pub root: Option<String>,
}

/// What a local identifier points at (`bindingTarget`): a symbol, a file, or an external package.
#[derive(Clone, Debug, Default)]
pub struct Binding {
    pub external: Option<String>,
    pub imported: Option<String>,
    pub unresolved: Option<String>,
    /// Index into [`Graph::files`].
    pub file: Option<usize>,
    pub namespace: bool,
    /// Index into [`Graph::syms`].
    pub symbol: Option<usize>,
}

/// One use of a symbol (`referencesTo`): the ref row, its file and the symbol it sits in.
#[derive(Clone, Copy, Debug)]
pub struct RefHit {
    /// Index into [`Graph::refs`].
    pub r: usize,
    /// Index into [`Graph::files`].
    pub file: usize,
    /// Index into [`Graph::syms`].
    pub owner: Option<usize>,
}

/// `lookup(target)`: routes, symbols, a file, or an error text.
#[derive(Clone, Debug, Default)]
pub struct Lookup {
    pub routes: Option<Vec<usize>>,
    pub symbols: Option<Vec<usize>>,
    pub file: Option<usize>,
    pub error: Option<String>,
}

#[derive(Default)]
struct BindingIndex {
    by_symbol: HashMap<usize, Vec<(i64, String)>>,
    namespace: Vec<(i64, usize)>,
}

/// The in-memory graph (`graph(db)` in JS).
pub struct Graph<'a> {
    pub db: &'a Connection,
    /// `files` rows in table order.
    pub files: Vec<FileRow>,
    file_ix: HashMap<i64, usize>,
    by_path: HashMap<String, usize>,
    /// `symbols` rows in table order (`symbolById` order).
    pub syms: Vec<Sym>,
    sym_ix: HashMap<i64, usize>,
    symbols_of: HashMap<i64, Vec<usize>>,
    pub imports: Vec<ImportRow>,
    imports_of: IndexMap<i64, Vec<usize>>,
    pub refs: Vec<RefRow>,
    refs_of: IndexMap<i64, Vec<usize>>,
    pub routes: Vec<RouteRow>,
    def_cache: RefCell<HashMap<(i64, Option<String>), Option<usize>>>,
    binding_index: OnceCell<BindingIndex>,
    data_model_cache: OnceCell<Option<Value>>,
}

fn group(rows: &[i64]) -> IndexMap<i64, Vec<usize>> {
    let mut m: IndexMap<i64, Vec<usize>> = IndexMap::new();
    for (i, f) in rows.iter().enumerate() {
        m.entry(*f).or_default().push(i);
    }
    m
}

/// `graph(db)`: the whole graph in memory, with per-query memoisation.
pub fn graph(db: &Connection) -> Result<Graph<'_>> {
    let files: Vec<FileRow> = {
        let mut st = db.prepare("SELECT * FROM files")?;
        let rows = st.query_map([], |r| {
            Ok(FileRow {
                id: r.get("id")?,
                path: r.get("path")?,
                lang: r.get("lang")?,
                mtime: r.get("mtime")?,
                size: r.get("size")?,
                directive: r.get("directive")?,
                parse_error: r.get("parse_error")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let syms: Vec<Sym> = {
        let mut st = db.prepare("SELECT * FROM symbols")?;
        let rows = st.query_map([], |r| {
            Ok(Sym {
                id: r.get("id")?,
                file_id: r.get("file_id")?,
                name: r.get("name")?,
                kind: r.get("kind")?,
                exported: r.get("exported")?,
                is_default: r.get("is_default")?,
                line: r.get("line")?,
                end_line: r.get("end_line")?,
                props: r.get("props")?,
                hash: r.get("hash")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let imports: Vec<ImportRow> = {
        let mut st = db.prepare("SELECT * FROM imports")?;
        let rows = st.query_map([], |r| {
            Ok(ImportRow {
                id: r.get("id")?,
                file_id: r.get("file_id")?,
                kind: r.get("kind")?,
                source: r.get("source")?,
                imported: r.get("imported")?,
                local: r.get("local")?,
                type_only: r.get("type_only")?,
                line: r.get("line")?,
                target_file_id: r.get("target_file_id")?,
                external: r.get("external")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let refs: Vec<RefRow> = {
        let mut st = db.prepare("SELECT * FROM refs")?;
        let rows = st.query_map([], |r| {
            Ok(RefRow {
                id: r.get("id")?,
                file_id: r.get("file_id")?,
                symbol_id: r.get("symbol_id")?,
                local: r.get("local")?,
                member: r.get("member")?,
                kind: r.get("kind")?,
                line: r.get("line")?,
                count: r.get("count")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let routes: Vec<RouteRow> = {
        let mut st = db.prepare("SELECT * FROM routes")?;
        let rows = st.query_map([], |r| {
            Ok(RouteRow {
                id: r.get("id")?,
                file_id: r.get("file_id")?,
                framework: r.get("framework")?,
                path: r.get("path")?,
                kind: r.get("kind")?,
                root: r.get("root")?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut file_ix = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        file_ix.insert(f.id, i);
    }
    let mut by_path = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        by_path.insert(f.path.clone(), i);
    }
    let mut sym_ix = HashMap::new();
    let mut symbols_of: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, s) in syms.iter().enumerate() {
        sym_ix.insert(s.id, i);
        symbols_of.entry(s.file_id).or_default().push(i);
    }
    let imports_of = group(&imports.iter().map(|i| i.file_id).collect::<Vec<_>>());
    let refs_of = group(&refs.iter().map(|r| r.file_id).collect::<Vec<_>>());
    let mut g = Graph {
        db,
        files,
        file_ix,
        by_path,
        syms,
        sym_ix,
        symbols_of,
        imports,
        imports_of,
        refs,
        refs_of,
        routes,
        def_cache: RefCell::new(HashMap::new()),
        binding_index: OnceCell::new(),
        data_model_cache: OnceCell::new(),
    };

    // Dart classes that extend a project widget (class Card extends AppCard) are widgets too.
    for _pass in 0..3 {
        for i in 0..g.syms.len() {
            if g.syms[i].kind != "class" || g.syms[i].props.as_deref().is_none_or(str::is_empty) {
                continue;
            }
            let ext = g.syms[i].props.as_deref().and_then(|p| json_parse(p).ok()).and_then(|v| v.get("extends").cloned());
            let Some(ext) = ext.filter(|e| t(Some(e))) else { continue };
            let base = g.binding_target(g.syms[i].file_id, &s(Some(&ext))).and_then(|b| b.symbol);
            if base.is_some_and(|b| g.syms[b].kind == "widget") {
                g.syms[i].kind = "widget".into();
            }
        }
    }
    // Anything rendered as <Name /> is a component, even when its own body has no JSX.
    let ref_files: Vec<i64> = g.refs_of.keys().copied().collect();
    for file_id in ref_files {
        let list = g.refs_of[&file_id].clone();
        for ri in list {
            if g.refs[ri].kind != "jsx" {
                continue;
            }
            let local = g.refs[ri].local.clone();
            if let Some(si) = g.binding_target(file_id, &local).and_then(|b| b.symbol) {
                if g.refs[ri].member.is_none() && (g.syms[si].kind == "function" || g.syms[si].kind == "const") {
                    g.syms[si].kind = "component".into();
                }
            }
        }
    }
    Ok(g)
}

impl<'a> Graph<'a> {
    /// `files.get(id)`.
    pub fn file(&self, id: i64) -> Option<&FileRow> {
        self.file_ix.get(&id).map(|i| &self.files[*i])
    }
    pub fn file_index(&self, id: i64) -> Option<usize> {
        self.file_ix.get(&id).copied()
    }
    /// `symbolById.get(id)` (index into `syms`).
    pub fn sym_index(&self, id: i64) -> Option<usize> {
        self.sym_ix.get(&id).copied()
    }
    /// `symbolsOf.get(fileId) ?? []`.
    pub fn symbols_of(&self, file_id: i64) -> &[usize] {
        self.symbols_of.get(&file_id).map(Vec::as_slice).unwrap_or(&[])
    }
    /// `importsOf.get(fileId) ?? []`.
    pub fn imports_of(&self, file_id: i64) -> &[usize] {
        self.imports_of.get(&file_id).map(Vec::as_slice).unwrap_or(&[])
    }
    /// `refsOf.get(fileId) ?? []`.
    pub fn refs_of(&self, file_id: i64) -> &[usize] {
        self.refs_of.get(&file_id).map(Vec::as_slice).unwrap_or(&[])
    }
    /// `[...importsOf]` (file id, import indices) in JS Map order.
    pub fn imports_by_file(&self) -> impl Iterator<Item = (i64, &[usize])> {
        self.imports_of.iter().map(|(k, v)| (*k, v.as_slice()))
    }
    /// `[...refsOf]` in JS Map order.
    pub fn refs_by_file(&self) -> impl Iterator<Item = (i64, &[usize])> {
        self.refs_of.iter().map(|(k, v)| (*k, v.as_slice()))
    }
    /// `[...importsOf.values()].flat()`.
    pub fn imports_flat(&self) -> impl Iterator<Item = &ImportRow> {
        self.imports_of.values().flatten().map(|i| &self.imports[*i])
    }
    /// The file path of a symbol (`s.file.path`).
    pub fn sym_path(&self, si: usize) -> &str {
        self.file(self.syms[si].file_id).map(|f| f.path.as_str()).unwrap_or("undefined")
    }
    /// The file path of a file id (`files.get(id).path`).
    pub fn path_of(&self, file_id: i64) -> &str {
        self.file(file_id).map(|f| f.path.as_str()).unwrap_or("undefined")
    }

    /// `definition(fileId, name)`: where an exported name of a file is really declared, following
    /// re-exports and barrels.
    pub fn definition(&self, file_id: i64, name: Option<&str>) -> Option<usize> {
        self.definition_seen(file_id, name, &mut HashSet::new())
    }

    fn definition_seen(&self, file_id: i64, name: Option<&str>, seen: &mut HashSet<(i64, Option<String>)>) -> Option<usize> {
        let key = (file_id, name.map(String::from));
        if let Some(v) = self.def_cache.borrow().get(&key) {
            return *v;
        }
        if seen.contains(&key) {
            return None;
        }
        seen.insert(key.clone());
        let syms = self.symbols_of(file_id);
        let own = syms.iter().copied().find(|&i| {
            let s = &self.syms[i];
            s.exported != 0 && s.kind != "alias" && if name == Some("default") { s.is_default != 0 } else { Some(s.name.as_str()) == name }
        });
        let alias = if own.is_none() {
            syms.iter().copied().find(|&i| {
                let s = &self.syms[i];
                s.exported != 0 && s.kind == "alias" && Some(s.name.as_str()) == name
            })
        } else {
            None
        };
        let mut out = None;
        if own.is_some() {
            out = own;
        } else if let Some(a) = alias {
            let of = self.syms[a].props.as_deref().and_then(|p| json_parse(p).ok()).and_then(|v| v.get("of").cloned());
            out = syms
                .iter()
                .copied()
                .find(|&i| matches!(&of, Some(Value::String(o)) if *o == self.syms[i].name) && self.syms[i].kind != "alias");
        } else {
            for &ii in self.imports_of(file_id) {
                let imp = &self.imports[ii];
                if imp.kind == "reexport" && imp.local.as_deref() == name {
                    if let Some(tf) = imp.target_file_id.filter(|x| *x != 0) {
                        out = self.definition_seen(tf, imp.imported.as_deref(), seen);
                    }
                }
                if out.is_some() {
                    break;
                }
            }
            if out.is_none() {
                for &ii in self.imports_of(file_id) {
                    let imp = &self.imports[ii];
                    if imp.kind == "reexport_all" && name != Some("default") {
                        if let Some(tf) = imp.target_file_id.filter(|x| *x != 0) {
                            out = self.definition_seen(tf, name, seen);
                        }
                    }
                    if out.is_some() {
                        break;
                    }
                }
            }
        }
        self.def_cache.borrow_mut().insert(key, out);
        out
    }

    /// `bindingTarget(fileId, local)`.
    pub fn binding_target(&self, file_id: i64, local: &str) -> Option<Binding> {
        let imp = self
            .imports_of(file_id)
            .iter()
            .map(|i| &self.imports[*i])
            .find(|i| i.kind == "import" && i.local.as_deref() == Some(local));
        if let Some(imp) = imp {
            if let Some(e) = imp.external.as_deref().filter(|e| !e.is_empty()) {
                return Some(Binding {
                    external: Some(e.to_string()),
                    imported: imp.imported.clone(),
                    ..Default::default()
                });
            }
            let Some(tf) = imp.target_file_id.filter(|x| *x != 0) else {
                return Some(Binding {
                    unresolved: Some(imp.source.clone()),
                    ..Default::default()
                });
            };
            if imp.imported.as_deref() == Some("*") {
                return Some(Binding {
                    file: self.file_index(tf),
                    namespace: true,
                    ..Default::default()
                });
            }
            return Some(Binding {
                symbol: self.definition(tf, imp.imported.as_deref()),
                file: self.file_index(tf),
                imported: imp.imported.clone(),
                ..Default::default()
            });
        }
        let own = self
            .symbols_of(file_id)
            .iter()
            .copied()
            .find(|&i| self.syms[i].name == local && self.syms[i].kind != "alias");
        if let Some(o) = own {
            return Some(Binding {
                symbol: Some(o),
                ..Default::default()
            });
        }
        // Dart: `import 'x.dart'` brings in every public name of x (and of what x exports).
        if self.file(file_id).is_some_and(|f| f.lang == "dart") {
            for &ii in self.imports_of(file_id) {
                let imp = &self.imports[ii];
                if imp.kind != "import" || imp.local.as_deref().is_some_and(|l| !l.is_empty()) {
                    continue;
                }
                let Some(tf) = imp.target_file_id.filter(|x| *x != 0) else { continue };
                if let Some(def) = self.definition(tf, Some(local)) {
                    return Some(Binding {
                        symbol: Some(def),
                        file: self.file_index(tf),
                        ..Default::default()
                    });
                }
            }
        }
        None
    }

    fn index(&self) -> &BindingIndex {
        self.binding_index.get_or_init(|| {
            let mut ix = BindingIndex::default();
            for (file_id, imps) in self.imports_of.iter() {
                for &ii in imps {
                    let imp = &self.imports[ii];
                    let Some(tf) = imp.target_file_id.filter(|x| *x != 0) else { continue };
                    let Some(local) = imp.local.as_deref().filter(|l| !l.is_empty()) else { continue };
                    if imp.kind != "import" {
                        continue;
                    }
                    if imp.imported.as_deref() == Some("*") {
                        ix.namespace.push((*file_id, ii));
                        continue;
                    }
                    let Some(def) = self.definition(tf, imp.imported.as_deref()) else { continue };
                    ix.by_symbol.entry(def).or_default().push((*file_id, local.to_string()));
                }
            }
            for (file_id, refs) in self.refs_of.iter() {
                if !self.file(*file_id).is_some_and(|f| f.lang == "dart") {
                    continue;
                }
                let mut seen = HashSet::new();
                for &ri in refs {
                    let r = &self.refs[ri];
                    if !seen.insert(r.local.clone()) {
                        continue;
                    }
                    let Some(sym) = self.binding_target(*file_id, &r.local).and_then(|b| b.symbol) else {
                        continue;
                    };
                    if self.syms[sym].file_id == *file_id {
                        continue;
                    }
                    ix.by_symbol.entry(sym).or_default().push((*file_id, r.local.clone()));
                }
            }
            ix
        })
    }

    /// `referencesTo(sym)`: every place that uses a symbol (same file, imports, barrels, namespaces).
    pub fn references_to(&self, si: usize) -> Vec<RefHit> {
        let ix = self.index();
        let sym = &self.syms[si];
        let mut out = Vec::new();
        let mut push = |file_id: i64, local: &str, member: Option<&str>| {
            let Some(fi) = self.file_index(file_id) else {
                // `files.get(fileId)` undefined: the JS pushes { file: undefined }; never happens with a consistent db
                return;
            };
            for &ri in self.refs_of(file_id) {
                let r = &self.refs[ri];
                if r.local != local {
                    continue;
                }
                if let Some(m) = member {
                    if r.member.as_deref() != Some(m) && !r.member.as_deref().unwrap_or("").starts_with(&format!("{m}.")) {
                        continue;
                    }
                }
                if r.symbol_id == Some(sym.id) {
                    continue;
                }
                out.push(RefHit {
                    r: ri,
                    file: fi,
                    owner: r.symbol_id.and_then(|id| self.sym_index(id)),
                });
            }
        };
        push(sym.file_id, &sym.name, None);
        if let Some(list) = ix.by_symbol.get(&si) {
            for (f, l) in list {
                push(*f, l, None);
            }
        }
        for (file_id, ii) in &ix.namespace {
            let imp = &self.imports[*ii];
            if let Some(tf) = imp.target_file_id {
                if self.definition(tf, Some(&sym.name)) == Some(si) {
                    push(*file_id, imp.local.as_deref().unwrap_or(""), Some(&sym.name));
                }
            }
        }
        out
    }

    /// `lookup(target, { routesFirst })`.
    pub fn lookup(&self, target: &str, routes_first: bool) -> Lookup {
        if target.starts_with('/') && !target.contains('.') {
            let rs: Vec<usize> = (0..self.routes.len())
                .filter(|&i| self.routes[i].path == target || route_matches(&self.routes[i].path, target))
                .collect();
            if !rs.is_empty() {
                return Lookup {
                    routes: Some(rs),
                    ..Default::default()
                };
            }
        }
        if routes_first {
            let screens: Vec<usize> = (0..self.routes.len())
                .filter(|&i| self.routes[i].framework == "flutter" && self.routes[i].path == target)
                .collect();
            if !screens.is_empty() {
                return Lookup {
                    routes: Some(screens),
                    ..Default::default()
                };
            }
        }
        let mut parts = target.split('#');
        let path_part = parts.next().unwrap_or("");
        let sym_part = parts.next();
        let file = self.file_by_path(path_part);
        if let (Some(fi), Some(sp)) = (file, sym_part.filter(|x| !x.is_empty())) {
            let sy: Vec<usize> = self
                .symbols_of(self.files[fi].id)
                .iter()
                .copied()
                .filter(|&i| self.syms[i].name == sp || (sp == "default" && self.syms[i].is_default != 0))
                .collect();
            return if sy.is_empty() {
                Lookup {
                    error: Some(format!("No symbol \"{sp}\" in {}", self.files[fi].path)),
                    ..Default::default()
                }
            } else {
                Lookup {
                    symbols: Some(sy),
                    ..Default::default()
                }
            };
        }
        if let Some(fi) = file {
            return Lookup {
                file: Some(fi),
                ..Default::default()
            };
        }
        let exact: Vec<usize> = (0..self.syms.len()).filter(|&i| self.syms[i].name == target && self.syms[i].kind != "alias").collect();
        if !exact.is_empty() {
            return Lookup {
                symbols: Some(exact),
                ..Default::default()
            };
        }
        Lookup {
            error: Some(format!("Nothing named \"{target}\". Try find {target}.")),
            ..Default::default()
        }
    }

    /// `fileByPath(p)`: exact path, or the one file whose path ends with `/p`.
    pub fn file_by_path(&self, p: &str) -> Option<usize> {
        if p.is_empty() {
            return None;
        }
        let norm = re!(r"^\.\/").replace_first_str(&p.replace('\\', "/"), "");
        if let Some(i) = self.by_path.get(&norm) {
            return Some(*i);
        }
        let suffix = format!("/{norm}");
        let hits: Vec<usize> = (0..self.files.len()).filter(|&i| self.files[i].path.ends_with(&suffix)).collect();
        if hits.len() == 1 {
            Some(hits[0])
        } else {
            None
        }
    }

    /// `byPath.get(p)`: exact path only.
    pub fn file_by_path_exact(&self, p: &str) -> Option<usize> {
        self.by_path.get(p).copied()
    }

    /// The `data_model` meta, parsed once (`null` when missing or unreadable).
    pub fn data_model(&self) -> Option<&Value> {
        self.data_model_cache
            .get_or_init(|| {
                let v: Option<Option<String>> = self
                    .db
                    .query_row("SELECT value FROM meta WHERE key = 'data_model'", [], |r| r.get(0))
                    .optional()
                    .ok()
                    .flatten();
                match v.flatten() {
                    None => None,
                    Some(text) => json_parse(&text).ok().filter(|v| !v.is_null()),
                }
            })
            .as_ref()
    }

    /// The top-level symbol whose lines contain `line` (the function doing a query), smallest first.
    pub fn owner_at(&self, file_id: i64, line: Option<i64>) -> Option<usize> {
        let line = line.unwrap_or(0);
        let mut c: Vec<usize> = self
            .symbols_of(file_id)
            .iter()
            .copied()
            .filter(|&i| {
                let s = &self.syms[i];
                s.kind != "alias" && s.line.unwrap_or(0) <= line && s.end_line.or(s.line).unwrap_or(0) >= line
            })
            .collect();
        c.sort_by_key(|&i| self.syms[i].end_line.unwrap_or(0) - self.syms[i].line.unwrap_or(0));
        c.first().copied()
    }

    /// `where(s)`: `path:line`.
    pub fn where_(&self, si: usize) -> String {
        format!("{}:{}", self.sym_path(si), num(self.syms[si].line))
    }

    /// `label(s)`: `Name [kind]` / `Name [kind, private]` / `path (default export) [kind]`.
    pub fn label(&self, si: usize) -> String {
        let s = &self.syms[si];
        let name = if s.name == "default" {
            format!("{} (default export)", self.sym_path(si))
        } else {
            s.name.clone()
        };
        format!("{name} [{}{}]", s.kind, if s.exported != 0 { "" } else { ", private" })
    }

    fn routes_of_file(&self, file_id: i64) -> Vec<String> {
        self.routes.iter().filter(|r| r.file_id == file_id).map(|r| format!("{} {}", r.kind, r.path)).collect()
    }
}

// ---------- runQuery ----------

/// Options of [`run_query`] (`{ depth, limit = 40, root }`).
#[derive(Clone, Debug)]
pub struct QueryOptions {
    pub depth: Option<i64>,
    pub limit: i64,
    /// The project folder as the caller gave it (`dir`); enables the brain signposts.
    pub root: Option<String>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        QueryOptions {
            depth: None,
            limit: 40,
            root: None,
        }
    }
}

/// `runQuery(db, query, target, { depth, limit, root })`.
pub fn run_query(db: &Connection, query: &str, target: Option<&str>, opts: &QueryOptions) -> Result<String> {
    let g = graph(db)?;
    let text = answer(&g, query, target, opts)?;
    Ok(match opts.root.as_deref() {
        Some(root) if !root.is_empty() && ["find", "file", "used_by", "uses", "impact"].contains(&query) => text + &brain_signpost(&g, root, query, target),
        _ => text,
    })
}

// The brain's notes about what was asked, one line each.
fn brain_signpost(g: &Graph, root: &str, query: &str, target: Option<&str>) -> String {
    let block = |title: &str, rows: Vec<(String, String)>| -> String {
        if rows.is_empty() {
            return String::new();
        }
        let list: Vec<String> = rows
            .iter()
            .map(|(n, l)| format!("- {n}{}", if l.is_empty() { String::new() } else { format!(" — {l}") }))
            .collect();
        format!("\n\n## {title} ({}) — project_memory read <note>\n{}", rows.len(), list.join("\n"))
    };
    let run = || -> Result<String> {
        if query == "find" {
            let hits = brain::find_notes(root, target.unwrap_or("undefined"), 3)?;
            return Ok(block("Brain notes", hits.iter().map(|h| (s(h.get("note")), s(h.get("line")))).collect()));
        }
        let tt = js::js_trim(target.unwrap_or("")).to_string();
        let hit = g.lookup(&tt, false);
        let sym = hit.symbols.as_ref().filter(|v| v.len() == 1).map(|v| v[0]);
        let path = hit
            .file
            .map(|f| g.files[f].path.clone())
            .or_else(|| sym.map(|si| g.sym_path(si).to_string()))
            .unwrap_or_else(|| tt.split('#').next().unwrap_or("").to_string());
        let name = sym.map(|si| g.syms[si].name.clone()).or_else(|| tt.split('#').nth(1).map(String::from));
        let rows = brain::notes_about(root, Some(&path), name.as_deref())?;
        Ok(block("Brain notes linking here", rows.iter().map(|h| (s(h.get("note")), s(h.get("line")))).collect()))
    };
    run().unwrap_or_default()
}

fn answer(g: &Graph, query: &str, target: Option<&str>, opts: &QueryOptions) -> Result<String> {
    let limit = opts.limit;
    let root = opts.root.as_deref();
    Ok(match query {
        "overview" => overview(g, limit)?,
        "find" => find(g, &need(target, query)?, limit),
        "file" => file_info(g, &need(target, query)?)?,
        "used_by" => used_by(g, &need(target, query)?, limit)?,
        "uses" => uses(g, &need(target, query)?, limit)?,
        "impact" => impact(g, &need(target, query)?, opts.depth.unwrap_or(3), limit * 4),
        "route" => route(g, &need(target, query)?, opts.depth.unwrap_or(3)),
        "design" => design_answer(g, root.unwrap_or("undefined"), limit)?,
        "tokens" => tokens(g, target, limit)?,
        "notes" => notes_answer(g, limit)?,
        "data" => data_overview(g, limit)?,
        "table" => table_info(g, &need(target, query)?, limit)?,
        "flows" => flows::flows_text(g, root.unwrap_or("undefined"), target, limit)?,
        "trace" => flows::trace_text(g, &need(target, query)?)?,
        _ => bail!("Unknown query \"{query}\". Use one of: {}", QUERIES.join(", ")),
    })
}

fn need(target: Option<&str>, query: &str) -> Result<String> {
    match target {
        Some(t) if !t.is_empty() => Ok(js::js_trim(t).to_string()),
        _ => bail!("\"{query}\" needs a target: a symbol name (Button), a file path (components/ui/button.tsx), path#Symbol, or a route (/jobs/[id])"),
    }
}

// ---------- formatting ----------

fn more(n: usize, shown: i64) -> Vec<String> {
    if n as i64 > shown {
        vec![format!("… {} more (raise limit)", n as i64 - shown)]
    } else {
        Vec::new()
    }
}

fn take(limit: i64) -> usize {
    limit.max(0) as usize
}

fn ambiguous(g: &Graph, syms: &[usize]) -> String {
    let mut lines = vec![format!("\"{}\" is declared in {} places — pick one with path#Name:", g.syms[syms[0]].name, syms.len())];
    for &si in syms {
        lines.push(format!("  {}#{} [{}] :{}", g.sym_path(si), g.syms[si].name, g.syms[si].kind, num(g.syms[si].line)));
    }
    lines.join("\n")
}

// ---------- data model ----------

/// A `data_refs` row with its file, columns/filters parsed (`dataRefs` in JS).
#[derive(Clone, Debug)]
pub struct DataRefRow {
    pub file_id: i64,
    pub form: String,
    pub entity: String,
    pub op: String,
    pub detail: Option<String>,
    pub columns: Option<Value>,
    pub filters: Option<Value>,
    pub via: Option<String>,
    pub line: Option<i64>,
    pub path: String,
}

fn parse_json_col(v: Option<String>) -> Option<Value> {
    v.filter(|s| !s.is_empty()).and_then(|s| json_parse(&s).ok())
}

/// `SELECT * FROM data_refs WHERE [entity = ? AND] op != 'ddl'` with files and parsed JSON columns.
pub fn data_refs(g: &Graph, entity: Option<&str>) -> Result<Vec<DataRefRow>> {
    let sql = if entity.is_some() {
        "SELECT * FROM data_refs WHERE entity = ? AND op != 'ddl'"
    } else {
        "SELECT * FROM data_refs WHERE op != 'ddl'"
    };
    let mut st = g.db.prepare(sql)?;
    let map = |r: &rusqlite::Row| -> rusqlite::Result<DataRefRow> {
        let file_id: i64 = r.get("file_id")?;
        Ok(DataRefRow {
            file_id,
            form: r.get("form")?,
            entity: r.get("entity")?,
            op: r.get("op")?,
            detail: r.get("detail")?,
            columns: parse_json_col(r.get("columns")?),
            filters: parse_json_col(r.get("filters")?),
            via: r.get("via")?,
            line: r.get("line")?,
            path: g.path_of(file_id).to_string(),
        })
    };
    let rows = match entity {
        Some(e) => st.query_map([e], map)?.collect::<rusqlite::Result<Vec<_>>>()?,
        None => st.query_map([], map)?.collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok(rows)
}

// Refs to an entity by its model name or its table name (Prisma User / users).
fn data_refs_for(g: &Graph, ent: &Value) -> Result<Vec<DataRefRow>> {
    let mut names: Vec<String> = Vec::new();
    for k in ["name", "table"] {
        if t(ent.get(k)) {
            let n = s(ent.get(k));
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    let mut out = Vec::new();
    for n in &names {
        out.extend(data_refs(g, Some(n))?);
    }
    out.sort_by(|a, b| locale_compare(&a.path, &b.path).then(a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0))));
    Ok(out)
}

fn col_flags(c: &Value) -> String {
    let mut f: Vec<String> = Vec::new();
    if t(c.get("db")) {
        f.push(format!("column {}", s(c.get("db"))));
    }
    if t(c.get("pk")) {
        f.push("PK".into());
    }
    if t(c.get("notNull")) && !t(c.get("pk")) {
        f.push("not null".into());
    }
    if t(c.get("unique")) {
        f.push("unique".into());
    }
    if t(c.get("ref")) {
        let r = c.get("ref").unwrap();
        f.push(format!("→ {}.{}", s(r.get("table")), s(r.get("column"))));
    }
    if t(c.get("default")) {
        f.push(format!("= {}", s(c.get("default"))));
    }
    f.join(", ")
}

fn col_line(name: &str, c: &Value) -> String {
    let flags = col_flags(c);
    format!("{name}  {}{}", s(c.get("type")), if flags.is_empty() { String::new() } else { format!("  ({flags})") })
}

fn data_overview(g: &Graph, limit: i64) -> Result<String> {
    let dm = g.data_model();
    let stores = dm.map(|d| arr(d.get("stores"))).unwrap_or(&[]);
    if stores.is_empty() {
        return Ok("No data model found. Read: Supabase migrations, schema.prisma, .sql files, and CREATE/ALTER TABLE strings in TS/JS/Dart code (tests excluded). Not read: Drizzle, Mongo/NoSQL, Go struct ORMs.".into());
    }
    let refs = data_refs(g, None)?;
    let mut lines: Vec<String> = Vec::new();
    for st in stores {
        let ents = values(st.get("entities"));
        let unparsed = arr(st.get("unparsed"));
        lines.push(format!(
            "# {} — from {} source file(s){}",
            s(st.get("label")),
            len_of(st.get("sources")),
            if unparsed.is_empty() {
                String::new()
            } else {
                format!(" · {} statement(s) not understood", unparsed.len())
            }
        ));
        if len_of(st.get("skipped")) > 0 {
            lines.push(format!(
                "Skipped database dumps (snapshots, not schema history): {}",
                join_values(arr(st.get("skipped")), ", ")
            ));
        }
        for u in unparsed.iter().take(3) {
            lines.push(format!("Not understood: {}:{} {}", s(u.get("file")), s(u.get("line")), s(u.get("sql"))));
        }
        let use_ = |e: &Value| -> String {
            let mut names: HashSet<String> = HashSet::new();
            for k in ["name", "table"] {
                if t(e.get(k)) {
                    names.insert(s(e.get(k)));
                }
            }
            let rs: Vec<&DataRefRow> = refs.iter().filter(|r| names.contains(&r.entity)).collect();
            let files = |op: &str| rs.iter().filter(|r| r.op == op).map(|r| r.file_id).collect::<HashSet<_>>().len();
            if rs.is_empty() {
                " · not used in code".into()
            } else {
                let call = files("call");
                format!(
                    " · read in {} file(s), written in {}{}",
                    files("read"),
                    files("write"),
                    if call > 0 { format!(", called in {call}") } else { String::new() }
                )
            }
        };
        let tables: Vec<&Value> = ents.iter().copied().filter(|x| s(x.get("kind")) == "table" && !t(x.get("external"))).collect();
        let supabase = s(st.get("kind")) == "supabase";
        let mut groups: IndexMap<String, Vec<&Value>> = IndexMap::new();
        for e in &tables {
            let k = if supabase {
                "all".to_string()
            } else {
                match e.get("source").and_then(|x| x.get("file")) {
                    Some(f) if !f.is_null() => s(Some(f)),
                    _ => "(never created in a schema file)".to_string(),
                }
            };
            groups.entry(k).or_default().push(e);
        }
        let mut shown = 0i64;
        let ngroups = groups.len();
        for (file, list) in &groups {
            lines.push(String::new());
            lines.push(if ngroups > 1 {
                format!("## Tables in {file} ({})", list.len())
            } else {
                "## Tables".into()
            });
            for e in list {
                let over = shown >= limit;
                shown += 1;
                if over {
                    break;
                }
                let rls = if !supabase {
                    String::new()
                } else if t(e.get("outsideMigrations")) {
                    if t(e.get("rls")) {
                        ", RLS on".into()
                    } else {
                        ", RLS unknown".into()
                    }
                } else if t(e.get("rls")) {
                    format!(", RLS {} policies", len_of(e.get("policies")))
                } else {
                    ", NO RLS".into()
                };
                lines.push(format!(
                    "{} ({} cols{rls}){}{}{}",
                    s(e.get("name")),
                    len_of(e.get("columns")),
                    use_(e),
                    if t(e.get("outsideMigrations")) {
                        " · ⚠ never created in a migration/schema file — columns incomplete"
                    } else {
                        ""
                    },
                    if t(e.get("unknownColumns")) {
                        format!(" · ⚠ indexes columns not defined: {}", join_values(arr(e.get("unknownColumns")), ", "))
                    } else {
                        String::new()
                    }
                ));
            }
        }
        lines.extend(more(tables.len(), limit));
        let views: Vec<&Value> = ents.iter().copied().filter(|x| s(x.get("kind")) == "view").collect();
        if !views.is_empty() {
            lines.push(String::new());
            lines.push("## Views".into());
            for v in &views {
                lines.push(format!("{}{}", s(v.get("name")), use_(v)));
            }
        }
        let fns: Vec<&Value> = ents.iter().copied().filter(|x| s(x.get("kind")) == "function").collect();
        if !fns.is_empty() {
            let called: HashSet<&str> = refs.iter().filter(|r| r.op == "call").map(|r| r.entity.as_str()).collect();
            let list: Vec<String> = fns.iter().filter(|f| called.contains(s(f.get("name")).as_str())).map(|f| s(f.get("name"))).collect();
            lines.push(String::new());
            lines.push(format!(
                "## Functions ({}) — called from code: {}",
                fns.len(),
                if list.is_empty() { "none".into() } else { list.join(", ") }
            ));
        }
        let ext: Vec<String> = ents.iter().filter(|x| t(x.get("external"))).map(|x| s(x.get("name"))).collect();
        if !ext.is_empty() {
            lines.push(String::new());
            lines.push(format!("Platform tables referenced: {}", ext.join(", ")));
        }
    }
    let mut known: HashSet<String> = HashSet::new();
    for st in stores {
        for e in values(st.get("entities")) {
            for k in ["name", "table"] {
                if t(e.get(k)) {
                    known.insert(s(e.get(k)));
                }
            }
        }
    }
    let mut unknown: Vec<&str> = Vec::new();
    for r in &refs {
        if !known.contains(&r.entity) && r.form != "prisma" && !unknown.contains(&r.entity.as_str()) {
            unknown.push(&r.entity);
        }
    }
    if !unknown.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "## Used in code but not in any schema source ({}) — made outside migrations, misspelled, a CTE/alias, or created elsewhere",
            unknown.len()
        ));
        lines.push(unknown.join(", "));
    }
    Ok(lines.join("\n"))
}

fn table_info(g: &Graph, name: &str, limit: i64) -> Result<String> {
    let Some(dm) = g.data_model() else {
        return Ok(format!("No table/view/function \"{name}\" in the data model. Try: code_graph data"));
    };
    let Some((store, ent)) = find_entity(dm, &re!(r"^table:").replace_first_str(name, "")) else {
        return Ok(format!("No table/view/function \"{name}\" in the data model. Try: code_graph data"));
    };
    let origin = if t(ent.get("source")) {
        let src = ent.get("source").unwrap();
        format!(" — created {}:{}", s(src.get("file")), s(src.get("line")))
    } else if t(ent.get("outsideMigrations")) {
        " — ⚠ created outside migrations (dashboard?); only later changes are in code, so columns/RLS below are incomplete".into()
    } else {
        " (platform table, not created by this project)".into()
    };
    let changed = arr(ent.get("changedIn"));
    let mut lines = vec![format!(
        "{} {}{origin}{}",
        s(ent.get("kind")),
        s(ent.get("name")),
        if changed.is_empty() {
            String::new()
        } else {
            format!(", changed in {}", join_values(changed, ", "))
        }
    )];
    let ename = s(ent.get("name"));
    let columns = arr(ent.get("columns"));
    if !columns.is_empty() {
        lines.push(String::new());
        lines.push("## Columns".into());
        for c in columns {
            lines.push(col_line(&s(c.get("name")), c));
        }
    }
    if t(ent.get("unknownColumns")) {
        lines.push(format!("⚠ Indexed but not defined in any migration: {}", join_values(arr(ent.get("unknownColumns")), ", ")));
    }
    let indexes = arr(ent.get("indexes"));
    if !indexes.is_empty() {
        lines.push(String::new());
        lines.push("## Indexes".into());
        for i in indexes {
            let nm = match i.get("name") {
                None | Some(Value::Null) => "(inline)".to_string(),
                v => s(v),
            };
            lines.push(format!(
                "{}{nm} ({})",
                if t(i.get("unique")) { "unique " } else { "" },
                join_values(arr(i.get("columns")), ", ")
            ));
        }
    }
    let mut incoming: Vec<String> = Vec::new();
    for o in values(store.get("entities")) {
        for f in arr(o.get("fks")) {
            if f.get("table") == Some(&Value::String(ename.clone())) {
                incoming.push(format!("{}.{}", s(o.get("name")), join_values(arr(f.get("columns")), ",")));
            }
        }
    }
    let fks = arr(ent.get("fks"));
    if !fks.is_empty() || !incoming.is_empty() {
        lines.push(String::new());
        lines.push("## Relations".into());
        for f in fks {
            lines.push(format!(
                "→ {} references {}.{}",
                join_values(arr(f.get("columns")), ","),
                s(f.get("table")),
                join_values(arr(f.get("refColumns")), ",")
            ));
        }
        for x in &incoming {
            lines.push(format!("← {x}"));
        }
    }
    let policies = arr(ent.get("policies"));
    if s(ent.get("kind")) == "table" && !t(ent.get("external")) && s(store.get("kind")) == "supabase" {
        lines.push(String::new());
        if t(ent.get("rls")) {
            lines.push(format!(
                "## RLS on — {} policies{}",
                policies.len(),
                if policies.is_empty() { " (none: only the service role can access it)" } else { "" }
            ));
        } else if t(ent.get("outsideMigrations")) {
            lines.push("## RLS unknown — not enabled in migrations; check the dashboard and add it to a migration".into());
        } else {
            lines.push("## RLS OFF — anyone with the anon key can read/write it".into());
        }
    }
    for p in policies {
        lines.push(format!(
            "{} [{}{}]{}{}",
            s(p.get("name")),
            s(p.get("cmd")),
            if t(p.get("roles")) { format!(" to {}", s(p.get("roles"))) } else { String::new() },
            if t(p.get("using")) {
                format!(" using ({})", slice_utf16(&s(p.get("using")), 0, 100))
            } else {
                String::new()
            },
            if t(p.get("check")) {
                format!(" check ({})", slice_utf16(&s(p.get("check")), 0, 80))
            } else {
                String::new()
            }
        ));
    }
    if len_of(ent.get("reads")) > 0 {
        lines.push(String::new());
        lines.push(format!("Reads: {}", join_values(arr(ent.get("reads")), ", ")));
    }
    let refs = data_refs_for(g, ent)?;
    if !refs.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Used in code ({})", refs.len()));
        for r in refs.iter().take(take(limit)) {
            let owner = g.owner_at(r.file_id, r.line);
            lines.push(format!(
                "{} {}{}{}{}  {}:{}{}",
                pad_end(&r.op, 5),
                r.detail.clone().unwrap_or_else(|| "null".into()),
                match &r.columns {
                    Some(c) if t(Some(c)) => format!(" [{}]", join_values(arr(Some(c)), ", ")),
                    _ => String::new(),
                },
                match &r.filters {
                    Some(c) if t(Some(c)) => format!(" by {}", join_values(arr(Some(c)), ", ")),
                    _ => String::new(),
                },
                r.via.as_deref().filter(|v| !v.is_empty()).map(|v| format!(" (embedded from {v})")).unwrap_or_default(),
                r.path,
                num(r.line),
                owner.map(|o| format!(" in {}", g.syms[o].name)).unwrap_or_default()
            ));
        }
        lines.extend(more(refs.len(), limit));
        // Filters on columns without an index: candidates for one.
        let mut indexed: HashSet<String> = HashSet::new();
        for i in indexes {
            if let Some(c) = arr(i.get("columns")).first() {
                indexed.insert(s(Some(c)));
            }
        }
        for c in columns {
            if t(c.get("pk")) || t(c.get("unique")) {
                indexed.insert(s(c.get("name")));
            }
        }
        let mut unindexed: Vec<String> = Vec::new();
        for r in &refs {
            for c in arr(r.filters.as_ref()) {
                let c = s(Some(c));
                if !indexed.contains(&c) && c != "order" && !unindexed.contains(&c) {
                    unindexed.push(c);
                }
            }
        }
        if !unindexed.is_empty() && s(ent.get("kind")) == "table" {
            lines.push(String::new());
            lines.push(format!("Filtered on without a leading index: {}", unindexed.join(", ")));
        }
    } else {
        lines.push(String::new());
        lines.push("Not used in code.".into());
    }
    Ok(lines.join("\n"))
}

fn has_str(v: Option<&Value>, x: &str) -> bool {
    arr(v).iter().any(|c| c.as_str() == Some(x))
}

// "orders" or "orders.status" → every query that touches it, then who calls those functions.
fn data_impact(g: &Graph, ent: &Value, column: Option<&str>, depth: i64, cap: i64) -> Result<String> {
    let mut refs = data_refs_for(g, ent)?;
    if let Some(col) = column.filter(|c| !c.is_empty()) {
        refs.retain(|r| {
            !r.columns.as_ref().is_some_and(|c| t(Some(c))) || has_str(r.columns.as_ref(), "*") || has_str(r.columns.as_ref(), col) || has_str(r.filters.as_ref(), col)
        });
    }
    let col = column.filter(|c| !c.is_empty());
    let ename = s(ent.get("name"));
    let mut lines = vec![format!("Impact of changing {ename}{}:", col.map(|c| format!(".{c}")).unwrap_or_default())];
    let store = g.data_model().and_then(|dm| find_entity(dm, &ename)).map(|(st, _)| st);
    let mut related: Vec<String> = Vec::new();
    if let Some(store) = store {
        for o in values(store.get("entities")) {
            for f in arr(o.get("fks")) {
                if f.get("table").and_then(Value::as_str) == Some(ename.as_str()) && col.is_none_or(|c| has_str(f.get("refColumns"), c)) {
                    related.push(format!("FK {}.{}", s(o.get("name")), join_values(arr(f.get("columns")), ",")));
                }
            }
            if has_str(o.get("reads"), &ename) {
                related.push(format!("{} {}", s(o.get("kind")), s(o.get("name"))));
            }
        }
    }
    let policies: Vec<String> = arr(ent.get("policies"))
        .iter()
        .filter(|p| {
            col.is_none_or(|c| {
                let u = match p.get("using") {
                    None | Some(Value::Null) => String::new(),
                    v => s(v),
                };
                let k = match p.get("check") {
                    None | Some(Value::Null) => String::new(),
                    v => s(v),
                };
                format!("{u} {k}").contains(c)
            })
        })
        .map(|p| format!("policy {}", s(p.get("name"))))
        .collect();
    if !related.is_empty() || !policies.is_empty() {
        lines.push("## In the database".into());
        lines.push(related.iter().chain(policies.iter()).cloned().collect::<Vec<_>>().join(" · "));
    }
    if refs.is_empty() {
        lines.push("No code touches it (columns unknown where a query uses a variable).".into());
        return Ok(lines.join("\n"));
    }
    lines.push(format!("## Queries ({})", refs.len()));
    let mut owners: IndexMap<i64, usize> = IndexMap::new();
    for r in refs.iter().take(take(cap)) {
        let owner = g.owner_at(r.file_id, r.line);
        let rs = g.routes_of_file(r.file_id);
        lines.push(format!(
            "{} {}:{}{}{}{}",
            r.op,
            r.path,
            num(r.line),
            owner.map(|o| format!(" in {}", g.syms[o].name)).unwrap_or_default(),
            if has_str(r.columns.as_ref(), "*") && col.is_some() { " (select *)" } else { "" },
            if rs.is_empty() { String::new() } else { format!("  ⚑ {}", rs.join(", ")) }
        ));
        if let Some(o) = owner {
            owners.insert(g.syms[o].id, o);
        }
    }
    // Callers of the functions/components that run those queries.
    let mut seen: HashSet<String> = owners.keys().map(|id| format!("s{id}")).collect();
    let mut frontier: Vec<usize> = owners.values().copied().collect();
    let mut level = 1;
    while level < depth && !frontier.is_empty() {
        let mut next = Vec::new();
        let mut shown = Vec::new();
        for &sym in &frontier {
            for r in g.references_to(sym) {
                let rr = &g.refs[r.r];
                let key = match r.owner {
                    Some(o) => format!("s{}", g.syms[o].id),
                    None => format!("m{}", rr.file_id),
                };
                if !seen.insert(key) {
                    continue;
                }
                let rs = g.routes_of_file(rr.file_id);
                let fpath = &g.files[r.file].path;
                shown.push(format!(
                    "{} {fpath}:{} ← {}{}",
                    match r.owner {
                        Some(o) => g.syms[o].name.clone(),
                        None => format!("{fpath} (module)"),
                    },
                    num(rr.line),
                    g.syms[sym].name,
                    if rs.is_empty() { String::new() } else { format!("  ⚑ {}", rs.join(", ")) }
                ));
                if let Some(o) = r.owner {
                    next.push(o);
                }
            }
        }
        if !shown.is_empty() {
            lines.push(format!("## Callers, level {level} ({})", shown.len()));
            lines.extend(shown.into_iter().take(take(cap)));
        }
        frontier = next;
        level += 1;
    }
    Ok(lines.join("\n"))
}

struct DataTarget<'v> {
    ent: &'v Value,
    column: Option<String>,
    explicit: bool,
}

fn data_target<'v>(g: &'v Graph, target: &str) -> Option<DataTarget<'v>> {
    let dm = g.data_model()?;
    let tt = re!(r"^table:").replace_first_str(target, "");
    if let Some((_, ent)) = find_entity(dm, &tt) {
        return Some(DataTarget {
            ent,
            column: None,
            explicit: target.starts_with("table:"),
        });
    }
    let units: Vec<u16> = tt.encode_utf16().collect();
    let dot = units.iter().rposition(|u| *u == b'.' as u16);
    if let Some(d) = dot.filter(|d| *d > 0) {
        if let Some((_, ent)) = find_entity(dm, &String::from_utf16_lossy(&units[..d])) {
            return Some(DataTarget {
                ent,
                column: Some(String::from_utf16_lossy(&units[d + 1..])),
                explicit: true,
            });
        }
    }
    None
}

// ---------- queries ----------

fn fmt_counts(c: &Counter) -> String {
    c.sorted_desc().iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")
}

fn overview(g: &Graph, limit: i64) -> Result<String> {
    let mut by_lang = Counter::default();
    for f in &g.files {
        by_lang.add(&f.lang, 1);
    }
    let mut by_kind = Counter::default();
    for s in &g.syms {
        by_kind.add(&s.kind, 1);
    }
    let updated: Option<String> = crate::db::get_meta(g.db, "updated_at")?;
    let mut lines = vec![
        format!("Files: {} ({}) · Symbols: {}", g.files.len(), fmt_counts(&by_lang), fmt_counts(&by_kind)),
        format!("Updated: {}", updated.unwrap_or_else(|| "never".into())),
    ];
    // Usage counts for components/hooks: how many places reference each.
    let mut used: Vec<(usize, usize)> = Vec::new();
    for (i, s) in g.syms.iter().enumerate() {
        if s.kind == "component" || s.kind == "hook" || s.kind == "widget" {
            used.push((i, 0));
        }
    }
    for u in used.iter_mut() {
        u.1 = g.references_to(u.0).iter().map(|r| g.refs[r.r].file_id).collect::<HashSet<_>>().len();
    }
    let mut top: Vec<(usize, usize)> = used.iter().copied().filter(|(_, n)| *n > 0).collect();
    top.sort_by_key(|x| std::cmp::Reverse(x.1));
    top.truncate(15);
    if !top.is_empty() {
        lines.push(String::new());
        lines.push("## Most used components/hooks (files using them)".into());
        lines.push(top.iter().map(|(s, n)| format!("{} {n}", g.syms[*s].name)).collect::<Vec<_>>().join(" · "));
    }
    if !g.routes.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Routes ({})", g.routes.len()));
        for r in g.routes.iter().take(take(limit)) {
            lines.push(format!("{} {}  → {}", pad_end(&r.kind, 8), r.path, g.path_of(r.file_id)));
        }
        lines.extend(more(g.routes.len(), limit));
    }
    let route_files: HashSet<i64> = g.routes.iter().map(|r| r.file_id).collect();
    let unused: Vec<String> = used
        .iter()
        .filter(|(s, n)| *n == 0 && g.syms[*s].exported != 0 && !route_files.contains(&g.syms[*s].file_id))
        .map(|(s, _)| format!("{} ({})", g.syms[*s].name, g.sym_path(*s)))
        .collect();
    if !unused.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Exported components/hooks used nowhere ({})", unused.len()));
        lines.push(unused.iter().take(20).cloned().collect::<Vec<_>>().join(" · "));
    }
    let mut ext = Counter::default();
    for i in g.imports_flat() {
        if let Some(e) = i.external.as_deref().filter(|e| !e.is_empty()) {
            ext.add(e, 1);
        }
    }
    let top_ext: Vec<(String, i64)> = ext.sorted_desc().into_iter().take(15).collect();
    if !top_ext.is_empty() {
        lines.push(String::new());
        lines.push("## Packages (imports)".into());
        lines.push(top_ext.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(" · "));
    }
    if let Some(dm) = g.data_model() {
        for st in arr(dm.get("stores")) {
            let ents: Vec<&Value> = values(st.get("entities")).into_iter().filter(|e| !t(e.get("external"))).collect();
            let n = |k: &str| ents.iter().filter(|e| s(e.get("kind")) == k).count();
            lines.push(String::new());
            lines.push(format!(
                "## Data: {} — {} tables, {} views, {} functions (code_graph data / table <name>)",
                s(st.get("label")),
                n("table"),
                n("view"),
                n("function")
            ));
        }
    }
    let unresolved: Vec<&ImportRow> = g.imports_flat().filter(|i| i.target_file_id.is_none() && i.external.is_none()).collect();
    if !unresolved.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Unresolved imports ({}) — usually generated or ignored files", unresolved.len()));
        for i in unresolved.iter().take(10) {
            lines.push(format!("{}: \"{}\"", g.path_of(i.file_id), i.source));
        }
    }
    let ns = notes::all_notes(g.db, g)?;
    let stale = ns.iter().filter(|n| n.status != "ok").count();
    lines.push(String::new());
    lines.push(format!(
        "## Notes ({}{}) — see the notes query",
        ns.len(),
        if stale > 0 { format!(", {stale} stale") } else { String::new() }
    ));
    for n in ns.iter().filter(|n| n.kind == "dir" && n.target.split('/').count() <= 2).take(12) {
        lines.push(format!(
            "{}: {}{}",
            n.label,
            slice_utf16(n.text.split('\n').next().unwrap_or(""), 0, 140),
            if n.status != "ok" { " (stale)" } else { "" }
        ));
    }
    let errors: Vec<&FileRow> = g.files.iter().filter(|f| f.parse_error.as_deref().is_some_and(|e| !e.is_empty())).collect();
    if !errors.is_empty() {
        lines.push(String::new());
        lines.push("## Parse errors".into());
        for f in errors {
            lines.push(format!("{}: {}", f.path, f.parse_error.as_deref().unwrap()));
        }
    }
    Ok(lines.join("\n"))
}

fn find(g: &Graph, q: &str, limit: i64) -> String {
    let needle = q.to_lowercase();
    let mut syms: Vec<usize> = (0..g.syms.len())
        .filter(|&i| g.syms[i].name.to_lowercase().contains(&needle) && g.syms[i].kind != "alias")
        .collect();
    syms.sort_by_key(|&i| (if g.syms[i].name.to_lowercase() == needle { 0 } else { 1 }, -g.syms[i].exported));
    let files: Vec<&FileRow> = g.files.iter().filter(|f| f.path.to_lowercase().contains(&needle)).collect();
    let routes: Vec<&RouteRow> = g.routes.iter().filter(|r| r.path.to_lowercase().contains(&needle)).collect();
    let mut lines: Vec<String> = Vec::new();
    let lim = take(limit);
    if !syms.is_empty() {
        lines.push(format!("## Symbols ({})", syms.len()));
        lines.extend(syms.iter().take(lim).map(|&i| format!("{}  {}", g.label(i), g.where_(i))));
        lines.extend(more(syms.len(), limit));
    }
    if !files.is_empty() {
        lines.push(format!("## Files ({})", files.len()));
        lines.extend(files.iter().take(lim).map(|f| f.path.clone()));
        lines.extend(more(files.len(), limit));
    }
    if !routes.is_empty() {
        lines.push(format!("## Routes ({})", routes.len()));
        lines.extend(routes.iter().take(lim).map(|r| format!("{} {} → {}", r.kind, r.path, g.path_of(r.file_id))));
    }
    // Data: which table/column already holds this — check before adding a table or column.
    let mut ents: Vec<&Value> = Vec::new();
    if let Some(dm) = g.data_model() {
        for st in arr(dm.get("stores")) {
            ents.extend(values(st.get("entities")).into_iter().filter(|e| !t(e.get("external"))));
        }
    }
    let lc = |v: Option<&Value>| -> Option<String> { v.and_then(Value::as_str).map(str::to_lowercase) };
    let tables: Vec<&Value> = ents
        .iter()
        .copied()
        .filter(|e| s(e.get("name")).to_lowercase().contains(&needle) || lc(e.get("table")).is_some_and(|x| x.contains(&needle)))
        .collect();
    let mut cols: Vec<(&Value, &Value)> = Vec::new();
    for e in &ents {
        for c in arr(e.get("columns")) {
            if s(c.get("name")).to_lowercase().contains(&needle) || lc(c.get("db")).is_some_and(|x| x.contains(&needle)) {
                cols.push((e, c));
            }
        }
    }
    if !tables.is_empty() {
        lines.push(format!("## Tables ({}) — code_graph table <name> for details", tables.len()));
        for e in tables.iter().take(lim) {
            let name = s(e.get("name"));
            let table = e.get("table");
            lines.push(format!(
                "{} {name}{} · {} cols{}",
                s(e.get("kind")),
                if t(table) && s(table) != name {
                    format!(" (table {})", s(table))
                } else {
                    String::new()
                },
                len_of(e.get("columns")),
                if t(e.get("source")) {
                    let src = e.get("source").unwrap();
                    format!(" · {}:{}", s(src.get("file")), s(src.get("line")))
                } else {
                    String::new()
                }
            ));
        }
    }
    if !cols.is_empty() {
        lines.push(format!("## Columns ({})", cols.len()));
        for (e, c) in cols.iter().take(lim) {
            lines.push(col_line(&format!("{}.{}", s(e.get("name")), s(c.get("name"))), c));
        }
        lines.extend(more(cols.len(), limit));
    }
    if lines.is_empty() {
        format!("Nothing matches \"{q}\".")
    } else {
        lines.join("\n")
    }
}

fn note_lines(list: &[&NoteInfo]) -> Vec<String> {
    list.iter()
        .map(|n| {
            format!(
                "📝 {}{}: {}",
                if n.kind == "dir" {
                    n.label.as_str()
                } else if n.kind == "file" {
                    "file"
                } else {
                    "note"
                },
                if n.status == "stale" {
                    " (STALE — code changed since written; verify and rewrite)"
                } else {
                    ""
                },
                n.text
            )
        })
        .collect()
}

fn file_info(g: &Graph, target: &str) -> Result<String> {
    let Some(fi) = g.file_by_path(target.split('#').next().unwrap_or("")) else {
        return Ok(format!("No file \"{target}\" in the graph (paths are relative to the project root)."));
    };
    let f = &g.files[fi];
    let mut lines = vec![format!(
        "{} · {}{}{}",
        f.path,
        f.lang,
        f.directive.as_deref().filter(|d| !d.is_empty()).map(|d| format!(" · \"{d}\"")).unwrap_or_default(),
        f.parse_error
            .as_deref()
            .filter(|d| !d.is_empty())
            .map(|d| format!(" · parse error: {d}"))
            .unwrap_or_default()
    )];
    let rs = g.routes_of_file(f.id);
    if !rs.is_empty() {
        lines.push(format!("Routes: {}", rs.join(", ")));
    }
    let nf = notes::notes_for(g.db, g, f)?;
    lines.extend(note_lines(&nf.iter().collect::<Vec<_>>()));
    let syms: Vec<usize> = g.symbols_of(f.id).iter().copied().filter(|&i| g.syms[i].kind != "alias").collect();
    if !syms.is_empty() {
        lines.push(String::new());
        lines.push("## Declares".into());
        for &si in &syms {
            let sy = &g.syms[si];
            let props = sy
                .props
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(json_parse)
                .transpose()?
                .unwrap_or(Value::Object(Default::default()));
            let mut extra: Vec<String> = Vec::new();
            if len_of(props.get("names")) > 0 {
                extra.push(format!("props: {}", join_values(arr(props.get("names")), ", ")));
            }
            if t(props.get("type")) {
                extra.push(format!("props type: {}", s(props.get("type"))));
            }
            if t(props.get("extends")) {
                extra.push(format!("extends {}", s(props.get("extends"))));
            }
            if len_of(props.get("statics")) > 0 {
                extra.push(format!("{} static values", len_of(props.get("statics"))));
            }
            let extra = extra.join(" · ");
            lines.push(format!(
                "{}{} :{}-{}{}",
                g.label(si),
                if sy.is_default != 0 { " default" } else { "" },
                num(sy.line),
                num(sy.end_line),
                if extra.is_empty() { String::new() } else { format!("  {extra}") }
            ));
        }
    }
    let imps = g.imports_of(f.id);
    if !imps.is_empty() {
        lines.push(String::new());
        lines.push("## Imports".into());
        let mut groups: IndexMap<&str, Vec<&ImportRow>> = IndexMap::new();
        for &ii in imps {
            groups.entry(g.imports[ii].source.as_str()).or_default().push(&g.imports[ii]);
        }
        for (source, list) in &groups {
            let i = list[0];
            let to = match i.target_file_id.filter(|x| *x != 0) {
                Some(tf) => g.path_of(tf).to_string(),
                None => match i.external.as_deref().filter(|e| !e.is_empty()) {
                    Some(e) => format!("pkg {e}"),
                    None => "unresolved".into(),
                },
            };
            let names: Vec<String> = list
                .iter()
                .map(|x| {
                    if x.kind == "reexport_all" {
                        Some("* (re-export)".to_string())
                    } else if x.imported == x.local || x.local.as_deref().is_none_or(str::is_empty) {
                        x.imported.clone()
                    } else {
                        Some(format!("{} as {}", x.imported.as_deref().unwrap_or("null"), x.local.as_deref().unwrap_or("null")))
                    }
                })
                .filter_map(|x| x.filter(|y| !y.is_empty()))
                .collect();
            lines.push(format!(
                "{source} → {to}{}{}",
                if names.is_empty() { String::new() } else { format!(": {}", names.join(", ")) },
                if list.iter().any(|x| x.kind.starts_with("reexport")) { " (re-export)" } else { "" }
            ));
        }
    }
    let mut importers: Vec<&str> = Vec::new();
    for i in g.imports_flat() {
        if i.target_file_id == Some(f.id) {
            let p = g.path_of(i.file_id);
            if !importers.contains(&p) {
                importers.push(p);
            }
        }
    }
    lines.push(String::new());
    lines.push(format!("## Imported by ({})", importers.len()));
    lines.extend(importers.iter().take(30).map(|s| s.to_string()));
    Ok(lines.join("\n"))
}

enum Target {
    Text(String),
    File(usize),
    Symbol(usize),
}

fn symbols_for(g: &Graph, target: &str) -> Target {
    let hit = g.lookup(target, false);
    if let Some(e) = hit.error {
        return Target::Text(e);
    }
    if hit.routes.is_some() {
        return Target::Text(format!("\"{target}\" is a route — use the route query."));
    }
    if let Some(f) = hit.file {
        return Target::File(f);
    }
    let syms = hit.symbols.unwrap_or_default();
    if syms.len() > 1 {
        return Target::Text(ambiguous(g, &syms));
    }
    Target::Symbol(syms[0])
}

fn symbol_note(g: &Graph, si: usize) -> Result<Option<NoteInfo>> {
    let key = format!("symbol:{}#{}", g.sym_path(si), g.syms[si].name);
    Ok(notes::all_notes(g.db, g)?.into_iter().find(|n| n.target == key))
}

fn used_by(g: &Graph, target: &str, limit: i64) -> Result<String> {
    match symbols_for(g, target) {
        Target::Text(t) => Ok(t),
        Target::File(fi) => {
            let id = g.files[fi].id;
            let imps: Vec<&ImportRow> = g.imports_flat().filter(|i| i.target_file_id == Some(id)).collect();
            if imps.is_empty() {
                return Ok(format!("{} is not imported anywhere.", g.files[fi].path));
            }
            let mut by_file: IndexMap<i64, Vec<&ImportRow>> = IndexMap::new();
            for i in imps {
                by_file.entry(i.file_id).or_default().push(i);
            }
            let mut lines = vec![format!("{} is imported by {} file(s):", g.files[fi].path, by_file.len())];
            for (fid, list) in by_file.iter().take(take(limit)) {
                lines.push(format!(
                    "{}: {}",
                    g.path_of(*fid),
                    list.iter()
                        .map(|i| i.imported.clone().unwrap_or_else(|| "(side effect)".into()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(lines.join("\n"))
        }
        Target::Symbol(si) => {
            let refs = g.references_to(si);
            let body = used_by_body(g, si, &refs, limit);
            Ok(match symbol_note(g, si)? {
                Some(sn) => format!("{}\n{body}", note_lines(&[&sn])[0]),
                None => body,
            })
        }
    }
}

fn used_by_body(g: &Graph, si: usize, refs: &[RefHit], limit: i64) -> String {
    if refs.is_empty() {
        return format!(
            "{} {} — not used anywhere{}.",
            g.label(si),
            g.where_(si),
            if g.syms[si].exported != 0 { " (exported but unused)" } else { "" }
        );
    }
    let files: HashSet<i64> = refs.iter().map(|r| g.refs[r.r].file_id).collect();
    let mut lines = vec![format!(
        "{} {} — used in {} place(s) across {} file(s):",
        g.label(si),
        g.where_(si),
        refs.len(),
        files.len()
    )];
    for r in refs.iter().take(take(limit)) {
        let rr = &g.refs[r.r];
        let owner = match r.owner {
            Some(o) if g.syms[o].name == "default" => "default export".to_string(),
            Some(o) => g.syms[o].name.clone(),
            None => "module".into(),
        };
        let rs = g.routes_of_file(rr.file_id);
        lines.push(format!(
            "{}:{}  in {owner}  {}{}{}",
            g.files[r.file].path,
            num(rr.line),
            rr.kind,
            if rr.count > 1 { format!(" ×{}", rr.count) } else { String::new() },
            if rs.is_empty() { String::new() } else { format!("  ({})", rs.join(", ")) }
        ));
    }
    lines.extend(more(refs.len(), limit));
    lines.join("\n")
}

fn uses(g: &Graph, target: &str, limit: i64) -> Result<String> {
    match symbols_for(g, target) {
        Target::Text(t) => Ok(t),
        Target::File(fi) => file_info(g, &g.files[fi].path.clone()),
        Target::Symbol(si) => {
            let sn = symbol_note(g, si)?;
            let refs: Vec<usize> = g
                .refs_of(g.syms[si].file_id)
                .iter()
                .copied()
                .filter(|&ri| g.refs[ri].symbol_id == Some(g.syms[si].id))
                .collect();
            let body = uses_body(g, si, &refs, limit);
            Ok(match sn {
                Some(sn) if !refs.is_empty() => format!("{}\n{body}", note_lines(&[&sn])[0]),
                _ => body,
            })
        }
    }
}

fn uses_body(g: &Graph, si: usize, refs: &[usize], limit: i64) -> String {
    if refs.is_empty() {
        return format!("{} {} uses nothing from this project or packages.", g.label(si), g.where_(si));
    }
    let mut lines = vec![format!("{} {} uses:", g.label(si), g.where_(si))];
    for &ri in refs.iter().take(take(limit)) {
        let r = &g.refs[ri];
        lines.push(format!(
            "{} {}{}",
            pad_end(&r.kind, 4),
            describe(g, g.syms[si].file_id, r),
            if r.count > 1 { format!(" ×{}", r.count) } else { String::new() }
        ));
    }
    lines.extend(more(refs.len(), limit));
    lines.join("\n")
}

fn describe(g: &Graph, file_id: i64, r: &RefRow) -> String {
    let name = match &r.member {
        Some(m) if !m.is_empty() => format!("{}.{m}", r.local),
        _ => r.local.clone(),
    };
    let Some(b) = g.binding_target(file_id, &r.local) else { return name };
    if let Some(e) = &b.external {
        return format!("{name} ← pkg {e}");
    }
    if let Some(u) = b.unresolved.as_deref().filter(|u| !u.is_empty()) {
        return format!("{name} ← unresolved \"{u}\"");
    }
    if b.namespace {
        return format!("{name} ← {}", b.file.map(|f| g.files[f].path.as_str()).unwrap_or("undefined"));
    }
    if let Some(si) = b.symbol {
        let sn = &g.syms[si].name;
        return format!(
            "{name} ← {}[{}] {}",
            if *sn == name { String::new() } else { format!("{sn} ") },
            g.syms[si].kind,
            g.where_(si)
        );
    }
    format!("{name} ← {} (not found there)", b.file.map(|f| g.files[f].path.as_str()).unwrap_or("?"))
}

// Everything that would be affected by changing the target, level by level (reverse dependencies).
fn impact(g: &Graph, target: &str, depth: i64, cap: i64) -> String {
    // A table / table.column (or "table:x") goes to the data impact, unless a code symbol has that exact name.
    if let Some(d) = data_target(g, target) {
        if d.explicit || g.lookup(target, false).symbols.is_none_or(|v| v.is_empty()) {
            return data_impact(g, d.ent, d.column.as_deref(), depth, cap).unwrap_or_else(|e| e.to_string());
        }
    }
    let (start, file) = match symbols_for(g, target) {
        Target::Text(t) => return t,
        Target::File(f) => (None, Some(f)),
        Target::Symbol(s) => (Some(s), None),
    };
    let mut frontier: Vec<usize> = match start {
        Some(s) => vec![s],
        None => {
            let fid = g.files[file.unwrap()].id;
            g.symbols_of(fid)
                .iter()
                .copied()
                .filter(|&i| g.syms[i].exported != 0 && g.syms[i].kind != "alias")
                .collect()
        }
    };
    let mut lines = vec![format!(
        "Impact of changing {} ({depth} level{}):",
        match start {
            Some(s) => format!("{} {}", g.label(s), g.where_(s)),
            None => g.files[file.unwrap()].path.clone(),
        },
        if depth > 1 { "s" } else { "" }
    )];
    let mut seen: HashSet<String> = frontier.iter().map(|&i| format!("s{}", g.syms[i].id)).collect();
    let mut count: i64 = 0;
    let mut level = 1;
    while level <= depth && !frontier.is_empty() {
        let mut next = Vec::new();
        let mut shown: Vec<String> = Vec::new();
        for &sym in &frontier {
            for r in g.references_to(sym) {
                let rr = &g.refs[r.r];
                let key = match r.owner {
                    Some(o) => format!("s{}", g.syms[o].id),
                    None => format!("m{}", rr.file_id),
                };
                if !seen.insert(key) {
                    continue;
                }
                let rs = g.routes_of_file(rr.file_id);
                let fpath = &g.files[r.file].path;
                let who = match r.owner {
                    Some(o) if g.syms[o].name == "default" => format!("{fpath} (default export)"),
                    Some(o) => format!("{} [{}]", g.syms[o].name, g.syms[o].kind),
                    None => format!("{fpath} (module code)"),
                };
                shown.push(format!(
                    "{who} {fpath}:{}  ← {}{}",
                    num(rr.line),
                    g.syms[sym].name,
                    if rs.is_empty() { String::new() } else { format!("  ⚑ {}", rs.join(", ")) }
                ));
                if let Some(o) = r.owner {
                    next.push(o);
                }
            }
        }
        if !shown.is_empty() {
            lines.push(format!("## Level {level} ({})", shown.len()));
            let n = shown.len() as i64;
            lines.extend(shown.into_iter().take((cap - count).max(0) as usize));
            count += n;
            if count >= cap {
                lines.push(format!("… stopped at {cap} items"));
                break;
            }
        }
        frontier = next;
        level += 1;
    }
    if lines.len() == 1 {
        lines.push("Nothing depends on it.".into());
    }
    lines.join("\n")
}

// A route: its files (page + layouts…) and the component tree its page renders.
fn route(g: &Graph, url: &str, depth: i64) -> String {
    let mut hit = g.lookup(url, true);
    if hit.routes.is_none() && !url.starts_with('/') {
        hit = g.lookup(&format!("/{url}"), true);
    }
    let Some(routes) = hit.routes else {
        let mut paths: Vec<&str> = Vec::new();
        for r in &g.routes {
            if !paths.contains(&r.path.as_str()) {
                paths.push(&r.path);
            }
        }
        return format!("No route matches \"{url}\". Routes: {}", paths.iter().take(30).copied().collect::<Vec<_>>().join(", "));
    };
    let mut lines: Vec<String> = Vec::new();
    for ri in routes.into_iter().filter(|&i| g.routes[i].kind != "layout") {
        let r = &g.routes[ri];
        let file = g.file(r.file_id);
        let dir = file.and_then(|f| f.directive.as_deref()).filter(|d| !d.is_empty());
        lines.push(format!(
            "## {} {} → {}{}",
            r.kind,
            r.path,
            g.path_of(r.file_id),
            dir.map(|d| format!(" (\"{d}\")")).unwrap_or_default()
        ));
        // Layouts wrap pages only; API route handlers have none.
        if r.kind != "route" {
            let layouts: Vec<&RouteRow> = g
                .routes
                .iter()
                .filter(|l| l.kind == "layout" && l.root == r.root && (l.path == "/" || r.path == l.path || r.path.starts_with(&format!("{}/", l.path))))
                .collect();
            if !layouts.is_empty() {
                lines.push(format!("Layouts: {}", layouts.iter().map(|l| g.path_of(l.file_id)).collect::<Vec<_>>().join(" → ")));
            }
        }
        let syms = g.symbols_of(r.file_id);
        let entry = if r.framework == "flutter" {
            syms.iter().copied().find(|&i| Some(g.syms[i].name.as_str()) == r.root.as_deref())
        } else {
            syms.iter()
                .copied()
                .find(|&i| g.syms[i].is_default != 0)
                .or_else(|| syms.iter().copied().find(|&i| g.syms[i].exported != 0))
        };
        let Some(entry) = entry else { continue };
        if r.kind == "route" {
            for &h in syms
                .iter()
                .filter(|&&i| g.syms[i].exported != 0 && re!(r"^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)$").test_str(&g.syms[i].name))
            {
                lines.push(format!("{}:", g.syms[h].name));
                for &rf in g.refs_of(r.file_id).iter().filter(|&&x| g.refs[x].symbol_id == Some(g.syms[h].id)) {
                    lines.push(format!("  {}", describe(g, r.file_id, &g.refs[rf])));
                }
            }
            continue;
        }
        lines.push(if r.framework == "flutter" {
            format!("Widget: {} {}", g.syms[entry].name, g.where_(entry))
        } else {
            String::new()
        });
        lines.push("Renders:".into());
        let dart = file.is_some_and(|f| f.lang == "dart");
        let mut seen: HashSet<usize> = HashSet::new();
        fn walk(g: &Graph, sym: usize, indent: &str, level: i64, depth: i64, dart: bool, seen: &mut HashSet<usize>, lines: &mut Vec<String>) {
            let s = &g.syms[sym];
            for &rf in g.refs_of(s.file_id) {
                let rr = &g.refs[rf];
                if rr.symbol_id != Some(s.id) || rr.kind != "jsx" {
                    continue;
                }
                let b = g.binding_target(s.file_id, &rr.local);
                let name = match &rr.member {
                    Some(m) if !m.is_empty() => format!("{}.{m}", rr.local),
                    _ => rr.local.clone(),
                };
                let bsym = b.as_ref().and_then(|b| b.symbol);
                // Flutter builds are mostly SDK widgets (Column, Text…); only project widgets are worth listing.
                if dart && !bsym.is_some_and(|x| g.syms[x].kind == "widget") {
                    continue;
                }
                if let Some(bs) = bsym {
                    let again = seen.contains(&bs);
                    lines.push(format!("{indent}{name}  {}{}", g.where_(bs), if again { " (see above)" } else { "" }));
                    if !again && level < depth {
                        seen.insert(bs);
                        walk(g, bs, &format!("{indent}  "), level + 1, depth, dart, seen, lines);
                    }
                } else {
                    let ext = b.as_ref().and_then(|b| b.external.clone());
                    lines.push(format!("{indent}{name}{}", ext.map(|e| format!("  (pkg {e})")).unwrap_or_default()));
                }
            }
        }
        walk(g, entry, "  ", 1, depth, dart, &mut seen, &mut lines);
        // Screens and overlays this one opens (Navigator.push, showDialog …)
        let es = &g.syms[entry];
        let opens: Vec<&RefRow> = g
            .refs_of(es.file_id)
            .iter()
            .map(|&x| &g.refs[x])
            .filter(|x| x.symbol_id == Some(es.id) && (x.kind == "nav" || x.kind == "overlay"))
            .collect();
        if !opens.is_empty() {
            lines.push(format!(
                "Opens: {}",
                opens
                    .iter()
                    .map(|x| format!("{}{}", x.local, if x.kind == "overlay" { " (overlay)" } else { "" }))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    lines.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

// ---------- design link ----------

fn design_answer(g: &Graph, root: &str, limit: i64) -> Result<String> {
    let snap = design::design_snapshot(g.db);
    let links = design::component_links(g, root, snap.as_ref())?;
    let lim = take(limit);
    let mut lines = vec![match &snap {
        Some(sn) => format!(
            "Figma \"{}\" · {} components · {} tokens · synced {}",
            s(sn.get("file")),
            len_of(sn.get("components")),
            len_of(sn.get("tokens")),
            s(sn.get("fetchedAt"))
        ),
        None => "No Figma snapshot yet — run code_design_sync with the Figma plugin open (component names below come from wara.components.json only).".into(),
    }];
    lines.push(String::new());
    lines.push(format!("## Linked in wara.components.json ({})", links.linked.len()));
    for l in links.linked.iter().take(lim) {
        let gone = if snap.is_some() && l.figma.is_none() { " · not in Figma anymore" } else { "" };
        if let Some(c) = &l.classes {
            let variants: Vec<String> = util::entries(l.entry.get("variants")).iter().map(|(k, v)| format!("{k}: {}", s(Some(v)))).collect();
            let variants = variants.join(", ");
            let element = match l.entry.get("element") {
                None | Some(Value::Null) => "div".to_string(),
                v => s(v),
            };
            lines.push(format!(
                "{} → <{element} class=\"{}\">{} · used in {} files{}{gone}",
                l.figma_name,
                s(l.entry.get("className")),
                if variants.is_empty() { String::new() } else { format!(" + {{{variants}}}") },
                c.files,
                if c.missing.is_empty() {
                    String::new()
                } else {
                    format!(" · NOT DEFINED in CSS: {}", c.missing.iter().map(|n| format!(".{n}")).collect::<Vec<_>>().join(" "))
                }
            ));
            continue;
        }
        lines.push(format!(
            "{} → {}{gone}",
            l.figma_name,
            match l.symbol {
                Some(si) => format!(
                    "{} {} · used in {} files",
                    g.syms[si].name,
                    g.where_(si),
                    g.references_to(si).iter().map(|r| g.refs[r.r].file_id).collect::<HashSet<_>>().len()
                ),
                None => format!("{} from \"{}\" — NOT FOUND in code", s(l.entry.get("component")), s(l.entry.get("import"))),
            }
        ));
    }
    if !links.suggested.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Suggested pairs by name ({}) — confirm with map_component", links.suggested.len()));
        for x in links.suggested.iter().take(lim) {
            lines.push(format!(
                "{} ({}) ≈ {} {}{}",
                s(x.figma.get("name")),
                s(x.figma.get("id")),
                g.syms[x.symbol].name,
                g.where_(x.symbol),
                if x.exact { "" } else { " (partial name match)" }
            ));
        }
    }
    if !links.figma_only.is_empty() {
        lines.push(String::new());
        lines.push(format!("## In Figma, no code component found ({})", links.figma_only.len()));
        lines.push(links.figma_only.iter().take(lim).map(|c| s(c.get("name"))).collect::<Vec<_>>().join(" · "));
    }
    if !links.code_only.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Code components with no Figma counterpart ({})", links.code_only.len()));
        lines.push(links.code_only.iter().take(lim).map(|&si| g.syms[si].name.clone()).collect::<Vec<_>>().join(" · "));
    }
    Ok(lines.join("\n"))
}

fn tokens(g: &Graph, target: Option<&str>, limit: i64) -> Result<String> {
    let snap = design::design_snapshot(g.db);
    let table = design::token_table(g.db, g, snap.as_ref())?;
    let rows = &table.rows;
    let lim = take(limit);
    if let Some(target) = target.filter(|x| !x.is_empty()) {
        let r = rows.iter().find(|x| {
            x.name == target
                || x.figma.as_ref().and_then(|f| f.get("token")).and_then(Value::as_str) == Some(target)
                || x.name.ends_with(&format!("-{target}"))
                || x.name.ends_with(&format!(".{target}"))
        });
        let Some(r) = r else {
            return Ok(format!("No token \"{target}\". Try the tokens query without a target."));
        };
        let mut lines = vec![format!(
            "{}{}",
            r.name,
            match &r.figma {
                Some(f) => format!(" · Figma \"{}\" = {}", s(f.get("token")), s(f.get("value"))),
                None => " · not in Figma".into(),
            }
        )];
        for d in &r.defs {
            lines.push(format!("defined {}:{} = {}", d.path, num(d.line), d.value.as_deref().unwrap_or("null")));
        }
        if r.mismatch {
            lines.push("⚠ code value differs from Figma".into());
        }
        lines.push(format!("used {}× in {} file(s):", r.count, r.files.len()));
        let mut by_file: IndexMap<&str, Vec<String>> = IndexMap::new();
        for u in &r.uses {
            by_file.entry(u.path.as_str()).or_default().push(format!(
                "{}{} :{}",
                if u.form == "tw" { u.name.as_str() } else { u.form.as_str() },
                if u.count > 1 { format!(" ×{}", u.count) } else { String::new() },
                num(u.line)
            ));
        }
        for (p, list) in by_file.iter().take(lim) {
            lines.push(format!("  {p}: {}", list.join(", ")));
        }
        return Ok(lines.join("\n"));
    }
    let defined: Vec<&design::TokenRow> = rows.iter().filter(|r| !r.defs.is_empty()).collect();
    let mut lines = vec![format!(
        "Tokens: {} defined in code{}",
        defined.len(),
        match &snap {
            Some(sn) => format!(" · {} in Figma \"{}\"", len_of(sn.get("tokens")), s(sn.get("file"))),
            None => " · no Figma snapshot (run code_design_sync)".into(),
        }
    )];
    let mism: Vec<&design::TokenRow> = rows.iter().filter(|r| r.mismatch).collect();
    if !mism.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Value differs from Figma ({})", mism.len()));
        for r in &mism {
            lines.push(format!(
                "{}: code {} · Figma {}",
                r.name,
                r.defs[0].value.as_deref().unwrap_or("null"),
                s(r.figma.as_ref().and_then(|f| f.get("value")))
            ));
        }
    }
    let figma_only: Vec<&design::TokenRow> = rows.iter().filter(|r| r.figma.is_some() && r.defs.is_empty()).collect();
    if !figma_only.is_empty() {
        lines.push(String::new());
        lines.push(format!("## In Figma, not defined in code ({})", figma_only.len()));
        lines.push(
            figma_only
                .iter()
                .take(lim)
                .map(|r| {
                    format!(
                        "{}{}",
                        s(r.figma.as_ref().unwrap().get("token")),
                        if r.name.starts_with("figma:") { String::new() } else { format!(" ({})", r.name) }
                    )
                })
                .collect::<Vec<_>>()
                .join(" · "),
        );
    }
    let mut used: Vec<&&design::TokenRow> = defined.iter().filter(|r| r.count != 0).collect();
    used.sort_by_key(|r| std::cmp::Reverse(r.count));
    lines.push(String::new());
    lines.push(format!("## Most used ({})", used.len()));
    lines.push(
        used.iter()
            .take(lim)
            .map(|r| format!("{} {}×/{}f", r.name, r.count, r.files.len()))
            .collect::<Vec<_>>()
            .join(" · "),
    );
    let unused: Vec<&&design::TokenRow> = defined.iter().filter(|r| r.count == 0).collect();
    if !unused.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Defined but never used ({})", unused.len()));
        lines.push(unused.iter().take(lim).map(|r| r.name.clone()).collect::<Vec<_>>().join(" · "));
    }
    if !table.hardcoded.is_empty() {
        lines.push(String::new());
        lines.push(format!("## Raw colors in code instead of tokens ({})", table.hardcoded.iter().map(|h| h.n).sum::<i64>()));
        lines.push(table.hardcoded.iter().take(15).map(|h| format!("{} {}×", h.name, h.n)).collect::<Vec<_>>().join(" · "));
        lines.push(format!(
            "Files: {}",
            table.hard_files.iter().take(10).map(|f| format!("{} {}", f.path, f.n)).collect::<Vec<_>>().join(" · ")
        ));
    }
    Ok(lines.join("\n"))
}

// ---------- notes ----------

fn notes_answer(g: &Graph, limit: i64) -> Result<String> {
    let ns = notes::all_notes(g.db, g)?;
    let count = |st: &str| ns.iter().filter(|n| n.status == st).count();
    let mut lines = vec![format!("Notes: {} · ok {} · stale {} · orphan {}", ns.len(), count("ok"), count("stale"), count("orphan"))];
    let bad: Vec<&NoteInfo> = ns.iter().filter(|n| n.status != "ok").collect();
    if !bad.is_empty() {
        lines.push(String::new());
        lines.push("## Needs rewriting (code changed / target gone)".into());
        for n in bad {
            lines.push(format!("[{}] {}: {}", n.status, n.target, slice_utf16(&n.text, 0, 160)));
        }
    }
    let ok: Vec<&NoteInfo> = ns.iter().filter(|n| n.status == "ok").collect();
    if !ok.is_empty() {
        lines.push(String::new());
        lines.push("## Current".into());
        for n in ok.iter().take(take(limit)) {
            lines.push(format!("{}: {}", n.target, slice_utf16(&n.text, 0, 160)));
        }
    }
    let miss = notes::missing_notes(g.db, g, limit.min(15))?;
    let dirs = arr(miss.get("dirs"));
    let syms = arr(miss.get("symbols"));
    let entries = arr(miss.get("entries"));
    if !dirs.is_empty() || !syms.is_empty() || !entries.is_empty() {
        lines.push(String::new());
        lines.push("## Worth a note (no note yet)".into());
        if !dirs.is_empty() {
            lines.push(format!(
                "Folders: {}",
                dirs.iter()
                    .map(|d| format!("{} ({} files)", s(d.get("target")), s(d.get("files"))))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        if !entries.is_empty() {
            lines.push(format!(
                "Entry points: {}",
                entries
                    .iter()
                    .map(|e| format!("{} ({})", s(e.get("target")), s(e.get("route"))))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        if !syms.is_empty() {
            lines.push(format!(
                "Widely used: {}",
                syms.iter()
                    .map(|x| format!("{} ({} files)", s(x.get("target")), s(x.get("usedInFiles"))))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
    }
    Ok(lines.join("\n"))
}
