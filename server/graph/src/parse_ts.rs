//! TS/TSX/JS → symbols, imports and references, with tree-sitter (port of `parse-ts.mjs`).
//!
//! Node uses web-tree-sitter 0.27.0 with the tree-sitter-typescript 0.23.2 WASM grammars; this
//! uses the native tree-sitter 0.27.0 with the same grammar release, so the trees (and therefore
//! the results) are identical. Byte offsets differ from the WASM build's UTF-16 offsets, but they
//! are only compared with each other, never stored.

use crate::re;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use std::cell::RefCell;
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

/// `hash(text)`: first 12 hex chars of the SHA-1 of the UTF-8 text.
pub fn hash(text: &str) -> String {
    let d = Sha1::digest(text.as_bytes());
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    hex[..12].to_string()
}

/// A top-level symbol (row of `symbols`).
#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub exported: i64,
    pub is_default: i64,
    pub line: i64,
    pub end_line: i64,
    /// JSON object (`{names, type}` for components, `{of}` for aliases, Dart: `{statics…}`).
    pub props: Option<Value>,
    pub hash: String,
}

/// An import / re-export (row of `imports`, before resolution).
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub kind: String,
    pub source: String,
    pub imported: Option<String>,
    pub local: Option<String>,
    pub type_only: i64,
    pub line: i64,
}

/// A use of an import binding or top-level name; `owner` = index into the symbols (-1 = module).
#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    pub owner: i64,
    pub local: String,
    pub member: Option<String>,
    pub kind: String,
    pub line: i64,
    pub count: i64,
}

/// A route declared in code (Dart: GoRoute, MaterialApp routes).
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredRoute {
    pub path: String,
    pub widget: Option<String>,
    pub kind: String,
    pub line: i64,
}

/// What a parser extracts from one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    pub directive: Option<String>,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub refs: Vec<Ref>,
    pub routes: Vec<DeclaredRoute>,
}

impl Parsed {
    /// The object `parseTs` returns in JS: `{ directive, symbols, imports, refs }` with the same
    /// keys and key order (props omitted keys stay omitted).
    pub fn to_json(&self) -> Value {
        let symbols: Vec<Value> = self
            .symbols
            .iter()
            .map(|s| json!({ "name": s.name, "kind": s.kind, "exported": s.exported, "isDefault": s.is_default, "line": s.line, "endLine": s.end_line, "props": s.props, "hash": s.hash }))
            .collect();
        let imports: Vec<Value> = self
            .imports
            .iter()
            .map(|i| json!({ "kind": i.kind, "source": i.source, "imported": i.imported, "local": i.local, "typeOnly": i.type_only, "line": i.line }))
            .collect();
        let refs: Vec<Value> = self
            .refs
            .iter()
            .map(|r| json!({ "owner": r.owner, "local": r.local, "member": r.member, "kind": r.kind, "line": r.line, "count": r.count }))
            .collect();
        json!({ "directive": self.directive, "symbols": symbols, "imports": imports, "refs": refs })
    }
}

thread_local! {
    static PARSERS: RefCell<Option<(Parser, Parser)>> = const { RefCell::new(None) };
}

/// `parseTs(source, ext)`: `.ts`/`.mts`/`.cts` use the TypeScript grammar (`<T>x` casts are JSX
/// to the TSX grammar), everything else the TSX grammar.
pub fn parse_ts(source: &str, ext: &str) -> Result<Parsed> {
    PARSERS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let mut ts = Parser::new();
            ts.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())?;
            let mut tsx = Parser::new();
            tsx.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())?;
            *slot = Some((ts, tsx));
        }
        let (ts, tsx) = slot.as_mut().unwrap();
        let parser = if [".ts", ".mts", ".cts"].contains(&ext) { ts } else { tsx };
        let tree = parser.parse(source, None).ok_or_else(|| anyhow!("Parsing failed"))?;
        Extractor::new(source.as_bytes()).extract(tree.root_node())
    })
}

const JSX: [&str; 3] = ["jsx_element", "jsx_self_closing_element", "jsx_fragment"];

fn null_text() -> anyhow::Error {
    anyhow!("Cannot read properties of null (reading 'text')")
}

fn line(n: Node) -> i64 {
    n.start_position().row as i64 + 1
}

fn named_children<'a>(n: Node<'a>) -> Vec<Node<'a>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

