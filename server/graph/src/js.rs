//! JavaScript-compatible helpers shared by every module (and by wave 2).
//!
//! The Node server is the reference: these helpers reproduce what V8 does so that Rust output is
//! byte-identical — `JSON.stringify` (key order, number format, escaping), `Number#toString`,
//! `String#trim` whitespace, UTF-16 string lengths/slices, `a < b` string comparison,
//! `localeCompare` (ICU, for ASCII) and `Date#toISOString`.

use serde_json::Value;
use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------- whitespace ----------

/// JS `WhiteSpace` + `LineTerminator` (what `\s` and `String#trim` use).
pub fn is_js_space(c: u32) -> bool {
    matches!(
        c,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// `String#trim` (JS whitespace set, which includes U+FEFF but not U+0085).
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| is_js_space(c as u32))
}

/// `String#trimStart`.
pub fn js_trim_start(s: &str) -> &str {
    s.trim_start_matches(|c: char| is_js_space(c as u32))
}

// ---------- UTF-16 ----------

/// `s.length` in JS (UTF-16 code units).
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(start, end)` with UTF-16 indices (non-negative, clamped). A surrogate pair cut in half
/// becomes U+FFFD, which is what V8 writes to SQLite for a lone surrogate.
pub fn slice_utf16(s: &str, start: usize, end: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let end = end.min(units.len());
    if start >= end {
        return String::new();
    }
    String::from_utf16_lossy(&units[start..end])
}

/// JS `a < b` / default `Array#sort` order: compares UTF-16 code units.
pub fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Reads a file the way `readFileSync(file, "utf8")` does (BOM kept, invalid bytes → U+FFFD).
pub fn read_utf8(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}

// ---------- numbers ----------

