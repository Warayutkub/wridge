//! Port of `legacy-node/motion.mjs`: Lottie files in a code project, for the /motion page — found by what
//! they contain (no registry), read and written only inside the project. A file made by create_lottie keeps
//! its recipe in meta.wara (spec) and, once sent to Figma, the GIF component it fills (meta.wara.figma).

use crate::js;
use crate::node_path as path;
use crate::obj;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use std::sync::OnceLock;

const SKIP_DIRS: [&str; 17] = ["node_modules", ".git", ".wara", ".next", ".nuxt", ".dart_tool", ".gradle", ".idea", ".vscode", "build", "dist", "out", "coverage", "Pods", "vendor", ".turbo", ".cache"];
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// `isLottie(j)` as a boolean (the JS returns `j` itself when it is falsy).
pub fn is_lottie(j: &Value) -> bool {
    let Value::Object(o) = j else { return false };
    matches!(o.get("v"), Some(Value::String(_)))
        && js::to_number(o.get("fr")) > 0.0
        && matches!(o.get("layers"), Some(Value::Array(_)))
        && matches!(o.get("op"), Some(Value::Number(n)) if js::number_f64(n).is_finite())
        && js::to_number(o.get("w")) > 0.0
}

/// `contentHash(j)`: sha1 of the animation without its recipe (`meta`), first 16 hex digits — tells
/// whether the GIF in Figma is older than the file.
pub fn content_hash(j: &Value) -> String {
    let mut copy = Map::new();
    match j {
        Value::Object(o) => {
            for (k, v) in o {
                if k != "meta" {
                    copy.insert(k.clone(), v.clone());
                }
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                copy.insert(i.to_string(), v.clone());
            }
        }
        Value::String(s) => {
            for (i, c) in s.encode_utf16().enumerate() {
                copy.insert(i.to_string(), Value::String(String::from_utf16_lossy(&[c])));
            }
        }
        _ => {}
    }
    let text = js::stringify(&Value::Object(copy));
    let digest = Sha1::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect::<String>()[..16].to_string()
}

/// `insideProject(root, path)`: the absolute path, or an error when it leaves the project.
pub fn inside_project(root: &str, p: &str) -> Result<String> {
    let abs = path::resolve(&[root, p]);
    let rel = path::relative(&path::resolve(&[root]), &abs);
    if rel.is_empty() || rel.starts_with("..") || path::is_absolute(&rel) {
        return Err(anyhow!("Path must be inside the project: {p}"));
    }
    Ok(abs)
}

/// The text of a Node fs error (`ENOENT: no such file or directory, open '…'`).
pub(crate) fn fs_error(e: &std::io::Error, syscall: &str, p: &str) -> anyhow::Error {
    use std::io::ErrorKind::*;
    let (code, text) = match e.kind() {
        NotFound => ("ENOENT", "no such file or directory"),
        PermissionDenied => ("EPERM", "operation not permitted"),
        IsADirectory => ("EISDIR", "illegal operation on a directory"),
        AlreadyExists => ("EEXIST", "file already exists"),
        _ => return anyhow!("{e}"),
    };
    if code == "EISDIR" {
        return anyhow!("{code}: {text}, {syscall}");
    }
    anyhow!("{code}: {text}, {syscall} '{p}'")
}

/// `readFileSync(p, "utf8")` with Node's error texts.
pub(crate) fn read_text(p: &str) -> Result<String> {
    if std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false) {
        return Err(anyhow!("EISDIR: illegal operation on a directory, read"));
    }
    let bytes = std::fs::read(p).map_err(|e| fs_error(&e, "open", p))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn summary(root: &str, abs: &str, json: &Value, bytes: u64) -> Value {
    let j = Some(json);
    let wara = js::nullish(js::get(js::get(j, "meta"), "wara"));
    let frames = js::to_number(js::get(j, "op")) - js::nullish(js::get(j, "ip")).map_or(0.0, js::number);
    let hash = content_hash(json);
    let fr = js::to_number(js::get(j, "fr"));
    let name = match js::get(j, "nm") {
        Some(nm) if js::truthy(Some(nm)) => nm.clone(),
        _ => {
            let file = abs.rsplit(['\\', '/']).next().unwrap_or("");
            let lower = file.to_lowercase();
            Value::String(if lower.ends_with(".json") { file[..file.len() - 5].to_string() } else { file.to_string() })
        }
    };
    let figma = js::get(wara, "figma").filter(|f| js::truthy(Some(f)));
    obj! {
        "path" => path::relative(root, abs).replace('\\', "/"),
        "name" => name,
        "bytes" => bytes as f64, "w" => js::get(j, "w").cloned(), "h" => js::get(j, "h").cloned(), "fps" => js::get(j, "fr").cloned(),
        "frames" => frames, "durationMs" => js::round((frames / fr) * 1000.0),
        "layers" => js::get(j, "layers").and_then(|l| l.as_array()).map_or(0, |l| l.len()),
        "wara" => js::truthy(js::get(wara, "spec")),
        "figma" => match figma {
            Some(f) => obj! {
                "componentId" => js::get(Some(f), "componentId").cloned(), "file" => js::get(Some(f), "file").cloned(),
                "sentAt" => js::get(Some(f), "sentAt").cloned(),
                "stale" => !js::strict_eq(js::get(Some(f), "hash"), Some(&Value::String(hash))),
            },
            None => Value::Null,
        },
    }
}

