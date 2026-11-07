// Raw SQL written in code (mysql2 / pg / sqlite …): finds SQL string literals, then which tables and
// columns each statement reads or writes. Also flags files that carry schema DDL (CREATE/ALTER TABLE).

const SQL_START = /^\s*(select|insert|update|delete|replace|with|create|alter|drop)\b/i;
// DDL only where a statement starts (string start or after ";") — prose that mentions "ALTER TABLE" is not DDL.
const DDL = /(^|;)\s*((create|alter)\s+table|create\s+(unique\s+)?index)\b/i;

// Before "/", these mean a regex literal follows (otherwise it is a division).
const REGEX_AFTER = new Set(["", "(", ",", "=", ":", "[", "!", "&", "|", "?", "{", "}", ";", "+", "-", "*", "%", "<", ">", "~", "^", "return", "typeof", "case", "do", "else", "in", "of", "yield", "await", "void", "delete", "throw", "new", "=>"]);

/**
 * String and template literals of JS/TS source: [{ raw, line }]. A small tokenizer, so quotes inside
 * comments ("don't") and regex literals (/[`"']/) never start a string.
 */
export function jsStringLiterals(text) {
  const out = [];
  const n = text.length;
  let i = 0;
  let line = 1;
  let prev = "";
  const countLines = (from, to) => { for (let k = from; k < to; k++) if (text.charCodeAt(k) === 10) line++; };
  // Index just past the template literal starting at `start` (handles ${ … } with nested strings/templates).
  const skipTemplate = (start) => {
    let k = start + 1;
    while (k < n && text[k] !== "`") {
      if (text[k] === "\\") { k += 2; continue; }
      if (text[k] === "$" && text[k + 1] === "{") {
        let depth = 1;
        k += 2;
        while (k < n && depth > 0) {
          const ch = text[k];
          if (ch === "{") depth++;
          else if (ch === "}") depth--;
          else if (ch === "`") { k = skipTemplate(k); continue; }
          else if (ch === "'" || ch === '"') { k++; while (k < n && text[k] !== ch && text[k] !== "\n") k += text[k] === "\\" ? 2 : 1; }
          k++;
        }
        continue;
      }
      k++;
    }
    return k + 1;
  };
  while (i < n) {
    const c = text[i];
    if (c === "\n") { line++; i++; continue; }
    if (c === " " || c === "\t" || c === "\r") { i++; continue; }
    if (c === "/" && text[i + 1] === "/") { while (i < n && text[i] !== "\n") i++; continue; }
    if (c === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      const stop = end < 0 ? n : end + 2;
      countLines(i, stop);
      i = stop;
      continue;
    }
    if (c === "/" && REGEX_AFTER.has(prev)) {
      let inClass = false;
      for (i++; i < n && text[i] !== "\n"; i++) {
        if (text[i] === "\\") { i++; continue; }
        if (text[i] === "[") inClass = true;
        else if (text[i] === "]") inClass = false;
        else if (text[i] === "/" && !inClass) { i++; break; }
      }
      while (i < n && /[a-z]/i.test(text[i])) i++;
      prev = "regex";
      continue;
    }
    if (c === "'" || c === '"' || c === "`") {
      const start = i;
      const startLine = line;
      if (c === "`") i = skipTemplate(i);
      else {
        for (i++; i < n && text[i] !== c && text[i] !== "\n"; i++) if (text[i] === "\\") i++;
        i++;
      }
      countLines(start, Math.min(i, n));
      out.push({ raw: text.slice(start, i), line: startLine });
      prev = "literal";
      continue;
    }
    if (/[A-Za-z_$]/.test(c)) {
      let j = i;
      while (j < n && /[\w$]/.test(text[j])) j++;
      prev = text.slice(i, j);
      i = j;
      continue;
    }
    if (c === "=" && text[i + 1] === ">") { prev = "=>"; i += 2; continue; }
    if (/[0-9.]/.test(c)) { while (i < n && /[\w.]/.test(text[i])) i++; prev = "number"; continue; }
    prev = c;
    i++;
  }
  return out;
}

