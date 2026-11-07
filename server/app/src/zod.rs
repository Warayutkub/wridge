//! The subset of zod 4 the tool registry uses, ported so the Rust server gives the same answers:
//! - `to_json_schema`: what `z.toJSONSchema(z.object(shape), { io: "input" })` prints (same keys, same order);
//! - `safe_parse`: zod's parse (defaults filled in, unknown keys dropped, same issues and messages);
//! - `prettify_error` (`z.prettifyError`) and `mcp_error_message` (the MCP SDK's `getParseErrorMessage`).
//!
//! Regexes run on `regress` (ECMAScript semantics), so `\d`, `\w` and the `i` flag behave as in JS.

use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::jsjson::{as_f64, num, number_to_string, MAX_SAFE_INTEGER};

pub type Shape = IndexMap<String, Schema>;

#[derive(Clone)]
pub struct Schema {
    kind: Kind,
    description: Option<String>,
}

#[derive(Clone)]
enum Kind {
    String(Vec<StrCheck>),
    Number(Vec<NumCheck>),
    Boolean,
    Any,
    Unknown,
    Enum(Vec<String>),
    Literal(Value),
    Array(Box<Schema>, Vec<LenCheck>),
    Object(Shape),
    /// `z.record(z.string(), v)` (keys None) or `z.partialRecord(z.enum(keys), v)`.
    Record(Option<Vec<String>>, Box<Schema>),
    Union(Vec<Schema>),
    Optional(Box<Schema>),
    Default(Box<Schema>, Value),
}

#[derive(Clone)]
enum StrCheck {
    Min(usize),
    Max(usize),
    Regex(Arc<JsRegex>),
    Url,
}

#[derive(Clone)]
enum NumCheck {
    Int,
    Gt(f64),
    Gte(f64),
    Lt(f64),
    Lte(f64),
}

#[derive(Clone)]
enum LenCheck {
    Min(usize),
    Max(usize),
    Exact(usize),
}

pub struct JsRegex {
    pub source: String,
    pub flags: String,
    message: Option<String>,
    re: regress::Regex,
}

impl JsRegex {
    pub fn new(source: &str, flags: &str) -> JsRegex {
        let re = regress::Regex::with_flags(source, flags).unwrap_or_else(|e| panic!("bad regex /{source}/{flags}: {e}"));
        JsRegex {
            source: source.into(),
            flags: flags.into(),
            message: None,
            re,
        }
    }
    pub fn test(&self, s: &str) -> bool {
        self.re.find(s).is_some()
    }
    /// `regex.toString()`.
    pub fn display(&self) -> String {
        format!("/{}/{}", self.source, self.flags)
    }
}

// ---------- constructors (z.*) ----------

fn schema(kind: Kind) -> Schema {
    Schema { kind, description: None }
}
pub fn string() -> Schema {
    schema(Kind::String(vec![]))
}
pub fn number() -> Schema {
    schema(Kind::Number(vec![]))
}
/// `z.number().int()`
pub fn int() -> Schema {
    schema(Kind::Number(vec![NumCheck::Int]))
}
pub fn boolean() -> Schema {
    schema(Kind::Boolean)
}
pub fn any() -> Schema {
    schema(Kind::Any)
}
pub fn unknown() -> Schema {
    schema(Kind::Unknown)
}
pub fn enumeration(values: &[&str]) -> Schema {
    schema(Kind::Enum(values.iter().map(|s| s.to_string()).collect()))
}
pub fn literal(value: Value) -> Schema {
    schema(Kind::Literal(value))
}
pub fn array(item: Schema) -> Schema {
    schema(Kind::Array(Box::new(item), vec![]))
}
pub fn object(shape: Shape) -> Schema {
    schema(Kind::Object(shape))
}
/// `z.record(z.string(), value)`
pub fn record(value: Schema) -> Schema {
    schema(Kind::Record(None, Box::new(value)))
}
/// `z.partialRecord(z.enum(keys), value)`
pub fn partial_record(keys: &[&str], value: Schema) -> Schema {
    schema(Kind::Record(Some(keys.iter().map(|s| s.to_string()).collect()), Box::new(value)))
}
pub fn union(options: Vec<Schema>) -> Schema {
    schema(Kind::Union(options))
}

/// Builds an object shape the way a JS object literal does: a later key (or spread) replaces the value
/// but keeps the key where it first appeared.
#[derive(Default, Clone)]
pub struct ShapeBuilder(Shape);

