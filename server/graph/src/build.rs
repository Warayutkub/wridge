//! Build / incrementally update `<project>/.wara/code-graph.db` (port of `build.mjs`).
//!
//! Only files whose mtime/size changed since the last run are parsed; imports and routes are
//! re-resolved when anything changed. The database is the one `db.rs` opens, so Node and Rust can
//! take turns updating the same graph.

use crate::data::model::{build_data_model, data_sources, sources_stamp};
use crate::data::usage::{data_access_styles, scan_dart_data, scan_data, sql_dialect, DATA_SCAN_VERSION};
use crate::data::DataRef;
use crate::db::{get_meta, open_db, set_meta, transaction};
use crate::files::{ensure_gitignored, list_files, ListedFile, WARA_DIR};
use crate::js::{iso_now, json_stringify, read_utf8};
use crate::parse_dart::{parse_dart_files, DartOutput};
use crate::parse_ts::{parse_ts, Parsed};
use crate::paths::{self, dir_key, posix_dirname, posix_join, posix_normalize};
use crate::re;
use crate::resolve_ts::{create_resolver, Resolved};
use crate::routes_next::next_routes;
use crate::styles::{dart_token_class, norm_color, scan_styles, Styles};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `graphFile(root)`: `<root>/.wara/code-graph.db`.
pub fn graph_file(root: &Path) -> PathBuf {
    root.join(WARA_DIR).join("code-graph.db")
}

/// `projectRoot(projectDir)`: the resolved folder, or the Node error messages.
pub fn project_root(project_dir: &str) -> Result<PathBuf> {
    if project_dir.is_empty() || !paths::is_absolute(project_dir) {
        bail!("projectDir must be an absolute path, got \"{project_dir}\"");
    }
    let root = paths::resolve(project_dir);
    if !std::fs::metadata(&root).map(|m| m.is_dir()).unwrap_or(false) {
        bail!("Not a folder: {root}");
    }
    Ok(PathBuf::from(root))
}

