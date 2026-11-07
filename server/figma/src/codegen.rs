//! Port of `legacy-node/codegen.mjs`: design → code helpers — token files (CSS / Tailwind v4 / Flutter) and
//! the Figma → code component map (`wara.components.json`, read and written byte-compatible).

use crate::js::{self, Obj};
use crate::motion::{fs_error, read_text};
use crate::node_path as path;
use crate::obj;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use unicode_normalization::UnicodeNormalization;

/// `COMPONENT_MAP_FILE`.
pub const COMPONENT_MAP_FILE: &str = "wara.components.json";

fn re(p: &str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap();
    c.entry(p.to_string()).or_insert_with(|| Regex::new(p).unwrap()).clone()
}

/// `requireAbsolute(path, label)`.
pub fn require_absolute(p: &str, label: &str) -> Result<String> {
    if !path::is_absolute(p) {
        return Err(anyhow!("{label} must be an absolute path, got \"{p}\""));
    }
    Ok(p.to_string())
}

/// `kebab(name)`: NFKD, lower case, runs of anything but a-z0-9 → "-".
pub fn kebab(name: &str) -> String {
    let lower: String = name.nfkd().collect::<String>().to_lowercase();
    let d = re("[^a-z0-9]+").replace_all(&lower, "-").into_owned();
    d.trim_matches('-').to_string()
}

/// `writeFileDeep(file, data)`.
pub fn write_file_deep(file: &str, data: &str) -> Result<()> {
    if let Some(dir) = std::path::Path::new(file).parent() {
        std::fs::create_dir_all(dir).map_err(|e| fs_error(&e, "mkdir", &dir.to_string_lossy()))?;
    }
    std::fs::write(file, data).map_err(|e| fs_error(&e, "open", file))
}

// ---------- component map

/// `readComponentMap(projectDir)`: `{}` when the file does not exist.
pub fn read_component_map(project_dir: &str) -> Result<Value> {
    let file = path::join(&[&require_absolute(project_dir, "projectDir")?, COMPONENT_MAP_FILE]);
    match std::fs::metadata(&file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Object(Obj::new())),
        _ => {}
    }
    let text = read_text(&file).map_err(|e| anyhow!("Cannot read {file}: {e}"))?;
    js::parse(&text).map_err(|e| anyhow!("Cannot read {file}: {e}"))
}

/// `writeComponentMap(projectDir, map)`: keys sorted with `localeCompare`, 2-space JSON + newline.
/// Returns the file written.
pub fn write_component_map(project_dir: &str, map: &Value) -> Result<String> {
    let mut entries: Vec<(String, Value)> = match map {
        Value::Object(o) => js::entries(o).into_iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Array(a) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
        _ => vec![],
    };
    entries.sort_by(|(a, _), (b, _)| js::locale_compare(a, b));
    let mut sorted = Obj::new();
    for (k, v) in entries {
        sorted.insert(k, v);
    }
    let file = path::join(&[project_dir, COMPONENT_MAP_FILE]);
    write_file_deep(&file, &format!("{}\n", js::stringify_pretty(&Value::Object(sorted), 2)))?;
    Ok(file)
}

/// `resolveEntry(entry, variant)`: base className + the classes of each "Prop=Value" in the instance's variant.
pub fn resolve_entry(entry: &Value, variant: Option<&Value>) -> Value {
    let e = Some(entry);
    let class_name = js::get(e, "className");
    let variants = js::get(e, "variants");
    if !js::truthy(class_name) || !js::truthy(variants) {
        return entry.clone();
    }
    let v = js::nullish(variant).map(js::string).unwrap_or_default();
    let mut parts = vec![class_name.cloned().unwrap()];
    for pair in v.split(',') {
        let key = pair.split('=').map(js::trim).collect::<Vec<_>>().join("=");
        let hit = match variants {
            Some(Value::Object(o)) => o.get(&key),
            Some(Value::Array(a)) => key.parse::<usize>().ok().filter(|i| i.to_string() == key).and_then(|i| a.get(i)),
            _ => None,
        };
        if let Some(h) = hit.filter(|h| js::truthy(Some(h))) {
            parts.push(h.clone());
        }
    }
    let mut out = Obj::new();
    if let Value::Object(o) = entry {
        for (k, val) in js::entries(o) {
            if k != "variants" {
                out.insert(k.clone(), val.clone());
            }
        }
    }
    out.insert("className".into(), Value::String(js::join(&parts, " ")));
    Value::Object(out)
}