pub fn shape() -> ShapeBuilder {
    ShapeBuilder::default()
}

impl ShapeBuilder {
    pub fn f(mut self, key: &str, s: Schema) -> Self {
        self.0.insert(key.to_string(), s);
        self
    }
    pub fn spread(mut self, other: &Shape) -> Self {
        for (k, v) in other {
            self.0.insert(k.clone(), v.clone());
        }
        self
    }
    pub fn build(self) -> Shape {
        self.0
    }
}

// ---------- modifiers ----------

impl Schema {
    pub fn describe(mut self, d: &str) -> Schema {
        self.description = Some(d.to_string());
        self
    }
    pub fn optional(self) -> Schema {
        schema(Kind::Optional(Box::new(self)))
    }
    pub fn default(self, v: Value) -> Schema {
        schema(Kind::Default(Box::new(self), v))
    }
    /// `.min(n)`: >= for numbers, length for strings and arrays.
    pub fn min(mut self, n: f64) -> Schema {
        match &mut self.kind {
            Kind::Number(c) => c.push(NumCheck::Gte(n)),
            Kind::String(c) => c.push(StrCheck::Min(n as usize)),
            Kind::Array(_, c) => c.push(LenCheck::Min(n as usize)),
            _ => panic!("min() on a schema without length or size"),
        }
        self
    }
    pub fn max(mut self, n: f64) -> Schema {
        match &mut self.kind {
            Kind::Number(c) => c.push(NumCheck::Lte(n)),
            Kind::String(c) => c.push(StrCheck::Max(n as usize)),
            Kind::Array(_, c) => c.push(LenCheck::Max(n as usize)),
            _ => panic!("max() on a schema without length or size"),
        }
        self
    }
    pub fn length(mut self, n: usize) -> Schema {
        match &mut self.kind {
            Kind::Array(_, c) => c.push(LenCheck::Exact(n)),
            _ => panic!("length() on a non-array"),
        }
        self
    }
    pub fn positive(mut self) -> Schema {
        match &mut self.kind {
            Kind::Number(c) => c.push(NumCheck::Gt(0.0)),
            _ => panic!("positive() on a non-number"),
        }
        self
    }
    pub fn lt(mut self, n: f64) -> Schema {
        if let Kind::Number(c) = &mut self.kind {
            c.push(NumCheck::Lt(n));
        }
        self
    }
    pub fn regex(self, source: &str, flags: &str) -> Schema {
        self.regex_with(JsRegex::new(source, flags))
    }
    /// `.regex(re, "custom message")`
    pub fn regex_msg(self, source: &str, flags: &str, message: &str) -> Schema {
        let mut re = JsRegex::new(source, flags);
        re.message = Some(message.to_string());
        self.regex_with(re)
    }
    fn regex_with(mut self, re: JsRegex) -> Schema {
        match &mut self.kind {
            Kind::String(c) => c.push(StrCheck::Regex(Arc::new(re))),
            _ => panic!("regex() on a non-string"),
        }
        self
    }
    pub fn url(mut self) -> Schema {
        match &mut self.kind {
            Kind::String(c) => c.push(StrCheck::Url),
            _ => panic!("url() on a non-string"),
        }
        self
    }