fn children<'a>(n: Node<'a>) -> Vec<Node<'a>> {
    let mut c = n.walk();
    n.children(&mut c).collect()
}

/// Depth-first walk with a cursor; `visit` returning false skips the subtree.
fn walk<'a>(node: Node<'a>, mut visit: impl FnMut(Node<'a>) -> Result<bool>) -> Result<()> {
    let mut c = node.walk();
    let mut depth = 0usize;
    loop {
        let descend = visit(c.node())?;
        if descend && c.goto_first_child() {
            depth += 1;
            continue;
        }
        loop {
            if depth == 0 {
                return Ok(());
            }
            if c.goto_next_sibling() {
                break;
            }
            c.goto_parent();
            depth -= 1;
        }
    }
}

/// `node.descendantsOfType(JSX).length > 0` (the node itself included; web-tree-sitter skips
/// nodes that end at the very start of the file).
fn has_jsx(node: Node) -> bool {
    let mut c = node.walk();
    let mut depth = 0usize;
    loop {
        let n = c.node();
        let p = n.end_position();
        let before_start = p.row == 0 && p.column == 0;
        if !before_start {
            if JSX.contains(&n.kind()) {
                return true;
            }
            if c.goto_first_child() {
                depth += 1;
                continue;
            }
        }
        loop {
            if depth == 0 {
                return false;
            }
            if c.goto_next_sibling() {
                break;
            }
            c.goto_parent();
            depth -= 1;
        }
    }
}

fn unquote(s: &str) -> String {
    let mut t = s;
    if t.starts_with(['\'', '"', '`']) {
        t = &t[1..];
    }
    if t.ends_with(['\'', '"', '`']) {
        t = &t[..t.len() - 1];
    }
    t.to_string()
}

fn is_pascal(s: &str) -> bool {
    re!(r"^[A-Z][A-Za-z0-9]*$").test_str(s)
}

fn is_wrapper(s: &str) -> bool {
    re!(r"(^|\.)(forwardRef|memo|observer|styled\.\w+|lazy)$").test_str(s)
}

struct Sym<'a> {
    s: Symbol,
    start: usize,
    end: usize,
    _node: Node<'a>,
}

struct Pending<'a> {
    name: String,
    alias: String,
    node: Node<'a>,
}

struct Extractor<'a> {
    src: &'a [u8],
    symbols: Vec<Sym<'a>>,
    imports: Vec<Import>,
    pending: Vec<Pending<'a>>,
    decl_names: HashSet<usize>,
    directive: Option<String>,
}

impl<'a> Extractor<'a> {
    fn new(src: &'a [u8]) -> Self {
        Extractor {
            src,
            symbols: Vec::new(),
            imports: Vec::new(),
            pending: Vec::new(),
            decl_names: HashSet::new(),
            directive: None,
        }
    }

