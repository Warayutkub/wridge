//! The project's data model and the code that touches data (port of `codegraph/data/*`).
//!
//! - [`sql_ddl`]: tolerant SQL DDL reader → entities (tables, views, functions, policies)
//! - [`prisma`]: `schema.prisma` reader and `prisma.<model>.<op>()` scanner
//! - [`sql_usage`]: SQL string literals in TS/Dart code, sqflite helpers, DML → tables/columns
//! - [`usage`]: supabase-js scanner, access styles, `scan_data` / `scan_dart_data`
//! - [`model`]: which files are schema sources, and the whole-project data model (meta `data_model`)
//!
//! Entities are JSON objects shared by reference (`Rc<RefCell<…>>`), like the JS objects they port:
//! key order, `delete` and aliasing behave the same, so `JSON.stringify` output is identical.

pub mod model;
pub mod prisma;
pub mod sql_ddl;
pub mod sql_usage;
pub mod usage;

use serde_json::{Map, Value};

/// A JSON object (insertion-ordered, like a JS object).
pub type Obj = Map<String, Value>;

/// One place where code reads/writes data (a row of `data_refs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataRef {
    /// Access style: `supabase` | `prisma` | `sql` | `sqflite`.
    pub form: String,
    pub entity: String,
    /// `read` | `write` | `call` | `ddl`.
    pub op: String,
    pub detail: Option<String>,
    /// `None` = unknown (the JS `undefined`/`null`); `Some(vec![])` is stored as `[]`.
    pub columns: Option<Vec<String>>,
    pub filters: Option<Vec<String>>,
    pub via: Option<String>,
    pub line: i64,
}

impl DataRef {
    /// As a JSON object; unknown (`None`) fields are left out, as `JSON.stringify` drops `undefined`.
    pub fn to_json(&self) -> Value {
        let mut o = Obj::new();
        o.insert("form".into(), Value::String(self.form.clone()));
        o.insert("entity".into(), Value::String(self.entity.clone()));
        o.insert("op".into(), Value::String(self.op.clone()));
        if let Some(x) = &self.detail {
            o.insert("detail".into(), Value::String(x.clone()));
        }
        if let Some(x) = &self.columns {
            o.insert("columns".into(), str_list(x));
        }
        if let Some(x) = &self.filters {
            o.insert("filters".into(), str_list(x));
        }
        if let Some(x) = &self.via {
            o.insert("via".into(), Value::String(x.clone()));
        }
        o.insert("line".into(), Value::from(self.line));
        Value::Object(o)
    }

    pub(crate) fn new(entity: impl Into<String>, op: &str, detail: Option<&str>, line: i64) -> Self {
        DataRef {
            entity: entity.into(),
            op: op.into(),
            detail: detail.map(String::from),
            line,
            ..Default::default()
        }
    }
}

/// `obj[key]` as a mutable array (created empty when missing or not an array).
pub(crate) fn arr_mut<'a>(o: &'a mut Obj, key: &str) -> &'a mut Vec<Value> {
    if !matches!(o.get(key), Some(Value::Array(_))) {
        o.insert(key.into(), Value::Array(Vec::new()));
    }
    match o.get_mut(key) {
        Some(Value::Array(a)) => a,
        _ => unreachable!(),
    }
}

/// `obj[key]` as a string.
pub(crate) fn str_of<'a>(o: &'a Obj, key: &str) -> Option<&'a str> {
    o.get(key).and_then(Value::as_str)
}

/// A string list as JSON.
pub(crate) fn str_list(v: &[String]) -> Value {
    Value::Array(v.iter().map(|s| Value::String(s.clone())).collect())
}

/// `String(v)` / template-literal conversion of a JSON value (`undefined` for `None`).
pub fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.as_i64().map(|i| i.to_string()).unwrap_or_else(|| crate::js::js_number(n.as_f64().unwrap_or(f64::NAN))),
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { js_string(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}