    /// `_zod.optin`
    fn optin(&self) -> Option<&'static str> {
        match &self.kind {
            Kind::Optional(inner) => Some(if inner.optin() == Some("defaulted") { "defaulted" } else { "optional" }),
            Kind::Default(..) => Some("defaulted"),
            Kind::Union(opts) => {
                if opts.iter().any(|o| o.optin() == Some("defaulted")) {
                    Some("defaulted")
                } else if opts.iter().any(|o| o.optin().is_some()) {
                    Some("optional")
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    /// `_zod.optout`
    fn optout(&self) -> Option<&'static str> {
        match &self.kind {
            Kind::Optional(_) => Some("optional"),
            Kind::Union(opts) if opts.iter().any(|o| o.optout().is_some()) => Some("optional"),
            _ => None,
        }
    }
}

// ---------- JSON Schema ----------

pub const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
pub const DRAFT_07: &str = "http://json-schema.org/draft-07/schema#";

/// `z.toJSONSchema(z.object(shape), { io: "input" })` with the given `$schema`.
pub fn to_json_schema(shape: &Shape, dialect: &str) -> Value {
    let mut root = Map::new();
    root.insert("$schema".into(), Value::String(dialect.into()));
    for (k, v) in object_json(shape) {
        root.insert(k, v);
    }
    Value::Object(root)
}

fn object_json(shape: &Shape) -> Map<String, Value> {
    let mut json = Map::new();
    json.insert("type".into(), "object".into());
    let mut props = Map::new();
    for (k, s) in shape {
        props.insert(k.clone(), Value::Object(s.json()));
    }
    json.insert("properties".into(), Value::Object(props));
    let required: Vec<Value> = shape.iter().filter(|(_, s)| s.optin().is_none()).map(|(k, _)| Value::String(k.clone())).collect();
    if !required.is_empty() {
        json.insert("required".into(), Value::Array(required));
    }
    json
}

fn narrow_min(slot: &mut Option<f64>, v: f64) {
    if slot.is_none_or(|cur| v > cur) {
        *slot = Some(v);
    }
}
fn narrow_max(slot: &mut Option<f64>, v: f64) {
    if slot.is_none_or(|cur| v < cur) {
        *slot = Some(v);
    }
}

impl Schema {
    fn json(&self) -> Map<String, Value> {
        let mut json = match &self.kind {
            // Wrappers: own keys first (default, description), then the inner schema's keys they don't have.
            Kind::Optional(inner) | Kind::Default(inner, _) => {
                let mut own = Map::new();
                if let Kind::Default(_, v) = &self.kind {
                    own.insert("default".into(), v.clone());
                }
                if let Some(d) = &self.description {
                    own.insert("description".into(), Value::String(d.clone()));
                }
                for (k, v) in inner.json() {
                    if !own.contains_key(&k) {
                        own.insert(k, v);
                    }
                }
                return own;
            }
            Kind::String(checks) => {
                let mut json = Map::new();
                json.insert("type".into(), "string".into());
                let (mut min, mut max) = (None, None);
                let mut format = None;
                let mut pattern = None;
                for c in checks {
                    match c {
                        StrCheck::Min(n) => narrow_min(&mut min, *n as f64),
                        StrCheck::Max(n) => narrow_max(&mut max, *n as f64),
                        StrCheck::Url => format = Some("uri"),
                        StrCheck::Regex(re) => pattern = Some(re.source.clone()),
                    }
                }
                if let Some(n) = min {
                    json.insert("minLength".into(), num(n));
                }
                if let Some(n) = max {
                    json.insert("maxLength".into(), num(n));
                }
                if let Some(f) = format {
                    json.insert("format".into(), f.into());
                }
                if let Some(p) = pattern {
                    json.insert("pattern".into(), Value::String(p));
                }
                json
            }
            Kind::Number(checks) => {
                let (mut min, mut max, mut ex_min, mut ex_max, mut is_int) = (None, None, None, None, false);
                for c in checks {
                    match c {
                        NumCheck::Int => {
                            is_int = true;
                            narrow_min(&mut min, -MAX_SAFE_INTEGER);
                            narrow_max(&mut max, MAX_SAFE_INTEGER);
                        }
                        NumCheck::Gte(v) => narrow_min(&mut min, *v),
                        NumCheck::Gt(v) => narrow_min(&mut ex_min, *v),
                        NumCheck::Lte(v) => narrow_max(&mut max, *v),
                        NumCheck::Lt(v) => narrow_max(&mut ex_max, *v),
                    }
                }
                let mut json = Map::new();
                json.insert("type".into(), if is_int { "integer" } else { "number" }.into());
                match ex_min {
                    Some(e) if e >= min.unwrap_or(f64::NEG_INFINITY) => {
                        json.insert("exclusiveMinimum".into(), num(e));
                    }
                    _ => {
                        if let Some(m) = min {
                            json.insert("minimum".into(), num(m));
                        }
                    }
                }
                match ex_max {
                    Some(e) if e <= max.unwrap_or(f64::INFINITY) => {
                        json.insert("exclusiveMaximum".into(), num(e));
                    }
                    _ => {
                        if let Some(m) = max {
                            json.insert("maximum".into(), num(m));
                        }
                    }
                }
                json
            }
            Kind::Boolean => single("type", "boolean".into()),
            Kind::Any | Kind::Unknown => Map::new(),
            Kind::Enum(values) => {
                let mut json = single("type", "string".into());
                json.insert("enum".into(), Value::Array(values.iter().map(|v| Value::String(v.clone())).collect()));
                json
            }
            Kind::Literal(v) => {
                let t = match v {
                    Value::String(_) => "string",
                    Value::Number(_) => "number",
                    Value::Bool(_) => "boolean",
                    Value::Null => "null",
                    _ => "object",
                };
                let mut json = single("type", t.into());
                json.insert("const".into(), v.clone());
                json
            }
            Kind::Array(item, checks) => {
                let (mut min, mut max) = (None, None);
                for c in checks {
                    match c {
                        LenCheck::Min(n) => narrow_min(&mut min, *n as f64),
                        LenCheck::Max(n) => narrow_max(&mut max, *n as f64),
                        LenCheck::Exact(n) => {
                            narrow_min(&mut min, *n as f64);
                            narrow_max(&mut max, *n as f64);
                        }
                    }
                }
                let mut json = Map::new();
                if let Some(n) = min {
                    json.insert("minItems".into(), num(n));
                }
                if let Some(n) = max {
                    json.insert("maxItems".into(), num(n));
                }
                json.insert("type".into(), "array".into());
                json.insert("items".into(), Value::Object(item.json()));
                json
            }
            Kind::Object(shape) => object_json(shape),
            Kind::Record(keys, value) => {
                let mut json = single("type", "object".into());
                let mut names = single("type", "string".into());
                if let Some(keys) = keys {
                    names.insert("enum".into(), Value::Array(keys.iter().map(|k| Value::String(k.clone())).collect()));
                }
                json.insert("propertyNames".into(), Value::Object(names));
                json.insert("additionalProperties".into(), Value::Object(value.json()));
                json
            }
            Kind::Union(options) => {
                let opts: Vec<Map<String, Value>> = options.iter().map(|o| o.json()).collect();
                let mut json = Map::new();
                json.insert("anyOf".into(), Value::Array(opts.into_iter().map(Value::Object).collect()));
                json
            }
        };
        if let Some(d) = &self.description {
            json.insert("description".into(), Value::String(d.clone()));
        }
        compact_type_union(&mut json);
        json
    }
}

fn single(k: &str, v: Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(k.into(), v);
    m
}

/// zod's `compactTypeUnion`: `anyOf` of bare `{type}` branches becomes `type: [...]`.
fn compact_type_union(json: &mut Map<String, Value>) {
    let Some(Value::Array(options)) = json.get("anyOf") else { return };
    if options.is_empty() || json.contains_key("type") {
        return;
    }
    let mut types: Vec<Value> = vec![];
    for o in options {
        let Value::Object(o) = o else { return };
        if o.len() != 1 || !o.contains_key("type") {
            return;
        }
        let members = match &o["type"] {
            Value::Array(a) => a.clone(),
            t => vec![t.clone()],
        };
        for m in members {
            if !m.is_string() {
                return;
            }
            if !types.contains(&m) {
                types.push(m);
            }
        }
    }
    json.shift_remove("anyOf");
    let t = if types.len() == 1 { types.remove(0) } else { Value::Array(types) };
    json.insert("type".into(), t);
}

// ---------- parsing ----------

#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub code: &'static str,
    pub message: String,
    pub path: Vec<Seg>,
    /// zod's `continue` flag: false means the issue aborts (a type error, an int error, a bad enum value).
    pub cont: bool,
    /// `expected` of invalid_type issues (for the MCP SDK's own request-schema errors).
    pub expected: Option<String>,
}

fn issue(code: &'static str, message: String, cont: bool) -> Issue {
    Issue {
        code,
        message,
        path: vec![],
        cont,
        expected: None,
    }
}

/// `util.parsedType(input)` (None = undefined).
pub fn parsed_type(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

pub fn invalid_type(expected: &str, input: Option<&Value>) -> Issue {
    let mut i = issue("invalid_type", format!("Invalid input: expected {expected}, received {}", parsed_type(input)), false);
    i.expected = Some(expected.to_string());
    i
}

fn stringify_primitive(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        other => crate::jsjson::js_string(Some(other)),
    }
}

fn invalid_value(values: &[Value]) -> Issue {
    let message = if values.len() == 1 {
        format!("Invalid input: expected {}", stringify_primitive(&values[0]))
    } else {
        format!("Invalid option: expected one of {}", values.iter().map(stringify_primitive).collect::<Vec<_>>().join("|"))
    };
    issue("invalid_value", message, false)
}

fn sizing(origin: &str) -> Option<&'static str> {
    match origin {
        "string" => Some("characters"),
        "array" => Some("items"),
        _ => None,
    }
}

