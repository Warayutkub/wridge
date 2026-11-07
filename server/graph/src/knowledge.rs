//! Researched guidance per project (port of `knowledge.mjs`):
//! `<project>/.wara/knowledge/<topic>/<tool>@<major>.md`. The AI researches how the project's tools
//! (at their versions) handle a topic once, saves a short note with sources, and reuses it until it
//! goes stale. Project facts the user answered live in `.wara/knowledge/facts.json`.

use crate::build::project_root;
use crate::files::{ensure_gitignored, WARA_DIR};
use crate::js::{self, cmp_utf16, json_parse, json_stringify, json_stringify_pretty};
use crate::jsre::JsRegex;
use crate::re;
use crate::util::{self, obj, s};
use anyhow::{bail, Result};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

const MAX_AGE_DAYS: f64 = 180.0;
const DIR: &str = "knowledge";

// Packages worth a note: frameworks, data access, auth, validation. Everything else is noise here.
static NOTABLE: LazyLock<Vec<JsRegex>> = LazyLock::new(|| {
    [
        r"^next$",
        r"^react$",
        r"^express$",
        r"^fastify$",
        r"^hono$",
        r"^@nestjs\/core$",
        r"^next-auth$",
        r"^@auth\/",
        r"^better-auth$",
        r"^lucia$",
        r"^iron-session$",
        r"^jose$",
        r"^jsonwebtoken$",
        r"^bcrypt(js)?$",
        r"^argon2$",
        r"^zod$",
        r"^valibot$",
        r"^yup$",
        r"^prisma$",
        r"^@prisma\/client$",
        r"^drizzle-orm$",
        r"^mysql2$",
        r"^pg$",
        r"^postgres$",
        r"^mongoose$",
        r"^mongodb$",
        r"^kysely$",
        r"^typeorm$",
        r"^sequelize$",
        r"^ioredis$",
        r"^redis$",
        r"^@upstash\/",
        r"^bullmq$",
        r"^stripe$",
        r"^@line\/",
        r"^nodemailer$",
        r"^multer$",
        r"^helmet$",
        r"github\.com\/gofiber\/fiber",
        r"github\.com\/gin-gonic\/gin",
        r"github\.com\/labstack\/echo",
        r"github\.com\/go-chi\/chi",
        r"gorm\.io\/gorm",
        r"github\.com\/jmoiron\/sqlx",
        r"github\.com\/jackc\/pgx",
        r"github\.com\/go-sql-driver\/mysql",
        r"github\.com\/golang-jwt\/jwt",
        r"github\.com\/redis\/go-redis",
        r"github\.com\/go-playground\/validator",
        r"golang\.org\/x\/crypto",
    ]
    .iter()
    .map(|p| JsRegex::new(p, ""))
    .collect()
});

fn notable(name: &str) -> bool {
    NOTABLE.iter().any(|re| re.test_str(name))
}

/// `major(version)`: the first number in the version, or `"x"`.
pub fn major(version: &str) -> String {
    re!(r"(\d+)").exec_str(version).and_then(|m| m[1].clone()).unwrap_or_else(|| "x".into())
}

fn major_v(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => major(""),
        x => major(&s(x)),
    }
}

/// `slug(s)`: lower case, `github.com/` dropped, anything but `a-z0-9.@-` → `-`.
pub fn slug(st: &str) -> String {
    let l = re!(r"^github\.com\/").replace_first_str(&st.to_lowercase(), "");
    let l = re!(r"[^a-z0-9.@-]+", "g").replace_all_str(&l, "-");
    re!(r"^-+|-+$", "g").replace_all_str(&l, "")
}

fn read_json(file: &str) -> Option<Value> {
    js::read_utf8(file).ok().and_then(|t| json_parse(&t).ok())
}

// Folders holding a package.json or go.mod: the root and one level down (frontend/, backend/, apps/*).
fn manifest_dirs(root: &str) -> Vec<String> {
    let children = |dir: &str| -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        rd.flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !re!(r"^(\.|node_modules$|vendor$|dist$|build$)").test_str(n))
            .map(|n| util::join(&[dir, &n]))
            .collect()
    };
    let mut dirs = vec![root.to_string()];
    for d in children(root) {
        dirs.push(d.clone());
        let last = d.rsplit(['\\', '/']).next().unwrap_or("");
        if re!(r"^(apps|packages|services)$").test_str(last) {
            dirs.extend(children(&d));
        }
    }
    dirs.into_iter()
        .filter(|d| util::exists(&util::join(&[d, "package.json"])) || util::exists(&util::join(&[d, "go.mod"])))
        .collect()
}