/// `Number.prototype.toString()` / `String(n)` for a double.
pub fn js_number(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // Shortest round-trip digits, as V8 uses.
    let e = format!("{:e}", x.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp + 1;
    let body = if k <= n && n <= 21 {
        format!("{}{}", digits, "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{}", "0".repeat((-n) as usize), digits)
    } else {
        let es = if n > 0 { format!("+{}", n - 1) } else { format!("-{}", 1 - n) };
        if k == 1 {
            format!("{}e{}", digits, es)
        } else {
            format!("{}.{}e{}", &digits[..1], &digits[1..], es)
        }
    };
    format!("{sign}{body}")
}

// ---------- JSON.stringify ----------

/// True for keys JS orders first in objects (canonical array indices "0" … "4294967294").
pub fn is_array_index(k: &str) -> bool {
    if k.is_empty() || k.len() > 10 || !k.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if k.len() > 1 && k.starts_with('0') {
        return false;
    }
    k.parse::<u64>().map(|n| n < 4_294_967_295).unwrap_or(false)
}

/// Object keys in JS enumeration order: integer-like keys ascending, then insertion order.
pub fn js_keys(map: &serde_json::Map<String, Value>) -> Vec<&String> {
    let mut idx: Vec<&String> = map.keys().filter(|k| is_array_index(k)).collect();
    if idx.is_empty() {
        return map.keys().collect();
    }
    idx.sort_by_key(|k| k.parse::<u64>().unwrap());
    idx.extend(map.keys().filter(|k| !is_array_index(k)));
    idx
}

/// `Object.values(obj)` in JS order.
pub fn js_values(map: &serde_json::Map<String, Value>) -> Vec<&Value> {
    js_keys(map).into_iter().map(|k| &map[k.as_str()]).collect()
}

fn quote_into(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `JSON.stringify(s)` for a string.
pub fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    quote_into(&mut out, s);
    out
}

fn number_into(out: &mut String, n: &serde_json::Number) {
    if let Some(i) = n.as_i64() {
        out.push_str(&i.to_string());
    } else if let Some(u) = n.as_u64() {
        out.push_str(&js_number(u as f64));
    } else {
        let f = n.as_f64().unwrap_or(f64::NAN);
        if f.is_finite() {
            out.push_str(&js_number(f));
        } else {
            out.push_str("null");
        }
    }
}

fn write_value(out: &mut String, v: &Value, indent: &str, cur: &str) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => number_into(out, n),
        Value::String(s) => quote_into(out, s),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            let inner = format!("{cur}{indent}");
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !indent.is_empty() {
                    out.push('\n');
                    out.push_str(&inner);
                }
                write_value(out, x, indent, &inner);
            }
            if !indent.is_empty() {
                out.push('\n');
                out.push_str(cur);
            }
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            let inner = format!("{cur}{indent}");
            out.push('{');
            for (i, k) in js_keys(m).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !indent.is_empty() {
                    out.push('\n');
                    out.push_str(&inner);
                }
                quote_into(out, k);
                out.push(':');
                if !indent.is_empty() {
                    out.push(' ');
                }
                write_value(out, &m[k.as_str()], indent, &inner);
            }
            if !indent.is_empty() {
                out.push('\n');
                out.push_str(cur);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(v)`: JS key order, JS number format, JS escaping.
pub fn json_stringify(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, "", "");
    out
}

/// `JSON.stringify(v, null, indent)`.
pub fn json_stringify_pretty(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_value(&mut out, v, &" ".repeat(indent.min(10)), "");
    out
}

/// `JSON.parse(text)`: object keys keep their order. Unlike serde_json, V8 accepts escaped lone
/// surrogates (`"\ud83c"`, which `JSON.stringify` writes for a string cut inside an emoji); they
/// become U+FFFD here — what V8 stores in SQLite for them — instead of failing the whole parse.
pub fn json_parse(text: &str) -> anyhow::Result<Value> {
    match serde_json::from_str(text) {
        Ok(v) => Ok(v),
        Err(e) => {
            let fixed = replace_lone_surrogate_escapes(text);
            if fixed == text {
                return Err(e.into());
            }
            serde_json::from_str(&fixed).map_err(|_| e.into())
        }
    }
}

/// Rewrites `\uD800`–`\uDFFF` escapes that are not part of a valid pair to `�` (inside strings).
fn replace_lone_surrogate_escapes(text: &str) -> String {
    let b = text.as_bytes();
    let hex4 = |i: usize| -> Option<u16> { text.get(i..i + 4).and_then(|h| u16::from_str_radix(h, 16).ok()) };
    let esc_at = |i: usize| -> Option<u16> {
        if b.get(i) == Some(&b'\\') && b.get(i + 1) == Some(&b'u') {
            hex4(i + 2)
        } else {
            None
        }
    };
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_str) = (0usize, false);
    while i < b.len() {
        let c = b[i];
        if in_str && c == b'\\' {
            if let Some(u) = esc_at(i) {
                if (0xD800..0xDC00).contains(&u) && esc_at(i + 6).is_some_and(|l| (0xDC00..0xE000).contains(&l)) {
                    out.push_str(&text[i..i + 12]);
                    i += 12;
                } else if (0xD800..0xE000).contains(&u) {
                    out.push_str("\\ufffd");
                    i += 6;
                } else {
                    out.push_str(&text[i..i + 6]);
                    i += 6;
                }
                continue;
            }
            // another escape: copy it whole, so an escaped quote does not end the string
            out.push('\\');
            i += 1;
            if let Some(next) = text[i..].chars().next() {
                out.push(next);
                i += next.len_utf8();
            }
            continue;
        }
        if c == b'"' {
            in_str = !in_str;
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// JS truthiness of a JSON value (`null`, `false`, `0`, `""` are falsy).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

// ---------- dates ----------

/// `new Date().toISOString()`.
pub fn iso_now() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    iso_from_ms(ms)
}

/// `new Date(ms).toISOString()` for a Unix time in milliseconds.
pub fn iso_from_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s, milli) = (rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000);
    if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
    } else {
        let sign = if y < 0 { '-' } else { '+' };
        format!("{sign}{:06}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z", y.abs())
    }
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `fs.statSync(p).mtimeMs`: seconds * 1e3 + nanoseconds / 1e6, in doubles, as Node computes it.
pub fn mtime_ms(meta: &std::fs::Metadata) -> f64 {
    match meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) => d.as_secs() as f64 * 1000.0 + d.subsec_nanos() as f64 / 1_000_000.0,
        None => match meta.modified().ok().and_then(|t| UNIX_EPOCH.duration_since(t).ok()) {
            // pre-1970: negative time (Node: sec and nsec carry the sign)
            Some(d) => -(d.as_secs() as f64 * 1000.0 + d.subsec_nanos() as f64 / 1_000_000.0),
            None => 0.0,
        },
    }
}

// ---------- localeCompare ----------

/// Collation behaviour of `String#localeCompare` with the process's default ICU locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collation {
    /// CLDR root (en, …): punctuation is significant.
    Root,
    /// Locales with `alternate=shifted` (Thai): spaces and punctuation are ignored.
    Shifted,
}

static COLLATION: std::sync::OnceLock<Collation> = std::sync::OnceLock::new();