/** String/template literals of a JS/TS file that look like SQL: [{ sql, line, ddl }]. */
export function sqlLiterals(text) {
  const out = [];
  for (const { raw, line } of jsStringLiterals(text)) {
    const body = raw.slice(1, -1);
    if (!SQL_START.test(body) && !DDL.test(body)) continue;
    // ${…} becomes a placeholder; escaped backticks (MySQL identifiers) are unescaped.
    const sql = body.replace(/\$\{(?:[^{}]|\{[^{}]*\})*\}/g, "?").replace(/\\`/g, "`").replace(/\\n/g, "\n");
    out.push({ sql, line, ddl: DDL.test(sql), lead: body.match(/^\s*/)[0].split("\n").length - 1 });
  }
  return out;
}

/** Dart string literals ('…', "…", '''…''', """…""", r'…') that look like SQL; $x / ${…} become "?". */
export function dartSqlLiterals(text) {
  const out = [];
  const re = /r?'''[\s\S]*?'''|r?"""[\s\S]*?"""|r?'(?:\\.|[^'\\\n])*'|r?"(?:\\.|[^"\\\n])*"/g;
  let lineNo = 1;
  let last = 0;
  for (const m of text.matchAll(re)) {
    const raw = m[0].replace(/^r/, "");
    const q = raw.startsWith("'''") || raw.startsWith('"""') ? 3 : 1;
    const body = raw.slice(q, -q);
    if (!SQL_START.test(body) && !DDL.test(body)) continue;
    for (let i = last; i < m.index; i++) if (text.charCodeAt(i) === 10) lineNo++;
    last = m.index;
    const sql = body.replace(/\$\{[^}]*\}|\$\w+/g, "?").replace(/\\'/g, "'");
    out.push({ sql, line: lineNo, ddl: DDL.test(sql), lead: body.match(/^\s*/)[0].split("\n").length - 1 });
  }
  return out;
}

