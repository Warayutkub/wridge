//! Prisma: `schema.prisma` → data model, and `prisma.<model>.<operation>({ select, where, data … })`
//! → data refs (port of `data/prisma.mjs`).

use super::sql_ddl::Model;
use super::{arr_mut, str_of, DataRef, Obj};
use crate::jsre::{JsRegex, JsStr};
use crate::re;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

fn blocks(text: &JsStr, kind: &str) -> Vec<(usize, JsStr, JsStr, JsStr)> {
    // [index, m[0], m[1], m[2]]
    let re = JsRegex::new(&format!(r"^\s*{kind}\s+(\w+)\s*\{{([\s\S]*?)^\s*\}}"), "gm");
    re.all(text)
        .into_iter()
        .map(|m| (m.index, m.whole(text), m.get(text, 1).unwrap(), m.get(text, 2).unwrap()))
        .collect()
}

fn list_arg(s: Option<&JsStr>) -> Vec<String> {
    let s = s.cloned().unwrap_or_default();
    s.split(",")
        .iter()
        .map(|x| re!(r"\(.*$").replace_first(&x.trim(), "").to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Applies one .prisma file to a model (entities keyed by model name; table = @@map name).
pub fn apply_prisma(model: &mut Model, text: &str, file: &str) {
    let text = JsStr::new(text);
    let clean = re!(r"\/\/[^\n]*", "g").replace_all_with(&text, |m| JsStr(vec![b' ' as u16; m.len()]));
    let models = blocks(&clean, "model");
    let enums: Vec<String> = blocks(&clean, "enum").into_iter().map(|b| b.2.to_string()).collect();
    let names: Vec<String> = models.iter().map(|b| b.2.to_string()).collect();
    for (index, whole, name, body) in models {
        let name = name.to_string();
        let at = index + whole.index_of("model", 0).unwrap_or(0);
        let mut ent = json!({ "name": name, "kind": "table", "table": name, "columns": [], "indexes": [], "fks": [], "policies": [], "rls": false,
            "source": { "file": file, "line": clean.line_at(at) }, "changedIn": [] });
        let e: &mut Obj = ent.as_object_mut().unwrap();
        for raw in body.split("\n") {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(a) = re!(r#"^@@map\(\s*"([^"]+)""#).exec(&line) {
                e.insert("table".into(), json!(a.str(&line, 1).unwrap()));
                continue;
            }
            if let Some(a) = re!(r"^@@(index|unique|id)\(\s*(?:fields:\s*)?\[([^\]]*)\]").exec(&line) {
                let cols = list_arg(a.get(&line, 2).as_ref());
                let what = a.str(&line, 1).unwrap();
                if what == "id" {
                    for c in arr_mut(e, "columns").iter_mut().filter_map(Value::as_object_mut) {
                        if str_of(c, "name").is_some_and(|n| cols.iter().any(|x| x == n)) {
                            c.insert("pk".into(), json!(true));
                            c.insert("notNull".into(), json!(true));
                        }
                    }
                } else {
                    arr_mut(e, "indexes").push(json!({ "name": null, "columns": cols, "unique": what == "unique" }));
                }
                continue;
            }
            if line.starts_with("@@") {
                continue;
            }
            let Some(f) = re!(r"^(\w+)\s+(\w+)(\[\])?(\?)?\s*(.*)$").exec(&line) else { continue };
            let field = f.str(&line, 1).unwrap();
            let ty = f.str(&line, 2).unwrap();
            let list = f.range(3).is_some_and(|(a, b)| b > a);
            let optional = f.range(4).is_some_and(|(a, b)| b > a);
            let attrs = f.get(&line, 5).unwrap_or_default();
            if names.contains(&ty) {
                // Relation field: not a column; the FK lives in @relation(fields: […], references: […]).
                if let Some(rel) = re!(r"@relation\([^)]*fields:\s*\[([^\]]*)\][^)]*references:\s*\[([^\]]*)\]").exec(&attrs) {
                    arr_mut(e, "fks").push(json!({ "columns": list_arg(rel.get(&attrs, 1).as_ref()), "table": ty, "refColumns": list_arg(rel.get(&attrs, 2).as_ref()) }));
                }
                continue;
            }
            let mut col = Obj::new();
            col.insert("name".into(), json!(field));
            col.insert(
                "type".into(),
                json!(format!("{ty}{}{}", if list { "[]" } else { "" }, if enums.contains(&ty) { " (enum)" } else { "" })),
            );
            if !optional {
                col.insert("notNull".into(), json!(true));
            }
            if re!(r"@id\b").test(&attrs) {
                col.insert("pk".into(), json!(true));
            }
            if re!(r"@unique\b").test(&attrs) {
                col.insert("unique".into(), json!(true));
            }
            if let Some(d) = re!(r"@default\(((?:[^()]|\([^()]*\))*)\)").exec(&attrs) {
                col.insert("default".into(), json!(d.str(&attrs, 1).unwrap()));
            }
            if let Some(mp) = re!(r#"@map\(\s*"([^"]+)""#).exec(&attrs) {
                col.insert("db".into(), json!(mp.str(&attrs, 1).unwrap()));
            }
            arr_mut(e, "columns").push(Value::Object(col));
        }
        // FK columns point at the referenced model's columns.
        let fks = e["fks"].as_array().cloned().unwrap_or_default();
        for fk in fks {
            let fcols: Vec<String> = fk["columns"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let mut r = Obj::new();
            r.insert("table".into(), fk["table"].clone());
            if let Some(c0) = fk["refColumns"].as_array().and_then(|a| a.first()) {
                r.insert("column".into(), c0.clone());
            }
            for c in arr_mut(e, "columns").iter_mut().filter_map(Value::as_object_mut) {
                if str_of(c, "name").is_some_and(|n| fcols.iter().any(|x| x == n)) {
                    c.insert("ref".into(), Value::Object(r.clone()));
                }
            }
        }
        let Value::Object(o) = ent else { unreachable!() };
        model.entities.insert(name, Rc::new(RefCell::new(o)));
    }
}

/// The datasource provider (`postgresql`, `mysql` …) of a schema file.
pub fn prisma_provider(text: &str) -> Option<String> {
    re!(r#"datasource\s+\w+\s*\{[^}]*provider\s*=\s*"(\w+)""#).exec_str(text).and_then(|g| g[1].clone())
}

const OPS: [(&str, &str); 17] = [
    ("findMany", "read"),
    ("findUnique", "read"),
    ("findUniqueOrThrow", "read"),
    ("findFirst", "read"),
    ("findFirstOrThrow", "read"),
    ("count", "read"),
    ("aggregate", "read"),
    ("groupBy", "read"),
    ("create", "write"),
    ("createMany", "write"),
    ("createManyAndReturn", "write"),
    ("update", "write"),
    ("updateMany", "write"),
    ("updateManyAndReturn", "write"),
    ("upsert", "write"),
    ("delete", "write"),
    ("deleteMany", "write"),
];

/// Top-level keys of the object literal that follows `key:` inside an argument object.
fn keys_of(arg: &JsStr, key: &str) -> Option<Vec<String>> {
    let re = JsRegex::new(&format!(r"\b{key}\s*:\s*\{{"), "");
    let at = re.exec(arg)?;
    let mut depth = 0i64;
    let mut body: Vec<u16> = Vec::new();
    let mut i = at.end - 1;
    while i < arg.len() {
        let ch = arg.at(i).unwrap();
        if b"{[(".iter().any(|c| *c as u16 == ch) {
            depth += 1;
        }
        if b"}])".iter().any(|c| *c as u16 == ch) {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }
        body.push(if depth == 1 { ch } else { b' ' as u16 });
        i += 1;
    }
    let body = JsStr(body).slice_from(1);
    let keys = re!(r#"(?:^|,)\s*(?:["']?([A-Za-z_]\w*)["']?\s*:|([A-Za-z_]\w*)\s*(?=,|$))"#, "g")
        .all(&body)
        .into_iter()
        .map(|k| k.str(&body, 1).or_else(|| k.str(&body, 2)).unwrap_or_else(|| "undefined".into()))
        .filter(|k| !["AND", "OR", "NOT"].contains(&k.as_str()))
        .collect();
    Some(keys)
}

fn upper_first(s: &JsStr) -> Result<String> {
    let first = s.at(0).ok_or_else(|| anyhow!("Cannot read properties of undefined (reading 'toUpperCase')"))?;
    Ok(format!("{}{}", JsStr(vec![first]).to_string().to_uppercase(), s.slice_from(1)))
}

/// Relation field → model name: `scenes` → `Scene`, `categories` → `Category`, `author` → `Author`.
fn model_of(field: &str) -> Result<String> {
    let f = JsStr::new(field);
    let one = if f.ends_with("ies") {
        f.slice_js(0, -3).concat(&JsStr::new("y"))
    } else if re!(r"(ss|us)$").test(&f) {
        f
    } else {
        re!(r"s$").replace_first(&f, "")
    };
    upper_first(&one)
}

/// `prisma.user.findMany({ select: { id: true }, where: { email } })` → refs (entity = model name "User").
pub fn scan_prisma(text: &str) -> Result<Vec<DataRef>> {
    let text = JsStr::new(text);
    let mut refs = Vec::new();
    let re = re!(&format!(r"\.([a-z]\w*)\s*\.\s*({})\(", OPS.iter().map(|o| o.0).collect::<Vec<_>>().join("|")), "g");
    for m in re.all(&text) {
        let accessor = m.get(&text, 1).unwrap();
        let entity = upper_first(&accessor)?;
        let start = m.end;
        let mut depth = 1i64;
        let mut end = start;
        while end < text.len() && end < start + 4000 && depth > 0 {
            if text.is(end, '(') {
                depth += 1;
            } else if text.is(end, ')') {
                depth -= 1;
            }
            end += 1;
        }
        let arg = text.slice_js(start as isize, end as isize - 1);
        let opname = m.str(&text, 2).unwrap();
        let op = OPS.iter().find(|o| o.0 == opname).map(|o| o.1).unwrap();
        let line = text.line_at(m.index) as i64;
        let mut r = DataRef::new(entity.clone(), op, Some(&opname), line);
        let cols = keys_of(&arg, "select").or_else(|| if op == "write" { keys_of(&arg, "data") } else { None });
        if cols.as_ref().is_some_and(|c| !c.is_empty()) {
            r.columns = cols;
        } else if op == "read" && !re!(r"\bselect\s*:").test(&arg) && opname != "count" {
            r.columns = Some(vec!["*".into()]);
        }
        if let Some(w) = keys_of(&arg, "where").filter(|w| !w.is_empty()) {
            r.filters = Some(w);
        }
        for inc in keys_of(&arg, "include").unwrap_or_default() {
            let mut x = DataRef::new(model_of(&inc)?, "read", Some("include"), line);
            x.via = Some(entity.clone());
            refs.push(x);
        }
        refs.push(r);
    }
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::sql_ddl::empty_model;

    // ports of the Prisma cases in legacy-node/test/data-usage.test.mjs

    #[test]
    fn schema_map_relations_enums_index() {
        let mut m = empty_model();
        apply_prisma(
            &mut m,
            "enum Status { draft done }\nmodel User {\n  id        String   @id @default(uuid())\n  avatarUrl String?  @map(\"avatar_url\")\n  projects  Project[]\n  @@map(\"users\")\n}\nmodel Project {\n  id     String @id\n  userId String @map(\"user_id\")\n  status Status @default(draft)\n  user   User   @relation(fields: [userId], references: [id])\n  @@index([userId, status])\n}",
            "prisma/schema.prisma",
        );
        let u = m.entities["User"].borrow();
        assert_eq!(u["table"], "users");
        let cols: Vec<_> = u["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["name"].clone(), c.get("db").cloned().unwrap_or(Value::Null), c.get("notNull").is_some()))
            .collect();
        assert_eq!(cols, vec![(json!("id"), Value::Null, true), (json!("avatarUrl"), json!("avatar_url"), false)]);
        let p = m.entities["Project"].borrow();
        let names: Vec<_> = p["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["id", "userId", "status"]);
        assert_eq!(p["fks"], json!([{ "columns": ["userId"], "table": "User", "refColumns": ["id"] }]));
        assert_eq!(p["columns"][2]["type"], "Status (enum)");
        assert_eq!(p["indexes"][0]["columns"], json!(["userId", "status"]));
    }

    #[test]
    fn client_calls() {
        let refs = scan_prisma("await prisma.orderItem.findMany({ where: { orderId: id, NOT: { qty: 0 } }, select: { id: true, qty: true } });\nawait db.user.create({ data: { email, name: n } });\nawait prisma.project.findUnique({ where: { id }, include: { scenes: true } });").unwrap();
        let got: Vec<_> = refs.iter().map(|r| (r.entity.clone(), r.op.clone(), r.columns.clone(), r.filters.clone())).collect();
        let s = |v: &[&str]| Some(v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(
            got,
            vec![
                ("OrderItem".to_string(), "read".to_string(), s(&["id", "qty"]), s(&["orderId"])),
                ("User".to_string(), "write".to_string(), s(&["email", "name"]), None),
                ("Scene".to_string(), "read".to_string(), None, None),
                ("Project".to_string(), "read".to_string(), s(&["*"]), s(&["id"])),
            ]
        );
    }
}
