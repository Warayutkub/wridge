//! Tolerant SQL DDL reader (Postgres / Supabase and MySQL dialects), port of `data/sql-ddl.mjs`.
//! Applies statements in order to a data model, so a schema spread over CREATE + later ALTER
//! migrations ends up as its latest state. Anything it does not understand is counted, never fatal.

use super::{arr_mut, str_list, str_of, Obj};
use crate::jsre::JsStr;
use crate::re;
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// An entity object, shared by reference like the JS object (a rename can alias it).
pub type Ent = Rc<RefCell<Obj>>;

/// `{ entities, unparsed }`.
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub entities: IndexMap<String, Ent>,
    pub unparsed: Vec<Value>,
}

impl Model {
    /// `model.entities` as a JSON object (insertion order; JSON output applies JS key order).
    pub fn entities_json(&self) -> Value {
        let mut m = Obj::new();
        for (k, e) in &self.entities {
            m.insert(k.clone(), Value::Object(e.borrow().clone()));
        }
        Value::Object(m)
    }

    /// `delete model.entities[key]` (keeps the order of the others).
    pub fn delete(&mut self, key: &str) {
        self.entities.shift_remove(key);
    }

    /// `model.entities[key]`.
    pub fn get(&self, key: &str) -> Option<Ent> {
        self.entities.get(key).cloned()
    }
}

/// `emptyModel()`.
pub fn empty_model() -> Model {
    Model::default()
}

/// A statement and the 1-based line it starts on.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub sql: JsStr,
    pub line: i64,
}

fn is_space(u: u16) -> bool {
    crate::js::is_js_space(u as u32)
}

