//! Raw SQL written in code (mysql2 / pg / sqlite …), port of `data/sql-usage.mjs`: finds SQL
//! string literals, then which tables and columns each statement reads or writes. Also flags
//! files that carry schema DDL (CREATE/ALTER TABLE).

use super::DataRef;
use crate::jsre::{JsRegex, JsStr};
use crate::re;
use indexmap::{IndexMap, IndexSet};

fn sql_start() -> &'static JsRegex {
    re!(r"^\s*(select|insert|update|delete|replace|with|create|alter|drop)\b", "i")
}

// DDL only where a statement starts (string start or after ";").
fn ddl() -> &'static JsRegex {
    re!(r"(^|;)\s*((create|alter)\s+table|create\s+(unique\s+)?index)\b", "i")
}

const REGEX_AFTER: &[&str] = &[
    "", "(", ",", "=", ":", "[", "!", "&", "|", "?", "{", "}", ";", "+", "-", "*", "%", "<", ">", "~", "^", "return", "typeof", "case", "do", "else", "in", "of", "yield", "await",
    "void", "delete", "throw", "new", "=>",
];

/// A string or template literal of JS/TS source.
#[derive(Clone, Debug, PartialEq)]
pub struct Literal {
    pub raw: JsStr,
    pub line: i64,
}

/// A literal that looks like SQL.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlLiteral {
    pub sql: JsStr,
    pub line: i64,
    pub ddl: bool,
    /// Newlines before the statement starts inside the literal.
    pub lead: i64,
}

fn ch(text: &JsStr, i: usize) -> Option<u16> {
    text.at(i)
}

fn is_c(u: Option<u16>, c: char) -> bool {
    u == Some(c as u16)
}

fn ascii_alpha(u: u16) -> bool {
    (u as u8 as u16 == u) && (u as u8).is_ascii_alphabetic()
}

fn word(u: u16) -> bool {
    (u as u8 as u16 == u) && ((u as u8).is_ascii_alphanumeric() || u == b'_' as u16)
}

/// Index just past the template literal starting at `start` (handles `${ … }` with nested strings/templates).
fn skip_template(text: &JsStr, start: usize) -> usize {
    let n = text.len();
    let mut k = start + 1;
    while k < n && !is_c(ch(text, k), '`') {
        if is_c(ch(text, k), '\\') {
            k += 2;
            continue;
        }
        if is_c(ch(text, k), '$') && is_c(ch(text, k + 1), '{') {
            let mut depth = 1;
            k += 2;
            while k < n && depth > 0 {
                let c = ch(text, k);
                if is_c(c, '{') {
                    depth += 1;
                } else if is_c(c, '}') {
                    depth -= 1;
                } else if is_c(c, '`') {
                    k = skip_template(text, k);
                    continue;
                } else if is_c(c, '\'') || is_c(c, '"') {
                    k += 1;
                    while k < n && ch(text, k) != c && !is_c(ch(text, k), '\n') {
                        k += if is_c(ch(text, k), '\\') { 2 } else { 1 };
                    }
                }
                k += 1;
            }
            continue;
        }
        k += 1;
    }
    k + 1
}