/// `detectStack(root)` → `[{ tool, version, lang, where }]`: the project's notable tools with
/// installed (or declared) versions.
pub fn detect_stack(root: &str) -> Vec<Value> {
    let mut stack: IndexMap<String, Value> = IndexMap::new();
    for dir in manifest_dirs(root) {
        let rel = if dir == root {
            ".".to_string()
        } else {
            crate::js::slice_utf16(&dir, js::utf16_len(root) + 1, usize::MAX).replace('\\', "/")
        };
        if let Some(pkg) = read_json(&util::join(&[&dir, "package.json"])).filter(|p| util::t(Some(p))) {
            // { ...pkg.devDependencies, ...pkg.dependencies }
            let mut deps: Map<String, Value> = Map::new();
            for key in ["devDependencies", "dependencies"] {
                for (k, v) in util::entries(pkg.get(key)) {
                    deps.insert(k.clone(), v.clone());
                }
            }
            for name in js::js_keys(&deps) {
                if !notable(name) {
                    continue;
                }
                let installed = read_json(&util::join(&[&dir, "node_modules", name, "package.json"]))
                    .and_then(|p| p.get("version").cloned())
                    .filter(|v| !v.is_null())
                    .or_else(|| {
                        read_json(&util::join(&[root, "node_modules", name, "package.json"]))
                            .and_then(|p| p.get("version").cloned())
                            .filter(|v| !v.is_null())
                    });
                let version = installed.unwrap_or_else(|| json!(re!(r"^[\^~>=<\s]+").replace_first_str(&s(deps.get(name.as_str())), "")));
                stack.insert(
                    name.clone(),
                    obj(vec![("tool", json!(name)), ("version", version), ("lang", json!("js")), ("where", json!(rel))]),
                );
            }
        }
        let gomod = util::join(&[&dir, "go.mod"]);
        if util::exists(&gomod) {
            let Ok(m) = js::read_utf8(&gomod) else { continue };
            if let Some(go) = re!(r"^go\s+(\S+)", "m").exec_str(&m).and_then(|x| x[1].clone()) {
                stack.insert(
                    "go".into(),
                    obj(vec![("tool", json!("go")), ("version", json!(go)), ("lang", json!("go")), ("where", json!(rel))]),
                );
            }
            let js_mod = crate::jsre::JsStr::new(&m);
            for x in re!(r"^\s*(?:require\s+)?([\w.\-/]+\.[\w.\-/]+)\s+(v[\w.\-+]+)", "gm").all(&js_mod) {
                let module = x.str(&js_mod, 1).unwrap_or_default();
                if !notable(&module) {
                    continue;
                }
                let tool = re!(r"\/v\d+$").replace_first_str(&module, "");
                stack.insert(
                    tool.clone(),
                    obj(vec![
                        ("tool", json!(tool)),
                        ("version", json!(x.str(&js_mod, 2).unwrap_or_default())),
                        ("lang", json!("go")),
                        ("where", json!(rel)),
                    ]),
                );
            }
        }
    }
    stack.into_values().collect()
}

fn knowledge_dir(root: &str) -> String {
    util::join(&[root, WARA_DIR, DIR])
}

fn file_for(root: &str, topic: &str, tool: &str, version: Option<&Value>) -> String {
    util::join(&[&knowledge_dir(root), &slug(topic), &format!("{}@{}.md", slug(tool), major_v(version))])
}

fn parse(file: &str) -> Result<(Map<String, Value>, String)> {
    let raw = util::read_text(file)?;
    let js = crate::jsre::JsStr::new(&raw);
    match re!(r"^<!--\s*([\s\S]*?)-->\n?").exec(&js) {
        Some(m) => {
            let meta = match json_parse(&m.str(&js, 1).unwrap_or_default()) {
                Ok(Value::Object(o)) => o,
                _ => Map::new(),
            };
            Ok((meta, js.slice_from(m.end).to_string()))
        }
        None => Ok((Map::new(), raw)),
    }
}