/// `annotateComponents(root, map)`: adds `code` to every instance that has a mapping; returns the component
/// names still unmapped.
pub fn annotate_components(root: &mut Value, map: &Value) -> Vec<Value> {
    let mut unmapped: Vec<Value> = vec![];
    fn visit(node: &mut Value, map: &Value, unmapped: &mut Vec<Value>) {
        if node.get("kind") == Some(&Value::String("instance".into())) {
            let comp = node.get("component");
            let name = js::get(comp, "name").cloned();
            let variant = js::get(comp, "variant").cloned();
            let lookup = |k: String| js::get(Some(map), &k).filter(|v| js::truthy(Some(v))).cloned();
            let entry = (if js::truthy(variant.as_ref()) { lookup(format!("{}/{}", js::to_string(name.as_ref()), js::to_string(variant.as_ref()))) } else { None })
                .or_else(|| lookup(js::to_string(name.as_ref())));
            match entry {
                Some(e) => {
                    let code = resolve_entry(&e, variant.as_ref());
                    node.as_object_mut().unwrap().insert("code".into(), code);
                }
                None => {
                    let n = name.unwrap_or(Value::Null);
                    if !unmapped.iter().any(|u| js::strict_eq(Some(u), Some(&n))) {
                        unmapped.push(n);
                    }
                }
            }
        }
        if let Some(Value::Array(children)) = node.get_mut("children") {
            for c in children {
                visit(c, map, unmapped);
            }
        }
    }
    visit(root, map, &mut unmapped);
    unmapped
}

// ---------- responsive merge

/// `BREAKPOINTS`.
pub const BREAKPOINTS: [&str; 5] = ["base", "sm", "md", "lg", "xl"];

fn flatten(value: &Value, prefix: &str, out: &mut Obj) {
    match value {
        Value::Object(o) if !o.contains_key("token") => {
            for (k, v) in js::entries(o) {
                let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(v, &p, out);
            }
        }
        other => {
            out.insert(prefix.to_string(), other.clone());
        }
    }
}

// name differs by design (Home/Mobile vs Home/Desktop); template and children are merged recursively.
const OWN_KEYS_SKIPPED: [&str; 8] = ["id", "name", "children", "template", "responsive", "hiddenAt", "onlyAt", "items"];

fn own_props(node: &Value) -> Obj {
    let mut filtered = Obj::new();
    if let Value::Object(o) = node {
        for (k, v) in js::entries(o) {
            if !OWN_KEYS_SKIPPED.contains(&k.as_str()) {
                filtered.insert(k.clone(), v.clone());
            }
        }
    }
    let mut out = Obj::new();
    flatten(&Value::Object(filtered), "", &mut out);
    out
}

fn child_key(n: &Value) -> String {
    let nv = Some(n);
    if js::get(nv, "kind") == Some(&Value::String("repeat".into())) {
        format!("repeat:{}", js::to_string(js::get(js::get(nv, "template"), "name")))
    } else {
        format!("{}:{}", js::to_string(js::get(nv, "kind")), js::to_string(js::get(nv, "name")))
    }
}

fn keyed(children: &[Value]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    children
        .iter()
        .map(|c| {
            let key = child_key(c);
            let n = seen.get(&key).copied().unwrap_or(0);
            seen.insert(key.clone(), n + 1);
            format!("{key}#{n}")
        })
        .collect()
}

fn children_of(v: &Value) -> Vec<Value> {
    match v.get("children") {
        Some(Value::Array(a)) => a.clone(),
        _ => vec![],
    }
}

fn get_or_insert<'a>(o: &'a mut Obj, k: &str, default: Value) -> &'a mut Value {
    if o.get(k).is_none_or(|v| v.is_null()) {
        o.insert(k.to_string(), default);
    }
    o.get_mut(k).unwrap()
}