/// Options of [`update_graph`].
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateOptions {
    /// Reparse every file, changed or not.
    pub force: bool,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn read_text_safe(file: &Path) -> String {
    read_utf8(file).unwrap_or_default()
}

fn read_json_safe(file: &Path) -> Option<Value> {
    read_utf8(file).ok().and_then(|t| crate::js::json_parse(&t).ok())
}

struct ParsedFile {
    f: ListedFile,
    r: Option<Parsed>,
    error: Option<String>,
    styles: Option<Styles>,
    data: Option<Vec<DataRef>>,
}

/// `updateGraph(projectDir, { force })` → the same info object as Node
/// (`root, db, gitignore, files, parsed, changed, removed, parseErrors, dartParserBuilt?, unresolvedImports, ms`).
pub fn update_graph(project_dir: &str, opts: UpdateOptions) -> Result<Value> {
    let root = project_root(project_dir)?;
    let started = now_ms();
    let gitignore = ensure_gitignored(&root)?; // before .wara/ exists, so the db is never picked up by git
    std::fs::create_dir_all(root.join(WARA_DIR))?;
    let db = open_db(graph_file(&root))?;

    let listed = list_files(&root);
    let mut known: Vec<(String, Option<f64>, Option<f64>)> = Vec::new();
    {
        let mut st = db.prepare("SELECT id, path, mtime, size FROM files")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(1)?, num(r.get_ref(2)?), num(r.get_ref(3)?))))?;
        for row in rows {
            known.push(row?);
        }
    }
    let known_map: HashMap<&str, (Option<f64>, Option<f64>)> = known.iter().map(|(p, m, s)| (p.as_str(), (*m, *s))).collect();
    let listed_paths: HashSet<&str> = listed.iter().map(|f| f.path.as_str()).collect();
    let changed: Vec<ListedFile> = listed
        .iter()
        .filter(|f| match known_map.get(f.path.as_str()) {
            None => true,
            Some((m, s)) => opts.force || *m != Some(f.mtime) || *s != Some(f.size as f64),
        })
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    let removed: Vec<String> = known
        .iter()
        .filter(|(p, _, _)| seen.insert(p.clone()) && !listed_paths.contains(p.as_str()))
        .map(|(p, _, _)| p.clone())
        .collect();

    // Data access styles come from the packages; when they change, every source file is rescanned for data use.
    let pkg_json = read_json_safe(&root.join("package.json"));
    let pubspec = read_text_safe(&root.join("pubspec.yaml"));
    let data_styles = data_access_styles(pkg_json.as_ref(), &pubspec);
    // The scanner version is part of the key, so improving a scanner rescans existing graphs once.
    let styles_key = format!("{DATA_SCAN_VERSION}:{}", data_styles.join(","));
    let rescan_data = get_meta(&db, "data_styles")?.unwrap_or_default() != styles_key;

    let mut parsed: Vec<ParsedFile> = Vec::new();
    // Dart goes to the analyzer-based parser in one batch (it may build itself on first use).
    let dart_files: Vec<String> = changed.iter().filter(|f| f.lang == "dart").map(|f| f.path.clone()).collect();
    let mut dart = DartOutput::default();
    let mut dart_error: Option<String> = None;
    match parse_dart_files(&root, &dart_files) {
        Ok(d) => dart = d,
        Err(e) => dart_error = Some(e.to_string()),
    }
    let styled_ext = re!(r"\.(css|scss|sass|less|html|vue|svelte|astro)$");
    for f in &changed {
        if f.lang == "dart" {
            let data = scan_dart_data(&read_utf8(root.join(&f.path))?, &data_styles);
            let r = dart.results.remove(&f.path);
            match r {
                Some(Ok(r)) => parsed.push(ParsedFile {
                    f: f.clone(),
                    r: Some(r),
                    error: None,
                    styles: None,
                    data: Some(data),
                }),
                other => {
                    let err = dart_error.clone().or_else(|| other.and_then(|x| x.err())).unwrap_or_else(|| "not parsed".into());
                    parsed.push(ParsedFile {
                        f: f.clone(),
                        r: None,
                        error: Some(err),
                        styles: None,
                        data: Some(data),
                    });
                }
            }
            continue;
        }
        // Stylesheets/markup: only token definitions and usages.
        let styled = f.lang == "ts" || styled_ext.test_str(&f.path);
        let text = if styled { Some(read_utf8(root.join(&f.path))?) } else { None };
        let styles = text.as_deref().map(|t| scan_styles(t, &f.path));
        if f.lang != "ts" {
            parsed.push(ParsedFile {
                f: f.clone(),
                r: None,
                error: None,
                styles,
                data: None,
            });
            continue;
        }
        let text = text.unwrap();
        let data = scan_data(&text, &data_styles)?;
        match parse_ts(&text, &paths::extname(&f.path)) {
            Ok(r) => parsed.push(ParsedFile {
                f: f.clone(),
                r: Some(r),
                error: None,
                styles,
                data: Some(data),
            }),
            Err(e) => parsed.push(ParsedFile {
                f: f.clone(),
                r: None,
                error: Some(crate::js::slice_utf16(&e.to_string(), 0, 300)),
                styles,
                data: Some(data),
            }),
        }
    }

    transaction(&db, || {
        {
            let mut del = db.prepare("DELETE FROM files WHERE path = ?")?;
            for p in &removed {
                del.execute([p])?;
            }
        }
        let mut upsert = db.prepare(
            "INSERT INTO files (path, lang, mtime, size, directive, parse_error) VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(path) DO UPDATE SET lang = excluded.lang, mtime = excluded.mtime, size = excluded.size,
        directive = excluded.directive, parse_error = excluded.parse_error RETURNING id",
        )?;
        let mut clear: Vec<rusqlite::Statement> = ["symbols", "imports", "refs", "routes", "declared_routes", "style_defs", "style_refs", "data_refs"]
            .iter()
            .map(|t| db.prepare(&format!("DELETE FROM {t} WHERE file_id = ?")))
            .collect::<rusqlite::Result<_>>()?;
        let mut add_data = db.prepare("INSERT INTO data_refs (file_id, form, entity, op, detail, columns, filters, via, line) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")?;
        let mut put_data = |file_id: i64, refs: &[DataRef]| -> Result<()> {
            for d in refs {
                let cols = d.columns.as_ref().map(|c| json_stringify(&json!(c)));
                let filt = d.filters.as_ref().map(|c| json_stringify(&json!(c)));
                add_data.execute(params![file_id, d.form, d.entity, d.op, d.detail, cols, filt, d.via, d.line])?;
            }
            Ok(())
        };
        let mut add_symbol =
            db.prepare("INSERT INTO symbols (file_id, name, kind, exported, is_default, line, end_line, props, hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id")?;
        let mut add_import = db.prepare("INSERT INTO imports (file_id, kind, source, imported, local, type_only, line) VALUES (?, ?, ?, ?, ?, ?, ?)")?;
        let mut add_ref = db.prepare("INSERT INTO refs (file_id, symbol_id, local, member, kind, line, count) VALUES (?, ?, ?, ?, ?, ?, ?)")?;
        let mut add_declared = db.prepare("INSERT INTO declared_routes (file_id, path, widget, kind, line) VALUES (?, ?, ?, ?, ?)")?;
        let mut add_def = db.prepare("INSERT INTO style_defs (file_id, name, value, line) VALUES (?, ?, ?, ?)")?;
        let mut add_use = db.prepare("INSERT INTO style_refs (file_id, form, name, line, count) VALUES (?, ?, ?, ?, ?)")?;
        for p in &parsed {
            let f = &p.f;
            let directive = p.r.as_ref().and_then(|r| r.directive.clone());
            let id: i64 = upsert.query_row(params![f.path, f.lang, f.mtime, f.size as i64, directive, p.error], |r| r.get(0))?;
            for c in clear.iter_mut() {
                c.execute([id])?;
            }
            if let Some(d) = &p.data {
                put_data(id, d)?;
            }
            if let Some(s) = &p.styles {
                for d in &s.defs {
                    add_def.execute(params![id, d.name, d.value, d.line])?;
                }
                for u in &s.refs {
                    add_use.execute(params![id, u.form, u.name, u.line, u.count])?;
                }
            }
            let Some(r) = &p.r else { continue };
            let mut refs = r.refs.clone();
            if f.lang == "dart" {
                // Dart tokens: statics of AppColors-like classes are definitions; AppColors.x references are uses.
                for sym in &r.symbols {
                    let statics = sym.props.as_ref().and_then(|p| p.get("statics")).and_then(Value::as_array).cloned().unwrap_or_default();
                    for st in statics {
                        let name = format!("{}.{}", sym.name, crate::data::js_string(st.get("name")));
                        let value = crate::db::js_sql_value(st.get("value").unwrap_or(&Value::Null), 3)?;
                        let line = crate::db::js_sql_value(st.get("line").unwrap_or(&Value::Null), 4)?;
                        add_def.execute(params![id, name, value, line])?;
                    }
                }
                for x in &refs {
                    if x.kind == "hex" {
                        let name = norm_color(x.member.as_deref()).or_else(|| x.member.clone());
                        add_use.execute(params![id, "hex", name, x.line, x.count])?;
                    } else if x.member.as_deref().is_some_and(|m| !m.is_empty()) && dart_token_class().test_str(&x.local) {
                        add_use.execute(params![id, "dart", format!("{}.{}", x.local, x.member.as_deref().unwrap()), x.line, x.count])?;
                    }
                }
                refs.retain(|x| x.kind != "hex");
            }
            let mut ids: Vec<i64> = Vec::with_capacity(r.symbols.len());
            for s in &r.symbols {
                let props = s.props.as_ref().map(json_stringify);
                ids.push(add_symbol.query_row(params![id, s.name, s.kind, s.exported, s.is_default, s.line, s.end_line, props, s.hash], |row| row.get(0))?);
            }
            for i in &r.imports {
                add_import.execute(params![id, i.kind, i.source, i.imported, i.local, i.type_only, i.line])?;
            }
            for x in &refs {
                let owner = if x.owner >= 0 { ids.get(x.owner as usize).copied() } else { None };
                add_ref.execute(params![id, owner, x.local, x.member, x.kind, x.line, x.count])?;
            }
            for rt in &r.routes {
                add_declared.execute(params![id, rt.path, rt.widget, rt.kind, rt.line])?;
            }
        }
        // Access style changed (e.g. supabase-js just added): rescan unchanged sources for data use too.
        if rescan_data {
            let changed_paths: HashSet<&str> = changed.iter().map(|f| f.path.as_str()).collect();
            let mut file_row = db.prepare("SELECT id FROM files WHERE path = ?")?;
            let mut clear_data = db.prepare("DELETE FROM data_refs WHERE file_id = ?")?;
            for f in &listed {
                if (f.lang != "ts" && f.lang != "dart") || changed_paths.contains(f.path.as_str()) {
                    continue;
                }
                let Some(row_id) = file_row.query_row([&f.path], |r| r.get::<_, i64>(0)).optional()? else {
                    continue;
                };
                clear_data.execute([row_id])?;
                let text = read_utf8(root.join(&f.path))?;
                let refs = if f.lang == "dart" {
                    scan_dart_data(&text, &data_styles)
                } else {
                    scan_data(&text, &data_styles)?
                };
                put_data(row_id, &refs)?;
            }
            set_meta(&db, "data_styles", &styles_key)?;
        }
        // The data model is rebuilt only when a schema source (migration…) changed.
        let ddl_files: Vec<String> = {
            let mut st = db.prepare("SELECT DISTINCT f.path FROM data_refs d JOIN files f ON f.id = d.file_id WHERE d.op = 'ddl'")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let all_paths: Vec<String> = listed.iter().map(|f| f.path.clone()).collect();
        let sources = data_sources(&all_paths, &ddl_files);
        let stamp = sources_stamp(&root, &sources);
        if get_meta(&db, "data_stamp")?.as_deref() != Some(stamp.as_str()) {
            let model = build_data_model(&root, &sources, &sql_dialect(pkg_json.as_ref(), &pubspec))?;
            set_meta(&db, "data_model", &json_stringify(&model))?;
            set_meta(&db, "data_stamp", &stamp)?;
        }
        if !changed.is_empty() || !removed.is_empty() {
            relink(&db, &root)?;
            set_meta(&db, "updated_at", &iso_now())?;
        }
        Ok(())
    })?;

    let unresolved: i64 = db.query_row("SELECT COUNT(*) AS n FROM imports WHERE target_file_id IS NULL AND external IS NULL", [], |r| r.get(0))?;
    let parse_errors: Vec<String> = match &dart_error {
        Some(e) => std::iter::once(format!("{} Dart file(s): {e}", dart_files.len()))
            .chain(
                parsed
                    .iter()
                    .filter(|p| p.error.is_some() && p.f.lang != "dart")
                    .map(|p| format!("{}: {}", p.f.path, p.error.as_deref().unwrap())),
            )
            .collect(),
        None => parsed
            .iter()
            .filter(|p| p.error.is_some())
            .map(|p| format!("{}: {}", p.f.path, p.error.as_deref().unwrap()))
            .collect(),
    };
    let root_s = root.to_string_lossy().replace('\\', "/");
    let mut info = Map::new();
    info.insert("root".into(), json!(root_s));
    info.insert("db".into(), json!(graph_file(&root).to_string_lossy().replace('\\', "/")));
    info.insert("gitignore".into(), gitignore);
    info.insert("files".into(), json!(listed.len()));
    info.insert("parsed".into(), json!(parsed.iter().filter(|p| p.r.is_some()).count()));
    info.insert("changed".into(), json!(changed.len()));
    info.insert("removed".into(), json!(removed.len()));
    info.insert("parseErrors".into(), json!(parse_errors));
    if dart.built {
        info.insert("dartParserBuilt".into(), json!(true));
    }
    info.insert("unresolvedImports".into(), json!(unresolved));
    info.insert("ms".into(), json!(now_ms() - started));
    Ok(Value::Object(info))
}