/// String and template literals of JS/TS source. A small tokenizer, so quotes inside comments
/// ("don't") and regex literals (``/[`"']/``) never start a string.
pub fn js_string_literals(text: &JsStr) -> Vec<Literal> {
    let mut out = Vec::new();
    let n = text.len();
    let mut i = 0usize;
    let mut line = 1i64;
    let mut prev: JsStr = JsStr::default();
    let count_lines = |from: usize, to: usize, line: &mut i64| {
        for k in from..to {
            if is_c(ch(text, k), '\n') {
                *line += 1;
            }
        }
    };
    while i < n {
        let c = ch(text, i).unwrap();
        if c == b'\n' as u16 {
            line += 1;
            i += 1;
            continue;
        }
        if c == b' ' as u16 || c == b'\t' as u16 || c == b'\r' as u16 {
            i += 1;
            continue;
        }
        if c == b'/' as u16 && is_c(ch(text, i + 1), '/') {
            while i < n && !is_c(ch(text, i), '\n') {
                i += 1;
            }
            continue;
        }
        if c == b'/' as u16 && is_c(ch(text, i + 1), '*') {
            let stop = text.index_of("*/", i + 2).map(|e| e + 2).unwrap_or(n);
            count_lines(i, stop, &mut line);
            i = stop;
            continue;
        }
        if c == b'/' as u16 && REGEX_AFTER.iter().any(|p| prev.eq_str(p)) {
            let mut in_class = false;
            i += 1;
            while i < n && !is_c(ch(text, i), '\n') {
                let t = ch(text, i);
                if is_c(t, '\\') {
                    i += 2;
                    continue;
                }
                if is_c(t, '[') {
                    in_class = true;
                } else if is_c(t, ']') {
                    in_class = false;
                } else if is_c(t, '/') && !in_class {
                    i += 1;
                    break;
                }
                i += 1;
            }
            while i < n && ascii_alpha(ch(text, i).unwrap()) {
                i += 1;
            }
            prev = JsStr::new("regex");
            continue;
        }
        if c == b'\'' as u16 || c == b'"' as u16 || c == b'`' as u16 {
            let start = i;
            let start_line = line;
            if c == b'`' as u16 {
                i = skip_template(text, i);
            } else {
                i += 1;
                while i < n && ch(text, i) != Some(c) && !is_c(ch(text, i), '\n') {
                    if is_c(ch(text, i), '\\') {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            count_lines(start, i.min(n), &mut line);
            out.push(Literal {
                raw: text.slice(start, i),
                line: start_line,
            });
            prev = JsStr::new("literal");
            continue;
        }
        if ascii_alpha(c) || c == b'_' as u16 || c == b'$' as u16 {
            let mut j = i;
            while j < n && (word(ch(text, j).unwrap()) || is_c(ch(text, j), '$')) {
                j += 1;
            }
            prev = text.slice(i, j);
            i = j;
            continue;
        }
        if c == b'=' as u16 && is_c(ch(text, i + 1), '>') {
            prev = JsStr::new("=>");
            i += 2;
            continue;
        }
        if (b'0' as u16..=b'9' as u16).contains(&c) || c == b'.' as u16 {
            while i < n && (word(ch(text, i).unwrap()) || is_c(ch(text, i), '.')) {
                i += 1;
            }
            prev = JsStr::new("number");
            continue;
        }
        prev = JsStr(vec![c]);
        i += 1;
    }
    out
}

fn lead_of(body: &JsStr) -> i64 {
    let ws = re!(r"^\s*").exec(body).map(|m| m.whole(body)).unwrap_or_default();
    ws.split("\n").len() as i64 - 1
}

/// String/template literals of a JS/TS file that look like SQL.
pub fn sql_literals(text: &str) -> Vec<SqlLiteral> {
    sql_literals_js(&JsStr::new(text))
}

pub fn sql_literals_js(text: &JsStr) -> Vec<SqlLiteral> {
    let mut out = Vec::new();
    for Literal { raw, line } in js_string_literals(text) {
        let body = raw.slice_js(1, -1);
        if !sql_start().test(&body) && !ddl().test(&body) {
            continue;
        }
        // ${…} becomes a placeholder; escaped backticks (MySQL identifiers) are unescaped.
        let a = re!(r"\$\{(?:[^{}]|\{[^{}]*\})*\}", "g").replace_all(&body, "?");
        let b = re!(r"\\`", "g").replace_all(&a, "`");
        let sql = re!(r"\\n", "g").replace_all(&b, "\n");
        let is_ddl = ddl().test(&sql);
        out.push(SqlLiteral {
            sql,
            line,
            ddl: is_ddl,
            lead: lead_of(&body),
        });
    }
    out
}

/// Dart string literals (`'…'`, `"…"`, `'''…'''`, `"""…"""`, `r'…'`) that look like SQL; `$x` / `${…}` become `?`.
pub fn dart_sql_literals(text: &str) -> Vec<SqlLiteral> {
    dart_sql_literals_js(&JsStr::new(text))
}

pub fn dart_sql_literals_js(text: &JsStr) -> Vec<SqlLiteral> {
    let mut out = Vec::new();
    let re = re!(r#"r?'''[\s\S]*?'''|r?"""[\s\S]*?"""|r?'(?:\\.|[^'\\\n])*'|r?"(?:\\.|[^"\\\n])*""#, "g");
    let mut line_no = 1i64;
    let mut last = 0usize;
    for m in re.all(text) {
        let raw = re!(r"^r").replace_first(&m.whole(text), "");
        let q: isize = if raw.starts_with("'''") || raw.starts_with("\"\"\"") { 3 } else { 1 };
        let body = raw.slice_js(q, -q);
        if !sql_start().test(&body) && !ddl().test(&body) {
            continue;
        }
        for i in last..m.index {
            if text.is(i, '\n') {
                line_no += 1;
            }
        }
        last = m.index;
        let a = re!(r"\$\{[^}]*\}|\$\w+", "g").replace_all(&body, "?");
        let sql = re!(r"\\'", "g").replace_all(&a, "'");
        let is_ddl = ddl().test(&sql);
        out.push(SqlLiteral {
            sql,
            line: line_no,
            ddl: is_ddl,
            lead: lead_of(&body),
        });
    }
    out
}

/// sqflite helpers: `db.query('t', columns: [...], where: 'a = ?')`, `insert/update/delete('t', …)`.
pub fn scan_sqflite(text: &str) -> Vec<DataRef> {
    let text = JsStr::new(text);
    let mut refs = Vec::new();
    for m in re!(r#"\.(query|insert|update|delete)\(\s*(['"])(\w+)\2"#, "g").all(&text) {
        let mut depth = 1i64;
        let mut end = m.end;
        while end < text.len() && end < m.index + 3000 && depth > 0 {
            if text.is(end, '(') {
                depth += 1;
            } else if text.is(end, ')') {
                depth -= 1;
            }
            end += 1;
        }
        let args = text.slice_js(m.end as isize, end as isize - 1);
        let what = m.str(&text, 1).unwrap();
        let op = if what == "query" { "read" } else { "write" };
        let mut r = DataRef::new(m.str(&text, 3).unwrap(), op, Some(&what), text.line_at(m.index) as i64);
        if let Some(cols) = re!(r"columns\s*:\s*(?:const\s*)?\[([^\]]*)\]").exec(&args) {
            let inner = cols.get(&args, 1).unwrap();
            r.columns = Some(re!(r#"['"](\w+)['"]"#, "g").all(&inner).iter().map(|c| c.str(&inner, 1).unwrap()).collect());
        } else if what == "query" {
            r.columns = Some(vec!["*".into()]);
        }
        if what != "query" {
            if let Some(values) = re!(r"^\s*,\s*(?:const\s*)?\{([\s\S]*?)\}").exec(&args) {
                let inner = values.get(&args, 1).unwrap();
                let keys: Vec<String> = re!(r#"['"](\w+)['"]\s*:"#, "g").all(&inner).iter().map(|k| k.str(&inner, 1).unwrap()).collect();
                if !keys.is_empty() {
                    r.columns = Some(keys);
                }
            }
        }
        if let Some(w) = re!(r#"where\s*:\s*(['"])([\s\S]*?)\1"#).exec(&args) {
            let inner = w.get(&args, 2).unwrap();
            let mut set: IndexSet<String> = IndexSet::new();
            for x in re!(r"\b([A-Za-z_]\w*)\s*(?:=|<>|!=|<=|>=|<|>|\bIN\b|\bLIKE\b|\bIS\b)", "gi").all(&inner) {
                set.insert(x.str(&inner, 1).unwrap());
            }
            r.filters = Some(set.into_iter().collect());
        }
        refs.push(r);
    }
    refs
}

const KEYWORDS: &[&str] = &[
    "where",
    "on",
    "set",
    "left",
    "right",
    "inner",
    "outer",
    "full",
    "cross",
    "join",
    "order",
    "group",
    "limit",
    "offset",
    "using",
    "values",
    "select",
    "as",
    "natural",
    "straight_join",
    "union",
    "having",
    "for",
    "lock",
    "into",
    "window",
    "returning",
    "partition",
];

fn is_keyword(s: &str) -> bool {
    !s.is_empty() && KEYWORDS.contains(&s)
}

fn ident(s: &JsStr) -> JsStr {
    let t = re!(r#"[`"\[\]]"#, "g").replace_all(s, "");
    t.split(".").pop().unwrap_or_default()
}

/// Text after `from` up to the first top-level keyword in `to_keywords`.
fn top_level(sql: &JsStr, from: usize, to_keywords: &[&str]) -> JsStr {
    let mut depth = 0i64;
    let lower = sql.to_lower();
    let non_word = |u: Option<u16>| -> bool {
        let u = u.unwrap_or(b' ' as u16);
        !word(u)
    };
    let mut i = from;
    while i < sql.len() {
        let c = sql.at(i).unwrap();
        if c == b'(' as u16 {
            depth += 1;
        } else if c == b')' as u16 {
            depth -= 1;
        } else if depth == 0 {
            for k in to_keywords {
                let w = JsStr::new(k.trim());
                let before = if i == 0 { None } else { sql.at(i - 1) };
                if lower.starts_with_at(w.units(), i) && non_word(before) && non_word(sql.at(i + w.len())) {
                    return sql.slice(from, i);
                }
            }
        }
        i += 1;
    }
    sql.slice_from(from)
}

/// One DML statement → refs with `entity`, `op`, `detail`, `columns`, `filters` (columns `None` =
/// unknown). `form` is left empty and `line` 0: the caller fills them.
pub fn parse_dml(raw: &JsStr) -> Vec<DataRef> {
    let a = re!(r"--[^\n]*", "g").replace_all(raw, " ");
    let sql = re!(r"\s+", "g").replace_all(&a, " ").trim();
    let Some(kind) = re!(r"^(\w+)").exec(&sql).and_then(|m| m.get(&sql, 1)).map(|k| k.to_lower().to_string()) else {
        return Vec::new();
    };
    if !["select", "insert", "update", "delete", "replace", "with"].contains(&kind.as_str()) {
        return Vec::new();
    }
    let mut tables: IndexMap<String, String> = IndexMap::new(); // alias → table
    let mut order: Vec<String> = Vec::new();
    for m in re!(r#"\b(from|join|into|update)\s+((?:`[^`]+`|"[^"]+"|[\w.$]+))(?:\s+(?:as\s+)?(`[^`]+`|[a-z_]\w*))?"#, "gi").all(&sql) {
        let t2 = m.get(&sql, 2).unwrap();
        if t2.eq_str("?") || t2.starts_with("(") || re!(r"^(select|\?)$", "i").test(&t2) {
            continue;
        }
        // "UPDATE" only names a table at the start (not in ON DUPLICATE KEY UPDATE / ON CONFLICT DO UPDATE).
        if m.get(&sql, 1).unwrap().to_lower().eq_str("update") && m.index != 0 {
            continue;
        }
        let table = ident(&t2).to_string();
        if !re!(r"^[A-Za-z_]\w*$").test_str(&table) || is_keyword(&table.to_lowercase()) {
            continue;
        }
        let alias = m
            .get(&sql, 3)
            .filter(|a3| !is_keyword(&a3.to_lower().to_string()))
            .map(|a3| ident(&a3).to_string())
            .filter(|a| !a.is_empty());
        tables.insert(table.clone(), table.clone());
        if let Some(al) = alias {
            tables.insert(al, table.clone());
        }
        if !order.contains(&table) {
            order.push(table);
        }
    }
    if order.is_empty() {
        return Vec::new();
    }
    let single = if order.len() == 1 { Some(order[0].clone()) } else { None };
    let mut cols: IndexMap<String, IndexSet<String>> = order.iter().map(|t| (t.clone(), IndexSet::new())).collect();
    let mut filt: IndexMap<String, IndexSet<String>> = order.iter().map(|t| (t.clone(), IndexSet::new())).collect();
    let put = |map: &mut IndexMap<String, IndexSet<String>>, q: Option<String>, c: Option<String>| {
        let Some(c) = c else { return };
        if c.is_empty() || c == "?" || re!(r"^\d").test_str(&c) || is_keyword(&c.to_lowercase()) || re!(r"^(null|true|false|and|or|not|is|in|like|between|asc|desc|distinct|count|sum|avg|min|max|now|coalesce|ifnull|case|when|then|else|end|interval|date|current_timestamp)$", "i").test_str(&c) {
            return;
        }
        let q = q.filter(|x| !x.is_empty());
        let t = match &q {
            Some(q) => tables.get(q).cloned(),
            None => single.clone(),
        };
        if let Some(set) = t.and_then(|t| map.get_mut(&t)) {
            set.insert(c);
        }
        // (unknown columns of multi-table statements are dropped, as in the JS)
    };
    let lower = sql.to_lower();
    // SELECT list
    if let Some(sel_at) = lower.index_of("select ", 0) {
        let list = top_level(&sql, sel_at + 7, &[" from "]);
        for item in list.split(",") {
            let a = re!(r"\s+as\s+\w+$", "i").replace_first(&item.trim(), "");
            let it = re!(r"^distinct\s+", "i").replace_first(&a, "");
            if it.eq_str("*") {
                for s in cols.values_mut() {
                    s.insert("*".into());
                }
                continue;
            }
            if let Some(m) = re!(r"^(\w+)\.\*$").exec(&it) {
                put(&mut cols, m.str(&it, 1), Some("*".into()));
                continue;
            }
            for q in re!(r"\b(\w+)\.`?(\w+)`?", "g").all(&it) {
                put(&mut cols, q.str(&it, 1), q.str(&it, 2));
            }
            if let Some(m) = re!(r"^`?(\w+)`?$").exec(&it) {
                put(&mut cols, None, m.str(&it, 1));
            }
        }
    }
    // INSERT (a, b) / UPDATE … SET a = ?
    if let Some(ins) = re!(r#"\binto\s+[`"\w.]+\s*\(([^)]*)\)"#, "i").exec(&sql) {
        for c in ins.get(&sql, 1).unwrap().split(",") {
            put(&mut cols, None, Some(ident(&c.trim()).to_string()));
        }
    }
    if kind == "update" {
        let from = lower.index_of(" set ", 0).map(|x| x as isize).unwrap_or(-1) + 5;
        let set = top_level(&sql, from as usize, &[" where ", " order by ", " limit "]);
        for m in re!(r"(?:^|,)\s*(?:(\w+)\.)?`?(\w+)`?\s*=", "g").all(&set) {
            put(&mut cols, m.str(&set, 1), m.str(&set, 2));
        }
    }
    // WHERE / ON / HAVING conditions
    let conditions = re!(r"\bon\s+(duplicate\s+key\s+update|conflict)\b[\s\S]*$", "i").replace_first(&sql, "");
    for clause in re!(
        r"\b(where|on|having)\b([\s\S]*?)(?=\b(group by|order by|limit|left join|right join|inner join|join|union|having|returning)\b|$)",
        "gi"
    )
    .all(&conditions)
    {
        let body = clause.get(&conditions, 2).unwrap();
        for m in re!(r"(?:\b(\w+)\.)?`?\b([A-Za-z_]\w*)`?\)?\s*(?:=|<>|!=|<=|>=|<|>|\bin\b|\blike\b|\bis\b|\bbetween\b)", "gi").all(&body) {
            put(&mut filt, m.str(&body, 1), m.str(&body, 2));
        }
    }
    let write_target = match kind.as_str() {
        "insert" | "replace" => re!(r#"\binto\s+([`"\w.]+)"#, "i").exec(&sql).and_then(|m| m.get(&sql, 1)),
        "update" => re!(r#"^update\s+([`"\w.]+)"#, "i").exec(&sql).and_then(|m| m.get(&sql, 1)),
        "delete" => re!(r#"\bfrom\s+([`"\w.]+)"#, "i").exec(&sql).and_then(|m| m.get(&sql, 1)),
        _ => None,
    };
    let target = write_target.filter(|w| !w.is_empty()).map(|w| ident(&w).to_string());
    order
        .iter()
        .map(|t| {
            let c: Vec<String> = cols[t].iter().cloned().collect();
            let f: Vec<String> = filt[t].iter().cloned().collect();
            let is_target = target.as_deref() == Some(t.as_str());
            DataRef {
                entity: t.clone(),
                op: if is_target { "write".into() } else { "read".into() },
                detail: Some(if is_target { kind.clone() } else { "select".into() }),
                columns: if c.is_empty() { None } else { Some(c) },
                filters: if f.is_empty() { None } else { Some(f) },
                ..Default::default()
            }
        })
        .collect()
}

/// `parseDml` on a Rust string.
pub fn parse_dml_str(raw: &str) -> Vec<DataRef> {
    parse_dml(&JsStr::new(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Option<Vec<String>> {
        Some(v.iter().map(|x| x.to_string()).collect())
    }

    // ports of the raw-SQL / Dart cases in legacy-node/test/data-usage.test.mjs

    #[test]
    fn aliases_and_joins() {
        let r = parse_dml_str("SELECT s.id, s.first_name, c.name AS course FROM students s JOIN courses c ON c.id = s.course_id WHERE s.status = ?");
        assert_eq!(
            (r[0].entity.as_str(), r[0].columns.clone(), r[0].filters.clone()),
            ("students", s(&["id", "first_name"]), s(&["status"]))
        );
        assert_eq!((r[1].entity.as_str(), r[1].columns.clone()), ("courses", s(&["name"])));
    }

    #[test]
    fn on_duplicate_key_update() {
        let r = parse_dml_str("INSERT INTO line_users (line_user_id, display_name) VALUES (?, ?) ON DUPLICATE KEY UPDATE display_name = VALUES(display_name)");
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].entity.as_str(), r[0].op.as_str(), r[0].columns.clone(), r[0].filters.clone()),
            ("line_users", "write", s(&["line_user_id", "display_name"]), None)
        );
    }

    #[test]
    fn function_around_column_is_a_filter() {
        assert_eq!(
            parse_dml_str("SELECT COUNT(*) AS n FROM attendance WHERE DATE(check_in) = CURDATE()")[0].filters,
            s(&["check_in"])
        );
    }

    #[test]
    fn update_set_and_delete_filters() {
        let u = parse_dml_str("UPDATE line_link_tokens SET used = 1 WHERE id = ?");
        assert_eq!((u[0].op.as_str(), u[0].columns.clone(), u[0].filters.clone()), ("write", s(&["used"]), s(&["id"])));
        let d = parse_dml_str("DELETE FROM sessions WHERE user_id = ?");
        assert_eq!((d[0].op.as_str(), d[0].filters.clone()), ("write", s(&["user_id"])));
    }

    #[test]
    fn ui_text_is_not_sql() {
        assert!(parse_dml_str("Select a course").is_empty());
    }

    #[test]
    fn ts_literals_placeholders_backticks_ddl() {
        let code = "const a = await query(`\n  SELECT id FROM users\n  WHERE role = ${r}`);\nconst t = `CREATE TABLE x (\n  \\`key\\` VARCHAR(10))`;\nconst label = 'Hello';";
        let lits = sql_literals(code);
        assert_eq!(lits.len(), 2);
        assert!(re!(r"WHERE role = \?").test(&lits[0].sql));
        assert!(lits[1].ddl);
        assert!(re!(r"`key` VARCHAR").test(&lits[1].sql));
    }

    #[test]
    fn ts_literals_regex_and_comments() {
        let code = [
            "// don't parse this: it's a comment",
            "const re = /[`\"\\w]+\\s*'/i; const ok = s.match(/'/g);",
            "/* it's \"quoted\" */ const q = `SELECT id FROM users WHERE a = ${x}`;",
            "const r = a / b / c; const msg = 'done';",
        ]
        .join("\n");
        let lits = sql_literals(&code);
        assert_eq!(lits.len(), 1);
        assert!(re!(r"^SELECT id FROM users").test(&lits[0].sql));
        assert_eq!(lits[0].line, 3);
    }

    #[test]
    fn prose_is_not_ddl() {
        let lits = sql_literals("const a = 'Read: .sql files and CREATE/ALTER TABLE strings in code';\nconst b = `CREATE TABLE x (id int);\nALTER TABLE x ADD y int`;");
        assert_eq!(lits.iter().map(|l| l.ddl).collect::<Vec<_>>(), [true]);
    }

    #[test]
    fn dart_literals() {
        let code = "await db.execute('''\n  CREATE TABLE pockets(\n    id INTEGER PRIMARY KEY\n  )\n''');\nawait db.execute('CREATE INDEX idx_p ON pockets(name)');\nfinal q = 'SELECT * FROM pockets WHERE id = $id';";
        let lits = dart_sql_literals(code);
        assert_eq!(lits.iter().map(|l| l.ddl).collect::<Vec<_>>(), [true, true, false]);
        assert!(re!(r"id = \?").test(&lits[2].sql));
        assert_eq!(lits[0].line, 1);
    }

    #[test]
    fn sqflite_helpers() {
        let refs = scan_sqflite("final rows = await db.query('categories', columns: ['name'], where: 'kind = ? AND archived = 0');\nbatch.insert('pockets', {'name': n, 'icon': i});\nawait _db.update('pockets', values, where: 'id = ?', whereArgs: [id]);");
        let got: Vec<_> = refs.iter().map(|r| (r.entity.clone(), r.op.clone(), r.columns.clone(), r.filters.clone())).collect();
        assert_eq!(
            got,
            vec![
                ("categories".into(), "read".into(), s(&["name"]), s(&["kind", "archived"])),
                ("pockets".into(), "write".into(), s(&["name", "icon"]), None),
                ("pockets".into(), "write".into(), None, s(&["id"])),
            ]
        );
    }
}
