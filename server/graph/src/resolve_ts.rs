//! Module resolution for TS/JS imports, against the files already in the graph (port of
//! `resolve-ts.mjs`; no disk access per import). Covers relative paths, tsconfig/jsconfig "paths"
//! + "baseUrl" (with "extends"), workspace packages and index files.

use crate::jsre::JsStr;
use crate::paths::{dir_key, posix_dirname, posix_join, posix_normalize};
use crate::re;
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const EXTS: [&str; 9] = [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];

/// `readJsonc(file)`: JSON with comments and trailing commas (tsconfig style).
pub fn read_jsonc(file: &Path) -> Result<Value> {
    let text = JsStr::new(&crate::js::read_utf8(file)?);
    let a = re!(r#"\\"|"(?:\\"|[^"])*"|(\/\/.*|\/\*[\s\S]*?\*\/)"#, "g").replace_all_with(&text, |m| {
        if m.range(1).is_some_and(|(a, b)| b > a) {
            JsStr::default()
        } else {
            m.whole(&text)
        }
    });
    let b = re!(r",(\s*[}\]])", "g").replace_all_with(&a, |m| m.get(&a, 1).unwrap());
    crate::js::json_parse(&b.to_string())
}

fn norm(p: &str) -> String {
    re!(r"^\.\/").replace_first_str(&posix_normalize(&p.replace('\\', "/")), "")
}

/// Where an import points: a file of the graph, or an external package (`None`/`None` = unresolved).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    pub file: Option<String>,
    pub external: Option<String>,
}

#[derive(Clone, Debug)]
struct Config {
    paths: Vec<(String, Value)>,
    paths_base: String,
    base_url: Option<String>,
}

struct Workspace {
    dir: String,
    entries: Vec<String>,
}

/// The resolver `createResolver(root, filePaths)` returns.
pub struct TsResolver {
    files: HashSet<String>,
    configs: HashMap<String, Config>,
    workspaces: HashMap<String, Workspace>,
}

/// `createResolver(root, filePaths)`.
pub fn create_resolver(root: &Path, file_paths: &[String]) -> TsResolver {
    TsResolver {
        files: file_paths.iter().cloned().collect(),
        configs: load_tsconfigs(root, file_paths),
        workspaces: load_workspaces(root, file_paths),
    }
}

/// `packageName(spec)`: `@scope/name` or the first segment.
pub fn package_name(spec: &str) -> String {
    if spec.starts_with('@') {
        spec.split('/').take(2).collect::<Vec<_>>().join("/")
    } else {
        spec.split('/').next().unwrap_or("").to_string()
    }
}

fn entries_of(v: &Value) -> Vec<(String, Value)> {
    match v {
        Value::Object(m) => crate::js::js_keys(m).into_iter().map(|k| (k.clone(), m[k.as_str()].clone())).collect(),
        Value::Array(a) => a.iter().enumerate().map(|(i, x)| (i.to_string(), x.clone())).collect(),
        Value::String(s) => s
            .encode_utf16()
            .enumerate()
            .map(|(i, u)| (i.to_string(), Value::String(String::from_utf16_lossy(&[u]))))
            .collect(),
        _ => Vec::new(),
    }
}

impl TsResolver {
    fn try_file(&self, base: &str) -> Option<String> {
        let base = norm(base);
        if base.starts_with("../") {
            return None;
        }
        if self.files.contains(&base) {
            return Some(base);
        }
        // ESM-style "./x.js" that really points at x.ts
        let swapped = re!(r"\.(m|c)?jsx?$").replace_first_str(&base, "");
        let candidates = if swapped != base { vec![swapped, base.clone()] } else { vec![base.clone()] };
        for b in candidates {
            for ext in EXTS {
                let p = format!("{b}{ext}");
                if self.files.contains(&p) {
                    return Some(p);
                }
            }
            for ext in EXTS {
                let p = format!("{b}/index{ext}");
                if self.files.contains(&p) {
                    return Some(p);
                }
            }
        }
        None
    }

    // Closest tsconfig/jsconfig above the importing file.
    fn config_for(&self, file: &str) -> Option<&Config> {
        let mut dir = posix_dirname(file);
        loop {
            let key = if dir == "." { "" } else { dir.as_str() };
            if let Some(c) = self.configs.get(key) {
                return Some(c);
            }
            if dir == "." || dir.is_empty() {
                return None;
            }
            let next = posix_dirname(&dir);
            if next == dir {
                return None;
            }
            dir = next;
        }
    }

