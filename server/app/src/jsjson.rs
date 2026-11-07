//! JSON the way JavaScript writes it: `JSON.stringify` (compact and indented), `Number.prototype.toString`,
//! `String(value)` and a few other JS conversions the Node server relies on. serde_json alone would print
//! `3.0` for a float 3 and `1e21` where JS prints `1e+21`.

use serde_json::{Map, Number, Value};

/// Largest integer JS represents exactly (Number.MAX_SAFE_INTEGER).
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// `Number.prototype.toString()` for a finite or non-finite f64.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x == 0.0 {
        return "0".into();
    }
    if x < 0.0 {
        return format!("-{}", number_to_string(-x));
    }
    // Shortest round-trip digits from ryu, then laid out per ECMAScript Number::toString.
    let mut buf = ryu::Buffer::new();
    let s = buf.format_finite(x);
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], s[i + 1..].parse::<i32>().unwrap_or(0)),
        None => (s, 0),
    };
    let (int_part, frac_part) = mant.split_once('.').unwrap_or((mant, ""));
    let mut digits: String = format!("{int_part}{frac_part}");
    let mut point = int_part.len() as i32 + exp;
    while digits.len() > 1 && digits.starts_with('0') {
        digits.remove(0);
        point -= 1;
    }
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let k = digits.len() as i32;
    let n = point;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

/// A JSON number the way JS holds it: integral values become integers (so they print without `.0`).
pub fn num(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() <= MAX_SAFE_INTEGER {
        if x >= 0.0 {
            Value::Number(Number::from(x as u64))
        } else {
            Value::Number(Number::from(x as i64))
        }
    } else {
        Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// The f64 a JS program would see for this JSON number.
pub fn as_f64(n: &Number) -> f64 {
    if let Some(u) = n.as_u64() {
        u as f64
    } else if let Some(i) = n.as_i64() {
        i as f64
    } else {
        n.as_f64().unwrap_or(f64::NAN)
    }
}

fn write_number(out: &mut String, n: &Number) {
    if let Some(u) = n.as_u64() {
        if (u as f64) <= MAX_SAFE_INTEGER {
            out.push_str(&u.to_string());
            return;
        }
    } else if let Some(i) = n.as_i64() {
        if (i as f64).abs() <= MAX_SAFE_INTEGER {
            out.push_str(&i.to_string());
            return;
        }
    }
    let x = as_f64(n);
    if x.is_finite() {
        out.push_str(&number_to_string(x));
    } else {
        out.push_str("null");
    }
}

fn write_string(out: &mut String, s: &str) {
    // serde_json escapes exactly what JSON.stringify escapes for well-formed strings.
    out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()));
}