fn too_big(origin: &str, maximum: f64, inclusive: bool, exact: bool) -> Issue {
    let adj = if exact {
        "exactly ".to_string()
    } else if inclusive {
        "<=".into()
    } else {
        "<".into()
    };
    let max = number_to_string(maximum);
    let message = match sizing(origin) {
        Some(unit) => format!("Too big: expected {origin} to have {adj}{max} {unit}"),
        None => format!("Too big: expected {origin} to be {adj}{max}"),
    };
    issue("too_big", message, true)
}

fn too_small(origin: &str, minimum: f64, inclusive: bool, exact: bool) -> Issue {
    let adj = if exact {
        "exactly ".to_string()
    } else if inclusive {
        ">=".into()
    } else {
        ">".into()
    };
    let min = number_to_string(minimum);
    let message = match sizing(origin) {
        Some(unit) => format!("Too small: expected {origin} to have {adj}{min} {unit}"),
        None => format!("Too small: expected {origin} to be {adj}{min}"),
    };
    issue("too_small", message, true)
}

fn prefix(issues: &mut [Issue], seg: Seg) {
    for i in issues {
        i.path.insert(0, seg.clone());
    }
}

fn aborted(issues: &[Issue]) -> bool {
    issues.iter().any(|i| !i.cont)
}

