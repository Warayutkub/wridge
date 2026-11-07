// TS/TSX/JS → symbols, imports and references, using tree-sitter (web-tree-sitter, WASM — no native build).
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Language, Parser } from "web-tree-sitter";

const MODULES = join(dirname(fileURLToPath(import.meta.url)), "..", "node_modules");
let parsers = null;

async function load() {
  if (parsers) return parsers;
  await Parser.init();
  const make = async (wasm) => {
    const p = new Parser();
    p.setLanguage(await Language.load(join(MODULES, "tree-sitter-typescript", wasm)));
    return p;
  };
  // .ts must use the plain TypeScript grammar: `<T>value` casts are JSX to the TSX grammar.
  parsers = { ts: await make("tree-sitter-typescript.wasm"), tsx: await make("tree-sitter-tsx.wasm") };
  return parsers;
}

export const hash = (text) => createHash("sha1").update(text).digest("hex").slice(0, 12);
const line = (n) => n.startPosition.row + 1;
const unquote = (s) => s.replace(/^['"`]|['"`]$/g, "");
const isPascal = (s) => /^[A-Z][A-Za-z0-9]*$/.test(s);
const JSX = ["jsx_element", "jsx_self_closing_element", "jsx_fragment"];
const WRAPPERS = /(^|\.)(forwardRef|memo|observer|styled\.\w+|lazy)$/;

// Depth-first walk with a cursor (cheaper than node.children on big files). visit() returning false skips the subtree.
function walk(node, visit) {
  const c = node.walk();
  let depth = 0;
  for (;;) {
    const descend = visit(c.currentNode) !== false;
    if (descend && c.gotoFirstChild()) { depth++; continue; }
    for (;;) {
      if (depth === 0) return;
      if (c.gotoNextSibling()) break;
      c.gotoParent();
      depth--;
    }
  }
}

const hasJsx = (node) => node.descendantsOfType(JSX).length > 0;

function fnKind(name, fn) {
  if (/^use[A-Z0-9]/.test(name)) return "hook";
  if (isPascal(name) && fn && hasJsx(fn)) return "component";
  return "function";
}

// First parameter of a component: destructured prop names and/or its type.
function propsOf(fn) {
  if (!fn) return null;
  const params = fn.childForFieldName("parameters") ?? fn.childForFieldName("parameter");
  const first = params?.type === "formal_parameters" ? params.namedChildren.find((n) => /parameter$/.test(n.type)) : params;
  if (!first) return null;
  const pattern = first.childForFieldName?.("pattern") ?? first;
  const type = first.childForFieldName?.("type")?.text.replace(/^:\s*/, "");
  const names = [];
  if (pattern?.type === "object_pattern") {
    for (const p of pattern.namedChildren) {
      if (p.type === "shorthand_property_identifier_pattern") names.push(p.text);
      else if (p.type === "pair_pattern") names.push(p.childForFieldName("key").text);
      else if (p.type === "object_assignment_pattern") names.push(p.childForFieldName("left").text);
      else if (p.type === "rest_pattern") names.push(`...${p.namedChildren[0]?.text ?? ""}`);
    }
  }
  if (!names.length && !type) return null;
  return { names, type: type?.length > 200 ? `${type.slice(0, 200)}…` : type };
}

// The function inside forwardRef(...) / memo(...) wrappers, if any.
function innerFunction(node) {
  if (!node) return null;
  if (["arrow_function", "function_expression", "function"].includes(node.type)) return node;
  if (node.type === "call_expression") {
    const args = node.childForFieldName("arguments")?.namedChildren ?? [];
    for (const a of args) {
      const f = innerFunction(a);
      if (f) return f;
    }
  }
  if (node.type === "parenthesized_expression" || node.type === "as_expression" || node.type === "satisfies_expression") return innerFunction(node.namedChildren[0]);
  return null;
}

export async function parseTs(source, ext) {
  const { ts, tsx } = await load();
  const tree = ([".ts", ".mts", ".cts"].includes(ext) ? ts : tsx).parse(source);
  try {
    return extract(tree.rootNode);
  } finally {
    tree.delete();
  }
}

function extract(root) {
  const symbols = [];   // { name, kind, exported, isDefault, line, endLine, props, hash, node }
  const imports = [];   // { kind, source, imported, local, typeOnly, line }
  const pending = [];   // export markers resolved after every declaration is known
  const declNames = new Set(); // start index of identifiers that declare a top-level name
  let directive = null;

  const addSymbol = (name, kind, node, nameNode, extra = {}) => {
    if (nameNode) declNames.add(nameNode.startIndex);
    symbols.push({ name, kind, exported: 0, isDefault: 0, line: line(node), endLine: node.endPosition.row + 1, props: null, hash: hash(node.text), node, ...extra });
  };

  function declare(decl, exported, isDefault) {
    const before = symbols.length;
    const name = decl.childForFieldName("name");
    switch (decl.type) {
      case "function_declaration":
      case "generator_function_declaration":
      case "function_signature": {
        const kind = fnKind(name?.text ?? "default", decl);
        addSymbol(name?.text ?? "default", kind, decl, name, { props: kind === "component" ? propsOf(decl) : null });
        break;
      }
      case "class_declaration":
      case "abstract_class_declaration":
      case "class": {
        const heritage = decl.namedChildren.find((n) => n.type === "class_heritage")?.text ?? "";
        addSymbol(name?.text ?? "default", /\b(Component|PureComponent)\b/.test(heritage) ? "component" : "class", decl, name);
        break;
      }
      case "lexical_declaration":
      case "variable_declaration":
        for (const d of decl.namedChildren.filter((n) => n.type === "variable_declarator")) {
          const id = d.childForFieldName("name");
          const value = d.childForFieldName("value");
          if (id?.type !== "identifier") {
            // const { a, b } = … — every bound name is a const
            walk(id, (n) => { if (n.type === "shorthand_property_identifier_pattern" || n.type === "identifier") addSymbol(n.text, "const", d, n); });
            continue;
          }
          const fn = innerFunction(value);
          const wrapped = value?.type === "call_expression" && WRAPPERS.test(value.childForFieldName("function")?.text ?? "");
          let kind = "const";
          if (fn && (value === fn || wrapped)) kind = fnKind(id.text, fn);
          else if (wrapped && isPascal(id.text)) kind = "component";
          addSymbol(id.text, kind, d, id, { props: kind === "component" ? propsOf(fn) : null });
        }
        break;
      case "type_alias_declaration":
        addSymbol(name.text, "type", decl, name);
        break;
      case "interface_declaration":
        addSymbol(name.text, "interface", decl, name);
        break;
      case "enum_declaration":
        addSymbol(name.text, "enum", decl, name);
        break;
      default:
        return;
    }
    for (const s of symbols.slice(before)) {
      s.exported = exported ? 1 : 0;
      s.isDefault = isDefault && symbols.length - before === 1 ? 1 : 0;
    }
  }

  function importStatement(st) {
    const source = unquote(st.childForFieldName("source")?.text ?? "");
    const typeOnly = /^import\s+type\b/.test(st.text) ? 1 : 0;
    const clause = st.namedChildren.find((n) => n.type === "import_clause");
    if (!clause) return imports.push({ kind: "side_effect", source, imported: null, local: null, typeOnly, line: line(st) });
    for (const c of clause.namedChildren) {
      if (c.type === "identifier") imports.push({ kind: "import", source, imported: "default", local: c.text, typeOnly, line: line(st) });
      else if (c.type === "namespace_import") imports.push({ kind: "import", source, imported: "*", local: c.namedChildren[0]?.text, typeOnly, line: line(st) });
      else if (c.type === "named_imports") {
        for (const s of c.namedChildren.filter((n) => n.type === "import_specifier")) {
          const name = s.childForFieldName("name").text;
          const alias = s.childForFieldName("alias")?.text ?? name;
          imports.push({ kind: "import", source, imported: name, local: alias, typeOnly: typeOnly || (/^type\s/.test(s.text) ? 1 : 0), line: line(st) });
        }
      }
    }
  }

  function exportStatement(st) {
    const source = st.childForFieldName("source");
    const decl = st.childForFieldName("declaration");
    const value = st.childForFieldName("value");
    const isDefault = st.children.some((c) => c.type === "default");
    const typeOnly = /^export\s+type\b/.test(st.text) ? 1 : 0;
    const clause = st.namedChildren.find((n) => n.type === "export_clause");
    if (source) {
      const src = unquote(source.text);
      const ns = st.namedChildren.find((n) => n.type === "namespace_export");
      if (clause) {
        for (const s of clause.namedChildren.filter((n) => n.type === "export_specifier")) {
          const name = s.childForFieldName("name").text;
          imports.push({ kind: "reexport", source: src, imported: name, local: s.childForFieldName("alias")?.text ?? name, typeOnly, line: line(st) });
        }
      } else if (ns) imports.push({ kind: "reexport", source: src, imported: "*", local: ns.namedChildren.at(-1)?.text, typeOnly, line: line(st) });
      else imports.push({ kind: "reexport_all", source: src, imported: "*", local: null, typeOnly, line: line(st) });
      return;
    }
    if (decl) return declare(decl, true, isDefault);
    if (clause) {
      for (const s of clause.namedChildren.filter((n) => n.type === "export_specifier")) {
        const name = s.childForFieldName("name").text;
        pending.push({ name, alias: s.childForFieldName("alias")?.text ?? name, node: st });
      }
      return;
    }
    if (value) {
      if (value.type === "identifier") return pending.push({ name: value.text, alias: "default", node: st });
      // export default function() {} / memo(Foo) / {...}
      const fn = innerFunction(value);
      const wrapsComponent = value.type === "call_expression" && WRAPPERS.test(value.childForFieldName("function")?.text ?? "");
      const kind = value.type === "class" ? "class" : fn ? fnKind("Default", fn) : wrapsComponent ? "component" : "const";
      addSymbol("default", kind, value, null, { exported: 1, isDefault: 1, props: kind === "component" ? propsOf(fn) : null });
    }
  }

  const statements = root.namedChildren;
  for (const [i, st] of statements.entries()) {
    if (st.type === "comment") continue;
    if (st.type === "expression_statement" && st.namedChildren[0]?.type === "string" && !directive && i < 3) {
      const d = unquote(st.namedChildren[0].text);
      if (d === "use client" || d === "use server") { directive = d; continue; }
    }
    if (st.type === "import_statement") importStatement(st);
    else if (st.type === "export_statement") exportStatement(st);
    else declare(st, false, false);
  }

  // export { a as b } / export default a — point at the declared symbol, or keep an alias row.
  for (const { name, alias, node } of pending) {
    const s = symbols.find((x) => x.name === name && x.kind !== "alias");
    if (s && alias === name) s.exported = 1;
    else if (s && alias === "default") { s.exported = 1; s.isDefault = 1; }
    else if (s) addSymbol(alias, "alias", node, null, { exported: 1, props: { of: name } });
    else {
      // re-export of an imported binding: export { Button } (after import { Button } from …)
      const imp = imports.find((x) => x.local === name && x.kind === "import");
      if (imp) imports.push({ kind: "reexport", source: imp.source, imported: imp.imported, local: alias, typeOnly: imp.typeOnly, line: line(node) });
    }
  }

  // References: every use of an import binding or a top-level name, attributed to the enclosing top-level symbol.
  const watch = new Set([...imports.filter((x) => x.local).map((x) => x.local), ...symbols.map((s) => s.name)]);
  const refMap = new Map();
  const addRef = (owner, local, member, kind, at) => {
    const key = `${owner}|${local}|${member ?? ""}|${kind}`;
    const r = refMap.get(key);
    if (r) r.count++;
    else refMap.set(key, { owner, local, member, kind, line: at, count: 1 });
  };
  const ownerOf = (n) => {
    for (let i = 0; i < symbols.length; i++) {
      const s = symbols[i].node;
      if (n.startIndex >= s.startIndex && n.endIndex <= s.endIndex && symbols[i].kind !== "alias") return i;
    }
    return -1;
  };
  const JSX_NAME_PARENTS = new Set(["jsx_opening_element", "jsx_self_closing_element", "jsx_closing_element"]);

  for (const st of statements) {
    if (st.type === "import_statement" || st.childForFieldName("source")) continue;
    // export { a, b } only lists names; it doesn't use them.
    if (st.type === "export_statement" && !st.childForFieldName("declaration") && !st.childForFieldName("value")) continue;
    walk(st, (n) => {
      switch (n.type) {
        case "jsx_opening_element":
        case "jsx_self_closing_element": {
          const name = n.childForFieldName("name")?.text ?? "";
          const [rootName, ...rest] = name.split(".");
          if (watch.has(rootName)) addRef(ownerOf(n), rootName, rest.join(".") || null, "jsx", line(n));
          return true;
        }
        case "call_expression": {
          const fn = n.childForFieldName("function");
          const arg = n.childForFieldName("arguments")?.namedChildren[0];
          if ((fn?.type === "import" || fn?.text === "require") && arg?.type === "string") {
            imports.push({ kind: fn.type === "import" ? "dynamic" : "import", source: unquote(arg.text), imported: "*", local: null, typeOnly: 0, line: line(n) });
          }
          return true;
        }
        case "identifier":
        case "type_identifier":
        case "shorthand_property_identifier": {
          if (!watch.has(n.text) || declNames.has(n.startIndex)) return false;
          const parent = n.parent;
          if (JSX_NAME_PARENTS.has(parent?.type) || (parent?.type === "member_expression" && JSX_NAME_PARENTS.has(parent.parent?.type))) return false;
          let member = null;
          if (parent?.type === "member_expression" && parent.childForFieldName("object")?.startIndex === n.startIndex) member = parent.childForFieldName("property")?.text ?? null;
          if (parent?.type === "nested_type_identifier") member = parent.namedChildren.at(-1)?.text ?? null;
          const owner = ownerOf(n);
          if (owner >= 0 && symbols[owner].name === n.text && symbols[owner].kind !== "const") return false; // recursion, not a dependency
          addRef(owner, n.text, member, n.type === "type_identifier" || parent?.type === "nested_type_identifier" ? "type" : "ref", line(n));
          return false;
        }
        default:
          return true;
      }
    });
  }

  return {
    directive,
    symbols: symbols.map(({ node, ...s }) => s),
    imports,
    refs: [...refMap.values()],
  };
}