    fn text(&self, n: Node) -> &'a str {
        std::str::from_utf8(&self.src[n.start_byte()..n.end_byte()]).unwrap_or("")
    }

    fn field_text(&self, n: Node, field: &str) -> Option<&'a str> {
        n.child_by_field_name(field).map(|c| self.text(c))
    }

    fn fn_kind(&self, name: &str, f: Option<Node>) -> &'static str {
        if re!(r"^use[A-Z0-9]").test_str(name) {
            return "hook";
        }
        if is_pascal(name) && f.is_some_and(has_jsx) {
            return "component";
        }
        "function"
    }

    #[allow(clippy::too_many_arguments)]
    fn add_symbol(&mut self, name: &str, kind: &str, node: Node<'a>, name_node: Option<Node<'a>>, exported: i64, is_default: i64, props: Option<Value>) {
        if let Some(nn) = name_node {
            self.decl_names.insert(nn.start_byte());
        }
        let s = Symbol {
            name: name.to_string(),
            kind: kind.to_string(),
            exported,
            is_default,
            line: line(node),
            end_line: node.end_position().row as i64 + 1,
            props,
            hash: hash(self.text(node)),
        };
        self.symbols.push(Sym {
            s,
            start: node.start_byte(),
            end: node.end_byte(),
            _node: node,
        });
    }

    /// First parameter of a component: destructured prop names and/or its type.
    fn props_of(&self, f: Option<Node<'a>>) -> Result<Option<Value>> {
        let Some(f) = f else { return Ok(None) };
        let params = f.child_by_field_name("parameters").or_else(|| f.child_by_field_name("parameter"));
        let first = match params {
            Some(p) if p.kind() == "formal_parameters" => named_children(p).into_iter().find(|n| n.kind().ends_with("parameter")),
            other => other,
        };
        let Some(first) = first else { return Ok(None) };
        let pattern = first.child_by_field_name("pattern").unwrap_or(first);
        let ty = first.child_by_field_name("type").map(|t| re!(r"^:\s*").replace_first_str(self.text(t), ""));
        let mut names: Vec<Value> = Vec::new();
        if pattern.kind() == "object_pattern" {
            for p in named_children(pattern) {
                match p.kind() {
                    "shorthand_property_identifier_pattern" => names.push(json!(self.text(p))),
                    "pair_pattern" => names.push(json!(self.field_text(p, "key").ok_or_else(null_text)?)),
                    "object_assignment_pattern" => names.push(json!(self.field_text(p, "left").ok_or_else(null_text)?)),
                    "rest_pattern" => {
                        let inner = named_children(p).first().map(|n| self.text(*n)).unwrap_or("");
                        names.push(json!(format!("...{inner}")));
                    }
                    _ => {}
                }
            }
        }
        if names.is_empty() && ty.as_deref().is_none_or(str::is_empty) {
            return Ok(None);
        }
        let mut m = Map::new();
        m.insert("names".into(), Value::Array(names));
        if let Some(t) = ty {
            let t = if crate::js::utf16_len(&t) > 200 {
                format!("{}…", crate::js::slice_utf16(&t, 0, 200))
            } else {
                t
            };
            m.insert("type".into(), Value::String(t));
        }
        Ok(Some(Value::Object(m)))
    }

    fn declare(&mut self, decl: Node<'a>, exported: bool, is_default: bool) -> Result<()> {
        let before = self.symbols.len();
        let name = decl.child_by_field_name("name");
        match decl.kind() {
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                let n = name.map(|x| self.text(x)).unwrap_or("default");
                let kind = self.fn_kind(n, Some(decl));
                let props = if kind == "component" { self.props_of(Some(decl))? } else { None };
                self.add_symbol(n, kind, decl, name, 0, 0, props);
            }
            "class_declaration" | "abstract_class_declaration" | "class" => {
                let heritage = named_children(decl).into_iter().find(|n| n.kind() == "class_heritage").map(|n| self.text(n)).unwrap_or("");
                let kind = if re!(r"\b(Component|PureComponent)\b").test_str(heritage) {
                    "component"
                } else {
                    "class"
                };
                let n = name.map(|x| self.text(x)).unwrap_or("default");
                self.add_symbol(n, kind, decl, name, 0, 0, None);
            }
            "lexical_declaration" | "variable_declaration" => {
                for d in named_children(decl).into_iter().filter(|n| n.kind() == "variable_declarator") {
                    let id = d.child_by_field_name("name");
                    let value = d.child_by_field_name("value");
                    let Some(id) = id.filter(|i| i.kind() == "identifier") else {
                        // const { a, b } = … — every bound name is a const
                        let id = id.ok_or_else(|| anyhow!("Cannot read properties of null (reading 'walk')"))?;
                        let mut found = Vec::new();
                        walk(id, |n| {
                            if n.kind() == "shorthand_property_identifier_pattern" || n.kind() == "identifier" {
                                found.push(n);
                            }
                            Ok(true)
                        })?;
                        for n in found {
                            self.add_symbol(self.text(n), "const", d, Some(n), 0, 0, None);
                        }
                        continue;
                    };
                    let f = inner_function(value);
                    let wrapped = value.is_some_and(|v| v.kind() == "call_expression" && is_wrapper(self.field_text(v, "function").unwrap_or("")));
                    let id_text = self.text(id);
                    let mut kind = "const";
                    if f.is_some() && (value.map(|v| v.id()) == f.map(|x| x.id()) || wrapped) {
                        kind = self.fn_kind(id_text, f);
                    } else if wrapped && is_pascal(id_text) {
                        kind = "component";
                    }
                    let props = if kind == "component" { self.props_of(f)? } else { None };
                    self.add_symbol(id_text, kind, d, Some(id), 0, 0, props);
                }
            }
            "type_alias_declaration" => {
                let n = name.ok_or_else(null_text)?;
                self.add_symbol(self.text(n), "type", decl, Some(n), 0, 0, None);
            }
            "interface_declaration" => {
                let n = name.ok_or_else(null_text)?;
                self.add_symbol(self.text(n), "interface", decl, Some(n), 0, 0, None);
            }
            "enum_declaration" => {
                let n = name.ok_or_else(null_text)?;
                self.add_symbol(self.text(n), "enum", decl, Some(n), 0, 0, None);
            }
            _ => return Ok(()),
        }
        let added = self.symbols.len() - before;
        for s in &mut self.symbols[before..] {
            s.s.exported = if exported { 1 } else { 0 };
            s.s.is_default = if is_default && added == 1 { 1 } else { 0 };
        }
        Ok(())
    }

    fn import_statement(&mut self, st: Node<'a>) -> Result<()> {
        let source = unquote(self.field_text(st, "source").unwrap_or(""));
        let type_only = if re!(r"^import\s+type\b").test_str(self.text(st)) { 1 } else { 0 };
        let ln = line(st);
        let Some(clause) = named_children(st).into_iter().find(|n| n.kind() == "import_clause") else {
            self.imports.push(Import {
                kind: "side_effect".into(),
                source,
                imported: None,
                local: None,
                type_only,
                line: ln,
            });
            return Ok(());
        };
        for c in named_children(clause) {
            match c.kind() {
                "identifier" => self.imports.push(Import {
                    kind: "import".into(),
                    source: source.clone(),
                    imported: Some("default".into()),
                    local: Some(self.text(c).into()),
                    type_only,
                    line: ln,
                }),
                "namespace_import" => {
                    let local = named_children(c).first().map(|n| self.text(*n).to_string());
                    self.imports.push(Import {
                        kind: "import".into(),
                        source: source.clone(),
                        imported: Some("*".into()),
                        local,
                        type_only,
                        line: ln,
                    });
                }
                "named_imports" => {
                    for s in named_children(c).into_iter().filter(|n| n.kind() == "import_specifier") {
                        let name = self.field_text(s, "name").ok_or_else(null_text)?;
                        let alias = self.field_text(s, "alias").unwrap_or(name);
                        let t = if type_only == 1 || re!(r"^type\s").test_str(self.text(s)) { 1 } else { 0 };
                        self.imports.push(Import {
                            kind: "import".into(),
                            source: source.clone(),
                            imported: Some(name.into()),
                            local: Some(alias.into()),
                            type_only: t,
                            line: ln,
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn export_statement(&mut self, st: Node<'a>) -> Result<()> {
        let source = st.child_by_field_name("source");
        let decl = st.child_by_field_name("declaration");
        let value = st.child_by_field_name("value");
        let is_default = children(st).iter().any(|c| c.kind() == "default");
        let type_only = if re!(r"^export\s+type\b").test_str(self.text(st)) { 1 } else { 0 };
        let clause = named_children(st).into_iter().find(|n| n.kind() == "export_clause");
        let ln = line(st);
        if let Some(source) = source {
            let src = unquote(self.text(source));
            let ns = named_children(st).into_iter().find(|n| n.kind() == "namespace_export");
            if let Some(clause) = clause {
                for s in named_children(clause).into_iter().filter(|n| n.kind() == "export_specifier") {
                    let name = self.field_text(s, "name").ok_or_else(null_text)?;
                    let alias = self.field_text(s, "alias").unwrap_or(name);
                    self.imports.push(Import {
                        kind: "reexport".into(),
                        source: src.clone(),
                        imported: Some(name.into()),
                        local: Some(alias.into()),
                        type_only,
                        line: ln,
                    });
                }
            } else if let Some(ns) = ns {
                let local = named_children(ns).last().map(|n| self.text(*n).to_string());
                self.imports.push(Import {
                    kind: "reexport".into(),
                    source: src,
                    imported: Some("*".into()),
                    local,
                    type_only,
                    line: ln,
                });
            } else {
                self.imports.push(Import {
                    kind: "reexport_all".into(),
                    source: src,
                    imported: Some("*".into()),
                    local: None,
                    type_only,
                    line: ln,
                });
            }
            return Ok(());
        }
        if let Some(decl) = decl {
            return self.declare(decl, true, is_default);
        }
        if let Some(clause) = clause {
            for s in named_children(clause).into_iter().filter(|n| n.kind() == "export_specifier") {
                let name = self.field_text(s, "name").ok_or_else(null_text)?;
                let alias = self.field_text(s, "alias").unwrap_or(name);
                self.pending.push(Pending {
                    name: name.into(),
                    alias: alias.into(),
                    node: st,
                });
            }
            return Ok(());
        }
        if let Some(value) = value {
            if value.kind() == "identifier" {
                self.pending.push(Pending {
                    name: self.text(value).into(),
                    alias: "default".into(),
                    node: st,
                });
                return Ok(());
            }
            // export default function() {} / memo(Foo) / {...}
            let f = inner_function(Some(value));
            let wraps = value.kind() == "call_expression" && is_wrapper(self.field_text(value, "function").unwrap_or(""));
            let kind = if value.kind() == "class" {
                "class"
            } else if f.is_some() {
                self.fn_kind("Default", f)
            } else if wraps {
                "component"
            } else {
                "const"
            };
            let props = if kind == "component" { self.props_of(f)? } else { None };
            self.add_symbol("default", kind, value, None, 1, 1, props);
        }
        Ok(())
    }

    fn extract(mut self, root: Node<'a>) -> Result<Parsed> {
        let statements = named_children(root);
        for (i, st) in statements.iter().copied().enumerate() {
            if st.kind() == "comment" {
                continue;
            }
            if st.kind() == "expression_statement" && self.directive.is_none() && i < 3 {
                if let Some(first) = named_children(st).first().filter(|n| n.kind() == "string") {
                    let d = unquote(self.text(*first));
                    if d == "use client" || d == "use server" {
                        self.directive = Some(d);
                        continue;
                    }
                }
            }
            match st.kind() {
                "import_statement" => self.import_statement(st)?,
                "export_statement" => self.export_statement(st)?,
                _ => self.declare(st, false, false)?,
            }
        }

        // export { a as b } / export default a — point at the declared symbol, or keep an alias row.
        let pending = std::mem::take(&mut self.pending);
        for Pending { name, alias, node } in pending {
            let found = self.symbols.iter().position(|x| x.s.name == name && x.s.kind != "alias");
            match found {
                Some(i) if alias == name => self.symbols[i].s.exported = 1,
                Some(i) if alias == "default" => {
                    self.symbols[i].s.exported = 1;
                    self.symbols[i].s.is_default = 1;
                }
                Some(_) => self.add_symbol(&alias, "alias", node, None, 1, 0, Some(json!({ "of": name }))),
                None => {
                    // re-export of an imported binding: export { Button } (after import { Button } from …)
                    if let Some(imp) = self.imports.iter().find(|x| x.local.as_deref() == Some(name.as_str()) && x.kind == "import").cloned() {
                        self.imports.push(Import {
                            kind: "reexport".into(),
                            source: imp.source,
                            imported: imp.imported,
                            local: Some(alias),
                            type_only: imp.type_only,
                            line: line(node),
                        });
                    }
                }
            }
        }

        // References: every use of an import binding or a top-level name, attributed to the enclosing top-level symbol.
        let mut watch: HashSet<String> = self.imports.iter().filter_map(|x| x.local.clone().filter(|l| !l.is_empty())).collect();
        watch.extend(self.symbols.iter().map(|s| s.s.name.clone()));
        let mut ref_map: IndexMap<String, Ref> = IndexMap::new();
        let mut new_imports: Vec<Import> = Vec::new();
        {
            let symbols = &self.symbols;
            let decl_names = &self.decl_names;
            let src = self.src;
            let text = |n: Node| std::str::from_utf8(&src[n.start_byte()..n.end_byte()]).unwrap_or("");
            let owner_of = |n: Node| -> i64 {
                for (i, s) in symbols.iter().enumerate() {
                    if n.start_byte() >= s.start && n.end_byte() <= s.end && s.s.kind != "alias" {
                        return i as i64;
                    }
                }
                -1
            };
            let mut add_ref = |owner: i64, local: &str, member: Option<String>, kind: &str, at: i64| {
                let key = format!("{owner}|{local}|{}|{kind}", member.as_deref().unwrap_or(""));
                if let Some(r) = ref_map.get_mut(&key) {
                    r.count += 1;
                } else {
                    ref_map.insert(
                        key,
                        Ref {
                            owner,
                            local: local.into(),
                            member,
                            kind: kind.into(),
                            line: at,
                            count: 1,
                        },
                    );
                }
            };
            let jsx_name_parent = |k: &str| matches!(k, "jsx_opening_element" | "jsx_self_closing_element" | "jsx_closing_element");

            for st in statements.iter().copied() {
                if st.kind() == "import_statement" || st.child_by_field_name("source").is_some() {
                    continue;
                }
                // export { a, b } only lists names; it doesn't use them.
                if st.kind() == "export_statement" && st.child_by_field_name("declaration").is_none() && st.child_by_field_name("value").is_none() {
                    continue;
                }
                walk(st, |n| {
                    match n.kind() {
                        "jsx_opening_element" | "jsx_self_closing_element" => {
                            let name = n.child_by_field_name("name").map(text).unwrap_or("");
                            let mut parts = name.split('.');
                            let root_name = parts.next().unwrap_or("");
                            let rest: Vec<&str> = parts.collect();
                            if watch.contains(root_name) {
                                let member = if rest.is_empty() { None } else { Some(rest.join(".")) };
                                let member = member.filter(|m| !m.is_empty());
                                add_ref(owner_of(n), root_name, member, "jsx", line(n));
                            }
                            Ok(true)
                        }
                        "call_expression" => {
                            let f = n.child_by_field_name("function");
                            let arg = n.child_by_field_name("arguments").and_then(|a| named_children(a).into_iter().next());
                            if let (Some(f), Some(arg)) = (f, arg) {
                                if (f.kind() == "import" || text(f) == "require") && arg.kind() == "string" {
                                    new_imports.push(Import {
                                        kind: if f.kind() == "import" { "dynamic".into() } else { "import".into() },
                                        source: unquote(text(arg)),
                                        imported: Some("*".into()),
                                        local: None,
                                        type_only: 0,
                                        line: line(n),
                                    });
                                }
                            }
                            Ok(true)
                        }
                        "identifier" | "type_identifier" | "shorthand_property_identifier" => {
                            let t = text(n);
                            if !watch.contains(t) || decl_names.contains(&n.start_byte()) {
                                return Ok(false);
                            }
                            let parent = n.parent();
                            let pk = parent.map(|p| p.kind()).unwrap_or("");
                            if jsx_name_parent(pk) || (pk == "member_expression" && parent.and_then(|p| p.parent()).is_some_and(|g| jsx_name_parent(g.kind()))) {
                                return Ok(false);
                            }
                            let mut member = None;
                            if let Some(p) = parent {
                                if pk == "member_expression" && p.child_by_field_name("object").map(|o| o.start_byte()) == Some(n.start_byte()) {
                                    member = p.child_by_field_name("property").map(|x| text(x).to_string());
                                }
                                if pk == "nested_type_identifier" {
                                    member = named_children(p).last().map(|x| text(*x).to_string());
                                }
                            }
                            let owner = owner_of(n);
                            if owner >= 0 && symbols[owner as usize].s.name == t && symbols[owner as usize].s.kind != "const" {
                                return Ok(false); // recursion, not a dependency
                            }
                            let kind = if n.kind() == "type_identifier" || pk == "nested_type_identifier" {
                                "type"
                            } else {
                                "ref"
                            };
                            add_ref(owner, t, member, kind, line(n));
                            Ok(false)
                        }
                        _ => Ok(true),
                    }
                })?;
            }
        }
        self.imports.extend(new_imports);

        Ok(Parsed {
            directive: self.directive,
            symbols: self.symbols.into_iter().map(|s| s.s).collect(),
            imports: self.imports,
            refs: ref_map.into_values().collect(),
            routes: Vec::new(),
        })
    }
}

/// The function inside forwardRef(...) / memo(...) wrappers, if any.
fn inner_function(node: Option<Node>) -> Option<Node> {
    let node = node?;
    match node.kind() {
        "arrow_function" | "function_expression" | "function" => Some(node),
        "call_expression" => {
            let args = node.child_by_field_name("arguments").map(named_children).unwrap_or_default();
            args.into_iter().find_map(|a| inner_function(Some(a)))
        }
        "parenthesized_expression" | "as_expression" | "satisfies_expression" => inner_function(named_children(node).into_iter().next()),
        _ => None,
    }
}
