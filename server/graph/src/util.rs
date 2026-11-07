//! Small JS-behaviour helpers shared by the wave-2 modules (paths built like `path.join`, line
//! splitting like `split(/\r?\n/)`, object-key order of counters, `fs` wrappers with Node's checks).

use crate::js;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use std::path::Path;

/// `path.join(a, b, …)` for an absolute first part: one string, separators and `.`/`..` normalised
/// like Node (win32 on Windows). Never lets a later absolute part replace the root (unlike
/// `PathBuf::join`), so `join(root, userPath)` stays inside the same string as in Node.
pub fn join(parts: &[&str]) -> String {
    let sep = if cfg!(windows) { "\\" } else { "/" };
    let joined = parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(sep);
    let keep_trailing = joined.ends_with('/') || joined.ends_with('\\');
    let mut out = crate::paths::resolve(&joined);
    if keep_trailing && !out.ends_with(sep) {
        out.push_str(sep);
    }
    out
}

/// `existsSync(p)`.
pub fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

/// `p.replaceAll("\\", "/")`.
pub fn slash(p: &str) -> String {
    p.replace('\\', "/")
}

/// `s.split(/\r?\n/)`.
pub fn split_lines(s: &str) -> Vec<&str> {
    let parts: Vec<&str> = s.split('\n').collect();
    let n = parts.len();
    parts
        .into_iter()
        .enumerate()
        .map(|(i, p)| if i + 1 < n { p.strip_suffix('\r').unwrap_or(p) } else { p })
        .collect()
}

/// `s.split(/\s+/).filter(Boolean)` (JS whitespace).
pub fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| js::is_js_space(c as u32)).filter(|w| !w.is_empty()).collect()
}

/// `s.trimEnd()` (JS whitespace).
pub fn trim_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| js::is_js_space(c as u32))
}

/// sha1 hex of a UTF-8 string, first `n` hex digits.
pub fn sha1_hex(s: &str, n: usize) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    hex[..n.min(hex.len())].to_string()
}

/// `String(x)` for a JSON value (`undefined` for `None`).
pub fn s(v: Option<&Value>) -> String {
    crate::data::js_string(v)
}

/// JS truthiness of an optional JSON value.
pub fn t(v: Option<&Value>) -> bool {
    js::truthy(v)
}

/// `o.key` as an array (`[]` when missing or not an array).
pub fn arr(v: Option<&Value>) -> &[Value] {
    match v {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

/// `.length` of an array/string value (0 otherwise).
pub fn len_of(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Array(a)) => a.len(),
        Some(Value::String(s)) => js::utf16_len(s),
        _ => 0,
    }
}

/// `arr.join(sep)` with JS element conversion (`null`/`undefined` → "").
pub fn join_values(v: &[Value], sep: &str) -> String {
    v.iter().map(|x| if x.is_null() { String::new() } else { s(Some(x)) }).collect::<Vec<_>>().join(sep)
}

/// `Object.values(o)` in JS key order (`[]` when not an object).
pub fn values(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Object(o)) => js::js_values(o),
        _ => Vec::new(),
    }
}

/// `Object.entries(o)` in JS key order.
pub fn entries(v: Option<&Value>) -> Vec<(&String, &Value)> {
    match v {
        Some(Value::Object(o)) => js::js_keys(o).into_iter().map(|k| (k, &o[k.as_str()])).collect(),
        _ => Vec::new(),
    }
}

/// A `{}` used as a counter (`o[k] = (o[k] ?? 0) + 1`), read back with `Object.entries` order.
#[derive(Default, Debug, Clone)]
pub struct Counter {
    keys: Vec<String>,
    counts: std::collections::HashMap<String, i64>,
}

impl Counter {
    pub fn add(&mut self, k: &str, n: i64) {
        match self.counts.get_mut(k) {
            Some(c) => *c += n,
            None => {
                self.keys.push(k.to_string());
                self.counts.insert(k.to_string(), n);
            }
        }
    }
    /// `Object.entries(o)`: integer-like keys ascending first, then insertion order.
    pub fn entries(&self) -> Vec<(String, i64)> {
        let mut idx: Vec<&String> = self.keys.iter().filter(|k| js::is_array_index(k)).collect();
        idx.sort_by_key(|k| k.parse::<u64>().unwrap_or(0));
        idx.extend(self.keys.iter().filter(|k| !js::is_array_index(k)));
        idx.into_iter().map(|k| (k.clone(), self.counts[k])).collect()
    }
    /// `Object.entries(o).sort((a, b) => b[1] - a[1])`.
    pub fn sorted_desc(&self) -> Vec<(String, i64)> {
        let mut e = self.entries();
        e.sort_by_key(|x| std::cmp::Reverse(x.1));
        e
    }
}

/// `String.prototype.padEnd(n)` (UTF-16 length).
pub fn pad_end(s: &str, n: usize) -> String {
    let l = js::utf16_len(s);
    if l >= n {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(n - l))
    }
}

/// `${n}` for an optional SQLite integer (`null` when NULL).
pub fn num(v: Option<i64>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "null".into())
}

/// An optional integer as JSON (`null` when NULL).
pub fn jnum(v: Option<i64>) -> Value {
    v.map(Value::from).unwrap_or(Value::Null)
}

/// An optional string as JSON (`null` when NULL).
pub fn jstr(v: Option<&str>) -> Value {
    v.map(|x| Value::String(x.to_string())).unwrap_or(Value::Null)
}

/// `stat.mtime.toISOString()` (Node rounds `mtimeMs` to whole milliseconds for the Date).
pub fn mtime_iso(meta: &std::fs::Metadata) -> String {
    js::iso_from_ms((js::mtime_ms(meta) + 0.5).floor() as i64)
}

/// `utimesSync(p, now, now)`.
pub fn touch_now(p: &str) -> std::io::Result<()> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms);
    let f = std::fs::OpenOptions::new().write(true).open(p)?;
    f.set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t))
}

/// `Date.now()`.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A JSON object built in order.
pub fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
}

/// `fs.readFileSync(p, "utf8")` with Node's error text for a missing file.
pub fn read_text(p: &str) -> anyhow::Result<String> {
    js::read_utf8(p).map_err(|e| fs_error(&e, "open", p))
}

/// Node's `ENOENT: no such file or directory, open '<path>'` style messages.
pub fn fs_error(e: &std::io::Error, syscall: &str, p: &str) -> anyhow::Error {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => "ENOENT: no such file or directory",
        std::io::ErrorKind::PermissionDenied => "EPERM: operation not permitted",
        std::io::ErrorKind::AlreadyExists => "EEXIST: file already exists",
        _ => {
            if e.raw_os_error() == Some(5) {
                "EPERM: operation not permitted"
            } else {
                return anyhow::anyhow!("{e}, {syscall} '{p}'");
            }
        }
    };
    anyhow::anyhow!("{code}, {syscall} '{p}'")
}