/** sqflite helpers: db.query('t', columns: [...], where: 'a = ?'), insert/update/delete('t', …). */
export function scanSqflite(text) {
  const refs = [];
  const lineAt = (i) => text.slice(0, i).split("\n").length;
  for (const m of text.matchAll(/\.(query|insert|update|delete)\(\s*(['"])(\w+)\2/g)) {
    // The call's argument list (balanced parentheses, capped).
    let depth = 1;
    let end = m.index + m[0].length;
    for (; end < text.length && end < m.index + 3000 && depth > 0; end++) {
      if (text[end] === "(") depth++;
      else if (text[end] === ")") depth--;
    }
    const args = text.slice(m.index + m[0].length, end - 1);
    const op = m[1] === "query" ? "read" : "write";
    const ref = { entity: m[3], op, detail: m[1], line: lineAt(m.index) };
    const cols = /columns\s*:\s*(?:const\s*)?\[([^\]]*)\]/.exec(args);
    if (cols) ref.columns = [...cols[1].matchAll(/['"](\w+)['"]/g)].map((c) => c[1]);
    else if (m[1] === "query") ref.columns = ["*"];
    const values = m[1] !== "query" && /^\s*,\s*(?:const\s*)?\{([\s\S]*?)\}/.exec(args);
    if (values) {
      const keys = [...values[1].matchAll(/['"](\w+)['"]\s*:/g)].map((k) => k[1]);
      if (keys.length) ref.columns = keys;
    }
    const where = /where\s*:\s*(['"])([\s\S]*?)\1/.exec(args);
    if (where) ref.filters = [...new Set([...where[2].matchAll(/\b([A-Za-z_]\w*)\s*(?:=|<>|!=|<=|>=|<|>|\bIN\b|\bLIKE\b|\bIS\b)/gi)].map((x) => x[1]))];
    refs.push(ref);
  }
  return refs;
}

const KEYWORDS = new Set("where on set left right inner outer full cross join order group limit offset using values select as natural straight_join union having for lock into window returning partition".split(" "));
const ident = (s) => s.replace(/[`"\[\]]/g, "").split(".").pop();

function topLevel(sql, from, toKeywords) {
  // Text after `from` up to the first top-level keyword in toKeywords.
  let depth = 0;
  const lower = sql.toLowerCase();
  for (let i = from; i < sql.length; i++) {
    const ch = sql[i];
    if (ch === "(") depth++;
    else if (ch === ")") depth--;
    else if (depth === 0) {
      for (const k of toKeywords) {
        const w = k.trim();
        if (lower.startsWith(w, i) && /\W/.test(sql[i - 1] ?? " ") && /\W/.test(sql[i + w.length] ?? " ")) return sql.slice(from, i);
      }
    }
  }
  return sql.slice(from);
}

/** One DML statement → [{ entity, op, detail, columns, filters }] (columns null = unknown). */
export function parseDml(raw) {
  const sql = raw.replace(/--[^\n]*/g, " ").replace(/\s+/g, " ").trim();
  const kind = /^(\w+)/.exec(sql)?.[1]?.toLowerCase();
  if (!kind || !["select", "insert", "update", "delete", "replace", "with"].includes(kind)) return [];
  const tables = new Map(); // alias → table
  const order = [];
  const tableRe = /\b(from|join|into|update)\s+((?:`[^`]+`|"[^"]+"|[\w.$]+))(?:\s+(?:as\s+)?(`[^`]+`|[a-z_]\w*))?/gi;
  for (const m of sql.matchAll(tableRe)) {
    if (m[2] === "?" || m[2].startsWith("(") || /^(select|\?)$/i.test(m[2])) continue;
    // "UPDATE" only names a table at the start (not in ON DUPLICATE KEY UPDATE / ON CONFLICT DO UPDATE).
    if (m[1].toLowerCase() === "update" && m.index !== 0) continue;
    const table = ident(m[2]);
    if (!/^[A-Za-z_]\w*$/.test(table) || KEYWORDS.has(table.toLowerCase())) continue;
    const alias = m[3] && !KEYWORDS.has(m[3].toLowerCase()) ? ident(m[3]) : null;
    tables.set(table, table);
    if (alias) tables.set(alias, table);
    if (!order.includes(table)) order.push(table);
  }
  if (!order.length) return [];
  const single = order.length === 1 ? order[0] : null;
  const cols = new Map(order.map((t) => [t, new Set()]));
  const filt = new Map(order.map((t) => [t, new Set()]));
  const unknownCols = new Set();
  const put = (map, q, c) => {
    if (!c || c === "?" || /^\d/.test(c) || KEYWORDS.has(c.toLowerCase()) || /^(null|true|false|and|or|not|is|in|like|between|asc|desc|distinct|count|sum|avg|min|max|now|coalesce|ifnull|case|when|then|else|end|interval|date|current_timestamp)$/i.test(c)) return;
    const t = q ? tables.get(q) : single;
    if (t && map.has(t)) map.get(t).add(c);
    else if (!q) unknownCols.add(c);
  };
  const lower = sql.toLowerCase();
  // SELECT list
  const selAt = lower.indexOf("select ");
  if (selAt >= 0) {
    const list = topLevel(sql, selAt + 7, [" from "]);
    for (const item of list.split(",")) {
      const it = item.trim().replace(/\s+as\s+\w+$/i, "").replace(/^distinct\s+/i, "");
      if (it === "*") { for (const s of cols.values()) s.add("*"); continue; }
      let m;
      if ((m = /^(\w+)\.\*$/.exec(it))) { put(cols, m[1], "*"); continue; }
      for (const q of it.matchAll(/\b(\w+)\.`?(\w+)`?/g)) put(cols, q[1], q[2]);
      if ((m = /^`?(\w+)`?$/.exec(it))) put(cols, null, m[1]);
    }
  }
  // INSERT (a, b) / UPDATE … SET a = ?
  const ins = /\binto\s+[`"\w.]+\s*\(([^)]*)\)/i.exec(sql);
  if (ins) for (const c of ins[1].split(",")) put(cols, null, ident(c.trim()));
  if (kind === "update") {
    const set = topLevel(sql, lower.indexOf(" set ") + 5, [" where ", " order by ", " limit "]);
    for (const m of set.matchAll(/(?:^|,)\s*(?:(\w+)\.)?`?(\w+)`?\s*=/g)) put(cols, m[1], m[2]);
  }
  // WHERE / ON / HAVING conditions
  const conditions = sql.replace(/\bon\s+(duplicate\s+key\s+update|conflict)\b[\s\S]*$/i, "");
  for (const clause of conditions.matchAll(/\b(where|on|having)\b([\s\S]*?)(?=\b(group by|order by|limit|left join|right join|inner join|join|union|having|returning)\b|$)/gi)) {
    for (const m of clause[2].matchAll(/(?:\b(\w+)\.)?`?\b([A-Za-z_]\w*)`?\)?\s*(?:=|<>|!=|<=|>=|<|>|\bin\b|\blike\b|\bis\b|\bbetween\b)/gi)) put(filt, m[1], m[2]);
  }
  const writeTarget = kind === "insert" || kind === "replace" ? /\binto\s+([`"\w.]+)/i.exec(sql)?.[1] : kind === "update" ? /^update\s+([`"\w.]+)/i.exec(sql)?.[1] : kind === "delete" ? /\bfrom\s+([`"\w.]+)/i.exec(sql)?.[1] : null;
  const target = writeTarget ? ident(writeTarget) : null;
  return order.map((t) => {
    const c = [...cols.get(t)];
    const f = [...filt.get(t)];
    return {
      entity: t,
      op: t === target ? "write" : "read",
      detail: t === target ? kind : "select",
      columns: c.length ? c : single ? (kind === "delete" ? null : c.length ? c : null) : null,
      filters: f.length ? f : null,
    };
  });
}