/// `Date.parse(s)` for the ISO forms JS writes (`toISOString`) and plain dates; NaN otherwise.
fn date_parse(st: &str) -> f64 {
    let Some(m) = re!(r"^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3})\d*)?)?(Z|[+-]\d{2}:\d{2})?)?$").exec_str(st) else {
        return f64::NAN;
    };
    let n = |i: usize| m[i].as_deref().map(|x| x.parse::<i64>().unwrap_or(0)).unwrap_or(0);
    let (y, mo, d) = (n(1), n(2), n(3));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || n(4) > 24 || n(5) > 59 || n(6) > 59 {
        return f64::NAN;
    }
    let ms = m[7].as_deref().map(|x| format!("{x:0<3}").parse::<i64>().unwrap_or(0)).unwrap_or(0);
    let days = days_from_civil(y, mo, d);
    let mut t = days * 86_400_000 + n(4) * 3_600_000 + n(5) * 60_000 + n(6) * 1000 + ms;
    match (m[4].is_some(), m[8].as_deref()) {
        (false, _) | (true, Some("Z")) => {} // date only, or UTC
        (true, Some(off)) => {
            let sign = if off.starts_with('-') { -1 } else { 1 };
            let h: i64 = off[1..3].parse().unwrap_or(0);
            let mi: i64 = off[4..6].parse().unwrap_or(0);
            t -= sign * (h * 3_600_000 + mi * 60_000);
        }
        (true, None) => t -= local_offset_ms(t), // date-time without offset: local time
    }
    t as f64
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(windows)]
fn local_offset_ms(_utc_ms: i64) -> i64 {
    #[repr(C)]
    struct Tzi {
        bias: i32,
        rest: [u8; 168],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(tz: *mut Tzi) -> u32;
    }
    let mut tz = Tzi { bias: 0, rest: [0; 168] };
    // SAFETY: TIME_ZONE_INFORMATION is 172 bytes; the buffer has that size.
    unsafe { GetTimeZoneInformation(&mut tz) };
    -(tz.bias as i64) * 60_000
}

#[cfg(not(windows))]
fn local_offset_ms(_utc_ms: i64) -> i64 {
    0
}

/// `Date.parse(stat.mtime)`: the Date is turned into its `toString()` first, so only whole seconds
/// survive.
fn mtime_seconds_ms(p: &str) -> f64 {
    match std::fs::metadata(p) {
        Ok(m) => {
            let ms = (js::mtime_ms(&m) + 0.5).floor();
            (ms / 1000.0).floor() * 1000.0
        }
        Err(_) => f64::NAN,
    }
}

fn age_days(saved_ms: f64) -> f64 {
    ((util::now_ms() as f64 - saved_ms) / 86_400_000.0).floor()
}

fn num_json(x: f64) -> Value {
    if x.is_nan() {
        Value::Null
    } else {
        json!(x as i64)
    }
}

