// Where code touches data: which table/view/function each file reads or writes, and which columns.
// Per access style (detected from the project's packages): supabase-js, raw SQL strings.
import { scanPrisma } from "./prisma.mjs";
import { dartSqlLiterals, parseDml, scanSqflite, sqlLiterals } from "./sql-usage.mjs";

const lineAt = (text, index) => text.slice(0, index).split("\n").length;

// Receivers whose .from() is not a table query.
const NOT_DATA = new Set(["Array", "Buffer", "Uint8Array", "Object", "storage", "Promise", "Set", "Map", "Readable", "Observable", "rxjs"]);

// "id, title, author:profiles(username), items(*)" → { columns: ["id","title"], embeds: ["profiles","items"] }
export function parseSelect(spec) {
  // Collapse nested "( … )" so top-level commas separate the items.
  let flat = spec;
  for (let prev = ""; prev !== flat; ) { prev = flat; flat = flat.replace(/\([^()]*\)/g, "()"); }
  const columns = [];
  const embeds = [];
  for (const raw of flat.split(",")) {
    const s = raw.trim();
    if (!s) continue;
    const embed = /^(?:\w+\s*:\s*)?(\w+)(?:!\w+)?\s*\(\)/.exec(s);
    if (embed) { embeds.push(embed[1]); continue; }
    const col = s.replace(/::\w+$/, "").replace(/^\w+\s*:\s*/, "").replace(/->.*$/, "").trim();
    if (col) columns.push(col);
  }
  return { columns, embeds };
}

const WRITE_OPS = ["insert", "update", "upsert", "delete"];