// Mobile-first: `base` keeps the smallest layout; each larger breakpoint records only what changes.
fn merge_node(base: &mut Value, other: &Value, bp: &str) {
    let Some(bo) = base.as_object_mut() else { return };
    if let Some(Value::Array(only)) = bo.get_mut("onlyAt") {
        only.push(Value::String(bp.into()));
    }
    let a = own_props(base);
    let b = own_props(other);
    let mut diff = Obj::new();
    let mut paths: Vec<&String> = js::keys(&a);
    for k in js::keys(&b) {
        if !paths.contains(&k) {
            paths.push(k);
        }
    }
    for p in paths {
        let sa = a.get(p).map(js::stringify);
        let sb = b.get(p).map(js::stringify);
        if sa != sb {
            diff.insert(p.clone(), b.get(p).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null));
        }
    }
    let bo = base.as_object_mut().unwrap();
    if !diff.is_empty() {
        let r = get_or_insert(bo, "responsive", Value::Object(Obj::new()));
        if let Value::Object(r) = r {
            r.insert(bp.into(), Value::Object(diff));
        }
    }
    let is_repeat = |v: Option<&Value>| js::get(v, "kind") == Some(&Value::String("repeat".into()));
    if is_repeat(Some(&*base)) && is_repeat(Some(other)) && js::truthy(base.get("template")) && js::truthy(other.get("template")) {
        let ot = other.get("template").unwrap().clone();
        merge_node(base.get_mut("template").unwrap(), &ot, bp);
    }
    let other_children = children_of(other);
    let mut other_keys: Vec<(String, Option<usize>)> = keyed(&other_children).into_iter().enumerate().map(|(i, k)| (k, Some(i))).collect();
    let base_keys = keyed(&children_of(base));
    if let Some(Value::Array(children)) = base.get_mut("children") {
        for (idx, key) in base_keys.iter().enumerate() {
            let child = &mut children[idx];
            match other_keys.iter_mut().find(|(k, i)| k == key && i.is_some()) {
                Some(entry) => {
                    let m = entry.1.take().unwrap();
                    merge_node(child, &other_children[m], bp);
                }
                None => {
                    if !js::truthy(child.get("onlyAt")) {
                        if let Some(co) = child.as_object_mut() {
                            if let Value::Array(h) = get_or_insert(co, "hiddenAt", Value::Array(vec![])) {
                                h.push(Value::String(bp.into()));
                            }
                        }
                    }
                }
            }
        }
    }
    let extras: Vec<usize> = other_keys.iter().filter_map(|(_, i)| *i).collect();
    if !extras.is_empty() {
        let bo = base.as_object_mut().unwrap();
        if let Value::Array(children) = get_or_insert(bo, "children", Value::Array(vec![])) {
            for i in extras {
                let mut e = match &other_children[i] {
                    Value::Object(o) => o.clone(),
                    _ => Obj::new(),
                };
                e.insert("onlyAt".into(), Value::Array(vec![Value::String(bp.into())]));
                children.push(Value::Object(e));
            }
        }
    }
}

/// `mergeBreakpoints(contexts)`: contexts `[{ bp, context }]` in breakpoint order; the first one is the base layout.
pub fn merge_breakpoints(contexts: &[Value]) -> Result<Value> {
    let first = contexts.first().ok_or_else(|| anyhow!("Cannot read properties of undefined (reading 'context')"))?;
    let ctx = |c: &Value| js::get(Some(c), "context").cloned().unwrap_or(Value::Null);
    let mut root = js::get(Some(&ctx(first)), "root").cloned().unwrap_or(Value::Null);
    for c in &contexts[1..] {
        let other = js::get(Some(&ctx(c)), "root").cloned().unwrap_or(Value::Null);
        merge_node(&mut root, &other, &js::to_string(js::get(Some(c), "bp")));
    }
    let mut assets: Vec<(String, Value)> = vec![];
    let mut components: Vec<(Value, Value)> = vec![];
    let mut fonts: Vec<Value> = vec![];
    let mut tokens = Obj::new();
    let mut breakpoints: Vec<(String, Option<Value>)> = vec![];
    for c in contexts {
        let cx = ctx(c);
        let cv = Some(&cx);
        let bp = js::to_string(js::get(Some(c), "bp"));
        let width = js::get(js::get(cv, "root"), "width").cloned();
        match breakpoints.iter_mut().find(|(k, _)| *k == bp) {
            Some(e) => e.1 = width,
            None => breakpoints.push((bp, width)),
        }
        if let Some(Value::Object(t)) = js::get(cv, "tokens") {
            for (k, v) in js::entries(t) {
                tokens.insert(k.clone(), v.clone());
            }
        }
        for comp in js::get(cv, "components").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let name = js::get(Some(&comp), "name").cloned().unwrap_or(Value::Null);
            match components.iter_mut().find(|(k, _)| js::strict_eq(Some(k), Some(&name))) {
                Some(e) => e.1 = comp,
                None => components.push((name, comp)),
            }
        }
        for f in js::get(cv, "fonts").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            if !fonts.iter().any(|x| js::strict_eq(Some(x), Some(&f))) {
                fonts.push(f);
            }
        }
        for a in js::get(cv, "assets").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let key = format!("{}.{}", js::to_string(js::get(Some(&a), "name")), js::to_string(js::get(Some(&a), "format")));
            match assets.iter_mut().find(|(k, _)| *k == key) {
                Some(e) => e.1 = a,
                None => assets.push((key, a)),
            }
        }
    }
    let mut bps = Obj::new();
    for (k, v) in breakpoints {
        if let Some(v) = v {
            bps.insert(k, v);
        }
    }
    Ok(obj! {
        "breakpoints" => Value::Object(bps),
        "note" => "Mobile-first: root holds the base layout; responsive[bp] lists only properties that change from that breakpoint up (null = back to default). hiddenAt/onlyAt mark visibility per breakpoint.",
        "root" => root,
        "tokens" => Value::Object(tokens),
        "components" => Value::Array(components.into_iter().map(|(_, c)| c).collect()),
        "fonts" => Value::Array(fonts),
        "assets" => Value::Array(assets.into_iter().map(|(_, a)| a).collect()),
    })
}