/// A SQLite value as a JS number (`null` for NULL/text).
fn num(v: rusqlite::types::ValueRef) -> Option<f64> {
    match v {
        rusqlite::types::ValueRef::Integer(i) => Some(i as f64),
        rusqlite::types::ValueRef::Real(f) => Some(f),
        _ => None,
    }
}

// Import targets and routes depend on other files (and on tsconfig/package.json), so they are recomputed as a whole.
fn relink(db: &Connection, root: &Path) -> Result<()> {
    let files: Vec<(i64, String)> = {
        let mut st = db.prepare("SELECT id, path FROM files")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let id_by_path: HashMap<&str, i64> = files.iter().map(|(i, p)| (p.as_str(), *i)).collect();
    let path_by_id: HashMap<i64, &str> = files.iter().map(|(i, p)| (*i, p.as_str())).collect();
    let paths: Vec<String> = files.iter().map(|f| f.1.clone()).collect();
    let resolve_ts = create_resolver(root, &paths);
    let resolve_dart = create_dart_resolver(root, &paths);
    let imports: Vec<(i64, i64, String)> = {
        let mut st = db.prepare("SELECT id, file_id, source FROM imports")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    {
        let mut set = db.prepare("UPDATE imports SET target_file_id = ?, external = ? WHERE id = ?")?;
        for (imp_id, file_id, source) in imports {
            let from = path_by_id.get(&file_id).copied().unwrap_or("");
            let r = if from.ends_with(".dart") {
                resolve_dart.resolve(from, &source)
            } else {
                resolve_ts.resolve(from, &source)?
            };
            let target = r.file.as_deref().and_then(|f| id_by_path.get(f).copied());
            let external = if r.file.is_some() { None } else { r.external };
            set.execute(params![target, external, imp_id])?;
        }
    }
    db.execute_batch("DELETE FROM routes")?;
    let mut add_route = db.prepare("INSERT INTO routes (file_id, framework, path, kind, root) VALUES (?, ?, ?, ?, ?)")?;
    for r in next_routes(root, &paths) {
        add_route.execute(params![id_by_path.get(r.file.as_str()), r.framework, r.path, r.kind, r.root])?;
    }
    flutter_routes(db, &mut add_route)?;
    Ok(())
}

/// Resolves Dart imports: `package:<name>/x.dart` → `<folder of the pubspec named <name>>/lib/x.dart`;
/// `dart:…` and other packages are external.
pub struct DartResolver {
    files: HashSet<String>,
    packages: HashMap<String, String>,
}

/// `createDartResolver(root, paths)`.
pub fn create_dart_resolver(root: &Path, paths: &[String]) -> DartResolver {
    let files = paths.iter().cloned().collect();
    let mut packages = HashMap::new();
    for p in paths {
        if !p.ends_with("pubspec.yaml") {
            continue;
        }
        let Ok(text) = read_utf8(root.join(p)) else { continue };
        if let Some(name) = re!(r"^name:\s*([A-Za-z0-9_]+)", "m").exec_str(&text).and_then(|g| g[1].clone()) {
            packages.insert(name, dir_key(p));
        }
    }
    DartResolver { files, packages }
}

impl DartResolver {
    pub fn resolve(&self, from: &str, spec: &str) -> Resolved {
        if spec.starts_with("dart:") {
            return Resolved {
                file: None,
                external: Some(spec.to_string()),
            };
        }
        if let Some(g) = re!(r"^package:([^/]+)\/(.+)$").exec_str(spec) {
            let (pkg, rest) = (g[1].clone().unwrap(), g[2].clone().unwrap());
            let Some(dir) = self.packages.get(&pkg) else {
                return Resolved { file: None, external: Some(pkg) };
            };
            let target = posix_join(&[dir, "lib", &rest]);
            return Resolved {
                file: self.files.contains(&target).then_some(target),
                external: None,
            };
        }
        let target = posix_normalize(&posix_join(&[&posix_dirname(from), spec]));
        Resolved {
            file: self.files.contains(&target).then_some(target),
            external: None,
        }
    }
}

// Flutter has no file-based routes: screens are widgets that get pushed (MaterialPageRoute → "nav" refs),
// plus paths declared in GoRoute / MaterialApp(routes:, home:). Widget names are matched project-wide.
fn flutter_routes(db: &Connection, add_route: &mut rusqlite::Statement) -> Result<()> {
    let mut widgets: HashMap<String, Vec<i64>> = HashMap::new();
    {
        let mut st = db.prepare("SELECT s.name, s.file_id, f.path FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.lang = 'dart' AND s.kind IN ('widget', 'class')")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (name, file_id) = row?;
            widgets.entry(name).or_default().push(file_id);
        }
    }
    let file_of = |name: Option<&str>, near: i64| -> Option<i64> {
        let list = name.and_then(|n| widgets.get(n))?;
        if list.contains(&near) {
            Some(near)
        } else if list.len() == 1 {
            Some(list[0])
        } else {
            None
        }
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut add = |file_id: Option<i64>, path: &str, kind: &str, widget: Option<&str>| -> Result<()> {
        let key = format!("{path}|{kind}|{}", widget.unwrap_or("null"));
        let Some(fid) = file_id.filter(|f| *f != 0) else { return Ok(()) };
        if seen.contains(&key) {
            return Ok(());
        }
        seen.insert(key);
        add_route.execute(params![fid, "flutter", path, kind, widget])?;
        Ok(())
    };
    // Tests build throwaway MaterialApps; they are not the app's routes.
    let mut is_test = db.prepare("SELECT path FROM files WHERE id = ?")?;
    let mut in_tests = |file_id: i64| -> Result<bool> {
        let p: Option<String> = is_test.query_row([file_id], |r| r.get(0)).optional()?;
        Ok(re!(r"(^|\/)(test|integration_test|test_driver)\/").test_str(p.as_deref().unwrap_or("")))
    };
    let declared: Vec<(i64, String, Option<String>, String)> = {
        let mut st = db.prepare("SELECT * FROM declared_routes")?;
        let rows = st.query_map([], |r| Ok((r.get("file_id")?, r.get("path")?, r.get("widget")?, r.get("kind")?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (file_id, path, widget, kind) in declared {
        if !in_tests(file_id)? {
            add(file_of(widget.as_deref(), file_id).or(Some(file_id)), &path, &kind, widget.as_deref())?;
        }
    }
    let navs: Vec<(String, i64)> = {
        let mut st = db.prepare("SELECT DISTINCT r.local, r.file_id FROM refs r JOIN files f ON f.id = r.file_id WHERE r.kind = 'nav' AND f.lang = 'dart' AND r.member IS NULL")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (local, file_id) in navs {
        if !in_tests(file_id)? {
            add(file_of(Some(&local), file_id), &local, "screen", Some(&local))?;
        }
    }
    Ok(())
}

/// `openGraph(projectDir)`: brings the graph up to date (cheap when nothing changed), then opens it.
pub fn open_graph(project_dir: &str) -> Result<(Connection, Value)> {
    let info = update_graph(project_dir, UpdateOptions::default())?;
    let db = open_db(graph_file(&project_root(project_dir)?))?;
    Ok((db, info))
}
