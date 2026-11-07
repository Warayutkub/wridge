//! JavaScript semantics the ported modules rely on: number ↔ string conversion, `Number()`,
//! `parseFloat`, truthiness, `String()`, `JSON.stringify` (key order and number format as V8 writes
//! them), `Math.round`/`min`/`max`, UTF-16 lengths and `localeCompare`.
//!
//! JS values are carried as `serde_json::Value`; `undefined` is `None` (an absent key).
//! Numbers that are integers are stored as integers, so `serde_json` prints `3`, never `3.0`.
//! Use [`stringify`] rather than `serde_json::to_string` to get byte-identical JSON: it also prints
//! floats the way JS does (`0.0000012`, not `1.2e-6`) and puts integer-like keys first.

use serde_json::{Map, Number, Value};
use std::cmp::Ordering;
use std::sync::OnceLock;

pub type Obj = Map<String, Value>;

// ---------------------------------------------------------------- numbers

/// `Number.prototype.toString()` / `String(x)` for a number.
pub fn num_str(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let (digits, exp) = shortest_digits(x.abs());
    let k = digits.len() as i32;
    let n = exp + 1;
    let mut out = String::new();
    if x < 0.0 {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.push_str(&"0".repeat((n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-n) as usize));
        out.push_str(&digits);
    } else {
        let e = n - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e >= 0 { '+' } else { '-' });
        out.push_str(&e.abs().to_string());
    }
    out
}

fn split_exp(s: &str) -> (String, i32) {
    let (mant, exp) = s.split_once('e').unwrap_or((s, "0"));
    (mant.chars().filter(|c| *c != '.').collect(), exp.parse().unwrap_or(0))
}

/// Shortest decimal digits that round-trip to `x` (> 0) and the exponent of the first digit.
/// Rust's `{:e}` finds the shortest length; when two digit strings of that length are equally near
/// `x`, JS takes the even one (ECMA-262 Number::toString note), so ties are settled here.
fn shortest_digits(x: f64) -> (String, i32) {
    let (digits, exp) = split_exp(&format!("{:e}", x));
    let k = digits.len();
    let last = digits.as_bytes()[k - 1] - b'0';
    if last.is_multiple_of(2) {
        return (digits, exp);
    }
    // A tie needs x to be exactly the midpoint: its exact expansion has k+1 digits ending in 5.
    let (d1, e1) = split_exp(&format!("{:.*e}", k + 24, x));
    if e1 != exp || d1.as_bytes()[k] != b'5' || d1[k + 1..].bytes().any(|b| b != b'0') {
        return (digits, exp);
    }
    let (full, _) = split_exp(&format!("{:.*e}", 1100, x));
    if full[k + 1..].bytes().any(|b| b != b'0') {
        return (digits, exp);
    }
    // the two candidates are full[..k] and full[..k] + 1; take the even one if it also round-trips
    let low = &full[..k];
    let mut cand = low.as_bytes().to_vec();
    let low_last = cand[k - 1] - b'0';
    if low_last % 2 == 1 {
        // low is odd: the even one is low + 1 (no carry: an odd digit is at most 9 → 10 needs a carry)
        if cand[k - 1] == b'9' {
            return (digits, exp);
        }
        cand[k - 1] += 1;
    }
    let cand = String::from_utf8(cand).unwrap_or_default();
    let text = format!("{}.{}e{}", &cand[..1], &cand[1..], exp);
    if text.parse::<f64>().ok() == Some(x) {
        return (cand.trim_end_matches('0').to_string(), exp);
    }
    (digits, exp)
}

/// A JS number as a JSON value: integers stay integers, NaN / ±Infinity become `null`
/// (what `JSON.stringify` writes for them).
pub fn num(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    if x.fract() == 0.0 && x.abs() < 9.0e18 {
        return Value::from(x as i64);
    }
    Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
}

/// Value of a JSON number as a JS double.
pub fn number_f64(n: &Number) -> f64 {
    n.as_f64().unwrap_or(f64::NAN)
}