// ---------- tokens

fn font_weight(style: Option<&Value>) -> i64 {
    let s = js::to_string(style).to_lowercase();
    let key = re(&format!("italic|oblique|[{}_-]", js::WS)).replace_all(&s, "").into_owned();
    match key.as_str() {
        "thin" | "hairline" => 100,
        "extralight" | "ultralight" => 200,
        "light" => 300,
        "regular" | "normal" | "book" => 400,
        "medium" => 500,
        "semibold" | "demibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "black" | "heavy" => 900,
        _ => 400,
    }
}

fn line_height(v: &str) -> String {
    if v.ends_with('%') {
        js::num_str(js::parse_float(v) / 100.0)
    } else {
        v.to_string()
    }
}

/// `strip(token, re)`: the token without the prefix, or the token when nothing is left.
fn strip(token: &str, pattern: &str) -> String {
    let s = re(pattern).replacen(token, 1, "").into_owned();
    if s.is_empty() {
        token.to_string()
    } else {
        s
    }
}

fn test(pattern: &str, s: &str) -> bool {
    re(pattern).is_match(s)
}

fn tailwind_var(token: &str, ty: &str) -> String {
    if ty == "COLOR" {
        return format!("--color-{}", strip(token, "^colou?rs?-"));
    }
    if ty == "STRING" && test("font|family|typeface", token) {
        return format!("--font-{}", strip(token, "^(font-family|font|typeface|family)-"));
    }
    if ty == "FLOAT" {
        if test("radius|radii|corner|rounded", token) {
            return format!("--radius-{}", strip(token, "^(radius|radii|corner-radius|corner|rounded)-"));
        }
        if test("font-size|text-size|fontsize|type-size", token) {
            return format!("--text-{}", strip(token, "^(font-size|text-size|fontsize|type-size|text)-"));
        }
        if test("font-weight|weight", token) {
            return format!("--font-weight-{}", strip(token, "^(font-weight|weight)-"));
        }
        if test("line-height|leading", token) {
            return format!("--leading-{}", strip(token, "^(line-height|leading)-"));
        }
        if test("spacing|space|gap|padding|margin", token) {
            return format!("--spacing-{}", strip(token, "^(spacing|space|gap|padding|margin)-"));
        }
    }
    format!("--{token}")
}