    /// `resolve(fromFile, spec)`. Errors only where the JS throws (malformed tsconfig "paths").
    pub fn resolve(&self, from_file: &str, spec: &str) -> Result<Resolved> {
        if spec.starts_with('.') {
            return Ok(Resolved {
                file: self.try_file(&posix_join(&[&posix_dirname(from_file), spec])),
                external: None,
            });
        }
        if spec.starts_with('/') {
            return Ok(Resolved::default());
        }
        if let Some(config) = self.config_for(from_file) {
            let js_spec = JsStr::new(spec);
            for (pattern, targets) in &config.paths {
                let pat = JsStr::new(pattern);
                let star = pat.index_of("*", 0);
                let rest: Option<String> = match star {
                    None if spec == pattern => Some(String::new()),
                    None => None,
                    Some(st) => {
                        let prefix = pat.slice(0, st);
                        let suffix = pat.slice_from(st + 1);
                        if js_spec.starts_with_at(prefix.units(), 0) && js_spec.units().ends_with(suffix.units()) {
                            Some(js_spec.slice_js(st as isize, js_spec.len() as isize - (pat.len() as isize - st as isize - 1)).to_string())
                        } else {
                            None
                        }
                    }
                };
                let Some(rest) = rest else { continue };
                let list: Vec<Value> = match targets {
                    Value::Array(a) => a.clone(),
                    Value::String(_) => entries_of(targets).into_iter().map(|e| e.1).collect(),
                    _ => bail!("targets is not iterable"),
                };
                for t in list {
                    let Value::String(t) = t else { bail!("t.replace is not a function") };
                    let replaced = replace_first_literal(&t, "*", &rest);
                    if let Some(hit) = self.try_file(&posix_join(&[&config.paths_base, &replaced])) {
                        return Ok(Resolved { file: Some(hit), external: None });
                    }
                }
            }
            if let Some(base_url) = &config.base_url {
                if let Some(hit) = self.try_file(&posix_join(&[base_url, spec])) {
                    return Ok(Resolved { file: Some(hit), external: None });
                }
            }
        }
        let pkg = package_name(spec);
        if let Some(ws) = self.workspaces.get(&pkg) {
            let sub = re!(r"^\/").replace_first_str(&JsStr::new(spec).slice_from(JsStr::new(&pkg).len()).to_string(), "");
            let hit = if !sub.is_empty() {
                self.try_file(&posix_join(&[&ws.dir, &sub])).or_else(|| self.try_file(&posix_join(&[&ws.dir, "src", &sub])))
            } else {
                ws.entries.iter().find_map(|e| self.try_file(e))
            };
            if let Some(hit) = hit {
                return Ok(Resolved { file: Some(hit), external: None });
            }
        }
        Ok(Resolved { file: None, external: Some(pkg) })
    }
}

/// `s.replace(pat, rep)` with a string pattern: the first occurrence, with the replacement's
/// `$$`, `$&`, `` $` `` and `$'` patterns expanded the way JS does.
fn replace_first_literal(s: &str, pat: &str, rep: &str) -> String {
    let Some(at) = s.find(pat) else { return s.to_string() };
    let (before, after) = (&s[..at], &s[at + pat.len()..]);
    let mut out = String::from(before);
    let mut it = rep.chars().peekable();
    while let Some(c) = it.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some('$') => out.push('$'),
            Some('&') => out.push_str(pat),
            Some('`') => out.push_str(before),
            Some('\'') => out.push_str(after),
            _ => {
                out.push('$');
                continue;
            }
        }
        it.next();
    }
    out.push_str(after);
    out
}

fn load_tsconfigs(root: &Path, paths: &[String]) -> HashMap<String, Config> {
    let mut out = HashMap::new();
    for p in paths {
        if !re!(r"(^|\/)[tj]sconfig\.json$").test_str(p) {
            continue;
        }
        // unreadable config: imports from that folder fall back to relative/package resolution
        if let Ok(c) = compiler_paths(root, p, &mut HashSet::new()) {
            out.insert(dir_key(p), c);
        }
    }
    out
}

fn js_type_error(what: &str) -> anyhow::Error {
    anyhow!("{what}")
}