/// `Math.round`: halves go up (towards +∞).
pub fn round(x: f64) -> f64 {
    if !x.is_finite() || x.fract() == 0.0 {
        return x;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    if r == 0.0 && x < 0.0 {
        -0.0
    } else {
        r
    }
}

/// `Math.max(a, b)` (NaN wins, unlike `f64::max`).
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        // max(-0, +0) = +0
        return if a.is_sign_negative() { b } else { a };
    }
    if a > b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)` (NaN wins).
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == b {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b {
        a
    } else {
        b
    }
}

/// `Math.max(...xs)` (−Infinity for an empty list).
pub fn max_of(xs: impl IntoIterator<Item = f64>) -> f64 {
    xs.into_iter().fold(f64::NEG_INFINITY, max)
}

/// `Math.min(...xs)` (+Infinity for an empty list).
pub fn min_of(xs: impl IntoIterator<Item = f64>) -> f64 {
    xs.into_iter().fold(f64::INFINITY, min)
}

/// `Math.round(n * 1000) / 1000`.
pub fn r3(n: f64) -> f64 {
    round(n * 1000.0) / 1000.0
}

// ---------------------------------------------------------------- string → number

/// White space as JS `trim()` / `\s` see it (WhiteSpace + LineTerminator).
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// The regex class for JS `\s`.
pub const WS: &str = r"\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";

/// `String.prototype.trim()`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `Number(string)`.
pub fn str_to_number(s: &str) -> f64 {
    let t = trim(s);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            let mut v = 0f64;
            for c in t[2..].chars() {
                match c.to_digit(radix) {
                    Some(d) => v = v * radix as f64 + d as f64,
                    None => return f64::NAN,
                }
            }
            return v;
        }
    }
    if decimal_literal_len(t) == t.len() {
        t.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// Length of the longest prefix that is a StrDecimalLiteral without "Infinity" ([+-]? digits [. digits] [e[+-]digits]).
fn decimal_literal_len(t: &str) -> usize {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        frac_digits = j - i - 1;
        if int_digits > 0 || frac_digits > 0 {
            i = j;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return 0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    i
}

/// `Number.parseFloat(string)`.
pub fn parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_space);
    for (lit, v) in [("Infinity", f64::INFINITY), ("+Infinity", f64::INFINITY), ("-Infinity", f64::NEG_INFINITY)] {
        if t.starts_with(lit) {
            return v;
        }
    }
    let n = decimal_literal_len(t);
    if n == 0 {
        return f64::NAN;
    }
    let lit = &t[..n];
    // "5." is valid for Rust too; "-.5" as well.
    lit.parse::<f64>().unwrap_or(f64::NAN)
}

/// `parseInt(string, radix)` for radix 2–36.
pub fn parse_int(s: &str, radix: u32) -> f64 {
    let mut t = s.trim_start_matches(is_js_space);
    let mut sign = 1.0;
    if let Some(r) = t.strip_prefix('-') {
        sign = -1.0;
        t = r;
    } else if let Some(r) = t.strip_prefix('+') {
        t = r;
    }
    if radix == 16 {
        if let Some(r) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            t = r;
        }
    }
    let mut v = 0f64;
    let mut any = false;
    for c in t.chars() {
        match c.to_digit(radix) {
            Some(d) => {
                v = v * radix as f64 + d as f64;
                any = true;
            }
            None => break,
        }
    }
    if any {
        sign * v
    } else {
        f64::NAN
    }
}

// ---------------------------------------------------------------- conversions

/// `Number(v)` (None = undefined).
pub fn to_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Some(Value::Number(n)) => number_f64(n),
        Some(Value::String(s)) => str_to_number(s),
        Some(v @ Value::Array(_)) => str_to_number(&to_string(Some(v))),
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `Number(v)` for a value that is present.
pub fn number(v: &Value) -> f64 {
    to_number(Some(v))
}

/// `String(v)` / template-literal conversion (None = undefined).
pub fn to_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => num_str(number_f64(n)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => join(a, ","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `String(v)` for a value that is present.
pub fn string(v: &Value) -> String {
    to_string(Some(v))
}

/// `array.join(sep)` (null / undefined items become "").
pub fn join(a: &[Value], sep: &str) -> String {
    a.iter()
        .map(|x| match x {
            Value::Null => String::new(),
            other => string(other),
        })
        .collect::<Vec<_>>()
        .join(sep)
}

/// JS truthiness (None = undefined).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => {
            let x = number_f64(n);
            x != 0.0 && !x.is_nan()
        }
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `a ?? b` on optional JSON values: `None` and `null` are nullish.
pub fn nullish(v: Option<&Value>) -> Option<&Value> {
    match v {
        None | Some(Value::Null) => None,
        some => some,
    }
}

/// `obj.key` (None when `obj` is not an object or has no such key).
pub fn get<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v.and_then(|v| v.as_object()).and_then(|o| o.get(key))
}

/// `obj.key ?? undefined`-style access that also treats null as missing.
pub fn get_nn<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    nullish(get(v, key))
}

/// `Array.isArray(v) ? v : null`.
pub fn arr(v: Option<&Value>) -> Option<&Vec<Value>> {
    v.and_then(|v| v.as_array())
}

/// Strict equality (`===`) between two JSON values: primitives by value, objects/arrays never
/// equal unless both are the same JS object (which two JSON values can't express) — callers only
/// use this on primitives.
pub fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => number_f64(x) == number_f64(y),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// `typeof v === "number"`.
pub fn is_number(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Number(_)))
}

/// Length as JS counts it (UTF-16 code units).
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `JSON.stringify(s)` for a string.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Is `k` an array index ("0", "1", … no leading zeros, < 2^32 − 1)? JS lists those keys first.
fn is_index_key(k: &str) -> Option<u32> {
    if k.is_empty() || k.len() > 10 || !k.bytes().all(|b| b.is_ascii_digit()) || (k.len() > 1 && k.starts_with('0')) {
        return None;
    }
    let n: u64 = k.parse().ok()?;
    if n < u32::MAX as u64 {
        Some(n as u32)
    } else {
        None
    }
}

/// Keys of a JS object in the order JS enumerates them: integer-like keys ascending, then the
/// others in insertion order.
pub fn keys(o: &Obj) -> Vec<&String> {
    let mut idx: Vec<(u32, &String)> = o.keys().filter_map(|k| is_index_key(k).map(|n| (n, k))).collect();
    if idx.is_empty() {
        return o.keys().collect();
    }
    idx.sort_by_key(|(n, _)| *n);
    let mut out: Vec<&String> = idx.into_iter().map(|(_, k)| k).collect();
    out.extend(o.keys().filter(|k| is_index_key(k).is_none()));
    out
}

/// `Object.entries(o)`.
pub fn entries(o: &Obj) -> Vec<(&String, &Value)> {
    keys(o).into_iter().map(|k| (k, &o[k.as_str()])).collect()
}

fn write_number(n: &Number, out: &mut String) {
    if let Some(i) = n.as_i64() {
        if i.unsigned_abs() <= 1 << 53 {
            out.push_str(&i.to_string());
            return;
        }
    }
    out.push_str(&num_str(number_f64(n)));
}

/// `JSON.stringify(v)`.
pub fn stringify(v: &Value) -> String {
    let mut out = String::new();
    write_compact(v, &mut out);
    out
}

fn write_compact(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(n, out),
        Value::String(s) => out.push_str(&quote(s)),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_compact(x, out);
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push('{');
            for (i, (k, x)) in entries(o).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&quote(k));
                out.push(':');
                write_compact(x, out);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(v, null, indent)`.