fn shadow_css(shadows: &[Value]) -> String {
    shadows
        .iter()
        .map(|s| {
            let g = |k: &str| js::to_string(js::get(Some(s), k));
            format!("{}{}px {}px {}px {}px {}", if js::truthy(js::get(Some(s), "inset")) { "inset " } else { "" }, g("x"), g("y"), g("blur"), g("spread"), g("color"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------- Flutter (Dart) tokens

fn camel(token: &str) -> String {
    re("-([a-z0-9])").replace_all(token, |c: &regex::Captures| c[1].to_uppercase()).into_owned()
}
fn pascal(s: &str) -> String {
    let c = camel(&kebab(s));
    let mut it = c.chars();
    match it.next() {
        Some(f) => f.to_uppercase().collect::<String>() + it.as_str(),
        None => String::new(),
    }
}

fn dart_ident(token: &str, prefix_re: &str, fallback: &str) -> String {
    let ident = camel(&strip(token, prefix_re));
    if ident.starts_with(|c: char| c.is_ascii_digit()) {
        format!("{fallback}{ident}")
    } else {
        ident
    }
}

fn dart_color(hex: Option<&Value>) -> String {
    let h = js::to_string(hex).replacen('#', "", 1).to_uppercase();
    let u: Vec<u16> = h.encode_utf16().collect();
    let slice = |a: usize, b: usize| String::from_utf16_lossy(&u[a.min(u.len())..b.min(u.len())]);
    let alpha = if u.len() == 8 { slice(6, 8) } else { "FF".into() };
    format!("Color(0x{alpha}{})", slice(0, 6))
}

fn dart_group(ty: &str, token: &str) -> (&'static str, &'static str, &'static str) {
    if ty == "COLOR" {
        return ("AppColors", "^colou?rs?-", "c");
    }
    if ty == "STRING" {
        return ("AppStrings", "^(font-family|font|family|string)-", "s");
    }
    if test("radius|radii|corner|rounded", token) {
        return ("AppRadius", "^(radius|radii|corner-radius|corner|rounded)-", "r");
    }
    if test("spacing|space|gap|padding|margin", token) {
        return ("AppSpacing", "^(spacing|space|gap|padding|margin)-", "s");
    }
    if test("font-size|text-size|fontsize", token) {
        return ("AppFontSizes", "^(font-size|text-size|fontsize)-", "f");
    }
    ("AppDimens", "^$", "d")
}

fn list<'a>(data: &'a Value, k: &str) -> &'a [Value] {
    js::get(Some(data), k).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}
fn sget(v: &Value, k: &str) -> String {
    js::to_string(js::get(Some(v), k))
}
fn ty_of(v: &Value) -> String {
    sget(v, "type")
}
fn is_alias(raw: &Value) -> bool {
    matches!(raw, Value::Object(o) if o.contains_key("alias"))
}
fn default_modes(data: &Value) -> Vec<(Value, Option<Value>)> {
    let mut out: Vec<(Value, Option<Value>)> = vec![];
    for c in list(data, "collections") {
        let name = js::get(Some(c), "name").cloned().unwrap_or(Value::Null);
        let dm = js::get(Some(c), "defaultMode").cloned();
        match out.iter_mut().find(|(k, _)| js::strict_eq(Some(k), Some(&name))) {
            Some(e) => e.1 = dm,
            None => out.push((name, dm)),
        }
    }
    out
}
/// `defaultMode.get(collection)`.
fn default_of<'a>(dm: &'a [(Value, Option<Value>)], collection: Option<&Value>) -> Option<&'a Value> {
    let c = collection.unwrap_or(&Value::Null);
    dm.iter().find(|(k, _)| js::strict_eq(Some(k), Some(c))).and_then(|(_, v)| v.as_ref())
}
/// `mode === defaultMode.get(collection)`.
fn is_default(dm: &[(Value, Option<Value>)], collection: Option<&Value>, mode: &str) -> bool {
    default_of(dm, collection) == Some(&Value::String(mode.to_string()))
}
fn values_of(v: &Value) -> Vec<(String, Value)> {
    match js::get(Some(v), "values") {
        Some(Value::Object(o)) => js::entries(o).into_iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => vec![],
    }
}

fn format_dart(data: &Value) -> Value {
    let mut map = Obj::new();
    let mut classes: Vec<(String, Vec<String>)> = vec![];
    let mut add = |cls: String, line: String| match classes.iter_mut().find(|(c, _)| *c == cls) {
        Some(e) => e.1.push(line),
        None => classes.push((cls, vec![line])),
    };
    let dm = default_modes(data);
    let variables: Vec<&Value> = list(data, "variables").iter().filter(|v| ty_of(v) != "BOOLEAN").collect();
    for v in &variables {
        let token = sget(v, "token");
        let (cls, re_, prefix) = dart_group(&ty_of(v), &token);
        map.insert(token.clone(), Value::String(format!("{cls}.{}", dart_ident(&token, re_, prefix))));
    }
    let mut by_token: Vec<(String, &Value)> = vec![];
    for v in &variables {
        let t = sget(v, "token");
        match by_token.iter_mut().find(|(k, _)| *k == t) {
            Some(e) => e.1 = v,
            None => by_token.push((t, v)),
        }
    }
    let mode_class = |cls: &str, collection: Option<&Value>, mode: &str| if is_default(&dm, collection, mode) { cls.to_string() } else { format!("{cls}{}", pascal(mode)) };
    let ident_of = |map: &Obj, token: &str| -> String {
        match map.get(token).and_then(|v| v.as_str()) {
            Some(s) => s.split('.').nth(1).unwrap_or("undefined").to_string(),
            None => "undefined".into(),
        }
    };
    for v in &variables {
        let token = sget(v, "token");
        let (cls, _, _) = dart_group(&ty_of(v), &token);
        let ident = ident_of(&map, &token);
        let ty = if ty_of(v) == "FLOAT" { "double " } else { "" };
        for (mode, raw) in values_of(v) {
            let value = if is_alias(&raw) {
                let alias = js::to_string(raw.get("alias"));
                match by_token.iter().find(|(k, _)| *k == alias) {
                    None => map.get(&alias).map(js::string).unwrap_or_else(|| "null".into()),
                    Some((_, target)) => {
                        // An alias inside a mode points at the same mode of its target (Dark → AppColorsDark.x).
                        let (tcls, _, _) = dart_group(&ty_of(target), &alias);
                        let tcoll = js::get(Some(target), "collection");
                        let has_mode = matches!(js::get(Some(target), "values"), Some(Value::Object(o)) if o.contains_key(&mode));
                        // without that mode the target's default mode is used, which is its base class
                        let mc = if has_mode { mode_class(tcls, tcoll, &mode) } else { tcls.to_string() };
                        format!("{mc}.{}", ident_of(&map, &alias))
                    }
                }
            } else if ty_of(v) == "COLOR" {
                dart_color(Some(&raw))
            } else if ty_of(v) == "STRING" {
                js::quote(&js::string(&raw))
            } else {
                match &raw {
                    Value::Number(n) if js::number_f64(n).fract() == 0.0 && js::number_f64(n).is_finite() => format!("{}.0", js::num_str(js::number_f64(n))),
                    other => js::string(other),
                }
            };
            add(mode_class(cls, js::get(Some(v), "collection"), &mode), format!("  static const {ty}{ident} = {value};"));
        }
    }
    for s in list(data, "paintStyles") {
        let v = js::get(Some(s), "value");
        let token = sget(s, "token");
        let is_str = matches!(v, Some(Value::String(_)));
        if is_str || js::truthy(js::get(v, "token")) {
            let ident = dart_ident(&token, "^colou?rs?-", "c");
            map.insert(token.clone(), Value::String(format!("AppColors.{ident}")));
            let value = if is_str { dart_color(v) } else { js::to_string(map.get(&js::to_string(js::get(v, "token")))) };
            add("AppColors".into(), format!("  static const {ident} = {value};"));
        } else if js::get(v, "gradient") == Some(&Value::String("linear".into())) {
            let ident = dart_ident(&token, "^gradients?-", "g");
            map.insert(token.clone(), Value::String(format!("AppGradients.{ident}")));
            let stops = js::get(v, "stops").and_then(|s| s.as_array()).cloned().unwrap_or_default();
            let colors = stops.iter().map(|st| dart_color(js::get(Some(st), "color"))).collect::<Vec<_>>().join(", ");
            let pos: Vec<Value> = stops.iter().map(|st| js::get(Some(st), "position").cloned().unwrap_or(Value::Null)).collect();
            add(
                "AppGradients".into(),
                format!("  static const {ident} = LinearGradient(begin: Alignment.topCenter, end: Alignment.bottomCenter, colors: [{colors}], stops: [{}]);", js::join(&pos, ", ")),
            );
        }
    }
    for s in list(data, "textStyles") {
        let token = sget(s, "token");
        let ident = dart_ident(&token, "^(text|type|typography|font)-", "t");
        map.insert(token.clone(), Value::String(format!("AppTextStyles.{ident}")));
        let size = js::to_number(js::get(Some(s), "size"));
        let family = js::get(Some(s), "family").map(js::stringify).unwrap_or_else(|| "undefined".into());
        let mut parts = vec![format!("fontFamily: {family}"), format!("fontSize: {}", sget(s, "size")), format!("fontWeight: FontWeight.w{}", font_weight(js::get(Some(s), "style")))];
        let lh = sget(s, "lineHeight");
        if lh.ends_with('%') {
            parts.push(format!("height: {}", js::num_str(js::parse_float(&lh) / 100.0)));
        } else if lh.ends_with("px") {
            parts.push(format!("height: {}", js::num_str(js::round((js::parse_float(&lh) / size) * 1000.0) / 1000.0)));
        }
        let ls_s = sget(s, "letterSpacing");
        let ls = js::parse_float(&ls_s);
        if ls != 0.0 && !ls.is_nan() {
            let v = if ls_s.ends_with("em") { js::round(ls * size * 100.0) / 100.0 } else { ls };
            parts.push(format!("letterSpacing: {}", js::num_str(v)));
        }
        add("AppTextStyles".into(), format!("  static const {ident} = TextStyle({});", parts.join(", ")));
    }
    for s in list(data, "effectStyles") {
        let shadows = js::get(Some(s), "shadows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if shadows.is_empty() {
            continue;
        }
        let token = sget(s, "token");
        let ident = dart_ident(&token, "^(shadow|elevation|effect)-", "e");
        map.insert(token.clone(), Value::String(format!("AppShadows.{ident}")));
        let list: Vec<String> = shadows
            .iter()
            .filter(|sh| !js::truthy(js::get(Some(sh), "inset")))
            .map(|sh| {
                let g = |k: &str| sget(sh, k);
                format!("BoxShadow(offset: Offset({}, {}), blurRadius: {}, spreadRadius: {}, color: {})", g("x"), g("y"), g("blur"), g("spread"), dart_color(js::get(Some(sh), "color")))
            })
            .collect();
        add("AppShadows".into(), format!("  static const {ident} = [{}];", list.join(", ")));
    }
    let header = format!("// Generated by wridge from Figma \"{}\". Re-run get_tokens instead of editing by hand.\nimport 'package:flutter/material.dart';", sget(data, "file"));
    let mut blocks = vec![header];
    blocks.extend(classes.iter().map(|(cls, lines)| format!("abstract final class {cls} {{\n{}\n}}", lines.join("\n"))));
    obj! { "content" => format!("{}\n", blocks.join("\n\n")), "map" => Value::Object(map) }
}

/// `formatTokens(data, format)`: get_tokens output → `{ content, map }`, where map is design-token → CSS
/// variable (or Dart constant), so tokens seen in get_design_context can be translated.
pub fn format_tokens(data: &Value, format: &str) -> Value {
    if format == "json" {
        return obj! { "content" => js::stringify_pretty(data, 2), "map" => Value::Object(Obj::new()) };
    }
    if format == "dart" {
        return format_dart(data);
    }
    let tailwind = format == "tailwind";
    let mut map = Obj::new();
    let css_var = |map: &mut Obj, token: &str, ty: &str| -> String {
        if map.get(token).is_none_or(|v| v.is_null()) {
            let v = if tailwind { tailwind_var(token, ty) } else { format!("--{token}") };
            map.insert(token.to_string(), Value::String(v));
        }
        js::string(&map[token])
    };
    for v in list(data, "variables") {
        css_var(&mut map, &sget(v, "token"), &ty_of(v));
    }
    let unitless = re("(weight|opacity|z-index|zindex|ratio|scale)");
    let dm = default_modes(data);
    let mut base: Vec<String> = vec![];
    let mut modes: Vec<(String, Vec<String>)> = vec![];
    for v in list(data, "variables") {
        if ty_of(v) == "BOOLEAN" {
            continue;
        }
        let token = sget(v, "token");
        for (mode, raw) in values_of(v) {
            let value = if is_alias(&raw) {
                let alias = js::to_string(raw.get("alias"));
                format!("var({})", map.get(&alias).filter(|v| !v.is_null()).map(js::string).unwrap_or_else(|| format!("--{alias}")))
            } else if ty_of(v) == "FLOAT" {
                if unitless.is_match(&token) {
                    js::string(&raw)
                } else {
                    format!("{}px", js::string(&raw))
                }
            } else if ty_of(v) == "STRING" {
                js::quote(&js::string(&raw))
            } else {
                js::string(&raw)
            };
            let line = format!("  {}: {value};", js::to_string(map.get(&token)));
            if is_default(&dm, js::get(Some(v), "collection"), &mode) {
                base.push(line);
            } else {
                match modes.iter_mut().find(|(m, _)| *m == mode) {
                    Some(e) => e.1.push(line),
                    None => modes.push((mode, vec![line])),
                }
            }
        }
    }
    let mut skipped: Vec<Value> = vec![];
    for s in list(data, "paintStyles") {
        let v = js::get(Some(s), "value");
        let token = sget(s, "token");
        if let Some(Value::String(c)) = v {
            base.push(format!("  {}: {c};", css_var(&mut map, &token, "COLOR")));
        } else if js::truthy(js::get(v, "token")) {
            let t = js::to_string(js::get(v, "token"));
            let name = css_var(&mut map, &token, "COLOR");
            base.push(format!("  {name}: var({});", map.get(&t).filter(|v| !v.is_null()).map(js::string).unwrap_or_else(|| format!("--{t}"))));
        } else if js::truthy(js::get(v, "gradient")) {
            // Not a color: Tailwind would emit an invalid background-color; use bg-(image:--gradient-…).
            let name = format!("--gradient-{}", strip(&token, "^gradients?-"));
            map.insert(token.clone(), Value::String(name.clone()));
            let stops = js::get(v, "stops").and_then(|s| s.as_array()).cloned().unwrap_or_default();
            let stops = stops
                .iter()
                .map(|st| format!("{} {}%", js::to_string(js::get(Some(st), "color")), js::num_str(js::round(js::to_number(js::get(Some(st), "position")) * 100.0))))
                .collect::<Vec<_>>()
                .join(", ");
            let radial = js::get(v, "gradient") == Some(&Value::String("radial".into()));
            base.push(format!("  {name}: {}{stops});", if radial { "radial-gradient(" } else { "linear-gradient(180deg, " }));
        } else {
            skipped.push(js::get(Some(s), "name").cloned().unwrap_or(Value::Null));
        }
    }

    // Tailwind resolves `text-x` against both --color-x and --text-x (and `shadow-x` against --color-x and
    // --shadow-x), so a color and a text/shadow token with the same name make one utility ambiguous.
    let color_names: Vec<String> = map.values().filter_map(|v| v.as_str()).filter_map(|v| v.strip_prefix("--color-")).map(|s| s.to_string()).collect();
    let mut renamed: Vec<String> = vec![];
    let mut unique = |prefix: &str, alt: &str, name: String, token: &str| -> String {
        if !tailwind || !color_names.contains(&name) {
            return format!("{prefix}{name}");
        }
        let short = prefix.strip_prefix("--").unwrap_or(prefix);
        let short = short.strip_suffix('-').unwrap_or(short);
        renamed.push(format!("{token} → {prefix}{alt}{name} (the color {name} uses {short}-{name})"));
        format!("{prefix}{alt}{name}")
    };

    let mut text_rules: Vec<String> = vec![];
    let mut fonts: Vec<(String, String)> = vec![];
    for s in list(data, "textStyles") {
        let token = sget(s, "token");
        let family = js::get(Some(s), "family");
        let family_var = format!("--font-{}", kebab(&js::to_string(family)));
        let stack = format!("{}, ui-sans-serif, system-ui, sans-serif", family.map(js::stringify).unwrap_or_else(|| "undefined".into()));
        match fonts.iter_mut().find(|(k, _)| *k == family_var) {
            Some(e) => e.1 = stack,
            None => fonts.push((family_var.clone(), stack)),
        }
        let lh = line_height(&sget(s, "lineHeight"));
        let ls = sget(s, "letterSpacing");
        let size = sget(s, "size");
        let fw = font_weight(js::get(Some(s), "style"));
        if tailwind {
            let n = unique("--text-", "type-", strip(&token, "^(text|type|typography|font)-"), &token);
            map.insert(token.clone(), Value::String(n.clone()));
            base.push(format!("  {n}: {size}px;"));
            base.push(format!("  {n}--line-height: {lh};"));
            base.push(format!("  {n}--font-weight: {fw};"));
            if ls != "0px" && ls != "0em" {
                base.push(format!("  {n}--letter-spacing: {ls};"));
            }
        } else {
            let class_name = format!("text-{}", strip(&token, "^(text|type|typography|font)-"));
            map.insert(token.clone(), Value::String(format!(".{class_name}")));
            text_rules.push(format!(
                ".{class_name} {{\n  font-family: var({family_var});\n  font-size: {size}px;\n  font-weight: {fw};\n  line-height: {lh};\n  letter-spacing: {ls};\n}}"
            ));
        }
    }
    for (name, stack) in &fonts {
        base.push(format!("  {name}: {stack};"));
    }
    for s in list(data, "effectStyles") {
        let shadows = js::get(Some(s), "shadows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if shadows.is_empty() {
            continue;
        }
        let token = sget(s, "token");
        let n = if tailwind { unique("--shadow-", "depth-", strip(&token, "^(shadow|elevation|effect)-"), &token) } else { format!("--{token}") };
        map.insert(token.clone(), Value::String(n.clone()));
        base.push(format!("  {n}: {};", shadow_css(&shadows)));
    }
    let header = format!("/* Generated by wridge from Figma \"{}\". Re-run get_tokens instead of editing by hand. */", sget(data, "file"));
    let mut blocks = vec![header, format!("{} {{\n{}\n}}", if tailwind { "@theme" } else { ":root" }, base.join("\n"))];
    for (mode, lines) in &modes {
        blocks.push(format!("[data-theme=\"{}\"] {{\n{}\n}}", kebab(mode), lines.join("\n")));
    }
    blocks.extend(text_rules);
    if !skipped.is_empty() {
        blocks.push(format!("/* Not converted (image/mixed paints): {} */", js::join(&skipped, ", ")));
    }
    if !renamed.is_empty() {
        blocks.push(format!("/* Renamed to avoid clashing with a color of the same name — use the map:\n   {} */", renamed.join("\n   ")));
    }
    obj! { "content" => format!("{}\n", blocks.join("\n\n")), "map" => Value::Object(map) }
}