/** supabase-js: from('t').select('a,b') / .insert / .update / .upsert / .delete, and rpc('fn'). */
export function scanSupabase(text) {
  const refs = [];
  const fromRe = /([\w$]+)\s*\.from\(\s*(['"`])([A-Za-z_][\w.]*)\2\s*\)/g;
  for (const m of text.matchAll(fromRe)) {
    if (NOT_DATA.has(m[1])) continue;
    const start = m.index + m[0].length;
    // The chain that follows, up to the next statement or the next .from().
    let end = text.length;
    const stop = /;|\.from\(|\n\s*\n/g;
    stop.lastIndex = start;
    const s = stop.exec(text);
    if (s) end = s.index;
    const chain = text.slice(start, Math.min(end, start + 1500));
    const op = /^\s*\.\s*(select|insert|update|upsert|delete)\s*\(/.exec(chain)?.[1] ?? /\.\s*(select|insert|update|upsert|delete)\s*\(/.exec(chain)?.[1] ?? "select";
    const ref = { entity: m[3], op: WRITE_OPS.includes(op) ? "write" : "read", detail: op, line: lineAt(text, m.index + m[0].indexOf(".from")) };
    const sel = /\.\s*select\(\s*(['"`])([\s\S]*?)\1/.exec(chain);
    if (sel && !/\$\{/.test(sel[2])) {
      const { columns, embeds } = parseSelect(sel[2].replace(/\s+/g, " "));
      ref.columns = columns.includes("*") ? ["*"] : columns;
      for (const e of embeds) refs.push({ entity: e, op: "read", detail: "embed", line: ref.line, via: m[3] });
    } else if (op === "select" && !sel) ref.columns = ["*"];
    // Writes with an object literal: its keys are the columns written.
    const lit = /\.\s*(?:insert|update|upsert)\(\s*\[?\s*\{/.exec(chain);
    if (lit) {
      const open = lit.index + lit[0].length - 1;
      let depth = 0;
      let body = "";
      for (let i = open; i < chain.length; i++) {
        const ch = chain[i];
        if (ch === "{" || ch === "(" || ch === "[") depth++;
        if (ch === "}" || ch === ")" || ch === "]") depth--;
        if (depth === 1 && i > open) body += ch;
        else if (depth === 1) body += "";
        else if (depth > 1) body += " ";
        if (depth === 0) break;
      }
      const keys = [...body.matchAll(/(?:^|,)\s*(?:["']?([A-Za-z_]\w*)["']?\s*:|([A-Za-z_]\w*)\s*(?=,|$))/g)].map((k) => k[1] ?? k[2]);
      if (keys.length) ref.columns = [...new Set(keys)];
    }
    // Filters show which columns a query depends on (and should be indexed).
    const filters = [...chain.matchAll(/\.\s*(eq|neq|gt|gte|lt|lte|like|ilike|is|in|contains|match|order)\(\s*(['"`])([\w.]+)\2/g)].map((f) => f[3]);
    if (filters.length) ref.filters = [...new Set(filters)];
    refs.push(ref);
  }
  for (const m of text.matchAll(/\.rpc\(\s*(['"`])([A-Za-z_][\w.]*)\1/g)) {
    refs.push({ entity: m[2], op: "call", detail: "rpc", line: lineAt(text, m.index) });
  }
  return refs;
}

/** Which scanners apply to a project, from its packages. */
// Bump when a scanner changes: existing graphs rescan every file for data use once.
export const DATA_SCAN_VERSION = 5;

const SQL_DRIVERS = { mysql2: "MySQL", mysql: "MySQL", mariadb: "MySQL", pg: "Postgres", postgres: "Postgres", "@neondatabase/serverless": "Postgres", "@vercel/postgres": "Postgres", "better-sqlite3": "SQLite", sqlite3: "SQLite", "@libsql/client": "SQLite" };
const SQFLITE = /^\s+(sqflite|sqflite_common_ffi|sqflite_common)\s*:/m;

/** Access styles from package.json and (Flutter) pubspec.yaml text. */
export function dataAccessStyles(pkg, pubspec = "") {
  const deps = { ...pkg?.dependencies, ...pkg?.devDependencies };
  const styles = [];
  if (deps["@supabase/supabase-js"] || deps["@supabase/ssr"] || deps["@supabase/auth-helpers-nextjs"]) styles.push("supabase");
  if (Object.keys(SQL_DRIVERS).some((d) => deps[d])) styles.push("sql");
  if (deps["@prisma/client"] || deps.prisma) styles.push("prisma");
  if (SQFLITE.test(pubspec)) styles.push("sqflite");
  return styles;
}

/** "MySQL" / "Postgres" / "SQLite" from the driver the project uses (labels the plain-SQL store). */
export function sqlDialect(pkg, pubspec = "") {
  const deps = { ...pkg?.dependencies, ...pkg?.devDependencies };
  if (SQFLITE.test(pubspec)) return "SQLite";
  return SQL_DRIVERS[Object.keys(SQL_DRIVERS).find((d) => deps[d])] ?? "SQL";
}

/** Dart files: SQL strings (DDL + raw queries) and sqflite helper calls. */
export function scanDartData(text, styles) {
  const refs = [];
  const lits = dartSqlLiterals(text);
  if (lits.some((l) => l.ddl)) refs.push({ form: "sql", entity: "*", op: "ddl", line: lits.find((l) => l.ddl).line });
  if (styles.includes("sqflite")) {
    for (const l of lits) if (!l.ddl) for (const r of parseDml(l.sql)) refs.push({ ...r, form: "sql", line: l.line + l.lead });
    refs.push(...scanSqflite(text).map((r) => ({ ...r, form: "sqflite" })));
  }
  return refs;
}

export function scanData(text, styles) {
  const refs = [];
  if (styles.includes("supabase")) refs.push(...scanSupabase(text).map((r) => ({ ...r, form: "supabase" })));
  if (styles.includes("prisma")) refs.push(...scanPrisma(text).map((r) => ({ ...r, form: "prisma" })));
  const lits = sqlLiterals(text);
  // Schema DDL inside code (CREATE/ALTER TABLE strings) makes the file a schema source, whatever the driver.
  if (lits.some((l) => l.ddl)) refs.push({ form: "sql", entity: "*", op: "ddl", line: lits.find((l) => l.ddl).line });
  // Raw SQL strings: SQL drivers, and Prisma's $queryRaw`…` / $executeRaw`…`.
  if (styles.includes("sql") || styles.includes("prisma")) {
    for (const l of lits) {
      if (l.ddl) continue;
      for (const r of parseDml(l.sql)) refs.push({ ...r, form: "sql", line: l.line + l.lead });
    }
  }
  return refs;
}