fn file_kind(e: &std::fs::DirEntry) -> (bool, bool) {
    e.file_type().map(|t| (t.is_dir(), t.is_file())).unwrap_or((false, false))
}

/// `listLottie(dir)`: every Lottie JSON under the project (skipping build / dependency folders), as
/// summaries sorted by path (`localeCompare`).
pub fn list_lottie(dir: &str) -> Result<Vec<Value>> {
    static RE: OnceLock<(Regex, Regex, Regex, Regex)> = OnceLock::new();
    let (json_re, skip_re, layers_re, fr_re) = RE.get_or_init(|| {
        (
            Regex::new(r"(?i-u)\.json$").unwrap(),
            Regex::new(r"(?i-u)^(package|package-lock|tsconfig|composer|app|manifest)\.json$").unwrap(),
            Regex::new(&format!(r#""layers"[{}]*:"#, js::WS)).unwrap(),
            Regex::new(&format!(r#""fr"[{}]*:"#, js::WS)).unwrap(),
        )
    });
    let root = path::resolve(&[dir]);
    let mut out = vec![];
    let mut seen = 0usize;
    fn walk(d: &str, root: &str, seen: &mut usize, out: &mut Vec<Value>, res: (&Regex, &Regex, &Regex, &Regex)) -> Result<()> {
        let Ok(entries) = std::fs::read_dir(d) else { return Ok(()) };
        for e in entries.flatten() {
            *seen += 1;
            if *seen > MAX_FILES {
                return Ok(());
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let (is_dir, is_file) = file_kind(&e);
            if is_dir {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    walk(&path::join(&[d, &name]), root, seen, out, res)?;
                }
                continue;
            }
            if !is_file || !res.0.is_match(&name) || res.1.is_match(&name) {
                continue;
            }
            let abs = path::join(&[d, &name]);
            let size = std::fs::metadata(&abs).map_err(|e| fs_error(&e, "stat", &abs))?.len();
            if !(200..=MAX_BYTES).contains(&size) {
                continue;
            }
            let text = read_text(&abs)?;
            if !res.2.is_match(&text) || !res.3.is_match(&text) {
                continue; // cheap check before parsing
            }
            if let Ok(j) = serde_json::from_str::<Value>(&text) {
                if is_lottie(&j) {
                    out.push(summary(root, &abs, &j, size));
                }
            }
        }
        Ok(())
    }
    walk(&root, &root, &mut seen, &mut out, (json_re, skip_re, layers_re, fr_re))?;
    out.sort_by(|a, b| js::locale_compare(a["path"].as_str().unwrap_or(""), b["path"].as_str().unwrap_or("")));
    Ok(out)
}

/// `readLottie(dir, path)`.
pub fn read_lottie(dir: &str, p: &str) -> Result<Value> {
    let abs = inside_project(dir, p)?;
    let json: Value = js::parse(&read_text(&abs)?)?;
    if !is_lottie(&json) {
        return Err(anyhow!("{p} is not a Lottie file"));
    }
    Ok(json)
}

/// `markSentToFigma(dir, path, { componentId, file })`: after the GIF went to Figma, remember the component
/// and which version of the animation it shows. Writes the file.
pub fn mark_sent_to_figma(dir: &str, p: &str, info: &Value) -> Result<Value> {
    let abs = inside_project(dir, p)?;
    let mut json: Value = js::parse(&read_text(&abs)?)?;
    let hash = content_hash(&json);
    let Value::Object(o) = &mut json else {
        return Err(anyhow!("Cannot create property 'meta' on {}", js::stringify(&json)));
    };
    let mut meta = match o.get("meta") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let mut wara = match meta.get("wara") {
        Some(Value::Object(w)) => w.clone(),
        _ => Map::new(),
    };
    let i = Some(info);
    wara.insert(
        "figma".into(),
        obj! { "componentId" => js::get(i, "componentId").cloned(), "file" => js::get(i, "file").cloned(), "sentAt" => js::iso_now(), "hash" => hash },
    );
    meta.insert("wara".into(), Value::Object(wara));
    o.insert("meta".into(), Value::Object(meta));
    let text = js::stringify(&json);
    std::fs::write(&abs, &text).map_err(|e| fs_error(&e, "open", &abs))?;
    match json.get("layers") {
        None => return Err(anyhow!("Cannot read properties of undefined (reading 'length')")),
        Some(Value::Null) => return Err(anyhow!("Cannot read properties of null (reading 'length')")),
        _ => {}
    }
    Ok(summary(&path::resolve(&[dir]), &abs, &json, text.len() as u64))
}