/// `listKnowledge(projectDir, topic)` → `{ stack, facts, notes, missing? }`: saved notes, each marked
/// stale when old or when the project's major version moved on.
pub fn list_knowledge(project_dir: &str, topic: Option<&str>) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let topic = topic.filter(|t| !t.is_empty());
    let stack = detect_stack(&root);
    let current: std::collections::HashMap<String, String> = stack.iter().map(|x| (slug(&s(x.get("tool"))), major_v(x.get("version")))).collect();
    let mut notes: Vec<Value> = Vec::new();
    let base = knowledge_dir(&root);
    let mut topics: Vec<String> = Vec::new();
    if util::exists(&base) {
        for e in std::fs::read_dir(&base)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) && topic.is_none_or(|tp| name == slug(tp)) {
                topics.push(name);
            }
        }
    }
    for tp in &topics {
        let tdir = util::join(&[&base, tp]);
        for e in std::fs::read_dir(&tdir)?.flatten() {
            let f = e.file_name().to_string_lossy().into_owned();
            if !f.ends_with(".md") {
                continue;
            }
            let file = util::join(&[&tdir, &f]);
            let (meta, _) = parse(&file)?;
            let stem = &f[..f.len() - 3];
            let mut parts = stem.split('@');
            let tool = parts.next().unwrap_or("").to_string();
            let ver = parts.next();
            let saved = match meta.get("savedAt") {
                None | Some(Value::Null) => mtime_seconds_ms(&file),
                Some(v) => date_parse(&s(Some(v))),
            };
            let age = age_days(saved);
            let now = current.get(&tool);
            let stale = if age > MAX_AGE_DAYS {
                json!(format!("older than {} days", MAX_AGE_DAYS as i64))
            } else if let Some(now) = now.filter(|n| Some(n.as_str()) != ver) {
                json!(format!("project now uses major {now}"))
            } else {
                Value::Null
            };
            let pick = |k: &str| meta.get(k).filter(|v| !v.is_null()).cloned();
            let mut n = Map::new();
            n.insert("topic".into(), json!(tp));
            n.insert("tool".into(), pick("tool").unwrap_or_else(|| json!(tool)));
            if let Some(v) = pick("version").or_else(|| ver.map(|v| json!(v))) {
                n.insert("version".into(), v); // `undefined` (left out) when neither exists
            }
            n.insert("savedAt".into(), pick("savedAt").unwrap_or(Value::Null));
            n.insert("ageDays".into(), num_json(age));
            n.insert("stale".into(), stale);
            notes.push(Value::Object(n));
        }
    }
    let covered: std::collections::HashSet<String> = notes
        .iter()
        .filter(|n| n.get("stale").is_none_or(Value::is_null) && topic.is_none_or(|tp| n.get("topic").and_then(Value::as_str) == Some(slug(tp).as_str())))
        .map(|n| slug(&s(n.get("tool"))))
        .collect();
    let mut out = Map::new();
    out.insert("stack".into(), Value::Array(stack.clone()));
    out.insert("facts".into(), read_json(&util::join(&[&base, "facts.json"])).filter(|v| !v.is_null()).unwrap_or(json!({})));
    out.insert("notes".into(), Value::Array(notes));
    if topic.is_some() {
        out.insert(
            "missing".into(),
            json!(stack
                .iter()
                .filter(|x| !covered.contains(&slug(&s(x.get("tool")))))
                .map(|x| format!("{}@{}", s(x.get("tool")), major_v(x.get("version"))))
                .collect::<Vec<_>>()),
        );
    }
    Ok(Value::Object(out))
}

/// `getKnowledge(projectDir, topic, tool)` → `{ found: false, hint }` or
/// `{ found: true, ...meta, ageDays, stale, projectVersion, content }`.
pub fn get_knowledge(project_dir: &str, topic: &str, tool: &str) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let version = detect_stack(&root)
        .into_iter()
        .find(|x| slug(&s(x.get("tool"))) == slug(tool))
        .and_then(|x| x.get("version").cloned());
    let dir = util::join(&[&knowledge_dir(&root), &slug(topic)]);
    let mut file = version.as_ref().filter(|v| util::t(Some(v))).map(|v| file_for(&root, topic, tool, Some(v)));
    if !file.as_deref().is_some_and(util::exists) {
        // Fall back to the newest note for this tool (any version), flagged so the caller can re-check.
        let prefix = format!("{}@", slug(tool));
        let mut any: Vec<String> = if util::exists(&dir) {
            std::fs::read_dir(&dir)?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(&prefix))
                .collect()
        } else {
            Vec::new()
        };
        any.sort_by(|a, b| cmp_utf16(a, b));
        let Some(last) = any.pop() else {
            return Ok(obj(vec![
                ("found", json!(false)),
                (
                    "hint",
                    json!(format!("No {topic} note for {tool}. Research it (official docs first), then save it with action \"save\".")),
                ),
            ]));
        };
        file = Some(util::join(&[&dir, &last]));
    }
    let (meta, content) = parse(file.as_deref().unwrap())?;
    let saved = match meta.get("savedAt") {
        None | Some(Value::Null) => f64::NAN,
        Some(v) => date_parse(&s(Some(v))),
    };
    let age = age_days(saved);
    let version_moved = version.as_ref().is_some_and(|v| util::t(Some(v)) && major_v(Some(v)) != major_v(meta.get("version")));
    let mut out = Map::new();
    out.insert("found".into(), json!(true));
    for (k, v) in &meta {
        out.insert(k.clone(), v.clone());
    }
    out.insert("ageDays".into(), num_json(age));
    out.insert("stale".into(), json!(age > MAX_AGE_DAYS || version_moved));
    out.insert("projectVersion".into(), version.unwrap_or(Value::Null));
    out.insert("content".into(), json!(content));
    Ok(Value::Object(out))
}