struct Outcome {
    value: Option<Value>,
    issues: Vec<Issue>,
}

fn ok(value: Option<Value>) -> Outcome {
    Outcome { value, issues: vec![] }
}

/// Length checks also run on a value of the wrong type when it has a length (zod's `when: hasLength`).
fn length_issues(len: usize, origin: &str, checks: &[(Option<usize>, Option<usize>, Option<usize>)]) -> Vec<Issue> {
    let mut out = vec![];
    for (min, max, exact) in checks {
        if let Some(m) = min {
            if len < *m {
                out.push(too_small(origin, *m as f64, true, false));
            }
        }
        if let Some(m) = max {
            if len > *m {
                out.push(too_big(origin, *m as f64, true, false));
            }
        }
        if let Some(e) = exact {
            if len != *e {
                out.push(if len > *e {
                    too_big(origin, *e as f64, true, true)
                } else {
                    too_small(origin, *e as f64, true, true)
                });
            }
        }
    }
    out
}

fn str_len_checks(checks: &[StrCheck]) -> Vec<(Option<usize>, Option<usize>, Option<usize>)> {
    checks
        .iter()
        .filter_map(|c| match c {
            StrCheck::Min(n) => Some((Some(*n), None, None)),
            StrCheck::Max(n) => Some((None, Some(*n), None)),
            _ => None,
        })
        .collect()
}

fn arr_len_checks(checks: &[LenCheck]) -> Vec<(Option<usize>, Option<usize>, Option<usize>)> {
    checks
        .iter()
        .map(|c| match c {
            LenCheck::Min(n) => (Some(*n), None, None),
            LenCheck::Max(n) => (None, Some(*n), None),
            LenCheck::Exact(n) => (None, None, Some(*n)),
        })
        .collect()
}