/// Sets the collation used by [`locale_compare`] (default: detected from the OS locale, or
/// `WRIDGE_COLLATION=root|shifted`).
pub fn set_collation(c: Collation) {
    let _ = COLLATION.set(c);
}

/// The collation in effect (see [`set_collation`]).
pub fn collation() -> Collation {
    *COLLATION.get_or_init(|| {
        match std::env::var("WRIDGE_COLLATION").ok().as_deref() {
            Some("root") => return Collation::Root,
            Some("shifted") => return Collation::Shifted,
            _ => {}
        }
        let loc = default_locale().to_ascii_lowercase();
        if loc.starts_with("th") {
            Collation::Shifted
        } else {
            Collation::Root
        }
    })
}

#[cfg(windows)]
fn default_locale() -> String {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    // SAFETY: the buffer is LOCALE_NAME_MAX_LENGTH (85) wide chars, as the API requires.
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(n - 1) as usize])
}

#[cfg(not(windows))]
fn default_locale() -> String {
    for k in ["LC_ALL", "LC_COLLATE", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    String::new()
}

// ICU order of printable ASCII (taken from Node's Intl.Collator): punctuation/symbols, digits, letters.
const ROOT_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";
// With alternate=shifted these are ignorable; the rest keep the root order.
const SHIFTED_IGNORABLE: &str = " !\"#%&'()*,-./:;?@[\\]_{}";

fn collation_key(c: char, mode: Collation) -> Option<(u32, u8)> {
    if c.is_ascii() {
        if mode == Collation::Shifted && SHIFTED_IGNORABLE.contains(c) {
            return None;
        }
        let lower = c.to_ascii_lowercase();
        if let Some(p) = ROOT_ORDER.find(lower) {
            return Some((p as u32 + 1, if c.is_ascii_uppercase() { 1 } else { 0 }));
        }
        // control characters: ignorable
        return None;
    }
    // Outside ASCII: after every ASCII character, by code point (an approximation of ICU).
    Some((0x1_0000 + c as u32, 0))
}

/// `a.localeCompare(b)` as ICU does it for ASCII text (primary, then case: lowercase first).
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let mode = collation();
    let ka: Vec<(u32, u8)> = a.chars().filter_map(|c| collation_key(c, mode)).collect();
    let kb: Vec<(u32, u8)> = b.chars().filter_map(|c| collation_key(c, mode)).collect();
    let pa = ka.iter().map(|k| k.0);
    let pb = kb.iter().map(|k| k.0);
    match pa.cmp(pb) {
        Ordering::Equal => ka.iter().map(|k| k.1).cmp(kb.iter().map(|k| k.1)),
        o => o,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_js() {
        assert_eq!(js_number(3.0), "3");
        assert_eq!(js_number(0.1), "0.1");
        assert_eq!(js_number(1791274148123.4567), "1791274148123.4568");
        assert_eq!(js_number(1e21), "1e+21");
        assert_eq!(js_number(1.5e-7), "1.5e-7");
        assert_eq!(js_number(123456789012345680000.0), "123456789012345680000");
        assert_eq!(js_number(-2.5), "-2.5");
        assert_eq!(js_number(0.000001), "0.000001");
    }

    #[test]
    fn stringify_orders_integer_keys_first() {
        let v: Value = serde_json::from_str(r#"{"b":1,"2":true,"a":[1.0,"x\u0001"],"1":{}}"#).unwrap();
        assert_eq!(json_stringify(&v), r#"{"1":{},"2":true,"b":1,"a":[1,"x\u0001"]}"#);
        assert_eq!(json_stringify_pretty(&serde_json::json!([{"a": []}]), 2), "[\n  {\n    \"a\": []\n  }\n]");
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso_from_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_ms(1_791_274_148_123), "2026-10-06T08:09:08.123Z");
    }

    #[test]
    fn parse_accepts_lone_surrogate_escapes() {
        let v = json_parse(r#"{"a":"x\ud83c","b":"🎉","c":"q\\ud83c","d":"\"\udf89"}"#).unwrap();
        assert_eq!(v["a"], "x\u{fffd}");
        assert_eq!(v["b"], "\u{1f389}");
        assert_eq!(v["c"], "q\\ud83c");
        assert_eq!(v["d"], "\"\u{fffd}");
        assert!(json_parse("{").is_err());
    }

    #[test]
    fn trim_uses_js_whitespace() {
        assert_eq!(js_trim("\u{feff} a \u{a0}"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
    }
}