fn facts_file(root: &str) -> String {
    util::join(&[&knowledge_dir(root), "facts.json"])
}

/// `readFacts(projectDir)`: project facts only the user can answer (deploy target, Redis, CI…).
pub fn read_facts(project_dir: &str) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    Ok(read_json(&facts_file(&root)).filter(|v| !v.is_null()).unwrap_or(json!({})))
}

fn gitignore_note(g: &Value) -> Option<Value> {
    (g.get("added") == Some(&Value::Bool(true))).then(|| json!(format!("added {} to {}", s(g.get("pattern")), s(g.get("gitignore")))))
}

/// `setFact(projectDir, key, value)` → `{ facts, gitignore? }`; an empty value deletes the fact.
pub fn set_fact(project_dir: &str, key: &str, value: &str) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let gitignore = ensure_gitignored(std::path::Path::new(&root))?;
    let mut facts = match read_json(&facts_file(&root)) {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    if value.is_empty() {
        facts.shift_remove(key);
    } else {
        facts.insert(key.to_string(), obj(vec![("value", json!(value)), ("savedAt", json!(js::iso_now()))]));
    }
    std::fs::create_dir_all(knowledge_dir(&root))?;
    let facts = Value::Object(facts);
    std::fs::write(facts_file(&root), format!("{}\n", json_stringify_pretty(&facts, 2)))?;
    let mut out = Map::new();
    out.insert("facts".into(), facts);
    if let Some(g) = gitignore_note(&gitignore) {
        out.insert("gitignore".into(), g);
    }
    Ok(Value::Object(out))
}

/// Arguments of [`save_knowledge`].
#[derive(Clone, Debug, Default)]
pub struct SaveArgs {
    pub topic: String,
    pub tool: String,
    pub version: Option<String>,
    pub content: String,
    pub sources: Option<Vec<String>>,
}

/// `saveKnowledge(projectDir, { topic, tool, version, content, sources })` →
/// `{ file, topic, tool, version, savedAt, sources?, gitignore? }`.
pub fn save_knowledge(project_dir: &str, a: &SaveArgs) -> Result<Value> {
    let root = project_root(project_dir)?.to_string_lossy().into_owned();
    let detected = detect_stack(&root)
        .into_iter()
        .find(|x| slug(&s(x.get("tool"))) == slug(&a.tool))
        .and_then(|x| x.get("version").cloned());
    let ver = match &a.version {
        Some(v) => Some(json!(v)),
        None => detected,
    };
    let Some(ver) = ver.filter(|v| util::t(Some(v))) else {
        bail!("Unknown version of {} — pass version", a.tool)
    };
    let gitignore = ensure_gitignored(std::path::Path::new(&root))?; // .wara/ never goes into git
    let file = file_for(&root, &a.topic, &a.tool, Some(&ver));
    std::fs::create_dir_all(util::join(&[&knowledge_dir(&root), &slug(&a.topic)]))?;
    let mut meta = Map::new();
    meta.insert("topic".into(), json!(slug(&a.topic)));
    meta.insert("tool".into(), json!(a.tool));
    meta.insert("version".into(), ver);
    meta.insert("savedAt".into(), json!(js::iso_now()));
    if let Some(src) = &a.sources {
        meta.insert("sources".into(), json!(src));
    }
    let meta = Value::Object(meta);
    std::fs::write(&file, format!("<!-- {} -->\n{}\n", json_stringify(&meta), js::js_trim(&a.content)))?;
    let mut out = Map::new();
    out.insert("file".into(), json!(file.replace('\\', "/")));
    if let Value::Object(m) = meta {
        out.extend(m);
    }
    if let Some(g) = gitignore_note(&gitignore) {
        out.insert("gitignore".into(), g);
    }
    Ok(Value::Object(out))
}