impl Schema {
    fn run(&self, input: Option<&Value>) -> Outcome {
        match &self.kind {
            Kind::Optional(inner) => match input {
                None if inner.optin() != Some("defaulted") => ok(None),
                None => {
                    let r = inner.run(None);
                    ok(if r.issues.is_empty() { r.value } else { None })
                }
                Some(_) => inner.run(input),
            },
            Kind::Default(inner, d) => match input {
                None => ok(Some(d.clone())),
                Some(_) => {
                    let mut r = inner.run(input);
                    if r.value.is_none() {
                        r.value = Some(d.clone());
                    }
                    r
                }
            },
            Kind::Any | Kind::Unknown => ok(input.cloned()),
            Kind::Boolean => match input {
                Some(Value::Bool(_)) => ok(input.cloned()),
                _ => Outcome {
                    value: input.cloned(),
                    issues: vec![invalid_type("boolean", input)],
                },
            },
            Kind::String(checks) => {
                let Some(Value::String(s)) = input else {
                    let mut issues = vec![invalid_type("string", input)];
                    if let Some(Value::Array(a)) = input {
                        issues.extend(length_issues(a.len(), "array", &str_len_checks(checks)));
                    }
                    return Outcome { value: input.cloned(), issues };
                };
                let mut value = s.clone();
                let mut issues = vec![];
                for c in checks {
                    match c {
                        StrCheck::Min(n) => {
                            if s.chars().count() < *n {
                                issues.push(too_small("string", *n as f64, true, false));
                            }
                        }
                        StrCheck::Max(n) => {
                            if s.chars().count() > *n {
                                issues.push(too_big("string", *n as f64, true, false));
                            }
                        }
                        StrCheck::Regex(re) => {
                            if !re.test(&value) {
                                let message = re.message.clone().unwrap_or_else(|| format!("Invalid string: must match pattern {}", re.display()));
                                issues.push(issue("invalid_format", message, true));
                            }
                        }
                        StrCheck::Url => {
                            let trimmed = crate::jsjson::js_trim(&value).to_string();
                            if url::Url::parse(&trimmed).is_ok() {
                                value = trimmed.replace(['\t', '\n', '\r'], "");
                            } else {
                                issues.push(issue("invalid_format", "Invalid URL".into(), true));
                            }
                        }
                    }
                }
                Outcome {
                    value: Some(Value::String(value)),
                    issues,
                }
            }
            Kind::Number(checks) => {
                let Some(Value::Number(n)) = input else {
                    return Outcome {
                        value: input.cloned(),
                        issues: vec![invalid_type("number", input)],
                    };
                };
                let x = as_f64(n);
                let mut issues = vec![];
                for c in checks {
                    match c {
                        NumCheck::Int => {
                            if x.fract() != 0.0 {
                                let mut i = invalid_type("int", input);
                                i.cont = false;
                                issues.push(i);
                                break; // aborts the remaining checks
                            }
                            if x.abs() > MAX_SAFE_INTEGER {
                                issues.push(if x > 0.0 {
                                    too_big("int", MAX_SAFE_INTEGER, true, false)
                                } else {
                                    too_small("int", -MAX_SAFE_INTEGER, true, false)
                                });
                            }
                        }
                        NumCheck::Gte(v) if x < *v => issues.push(too_small("number", *v, true, false)),
                        NumCheck::Gt(v) if x <= *v => issues.push(too_small("number", *v, false, false)),
                        NumCheck::Lte(v) if x > *v => issues.push(too_big("number", *v, true, false)),
                        NumCheck::Lt(v) if x >= *v => issues.push(too_big("number", *v, false, false)),
                        _ => {}
                    }
                }
                Outcome { value: input.cloned(), issues }
            }
            Kind::Enum(values) => match input {
                Some(Value::String(s)) if values.contains(s) => ok(input.cloned()),
                _ => Outcome {
                    value: input.cloned(),
                    issues: vec![invalid_value(&values.iter().map(|v| Value::String(v.clone())).collect::<Vec<_>>())],
                },
            },
            Kind::Literal(v) => match input {
                Some(x) if x == v => ok(input.cloned()),
                _ => Outcome {
                    value: input.cloned(),
                    issues: vec![invalid_value(std::slice::from_ref(v))],
                },
            },
            Kind::Array(item, checks) => {
                let Some(Value::Array(items)) = input else {
                    let mut issues = vec![invalid_type("array", input)];
                    if let Some(Value::String(s)) = input {
                        issues.extend(length_issues(s.chars().count(), "string", &arr_len_checks(checks)));
                    }
                    return Outcome { value: input.cloned(), issues };
                };
                let mut out = Vec::with_capacity(items.len());
                let mut issues = vec![];
                for (i, x) in items.iter().enumerate() {
                    let mut r = item.run(Some(x));
                    prefix(&mut r.issues, Seg::Index(i));
                    issues.extend(r.issues);
                    out.push(r.value.unwrap_or(Value::Null));
                }
                issues.extend(length_issues(items.len(), "array", &arr_len_checks(checks)));
                Outcome {
                    value: Some(Value::Array(out)),
                    issues,
                }
            }
            Kind::Object(shape) => {
                let Some(Value::Object(obj)) = input else {
                    return Outcome {
                        value: input.cloned(),
                        issues: vec![invalid_type("object", input)],
                    };
                };
                parse_object(shape, obj)
            }
            Kind::Record(keys, value) => {
                let Some(Value::Object(obj)) = input else {
                    return Outcome {
                        value: input.cloned(),
                        issues: vec![invalid_type("record", input)],
                    };
                };
                let mut out = Map::new();
                let mut issues = vec![];
                let mut unrecognized = vec![];
                for (k, v) in obj {
                    if let Some(keys) = keys {
                        if !keys.contains(k) {
                            unrecognized.push(k.clone());
                            continue;
                        }
                    }
                    let mut r = value.run(Some(v));
                    prefix(&mut r.issues, Seg::Key(k.clone()));
                    issues.extend(r.issues);
                    if let Some(v) = r.value {
                        out.insert(k.clone(), v);
                    }
                }
                if !unrecognized.is_empty() {
                    let list = unrecognized.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", ");
                    let s = if unrecognized.len() > 1 { "s" } else { "" };
                    issues.push(issue("unrecognized_keys", format!("Unrecognized key{s}: {list}"), true));
                }
                Outcome {
                    value: Some(Value::Object(out)),
                    issues,
                }
            }
            Kind::Union(options) => {
                let mut results = vec![];
                for o in options {
                    let r = o.run(input);
                    if r.issues.is_empty() {
                        return r;
                    }
                    results.push(r);
                }
                let mut nonaborted: Vec<Outcome> = results.into_iter().filter(|r| !aborted(&r.issues)).collect();
                if nonaborted.len() == 1 {
                    return nonaborted.remove(0);
                }
                Outcome {
                    value: input.cloned(),
                    issues: vec![issue("invalid_union", "Invalid input".into(), false)],
                }
            }
        }
    }
}