/// SQL text → statements, respecting quotes, `$tag$` bodies and comments.
pub fn split_statements(text: &JsStr) -> Vec<Stmt> {
    let mut out = Vec::new();
    let n = text.len();
    let (mut start, mut line, mut start_line, mut seen_code) = (0usize, 1i64, 1i64, false);
    let mut i = 0usize;
    while i < n {
        let c = text.at(i).unwrap();
        if c == b'\n' as u16 {
            line += 1;
            i += 1;
            continue;
        }
        if c == b'-' as u16 && text.is(i + 1, '-') {
            while i < n && !text.is(i, '\n') {
                i += 1;
            }
            // JS: i-- then the loop's i++ → continue at the newline
            continue;
        }
        if c == b'/' as u16 && text.is(i + 1, '*') {
            let stop = match text.index_of("*/", i + 2) {
                Some(e) => e + 2,
                None => n,
            };
            for j in i..stop {
                if text.is(j, '\n') {
                    line += 1;
                }
            }
            i = stop;
            continue;
        }
        if !seen_code && !is_space(c) {
            seen_code = true;
            start_line = line;
            start = i;
        }
        if c == b'\'' as u16 || c == b'"' as u16 || c == b'`' as u16 {
            i += 1;
            while i < n && text.at(i) != Some(c) {
                if text.is(i, '\n') {
                    line += 1;
                }
                if c == b'\'' as u16 && text.is(i, '\\') {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == b'$' as u16 {
            let window = text.slice(i, i + 40);
            if let Some(m) = re!(r"^\$[A-Za-z_]*\$").exec(&window) {
                let tag = m.whole(&window);
                let stop = match text.index_of_units(tag.units(), i + tag.len()) {
                    Some(e) => e + tag.len(),
                    None => n,
                };
                for j in i..stop {
                    if text.is(j, '\n') {
                        line += 1;
                    }
                }
                i = stop;
                continue;
            }
        }
        if c == b';' as u16 {
            if seen_code {
                out.push(Stmt {
                    sql: text.slice(start, i).trim(),
                    line: start_line,
                });
            }
            seen_code = false;
        }
        i += 1;
    }
    if seen_code && !text.slice_from(start).trim().is_empty() {
        out.push(Stmt {
            sql: text.slice_from(start).trim(),
            line: start_line,
        });
    }
    out
}

/// Splits on `sep` where not inside parentheses or quotes (empty middle parts are kept).
pub fn split_top_level(s: &JsStr, sep: char) -> Vec<JsStr> {
    let mut parts = Vec::new();
    let mut depth = 0i64;
    let mut cur: Vec<u16> = Vec::new();
    let mut quote: Option<u16> = None;
    let sep = sep as u16;
    for &ch in s.units() {
        if let Some(q) = quote {
            cur.push(ch);
            if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == b'\'' as u16 || ch == b'"' as u16 || ch == b'`' as u16 {
            quote = Some(ch);
            cur.push(ch);
            continue;
        }
        if ch == b'(' as u16 {
            depth += 1;
        }
        if ch == b')' as u16 {
            depth -= 1;
        }
        if ch == sep && depth == 0 {
            parts.push(JsStr(std::mem::take(&mut cur)).trim());
            continue;
        }
        cur.push(ch);
    }
    let last = JsStr(cur).trim();
    if !last.is_empty() {
        parts.push(last);
    }
    parts
}

/// Strips comments that survived inside a statement.
fn strip_comments(s: &JsStr) -> JsStr {
    let a = re!(r"--[^\n]*", "g").replace_all(s, " ");
    re!(r"\/\*[\s\S]*?\*\/", "g").replace_all(&a, " ")
}

fn unquote(s: &JsStr) -> JsStr {
    re!(r#"^[`"\[]|[`"\]]$"#, "g").replace_all(&s.trim(), "")
}

fn unquote_s(s: &JsStr) -> String {
    unquote(s).to_string()
}

/// `"public.profiles"` / `` `x` `` / `"Users"` → schema, name and key (public and unqualified share one key).
#[derive(Clone, Debug, PartialEq)]
pub struct Qualified {
    pub schema: Option<String>,
    pub name: String,
    pub key: String,
}

/// `qualified(raw)`.
pub fn qualified(raw: &JsStr) -> Qualified {
    let mut parts: Vec<JsStr> = raw.trim().split(".").iter().map(unquote).collect();
    let name = parts.pop().unwrap_or_default().to_string();
    let schema = parts.pop().map(|s| s.to_string());
    let key = match &schema {
        Some(s) if !s.is_empty() && s != "public" => format!("{s}.{name}"),
        _ => name.clone(),
    };
    Qualified { schema, name, key }
}

const IDENT: &str = r#"(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)(?:\s*\.\s*(?:"[^"]+"|`[^`]+`|[\w$]+))?"#;

fn paren_list(s: &JsStr) -> Vec<String> {
    split_top_level(s, ',')
        .iter()
        .map(|c| {
            let a = re!(r"\s+(asc|desc)\b.*$", "i").replace_first(c, "");
            let b = re!(r"\(.*$").replace_first(&a, "");
            unquote_s(&b)
        })
        .collect()
}

/// Matching close paren for the `(` at index `i` (-1 when unbalanced).
fn close_paren(s: &JsStr, i: Option<usize>) -> isize {
    // JS: indexOf returned -1 → the loop starts at -1 (s[-1] is undefined)
    let start = i.unwrap_or(0);
    let mut depth = 0i64;
    let mut quote: Option<u16> = None;
    for j in start..s.len() {
        let ch = s.at(j).unwrap();
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == b'\'' as u16 || ch == b'"' as u16 || ch == b'`' as u16 {
            quote = Some(ch);
            continue;
        }
        if ch == b'(' as u16 {
            depth += 1;
        } else if ch == b')' as u16 {
            depth -= 1;
            if depth == 0 {
                return j as isize;
            }
        }
    }
    -1
}

/// `s.slice(open + 1, close)` where `open` came from indexOf (-1 when missing) and `close` from closeParen.
fn between(s: &JsStr, open: Option<usize>, close: isize) -> JsStr {
    let a = open.map(|o| o as isize + 1).unwrap_or(0);
    s.slice_js(a, close)
}

fn entity(model: &mut Model, key: &str, kind: &str) -> Ent {
    if let Some(e) = model.entities.get(key) {
        return e.clone();
    }
    let o = json!({ "name": key, "kind": kind, "columns": [], "indexes": [], "fks": [], "policies": [], "rls": false, "source": null, "changedIn": [] });
    let Value::Object(o) = o else { unreachable!() };
    let e = Rc::new(RefCell::new(o));
    model.entities.insert(key.to_string(), e.clone());
    e
}

fn find_col<'a>(ent: &'a mut Obj, name: &str) -> Option<&'a mut Obj> {
    arr_mut(ent, "columns")
        .iter_mut()
        .filter_map(Value::as_object_mut)
        .find(|c| str_of(c, "name") == Some(name))
}

fn column(ent: &mut Obj, def: &JsStr) -> Obj {
    let d = def.trim();
    let m = re!(r#"^("[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)\s*([\s\S]*)$"#).exec(&d);
    let name = match &m {
        Some(m) => unquote_s(&m.get(&d, 1).unwrap()),
        None => unquote_s(&re!(r"\s+").split_first(def)),
    };
    let rest = m.as_ref().and_then(|m| m.get(&d, 2)).unwrap_or_default();
    let lower = rest.to_lower();
    let ty0 = re!(r"^([a-z_][\w ]*?(?:\([^)]*\))?(?:\[\])?)(?=\s|$)", "i")
        .exec(&rest)
        .and_then(|m| m.get(&rest, 1))
        .unwrap_or_default()
        .trim();
    let ty = re!(
        r"\s+(not|null|primary|unique|default|references|check|constraint|generated|collate|auto_increment|comment)\b.*$",
        "i"
    )
    .replace_first(&ty0, "");
    let mut col = Obj::new();
    col.insert("name".into(), json!(name));
    col.insert("type".into(), json!(ty.to_string()));
    if re!(r"\bnot\s+null\b").test(&lower) || re!(r"\bprimary\s+key\b").test(&lower) {
        col.insert("notNull".into(), json!(true));
    }
    if re!(r"\bprimary\s+key\b").test(&lower) {
        col.insert("pk".into(), json!(true));
    }
    if re!(r"\bunique\b").test(&lower) {
        col.insert("unique".into(), json!(true));
    }
    if let Some(dm) = re!(r"\bdefault\s+((?:'[^']*'|\([^)]*\)|[^\s,]+)(?:::[\w\[\]]+)?)", "i").exec(&rest) {
        col.insert("default".into(), json!(dm.str(&rest, 1).unwrap()));
    }
    if let Some(r) = re!(&format!(r"\breferences\s+({IDENT})\s*(?:\(([^)]*)\))?"), "i").exec(&rest) {
        let table = qualified(&r.get(&rest, 1).unwrap()).key;
        let column = match r.get(&rest, 2).filter(|x| !x.is_empty()) {
            Some(c) => unquote_s(&c),
            None => "id".into(),
        };
        col.insert("ref".into(), json!({ "table": table, "column": column }));
        arr_mut(ent, "fks").push(json!({ "columns": [name], "table": table, "refColumns": [column] }));
    }
    col
}

fn table_constraint(ent: &mut Obj, item: &JsStr) -> bool {
    let s = re!(r"^constraint\s+\S+\s+", "i").replace_first(item, "");
    if let Some(m) = re!(r"^primary\s+key\s*\(([^)]*)\)", "i").exec(&s) {
        for c in paren_list(&m.get(&s, 1).unwrap()) {
            if let Some(col) = find_col(ent, &c) {
                col.insert("pk".into(), json!(true));
                col.insert("notNull".into(), json!(true));
            }
        }
        return true;
    }
    if let Some(m) = re!(&format!(r"^foreign\s+key\s*\(([^)]*)\)\s*references\s+({IDENT})\s*(?:\(([^)]*)\))?"), "i").exec(&s) {
        let cols = paren_list(&m.get(&s, 1).unwrap());
        let table = qualified(&m.get(&s, 2).unwrap()).key;
        let refs = match m.get(&s, 3).filter(|x| !x.is_empty()) {
            Some(r) => paren_list(&r),
            None => vec!["id".to_string()],
        };
        arr_mut(ent, "fks").push(json!({ "columns": cols, "table": table, "refColumns": refs }));
        return true;
    }
    if let Some(m) = re!(r#"^unique(?:\s+(?:key|index))?\s*(?:[`"\w]+\s*)?\(([^)]*)\)"#, "i").exec(&s) {
        let cols = paren_list(&m.get(&s, 1).unwrap());
        arr_mut(ent, "indexes").push(json!({ "name": null, "columns": cols, "unique": true }));
        if cols.len() == 1 {
            if let Some(col) = find_col(ent, &cols[0]) {
                col.insert("unique".into(), json!(true));
            }
        }
        return true;
    }
    if let Some(m) = re!(r#"^(?:key|index|fulltext\s+(?:key|index))\s+([`"\w]+)?\s*\(([^)]*)\)"#, "i").exec(&s) {
        let name = m.get(&s, 1).filter(|x| !x.is_empty()).map(|n| json!(unquote_s(&n))).unwrap_or(Value::Null);
        arr_mut(ent, "indexes").push(json!({ "name": name, "columns": paren_list(&m.get(&s, 2).unwrap()), "unique": false }));
        return true;
    }
    re!(r"^(check|exclude)\b", "i").test(&s)
}

/// source = the CREATE statement only; later changes are listed separately.
fn touch(ent: &mut Obj, src: &Value) {
    let file = src.get("file").cloned().unwrap_or(Value::Null);
    let last = ent.get("changedIn").and_then(Value::as_array).and_then(|a| a.last()).cloned();
    let source_file = ent.get("source").and_then(|s| s.get("file")).cloned();
    if last.as_ref() != Some(&file) && source_file.as_ref() != Some(&file) {
        arr_mut(ent, "changedIn").push(file);
    }
}

fn truthy_obj(v: Option<&Value>) -> bool {
    crate::js::truthy(v)
}

/// Applies one statement; returns false when it was not understood (and not ignorable).
/// `src` is the `{ file, line }` object recorded as the entity/policy source.
pub fn apply_statement(model: &mut Model, stmt: &JsStr, src: &Value) -> bool {
    let sql = re!(r"\s+", "g").replace_all(&strip_comments(stmt), " ").trim();
    // A ${…} placeholder where a table name belongs: a log message or dynamic DDL — nothing to model.
    if re!(r"^(create|alter|drop)\s+table\s+(if\s+(not\s+)?exists\s+)?\?", "i").test(&sql) {
        return true;
    }

    // CREATE TABLE
    if let Some(m) = re!(
        &format!(r"^create\s+(?:or\s+replace\s+)?(?:(?:global\s+|local\s+)?(?:temporary|temp|unlogged)\s+)?table\s+(?:if\s+not\s+exists\s+)?({IDENT})\s*\("),
        "i"
    )
    .exec(&sql)
    {
        let open = sql.index_of("(", m.len().saturating_sub(1));
        let close = close_paren(&sql, open);
        let key = qualified(&m.get(&sql, 1).unwrap()).key;
        if re!(r"\bif\s+not\s+exists\b", "i").test(&m.whole(&sql)) && model.entities.get(&key).is_some_and(|e| truthy_obj(e.borrow().get("source"))) {
            return true;
        }
        let ent = entity(model, &key, "table");
        let mut e = ent.borrow_mut();
        e.insert("columns".into(), json!([]));
        e.insert("fks".into(), json!([]));
        let kept: Vec<Value> = arr_mut(&mut e, "indexes").iter().filter(|i| truthy_obj(i.get("name"))).cloned().collect();
        e.insert("indexes".into(), Value::Array(kept));
        e.insert("source".into(), src.clone());
        for item in split_top_level(&between(&sql, open, close), ',') {
            if !table_constraint(&mut e, &item) {
                let col = column(&mut e, &item);
                arr_mut(&mut e, "columns").push(Value::Object(col));
            }
        }
        return true;
    }

    // ALTER TABLE
    if let Some(m) = re!(&format!(r"^alter\s+table\s+(?:if\s+exists\s+)?(?:only\s+)?({IDENT})\s+(.*)$"), "i").exec(&sql) {
        let key = qualified(&m.get(&sql, 1).unwrap()).key;
        let ent = entity(model, &key, "table");
        touch(&mut ent.borrow_mut(), src);
        let mut ok = true;
        for action in split_top_level(&m.get(&sql, 2).unwrap(), ',') {
            if re!(r"^enable\s+row\s+level\s+security", "i").test(&action) {
                ent.borrow_mut().insert("rls".into(), json!(true));
            } else if re!(r"^disable\s+row\s+level\s+security", "i").test(&action) {
                ent.borrow_mut().insert("rls".into(), json!(false));
            } else if re!(r"^force\s+row\s+level\s+security", "i").test(&action) {
                ent.borrow_mut().insert("rls".into(), json!(true));
            } else if let Some(a) = re!(r"^add\s+(?:column\s+)?(?:if\s+not\s+exists\s+)?(.*)$", "i").exec(&action).filter(|_| !re!(r"^add\s+(constraint|primary|foreign|unique|index|key|check)\b", "i").test(&action)) {
                let mut e = ent.borrow_mut();
                let col = column(&mut e, &a.get(&action, 1).unwrap());
                let cname = col.get("name").cloned();
                let cols = arr_mut(&mut e, "columns");
                cols.retain(|c| c.get("name") != cname.as_ref());
                cols.push(Value::Object(col));
            } else if let Some(a) = re!(r"^add\s+(.*)$", "i").exec(&action) {
                table_constraint(&mut ent.borrow_mut(), &a.get(&action, 1).unwrap());
            } else if let Some(a) = re!(r#"^drop\s+(?:column\s+)?(?:if\s+exists\s+)?([`"\w]+)"#, "i").exec(&action).filter(|_| !re!(r"^drop\s+(constraint|primary|foreign|index|key|default|not)\b", "i").test(&action)) {
                let name = unquote_s(&a.get(&action, 1).unwrap());
                let mut e = ent.borrow_mut();
                arr_mut(&mut e, "columns").retain(|c| c.get("name").and_then(Value::as_str) != Some(name.as_str()));
                arr_mut(&mut e, "fks").retain(|f| !f.get("columns").and_then(Value::as_array).is_some_and(|cs| cs.iter().any(|c| c.as_str() == Some(name.as_str()))));
            } else if let Some(a) = re!(r#"^rename\s+(?:column\s+)?([`"\w]+)\s+to\s+([`"\w]+)"#, "i").exec(&action).filter(|_| !re!(r"^rename\s+to\b", "i").test(&action)) {
                let from = unquote_s(&a.get(&action, 1).unwrap());
                let mut e = ent.borrow_mut();
                if let Some(col) = find_col(&mut e, &from) {
                    col.insert("name".into(), json!(unquote_s(&a.get(&action, 2).unwrap())));
                }
            } else if let Some(a) = re!(&format!(r"^rename\s+to\s+({IDENT})"), "i").exec(&action) {
                let next = qualified(&a.get(&action, 1).unwrap()).key;
                model.delete(&key);
                ent.borrow_mut().insert("name".into(), json!(next));
                model.entities.insert(next, ent.clone());
            } else if let Some(a) = re!(r#"^(?:alter|modify|change)\s+(?:column\s+)?([`"\w]+)\s+(.*)$"#, "i").exec(&action) {
                let cname = unquote_s(&a.get(&action, 1).unwrap());
                let rest = a.get(&action, 2).unwrap();
                let mut e = ent.borrow_mut();
                if let Some(col) = find_col(&mut e, &cname) {
                    if re!(r"^set\s+not\s+null", "i").test(&rest) {
                        col.insert("notNull".into(), json!(true));
                    } else if re!(r"^drop\s+not\s+null", "i").test(&rest) {
                        col.shift_remove("notNull");
                    } else if re!(r"^(set\s+data\s+)?type\s+", "i").test(&rest) {
                        let t = re!(r"^(set\s+data\s+)?type\s+", "i").replace_first(&rest, "");
                        col.insert("type".into(), json!(re!(r"\s+using\b", "i").split_first(&t).trim().to_string()));
                    } else if re!(r"^set\s+default\s+", "i").test(&rest) {
                        col.insert("default".into(), json!(re!(r"^set\s+default\s+", "i").replace_first(&rest, "").to_string()));
                    } else if re!(r"^drop\s+default", "i").test(&rest) {
                        col.shift_remove("default");
                    }
                }
            } else if re!(r"^(convert\s+to|(default\s+)?(character\s+set|charset|collate)\b|engine\s*=|auto_increment\s*=|comment\s*=|row_format|drop\s+(constraint|default|index|key|foreign|primary)|owner\s+to|set\s|reset\s|replica\s|alter\s+constraint|validate\s+constraint|cluster|inherit|no\s+inherit|attach|detach|enable\s+trigger|disable\s+trigger)", "i").test(&action) {
                // no structural effect we track
            } else {
                ok = false;
            }
        }
        return ok;
    }

    // DROP TABLE / VIEW
    if let Some(m) = re!(r"^drop\s+(table|view|materialized\s+view)\s+(?:if\s+exists\s+)?(.*?)(?:\s+(?:cascade|restrict))?$", "i").exec(&sql) {
        for t in split_top_level(&m.get(&sql, 2).unwrap(), ',') {
            model.delete(&qualified(&t).key);
        }
        return true;
    }

    // CREATE INDEX
    if let Some(m) = re!(
        &format!(r#"^create\s+(unique\s+)?index\s+(?:concurrently\s+)?(?:if\s+not\s+exists\s+)?([\w"`]+)?\s*on\s+(?:only\s+)?({IDENT})\s*(?:using\s+\w+\s*)?\("#),
        "i"
    )
    .exec(&sql)
    {
        let open = sql.index_of("(", m.len().saturating_sub(1));
        let ent = entity(model, &qualified(&m.get(&sql, 3).unwrap()).key, "table");
        let name = m.get(&sql, 2).filter(|x| !x.is_empty()).map(|n| json!(unquote_s(&n))).unwrap_or(Value::Null);
        let cols = paren_list(&between(&sql, open, close_paren(&sql, open)));
        arr_mut(&mut ent.borrow_mut(), "indexes").push(json!({ "name": name, "columns": cols, "unique": m.range(1).is_some() }));
        return true;
    }
    if let Some(m) = re!(r#"^drop\s+index\s+(?:concurrently\s+)?(?:if\s+exists\s+)?([\w."`]+)(?:\s+on\s+([\w."`]+))?"#, "i").exec(&sql) {
        let name = qualified(&m.get(&sql, 1).unwrap()).name;
        for ent in model.entities.values() {
            arr_mut(&mut ent.borrow_mut(), "indexes").retain(|i| i.get("name").and_then(Value::as_str) != Some(name.as_str()));
        }
        return true;
    }

    // RLS policies
    if let Some(m) = re!(&format!(r#"^create\s+policy\s+("[^"]+"|[\w]+)\s+on\s+({IDENT})(.*)$"#), "i").exec(&sql) {
        let ent = entity(model, &qualified(&m.get(&sql, 2).unwrap()).key, "table");
        let rest = m.get(&sql, 3).unwrap();
        let cmd = re!(r"\bfor\s+(all|select|insert|update|delete)\b", "i")
            .exec(&rest)
            .and_then(|x| x.get(&rest, 1))
            .map(|x| x.to_lower().to_string())
            .unwrap_or_else(|| "all".into());
        let roles = re!(r"\bto\s+([\w, ]+?)(?=\s+(?:using|with)\b|$)", "i")
            .exec(&rest)
            .and_then(|x| x.get(&rest, 1))
            .map(|x| json!(x.trim().to_string()))
            .unwrap_or(Value::Null);
        let using_idx = re!(r"\busing\s*\(", "i").exec(&rest).map(|x| x.index);
        let check_idx = re!(r"\bwith\s+check\s*\(", "i").exec(&rest).map(|x| x.index);
        let expr = |idx: Option<usize>| -> Value {
            let Some(idx) = idx else { return Value::Null };
            let open = rest.index_of("(", idx);
            json!(between(&rest, open, close_paren(&rest, open)).trim().to_string())
        };
        let name = unquote_s(&m.get(&sql, 1).unwrap());
        let mut e = ent.borrow_mut();
        let pols = arr_mut(&mut e, "policies");
        pols.retain(|p| p.get("name").and_then(Value::as_str) != Some(name.as_str()));
        pols.push(json!({ "name": name, "cmd": cmd, "roles": roles, "using": expr(using_idx), "check": expr(check_idx), "source": src }));
        return true;
    }
    if let Some(m) = re!(&format!(r#"^drop\s+policy\s+(?:if\s+exists\s+)?("[^"]+"|[\w]+)\s+on\s+({IDENT})"#), "i").exec(&sql) {
        if let Some(ent) = model.get(&qualified(&m.get(&sql, 2).unwrap()).key) {
            let name = unquote_s(&m.get(&sql, 1).unwrap());
            arr_mut(&mut ent.borrow_mut(), "policies").retain(|p| p.get("name").and_then(Value::as_str) != Some(name.as_str()));
        }
        return true;
    }
    if re!(&format!(r#"^alter\s+policy\s+("[^"]+"|[\w]+)\s+on\s+({IDENT})"#), "i").test(&sql) {
        return true;
    }

    // Views and functions: names only (functions are what supabase.rpc() calls)
    if let Some(m) = re!(
        &format!(r"^create\s+(?:or\s+replace\s+)?(?:materialized\s+)?view\s+(?:if\s+not\s+exists\s+)?({IDENT})"),
        "i"
    )
    .exec(&sql)
    {
        let ent = entity(model, &qualified(&m.get(&sql, 1).unwrap()).key, "view");
        let mut reads: Vec<String> = Vec::new();
        for x in re!(&format!(r"\b(?:from|join)\s+({IDENT})"), "gi").all(&sql) {
            let k = qualified(&x.get(&sql, 1).unwrap()).key;
            if !reads.contains(&k) {
                reads.push(k);
            }
        }
        let mut e = ent.borrow_mut();
        e.insert("kind".into(), json!("view"));
        e.insert("source".into(), src.clone());
        e.insert("reads".into(), str_list(&reads));
        return true;
    }
    if let Some(m) = re!(&format!(r"^create\s+(?:or\s+replace\s+)?function\s+({IDENT})\s*\("), "i").exec(&sql) {
        let ent = entity(model, &qualified(&m.get(&sql, 1).unwrap()).key, "function");
        let mut e = ent.borrow_mut();
        e.insert("kind".into(), json!("function"));
        e.insert("source".into(), src.clone());
        let own = e.get("name").cloned();
        let mut reads: Vec<String> = Vec::new();
        for x in re!(&format!(r"\b(?:from|join|update|into)\s+({IDENT})"), "gi").all(stmt) {
            let k = qualified(&x.get(stmt, 1).unwrap()).key;
            if !reads.contains(&k) {
                reads.push(k);
            }
        }
        reads.retain(|k| own.as_ref().and_then(Value::as_str) != Some(k.as_str()));
        e.insert("reads".into(), str_list(&reads));
        return true;
    }
    if let Some(m) = re!(&format!(r"^drop\s+function\s+(?:if\s+exists\s+)?({IDENT})"), "i").exec(&sql) {
        model.delete(&qualified(&m.get(&sql, 1).unwrap()).key);
        return true;
    }

    // Statements with no structural effect on tables/columns.
    if re!(
        r"^(create|drop|alter)\s+(extension|schema|type|domain|trigger|sequence|role|user|publication|subscription|event\s+trigger|collation|aggregate|operator)\b",
        "i"
    )
    .test(&sql)
    {
        return true;
    }
    if re!(r"^(grant|revoke|comment\s+on|insert|update|delete|select|set|begin|commit|rollback|do|notify|analyze|vacuum|refresh|truncate|lock|use|reset|start\s+transaction|savepoint|release|call|perform)\b", "i").test(&sql) {
        return true;
    }
    if re!(r"^alter\s+(default\s+privileges|function|view|database|sequence|index|role)\b", "i").test(&sql) {
        return true;
    }
    false
}

/// Applies a whole SQL text; unparsed statements are recorded with their location.
pub fn apply_sql(model: &mut Model, text: &JsStr, file: &str, line_offset: i64) {
    for Stmt { sql, line } in split_statements(text) {
        let src = json!({ "file": file, "line": line + line_offset });
        if !apply_statement(model, &sql, &src) {
            let flat = re!(r"\s+", "g").replace_all(&sql, " ");
            model
                .unparsed
                .push(json!({ "file": file, "line": line + line_offset, "sql": flat.slice(0, 80).to_string() }));
        }
    }
}

/// `applySql` on a Rust string.
pub fn apply_sql_str(model: &mut Model, text: &str, file: &str, line_offset: i64) {
    apply_sql(model, &JsStr::new(text), file, line_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(files: &[&str]) -> Model {
        let mut m = empty_model();
        for (i, sql) in files.iter().enumerate() {
            apply_sql_str(&mut m, sql, &format!("m{}.sql", i + 1), 0);
        }
        m
    }
    fn cols(m: &Model, key: &str) -> Vec<String> {
        m.entities[key].borrow()["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect()
    }

    // ports of legacy-node/test/sql-ddl.test.mjs

    #[test]
    fn splits_statements_without_breaking_bodies_quotes_or_comments() {
        let parts = split_statements(&JsStr::new(
            "\n    -- a comment; with a semicolon\n    create table a (id int, note text default 'x;y');\n    create function f() returns trigger language plpgsql as $$ begin new.x = 1; return new; end $$;\n    /* block; comment */ select 1;",
        ));
        assert_eq!(parts.len(), 3);
        assert!(re!(r"\$\$ begin new\.x = 1; return new; end \$\$").test(&parts[1].sql));
        assert_eq!(parts[0].line, 3);
    }

    #[test]
    fn create_then_alter_add_drop_rename_and_drop_table() {
        let m = model(&[
            "create table public.profiles (id uuid primary key references auth.users(id), username citext unique not null, resume_url text);",
            "alter table public.profiles add column plan text not null default 'free', drop column if exists resume_url; alter table profiles rename column username to handle;",
            "create table projects (id uuid primary key); drop table if exists public.projects;",
        ]);
        assert_eq!(cols(&m, "profiles"), ["id", "handle", "plan"]);
        assert_eq!(m.entities["profiles"].borrow()["columns"][0]["ref"]["table"], "auth.users");
        assert!(m.entities.get("projects").is_none());
        assert!(m.unparsed.is_empty());
    }

    #[test]
    fn create_if_not_exists_on_existing_table_changes_nothing() {
        let m = model(&[
            "create table settings (id int primary key, `key` varchar(100), value text);",
            "CREATE TABLE IF NOT EXISTS settings (id INT)",
        ]);
        assert_eq!(cols(&m, "settings"), ["id", "key", "value"]);
        assert_eq!(m.entities["settings"].borrow()["source"]["file"], "m1.sql");
    }

    #[test]
    fn mysql_backticks_enum_inline_keys_auto_increment_options() {
        let m = model(&["CREATE TABLE IF NOT EXISTS users (\n      id INT AUTO_INCREMENT PRIMARY KEY,\n      `role` ENUM('admin','teacher') NOT NULL DEFAULT 'teacher',\n      phone VARCHAR(20),\n      KEY idx_phone (phone),\n      UNIQUE KEY uq_role_phone (`role`, phone)\n    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"]);
        assert_eq!(cols(&m, "users"), ["id", "role", "phone"]);
        let u = m.entities["users"].borrow();
        assert_eq!(u["columns"][1]["type"], "ENUM('admin','teacher')");
        assert_eq!(
            u["indexes"],
            json!([{ "name": "idx_phone", "columns": ["phone"], "unique": false }, { "name": null, "columns": ["role", "phone"], "unique": true }])
        );
    }

    #[test]
    fn mysql_charset_engine_alters_are_understood() {
        let m = model(&[
            "create table attendance (id int);",
            "ALTER TABLE attendance CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci; ALTER TABLE attendance ENGINE=InnoDB;",
        ]);
        assert!(m.unparsed.is_empty());
    }

    #[test]
    fn placeholder_table_name_is_ignored() {
        let m = model(&["CREATE TABLE ? failed: ?"]);
        assert!(m.unparsed.is_empty());
        assert!(m.entities.is_empty());
    }

    #[test]
    fn unparsed_keep_line_offset() {
        let mut m = empty_model();
        apply_sql_str(&mut m, "frobnicate the table", "lib/x.ts", 47);
        assert_eq!(m.unparsed, vec![json!({ "file": "lib/x.ts", "line": 48, "sql": "frobnicate the table" })]);
    }

    #[test]
    fn rls_policies() {
        let m = model(&["create table orders (id uuid primary key, profile_id uuid);\n    alter table orders enable row level security;\n    create policy orders_read_own on public.orders for select to authenticated using (profile_id = auth.uid());\n    create policy tmp on orders for delete using (false);\n    drop policy if exists tmp on public.orders;"]);
        let o = m.entities["orders"].borrow();
        assert_eq!(o["rls"], true);
        let p: Vec<_> = o["policies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["name"].clone(), p["cmd"].clone(), p["roles"].clone(), p["using"].clone()))
            .collect();
        assert_eq!(
            p,
            vec![(json!("orders_read_own"), json!("select"), json!("authenticated"), json!("profile_id = auth.uid()"))]
        );
    }

    #[test]
    fn indexes_views_functions() {
        let m = model(&["create table orders (id int, status text);\n    create index orders_status_idx on public.orders (status);\n    create or replace view open_orders as select * from orders where status = 'open';\n    create or replace function redeem_code(c text) returns void language sql as $$ update orders set status = 'paid' $$;"]);
        assert_eq!(m.entities["orders"].borrow()["indexes"][0]["columns"], json!(["status"]));
        assert_eq!(m.entities["open_orders"].borrow()["kind"], "view");
        assert_eq!(m.entities["open_orders"].borrow()["reads"], json!(["orders"]));
        assert_eq!(m.entities["redeem_code"].borrow()["kind"], "function");
    }
}