pub fn stringify_pretty(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_pretty(v, &mut out, indent, 0);
    out
}

fn write_pretty(v: &Value, out: &mut String, indent: usize, depth: usize) {
    let pad = |out: &mut String, d: usize| {
        out.push('\n');
        out.push_str(&" ".repeat(indent * d));
    };
    match v {
        Value::Array(a) if !a.is_empty() => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                write_pretty(x, out, indent, depth + 1);
            }
            pad(out, depth);
            out.push(']');
        }
        Value::Object(o) if !o.is_empty() => {
            out.push('{');
            for (i, (k, x)) in entries(o).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                out.push_str(&quote(k));
                out.push_str(": ");
                write_pretty(x, out, indent, depth + 1);
            }
            pad(out, depth);
            out.push('}');
        }
        other => write_compact(other, out),
    }
}

/// `JSON.parse(text)` (numbers parsed exactly; the error text differs from V8's).
pub fn parse(text: &str) -> anyhow::Result<Value> {
    Ok(serde_json::from_str(text)?)
}

/// Sort comparator from a JS-style numeric result (NaN counts as equal, like V8).
pub fn cmp_num(d: f64) -> Ordering {
    if d > 0.0 {
        Ordering::Greater
    } else if d < 0.0 {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

// ---------------------------------------------------------------- localeCompare

/// The locale `String.prototype.localeCompare` uses without arguments: the OS user locale, which
/// is what Node's ICU picks (on Windows: the user's regional format, e.g. "th-TH").
pub fn default_locale() -> String {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
        }
        let mut buf = [0u16; 85];
        let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if n > 1 {
            return String::from_utf16_lossy(&buf[..(n - 1) as usize]);
        }
        "en-US".into()
    }
    #[cfg(not(windows))]
    {
        for var in ["LC_ALL", "LC_COLLATE", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                let v = v.split('.').next().unwrap_or("").split('@').next().unwrap_or("").replace('_', "-");
                if !v.is_empty() && v != "C" && v != "POSIX" {
                    return v;
                }
            }
        }
        "en-US".into()
    }
}

type Collator = icu_collator::CollatorBorrowed<'static>;