fn parse_object(shape: &Shape, obj: &Map<String, Value>) -> Outcome {
    let mut out = Map::new();
    let mut issues = vec![];
    for (key, el) in shape {
        let input = obj.get(key);
        let present = input.is_some();
        let (optin, optout) = (el.optin(), el.optout());
        let mut r = el.run(input);
        let optional_out = optout == Some("optional");
        if !present && optional_out && optin == Some("optional") {
            continue;
        }
        if !r.issues.is_empty() {
            if optin.is_some() && optional_out && !present {
                continue;
            }
            prefix(&mut r.issues, Seg::Key(key.clone()));
            issues.extend(r.issues.iter().cloned());
        }
        if !present && optin.is_none() {
            if r.issues.is_empty() {
                let mut i = invalid_type("nonoptional", None);
                i.path = vec![Seg::Key(key.clone())];
                issues.push(i);
            }
            continue;
        }
        match r.value {
            Some(v) => {
                out.insert(key.clone(), v);
            }
            None => {
                // JS keeps the key with an undefined value, which JSON drops anyway.
            }
        }
    }
    Outcome {
        value: Some(Value::Object(out)),
        issues,
    }
}

/// `z.object(shape).safeParse(input)` (input None = undefined).
pub fn safe_parse(shape: &Shape, input: Option<&Value>) -> Result<Value, Vec<Issue>> {
    let r = match input {
        Some(Value::Object(obj)) => parse_object(shape, obj),
        _ => Outcome {
            value: None,
            issues: vec![invalid_type("object", input)],
        },
    };
    if r.issues.is_empty() {
        Ok(r.value.unwrap_or(Value::Null))
    } else {
        Err(r.issues)
    }
}

/// zod's `toDotPath`.
fn to_dot_path(path: &[Seg]) -> String {
    let mut segs = String::new();
    let word = regress::Regex::new(r"[^\w$]").unwrap();
    for seg in path {
        match seg {
            Seg::Index(i) => segs.push_str(&format!("[{i}]")),
            Seg::Key(k) if word.find(k).is_some() => segs.push_str(&format!("[{}]", serde_json::to_string(k).unwrap())),
            Seg::Key(k) => {
                if !segs.is_empty() {
                    segs.push('.');
                }
                segs.push_str(k);
            }
        }
    }
    segs
}

/// `z.prettifyError(error)`.
pub fn prettify_error(issues: &[Issue]) -> String {
    let mut sorted: Vec<&Issue> = issues.iter().collect();
    sorted.sort_by_key(|i| i.path.len()); // stable, like Array.prototype.sort
    let mut lines = vec![];
    for i in sorted {
        lines.push(format!("✖ {}", i.message));
        if !i.path.is_empty() {
            lines.push(format!("  → at {}", to_dot_path(&i.path)));
        }
    }
    lines.join("\n")
}