// Effective "paths"/"baseUrl" after following relative "extends".
fn compiler_paths(root: &Path, rel: &str, seen: &mut HashSet<String>) -> Result<Config> {
    if seen.contains(rel) {
        return Ok(Config {
            paths: Vec::new(),
            paths_base: String::new(),
            base_url: None,
        });
    }
    seen.insert(rel.to_string());
    let json = read_jsonc(&root.join(rel))?;
    if json.is_null() {
        return Err(js_type_error("Cannot read properties of null (reading 'extends')"));
    }
    let dir = dir_key(rel);
    let mut base = Config {
        paths: Vec::new(),
        paths_base: dir.clone(),
        base_url: None,
    };
    let extends: Vec<Value> = match json.get("extends") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(v) => vec![v.clone()],
    };
    for ext in extends {
        let Value::String(ext) = ext else {
            return Err(js_type_error("ext.startsWith is not a function"));
        };
        if !ext.starts_with('.') {
            continue; // package configs (e.g. @tsconfig/next) rarely carry paths
        }
        let file = if ext.ends_with(".json") { ext.clone() } else { format!("{ext}.json") };
        let target = norm(&posix_join(&[&dir, &file]));
        if let Ok(c) = compiler_paths(root, &target, seen) {
            base = c; // missing base config: keep the previous one
        }
    }
    let empty = Value::Object(Default::default());
    let co = match json.get("compilerOptions") {
        None | Some(Value::Null) => &empty,
        Some(v) => v,
    };
    let base_url = match co.get("baseUrl") {
        None => base.base_url.clone(),
        Some(Value::String(b)) => Some(norm(&posix_join(&[&dir, b]))),
        Some(_) => return Err(js_type_error("The \"path\" argument must be of type string")),
    };
    if let Some(p) = co.get("paths").filter(|p| crate::js::truthy(Some(p))) {
        return Ok(Config {
            paths: entries_of(p),
            paths_base: base_url.clone().unwrap_or(dir),
            base_url,
        });
    }
    Ok(Config { base_url, ..base })
}

// package.json "name" → folder, so imports between packages of a monorepo stay inside the graph.
fn load_workspaces(root: &Path, paths: &[String]) -> HashMap<String, Workspace> {
    let mut out = HashMap::new();
    for p in paths {
        if !p.ends_with("package.json") {
            continue;
        }
        let Ok(text) = crate::js::read_utf8(root.join(p)) else { continue };
        let Ok(json) = crate::js::json_parse(&text) else { continue }; // invalid package.json
        if json.is_null() || !crate::js::truthy(json.get("name")) {
            continue;
        }
        let Some(name) = json.get("name").and_then(Value::as_str) else { continue }; // non-string names never match a spec
        let dir = dir_key(p);
        let exp = json.get("exports");
        let dot = exp.and_then(|e| if e.is_object() { e.get(".") } else { None });
        let from_exports = match exp {
            Some(Value::String(s)) => Some(Value::String(s.clone())),
            _ => {
                let import = dot.and_then(|d| if d.is_object() { d.get("import") } else { None }).filter(|v| !v.is_null());
                let default = dot.and_then(|d| if d.is_object() { d.get("default") } else { None }).filter(|v| !v.is_null());
                import.or(default).or(dot.filter(|v| !v.is_null())).cloned()
            }
        };
        let candidates = [
            json.get("source").cloned(),
            json.get("types").cloned(),
            json.get("module").cloned(),
            json.get("main").cloned(),
            from_exports,
        ];
        let mut entries: Vec<String> = candidates
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .map(|x| posix_join(&[&dir, &re!(r"^\.\/(dist|build|lib)\/").replace_first_str(&x, "./src/")]))
            .collect();
        entries.push(posix_join(&[&dir, "src/index"]));
        entries.push(posix_join(&[&dir, "index"]));
        out.insert(name.to_string(), Workspace { dir, entries });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_paths_aliases_and_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("tsconfig.json"),
            "{\n // comment\n \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"@/*\": [\"./src/*\"], }, },\n}",
        )
        .unwrap();
        let files: Vec<String> = ["tsconfig.json", "src/a.ts", "src/ui/button.tsx", "src/lib/index.ts"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let r = create_resolver(dir.path(), &files);
        assert_eq!(r.resolve("src/a.ts", "./ui/button.js").unwrap().file.as_deref(), Some("src/ui/button.tsx"));
        assert_eq!(r.resolve("src/a.ts", "@/lib").unwrap().file.as_deref(), Some("src/lib/index.ts"));
        assert_eq!(r.resolve("src/a.ts", "src/a").unwrap().file.as_deref(), Some("src/a.ts"));
        assert_eq!(
            r.resolve("src/a.ts", "react-dom/client").unwrap(),
            Resolved {
                file: None,
                external: Some("react-dom".into())
            }
        );
        assert_eq!(r.resolve("src/a.ts", "@scope/pkg/x").unwrap().external.as_deref(), Some("@scope/pkg"));
        assert_eq!(r.resolve("src/a.ts", "../../x").unwrap(), Resolved::default());
    }
}