fn write(out: &mut String, v: &Value, indent: &str, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_string(out, k);
                out.push(':');
                if !indent.is_empty() {
                    out.push(' ');
                }
                write(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

fn newline(out: &mut String, indent: &str, level: usize) {
    if !indent.is_empty() {
        out.push('\n');
        for _ in 0..level {
            out.push_str(indent);
        }
    }
}

/// `JSON.stringify(value)`.
pub fn stringify(v: &Value) -> String {
    let mut out = String::new();
    write(&mut out, v, "", 0);
    out
}

/// `JSON.stringify(value, null, spaces)`.
pub fn stringify_pretty(v: &Value, spaces: usize) -> String {
    let mut out = String::new();
    write(&mut out, v, &" ".repeat(spaces), 0);
    out
}

/// `String(value)` for a JSON value (`None` = undefined).
pub fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => {
            let mut s = String::new();
            write_number(&mut s, n);
            s
        }
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => js_join(items, ","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `array.join(sep)`: null and undefined become "".
pub fn js_join(items: &[Value], sep: &str) -> String {
    items
        .iter()
        .map(|x| match x {
            Value::Null => String::new(),
            other => js_string(Some(other)),
        })
        .collect::<Vec<_>>()
        .join(sep)
}

/// JS truthiness (`Boolean(value)`); `None` = undefined.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => {
            let x = as_f64(n);
            x != 0.0 && !x.is_nan()
        }
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// JS `String.prototype.trim` (WhiteSpace + LineTerminator, which is not quite Rust's `trim`).
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}' | '\u{000A}' | '\u{000B}' | '\u{000C}' | '\u{000D}' | '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `s.slice(0, n)` counted in UTF-16 code units (a split surrogate pair loses its half).
pub fn slice_utf16(s: &str, n: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in s.chars() {
        let len = c.len_utf16();
        if units + len > n {
            break;
        }
        units += len;
        out.push(c);
    }
    out
}

/// Number of UTF-16 code units (JS `length`).
pub fn len_utf16(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `{ ...a, ...b }` on two JSON objects.
pub fn spread(a: &Map<String, Value>, b: &Map<String, Value>) -> Map<String, Value> {
    let mut out = a.clone();
    for (k, v) in b {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// The TypeError JS throws when reading `key` of null / undefined (None = undefined).
pub fn read_error(v: Option<&Value>, key: &str) -> anyhow::Error {
    let what = if v.is_none() { "undefined" } else { "null" };
    anyhow::anyhow!("Cannot read properties of {what} (reading '{key}')")
}

/// `v.key`: throws like JS on null / undefined; undefined (None) for a missing key or a primitive.
pub fn prop<'a>(v: Option<&'a Value>, key: &str) -> anyhow::Result<Option<&'a Value>> {
    match v {
        None | Some(Value::Null) => Err(read_error(v, key)),
        Some(Value::Object(o)) => Ok(o.get(key)),
        Some(_) => Ok(None),
    }
}

/// `v.length`: arrays and strings (UTF-16 units) have one, other values give undefined (None); throws on
/// null / undefined.
pub fn length(v: Option<&Value>) -> anyhow::Result<Option<usize>> {
    match v {
        None | Some(Value::Null) => Err(read_error(v, "length")),
        Some(Value::Array(a)) => Ok(Some(a.len())),
        Some(Value::String(s)) => Ok(Some(len_utf16(s))),
        Some(_) => Ok(None),
    }
}

/// `const [first] = await …`: the first item (None = undefined) of an array or string, else V8's TypeError.
pub fn first_of(v: Value) -> anyhow::Result<Option<Value>> {
    match v {
        Value::Array(a) => Ok(a.into_iter().next()),
        Value::String(s) => Ok(s.chars().next().map(|c| Value::String(c.to_string()))),
        _ => Err(anyhow::anyhow!("(intermediate value) is not iterable")),
    }
}

/// `Buffer.from(v, "base64")`, with Node's check of the argument.
pub fn buffer_from_base64(v: Option<&Value>) -> anyhow::Result<Vec<u8>> {
    let received = match v {
        Some(Value::String(s)) => return Ok(crate::util::base64_lenient(s)),
        // an array is taken as bytes (the encoding is ignored)
        Some(Value::Array(a)) => return Ok(a.iter().map(|x| (x.as_f64().unwrap_or(0.0) as i64 & 255) as u8).collect()),
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(b)) => format!("type boolean ({b})"),
        Some(Value::Number(n)) => format!("type number ({})", number_to_string(as_f64(n))),
        Some(Value::Object(_)) => "an instance of Object".to_string(),
    };
    Err(anyhow::anyhow!(
        "The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received {received}"
    ))
}

/// Approximates V8's `JSON.parse` error messages for the common cases (the Node server returns them as
/// `{ error }` with status 500).
pub fn parse(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => Ok(v),
        Err(_) => Err(crate::v8json::error_message(text).unwrap_or_else(|| "Unexpected end of JSON input".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_print_like_js() {
        for (x, s) in [
            (3.0, "3"),
            (0.3, "0.3"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (-2.5, "-2.5"),
            (123456.789, "123456.789"),
            (0.1 + 0.2, "0.30000000000000004"),
            (9007199254740991.0, "9007199254740991"),
            (-0.0, "0"),
        ] {
            assert_eq!(number_to_string(x), s, "{x}");
        }
    }

    #[test]
    fn stringify_matches_js() {
        let v = json!({"a": 1, "b": [true, null, "x\n\"y\""], "c": {}, "d": [], "e": 2.0f64});
        assert_eq!(stringify(&v), r#"{"a":1,"b":[true,null,"x\n\"y\""],"c":{},"d":[],"e":2}"#);
        assert_eq!(
            stringify_pretty(&json!({"a": [1, {"b": 2}]}), 2),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ]\n}"
        );
    }

    #[test]
    fn string_conversion() {
        assert_eq!(js_string(Some(&json!([1, null, "a"]))), "1,,a");
        assert_eq!(js_string(Some(&json!({"a": 1}))), "[object Object]");
        assert_eq!(js_string(None), "undefined");
        assert!(truthy(Some(&json!([]))));
        assert!(!truthy(Some(&json!(""))));
    }

    #[test]
    fn property_access_throws_like_js() {
        assert_eq!(prop(None, "a").unwrap_err().to_string(), "Cannot read properties of undefined (reading 'a')");
        assert_eq!(prop(Some(&json!(null)), "a").unwrap_err().to_string(), "Cannot read properties of null (reading 'a')");
        assert_eq!(prop(Some(&json!(5)), "a").unwrap(), None);
        assert_eq!(length(Some(&json!("ไทย😀"))).unwrap(), Some(5));
        assert_eq!(length(Some(&json!({"length": 3}))).unwrap(), None);
        assert_eq!(first_of(json!(["a", "b"])).unwrap(), Some(json!("a")));
        assert_eq!(first_of(json!([])).unwrap(), None);
        assert_eq!(first_of(json!({})).unwrap_err().to_string(), "(intermediate value) is not iterable");
    }

    #[test]
    fn buffer_from_base64_checks_its_argument() {
        assert_eq!(buffer_from_base64(Some(&json!("aGk="))).unwrap(), b"hi");
        let msg = |v: Option<&Value>| buffer_from_base64(v).unwrap_err().to_string();
        assert!(msg(None).ends_with("Received undefined"));
        assert!(msg(Some(&json!(5))).ends_with("Received type number (5)"));
        assert!(msg(Some(&json!(true))).ends_with("Received type boolean (true)"));
        assert!(msg(Some(&json!({}))).ends_with("Received an instance of Object"));
    }
}
