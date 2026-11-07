// Prisma: schema.prisma → data model, and prisma.<model>.<operation>({ select, where, data … }) → data refs.

const blocks = (text, kind) => [...text.matchAll(new RegExp(`^\\s*${kind}\\s+(\\w+)\\s*\\{([\\s\\S]*?)^\\s*\\}`, "gm"))];
const lineAt = (text, index) => text.slice(0, index).split("\n").length;
const listArg = (s) => (s ?? "").split(",").map((x) => x.trim().replace(/\(.*$/, "")).filter(Boolean);

/** Applies one .prisma file to a model (entities keyed by model name; table = @@map name). */
export function applyPrisma(model, text, file) {
  const clean = text.replace(/\/\/[^\n]*/g, (c) => " ".repeat(c.length));
  const models = blocks(clean, "model");
  const enums = new Set(blocks(clean, "enum").map((m) => m[1]));
  const names = new Set(models.map((m) => m[1]));
  for (const m of models) {
    const ent = { name: m[1], kind: "table", table: m[1], columns: [], indexes: [], fks: [], policies: [], rls: false, source: { file, line: lineAt(clean, m.index + m[0].indexOf("model")) }, changedIn: [] };
    for (const raw of m[2].split("\n")) {
      const line = raw.trim();
      if (!line) continue;
      let a;
      if ((a = /^@@map\(\s*"([^"]+)"/.exec(line))) { ent.table = a[1]; continue; }
      if ((a = /^@@(index|unique|id)\(\s*(?:fields:\s*)?\[([^\]]*)\]/.exec(line))) {
        const cols = listArg(a[2]);
        if (a[1] === "id") for (const c of ent.columns.filter((x) => cols.includes(x.name))) { c.pk = true; c.notNull = true; }
        else ent.indexes.push({ name: null, columns: cols, unique: a[1] === "unique" });
        continue;
      }
      if (line.startsWith("@@")) continue;
      const f = /^(\w+)\s+(\w+)(\[\])?(\?)?\s*(.*)$/.exec(line);
      if (!f) continue;
      const [, field, type, list, optional, attrs] = f;
      if (names.has(type)) {
        // Relation field: not a column; the FK lives in @relation(fields: […], references: […]).
        const rel = /@relation\([^)]*fields:\s*\[([^\]]*)\][^)]*references:\s*\[([^\]]*)\]/.exec(attrs);
        if (rel) ent.fks.push({ columns: listArg(rel[1]), table: type, refColumns: listArg(rel[2]) });
        continue;
      }
      const col = { name: field, type: `${type}${list ? "[]" : ""}${enums.has(type) ? " (enum)" : ""}` };
      if (!optional) col.notNull = true;
      if (/@id\b/.test(attrs)) col.pk = true;
      if (/@unique\b/.test(attrs)) col.unique = true;
      const dflt = /@default\(((?:[^()]|\([^()]*\))*)\)/.exec(attrs);
      if (dflt) col.default = dflt[1];
      const map = /@map\(\s*"([^"]+)"/.exec(attrs);
      if (map) col.db = map[1];
      ent.columns.push(col);
    }
    // FK columns point at the referenced model's columns.
    for (const fk of ent.fks) for (const c of ent.columns.filter((x) => fk.columns.includes(x.name))) c.ref = { table: fk.table, column: fk.refColumns[0] };
    model.entities[ent.name] = ent;
  }
  return model;
}

export const prismaProvider = (text) => /datasource\s+\w+\s*\{[^}]*provider\s*=\s*"(\w+)"/.exec(text)?.[1] ?? null;

const OPS = {
  findMany: "read", findUnique: "read", findUniqueOrThrow: "read", findFirst: "read", findFirstOrThrow: "read",
  count: "read", aggregate: "read", groupBy: "read",
  create: "write", createMany: "write", createManyAndReturn: "write", update: "write", updateMany: "write",
  updateManyAndReturn: "write", upsert: "write", delete: "write", deleteMany: "write",
};

// Top-level keys of the object literal that follows `key:` inside an argument object.
function keysOf(arg, key) {
  const at = new RegExp(`\\b${key}\\s*:\\s*\\{`).exec(arg);
  if (!at) return null;
  let depth = 0;
  let body = "";
  for (let i = at.index + at[0].length - 1; i < arg.length; i++) {
    const ch = arg[i];
    if ("{[(".includes(ch)) depth++;
    if ("}])".includes(ch)) depth--;
    if (depth === 0) break;
    body += depth === 1 ? ch : " ";
  }
  const keys = [...body.slice(1).matchAll(/(?:^|,)\s*(?:["']?([A-Za-z_]\w*)["']?\s*:|([A-Za-z_]\w*)\s*(?=,|$))/g)].map((k) => k[1] ?? k[2]);
  return keys.filter((k) => !["AND", "OR", "NOT"].includes(k));
}

// Relation field → model name: "scenes" → "Scene", "categories" → "Category", "author" → "Author".
// A guess: a relation named unlike its model (author → User) stays unmatched rather than wrong.
function modelOf(field) {
  const one = field.endsWith("ies") ? `${field.slice(0, -3)}y` : /(ss|us)$/.test(field) ? field : field.replace(/s$/, "");
  return one[0].toUpperCase() + one.slice(1);
}

/** prisma.user.findMany({ select: { id: true }, where: { email } }) → refs (entity = model name "User"). */
export function scanPrisma(text) {
  const refs = [];
  const re = new RegExp(`\\.([a-z]\\w*)\\s*\\.\\s*(${Object.keys(OPS).join("|")})\\(`, "g");
  for (const m of text.matchAll(re)) {
    const entity = m[1][0].toUpperCase() + m[1].slice(1);
    const start = m.index + m[0].length;
    let depth = 1;
    let end = start;
    for (; end < text.length && end < start + 4000 && depth > 0; end++) {
      if (text[end] === "(") depth++;
      else if (text[end] === ")") depth--;
    }
    const arg = text.slice(start, end - 1);
    const op = OPS[m[2]];
    const ref = { entity, op, detail: m[2], line: lineAt(text, m.index) };
    const cols = keysOf(arg, "select") ?? (op === "write" ? keysOf(arg, "data") : null);
    if (cols?.length) ref.columns = cols;
    // include adds relations but still returns every scalar column; only select narrows them.
    else if (op === "read" && !/\bselect\s*:/.test(arg) && m[2] !== "count") ref.columns = ["*"];
    const where = keysOf(arg, "where");
    if (where?.length) ref.filters = where;
    for (const inc of keysOf(arg, "include") ?? []) refs.push({ entity: modelOf(inc), op: "read", detail: "include", line: ref.line, via: entity });
    refs.push(ref);
  }
  return refs;
}