/// The MCP SDK's `getParseErrorMessage` (zod-compat.js).
pub fn mcp_error_message(issues: &[Issue]) -> String {
    issues
        .iter()
        .map(|i| {
            if i.path.is_empty() {
                i.message.clone()
            } else {
                let mut p = String::new();
                for (n, seg) in i.path.iter().enumerate() {
                    match seg {
                        Seg::Key(k) if n == 0 => p.push_str(k),
                        Seg::Index(x) if n == 0 => p.push_str(&x.to_string()),
                        Seg::Index(x) => p.push_str(&format!("[{x}]")),
                        Seg::Key(k) => {
                            p.push('.');
                            p.push_str(k);
                        }
                    }
                }
                format!("{} at {p}", i.message)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsjson::stringify;
    use serde_json::json;

    fn hex() -> Schema {
        string().regex_msg("^#([0-9a-f]{6}|[0-9a-f]{8})$", "i", "hex color like #FF8866 or #FF886680")
    }

    #[test]
    fn json_schema_keys_in_zod_order() {
        let s = shape()
            .f("name", string().default(json!("Frame")))
            .f("width", number().positive().default(json!(375)))
            .f("parentId", string().optional().describe("Parent frame id; defaults to the current page"))
            .f("fill", hex().optional().describe("Omit for a transparent frame"))
            .f("index", int().min(0.0).optional())
            .f("depth", int().min(0.0).max(5.0).default(json!(1)))
            .f("ids", array(string()).min(1.0).max(500.0))
            .f("props", record(union(vec![string(), boolean()])).optional().describe("Variant/text/boolean props"))
            .f("color", union(vec![hex(), literal(json!("none"))]))
            .build();
        let got = stringify(&to_json_schema(&s, DRAFT_2020_12));
        assert_eq!(
            got,
            r##"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"name":{"default":"Frame","type":"string"},"width":{"default":375,"type":"number","exclusiveMinimum":0},"parentId":{"description":"Parent frame id; defaults to the current page","type":"string"},"fill":{"description":"Omit for a transparent frame","type":"string","pattern":"^#([0-9a-f]{6}|[0-9a-f]{8})$"},"index":{"type":"integer","minimum":0,"maximum":9007199254740991},"depth":{"default":1,"type":"integer","minimum":0,"maximum":5},"ids":{"minItems":1,"maxItems":500,"type":"array","items":{"type":"string"}},"props":{"description":"Variant/text/boolean props","type":"object","propertyNames":{"type":"string"},"additionalProperties":{"type":["string","boolean"]}},"color":{"anyOf":[{"type":"string","pattern":"^#([0-9a-f]{6}|[0-9a-f]{8})$"},{"type":"string","const":"none"}]}},"required":["ids","color"]}"##
        );
    }

    #[test]
    fn parse_fills_defaults_and_reports_like_zod() {
        let s = shape()
            .f("width", number().positive().default(json!(375)))
            .f("fill", hex().optional())
            .f("layoutMode", enumeration(&["NONE", "HORIZONTAL", "VERTICAL"]).default(json!("VERTICAL")))
            .f("x", number().optional())
            .build();
        assert_eq!(
            stringify(&safe_parse(&s, Some(&json!({"x": 1, "extra": 2}))).unwrap()),
            r#"{"width":375,"layoutMode":"VERTICAL","x":1}"#
        );
        let issues = safe_parse(&s, Some(&json!({"width": -1, "fill": "red", "layoutMode": "X"}))).unwrap_err();
        assert_eq!(
            mcp_error_message(&issues),
            "Too small: expected number to be >0 at width\nhex color like #FF8866 or #FF886680 at fill\nInvalid option: expected one of \"NONE\"|\"HORIZONTAL\"|\"VERTICAL\" at layoutMode"
        );
        assert_eq!(
            prettify_error(&safe_parse(&shape().f("nodeId", string()).build(), Some(&json!({}))).unwrap_err()),
            "✖ Invalid input: expected string, received undefined\n  → at nodeId"
        );
        assert_eq!(mcp_error_message(&safe_parse(&s, None).unwrap_err()), "Invalid input: expected object, received undefined");
    }

    #[test]
    fn unions_pick_the_branch_that_got_furthest() {
        let lh = shape()
            .f("lineHeight", union(vec![number().positive(), string().regex("^(auto|\\d+(\\.\\d+)?%)$", "i")]).optional())
            .build();
        let msg = |v: Value| prettify_error(&safe_parse(&lh, Some(&json!({ "lineHeight": v }))).unwrap_err());
        assert_eq!(msg(json!(-5)), "✖ Too small: expected number to be >0\n  → at lineHeight");
        assert_eq!(msg(json!("abc")), "✖ Invalid string: must match pattern /^(auto|\\d+(\\.\\d+)?%)$/i\n  → at lineHeight");
        assert_eq!(msg(json!(true)), "✖ Invalid input\n  → at lineHeight");
        assert!(safe_parse(&lh, Some(&json!({"lineHeight": "150%"}))).is_ok());
    }
}