fn collator() -> &'static Collator {
    static C: OnceLock<Collator> = OnceLock::new();
    C.get_or_init(|| {
        let loc: icu_locale_core::Locale = default_locale().parse().unwrap_or_else(|_| "en-US".parse().unwrap());
        let prefs = icu_collator::CollatorPreferences::from(&loc);
        icu_collator::CollatorBorrowed::try_new(prefs, Default::default())
            .or_else(|_| icu_collator::CollatorBorrowed::try_new(Default::default(), Default::default()))
            .expect("root collator")
    })
}

/// `a.localeCompare(b)` in the default locale.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    collator().compare(a, b)
}

// ---------------------------------------------------------------- misc

/// `new Date().toISOString()`.
pub fn iso_now() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    iso_from_millis(d.as_millis() as i64)
}

/// `new Date(ms).toISOString()`.
pub fn iso_from_millis(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// Build a JSON object from (key, value) pairs; `None` values are left out (undefined).
#[macro_export]
macro_rules! obj {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = serde_json::Map::new();
        $( $crate::js::put(&mut m, $k, $v); )*
        serde_json::Value::Object(m)
    }};
}

/// Something that may become a property value (`None` = undefined = no key).
pub trait IntoProp {
    fn into_prop(self) -> Option<Value>;
}
impl IntoProp for Value {
    fn into_prop(self) -> Option<Value> {
        Some(self)
    }
}
impl IntoProp for Option<Value> {
    fn into_prop(self) -> Option<Value> {
        self
    }
}
impl IntoProp for &Value {
    fn into_prop(self) -> Option<Value> {
        Some(self.clone())
    }
}
impl IntoProp for Option<&Value> {
    fn into_prop(self) -> Option<Value> {
        self.cloned()
    }
}
impl IntoProp for f64 {
    fn into_prop(self) -> Option<Value> {
        Some(num(self))
    }
}
impl IntoProp for i64 {
    fn into_prop(self) -> Option<Value> {
        Some(Value::from(self))
    }
}
impl IntoProp for usize {
    fn into_prop(self) -> Option<Value> {
        Some(Value::from(self as i64))
    }
}
impl IntoProp for bool {
    fn into_prop(self) -> Option<Value> {
        Some(Value::Bool(self))
    }
}
impl IntoProp for &str {
    fn into_prop(self) -> Option<Value> {
        Some(Value::String(self.to_string()))
    }
}
impl IntoProp for String {
    fn into_prop(self) -> Option<Value> {
        Some(Value::String(self))
    }
}
impl IntoProp for Vec<Value> {
    fn into_prop(self) -> Option<Value> {
        Some(Value::Array(self))
    }
}

/// Set `m[k] = v` unless `v` is undefined.
pub fn put(m: &mut Obj, k: &str, v: impl IntoProp) {
    if let Some(v) = v.into_prop() {
        m.insert(k.to_string(), v);
    }
}

/// A list of numbers as a JSON array.
pub fn nums(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|x| num(*x)).collect())
}

impl IntoProp for Option<String> {
    fn into_prop(self) -> Option<Value> {
        self.map(Value::String)
    }
}
impl IntoProp for Option<f64> {
    fn into_prop(self) -> Option<Value> {
        self.map(num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_strings() {
        for (x, s) in [
            (3.0, "3"),
            (0.1, "0.1"),
            (-0.0, "0"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.2e-6, "0.0000012"),
            (1.2e-7, "1.2e-7"),
            (123.456, "123.456"),
            (std::f64::consts::FRAC_PI_2, "1.5707963267948966"),
            (f64::NAN, "NaN"),
        ] {
            assert_eq!(num_str(x), s);
        }
    }

    #[test]
    fn conversions() {
        assert_eq!(str_to_number(" 12 "), 12.0);
        assert_eq!(str_to_number(""), 0.0);
        assert!(str_to_number("1px").is_nan());
        assert_eq!(str_to_number("0x1f"), 31.0);
        assert_eq!(str_to_number("1."), 1.0);
        assert_eq!(str_to_number(".5"), 0.5);
        assert_eq!(parse_float("12.5px"), 12.5);
        assert_eq!(parse_float("-.5e1x"), -5.0);
        assert!(parse_float("px").is_nan());
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_millis(1_762_400_000_123), "2025-11-06T03:33:20.123Z");
    }

    #[test]
    fn json_text() {
        let v: Value = serde_json::from_str(r#"{"b":1,"2":2,"a":[1.5,"x\n"],"1":{}}"#).unwrap();
        assert_eq!(stringify(&v), r#"{"1":{},"2":2,"b":1,"a":[1.5,"x\n"]}"#);
        assert_eq!(stringify_pretty(&serde_json::json!({"a": [1, {}], "b": []}), 2), "{\n  \"a\": [\n    1,\n    {}\n  ],\n  \"b\": []\n}");
    }
}
