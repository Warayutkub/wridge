// Tolerant SQL DDL reader (Postgres / Supabase and MySQL dialects). Applies statements in order to a
// data model, so a schema spread over CREATE + later ALTER migrations ends up as its latest state.
// Anything it does not understand is counted, never fatal.

// ---------- splitting ----------

/** SQL text → [{ sql, line }] statements, respecting quotes, $tag$ bodies and comments. */
export function splitStatements(text) {
  const out = [];
  let start = 0;
  let line = 1;
  let startLine = 1;
  let seenCode = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\n") { line++; continue; }
    if (c === "-" && text[i + 1] === "-") { while (i < text.length && text[i] !== "\n") i++; i--; continue; }
    if (c === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      const stop = end < 0 ? text.length : end + 2;
      for (let j = i; j < stop; j++) if (text[j] === "\n") line++;
      i = stop - 1;
      continue;
    }
    if (!seenCode && !/\s/.test(c)) { seenCode = true; startLine = line; start = i; }
    if (c === "'" || c === '"' || c === "`") {
      for (i++; i < text.length && text[i] !== c; i++) {
        if (text[i] === "\n") line++;
        if (c === "'" && text[i] === "\\") i++;
      }
      continue;
    }
    if (c === "$") {
      const m = /^\$[A-Za-z_]*\$/.exec(text.slice(i, i + 40));
      if (m) {
        const end = text.indexOf(m[0], i + m[0].length);
        const stop = end < 0 ? text.length : end + m[0].length;
        for (let j = i; j < stop; j++) if (text[j] === "\n") line++;
        i = stop - 1;
        continue;
      }
    }
    if (c === ";") {
      if (seenCode) out.push({ sql: text.slice(start, i).trim(), line: startLine });
      seenCode = false;
    }
  }
  if (seenCode && text.slice(start).trim()) out.push({ sql: text.slice(start).trim(), line: startLine });
  return out;
}

// Splits on commas that are not inside parentheses or quotes.
export function splitTopLevel(s, sep = ",") {
  const parts = [];
  let depth = 0;
  let cur = "";
  let quote = null;
  for (const ch of s) {
    if (quote) { cur += ch; if (ch === quote) quote = null; continue; }
    if (ch === "'" || ch === '"' || ch === "`") { quote = ch; cur += ch; continue; }
    if (ch === "(") depth++;
    if (ch === ")") depth--;
    if (ch === sep && depth === 0) { parts.push(cur.trim()); cur = ""; continue; }
    cur += ch;
  }
  if (cur.trim()) parts.push(cur.trim());
  return parts;
}

// Strips comments that survived inside a statement (column lists often carry "-- note" lines).
const stripComments = (s) => s.replace(/--[^\n]*/g, " ").replace(/\/\*[\s\S]*?\*\//g, " ");
const unquote = (s) => String(s ?? "").trim().replace(/^[`"\[]|[`"\]]$/g, "");

/** "public.profiles" / `x` / "Users" → { schema, name, key } — public and unqualified share one key. */
export function qualified(raw) {
  const parts = String(raw).trim().split(".").map(unquote);
  const name = parts.pop();
  const schema = parts.pop() ?? null;
  const key = !schema || schema === "public" ? name : `${schema}.${name}`;
  return { schema, name, key };
}

const IDENT = String.raw`(?:"[^"]+"|\`[^\`]+\`|\[[^\]]+\]|[\w$]+)(?:\s*\.\s*(?:"[^"]+"|\`[^\`]+\`|[\w$]+))?`;
const parenList = (s) => splitTopLevel(s).map((c) => unquote(c.replace(/\s+(asc|desc)\b.*$/i, "").replace(/\(.*$/, "")));

// Matching close paren for the "(" at index i.
function closeParen(s, i) {
  let depth = 0;
  let quote = null;
  for (let j = i; j < s.length; j++) {
    const ch = s[j];
    if (quote) { if (ch === quote) quote = null; continue; }
    if (ch === "'" || ch === '"' || ch === "`") { quote = ch; continue; }
    if (ch === "(") depth++;
    else if (ch === ")" && --depth === 0) return j;
  }
  return -1;
}

// ---------- model ----------

export function emptyModel() {
  return { entities: {}, unparsed: [] };
}

function entity(model, key, kind = "table") {
  return (model.entities[key] ??= { name: key, kind, columns: [], indexes: [], fks: [], policies: [], rls: false, source: null, changedIn: [] });
}

function column(ent, def) {
  const m = /^("[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)\s*([\s\S]*)$/.exec(def.trim());
  const name = unquote(m?.[1] ?? def.split(/\s+/)[0]);
  const rest = m?.[2] ?? "";
  const lower = rest.toLowerCase();
  const type = (/^([a-z_][\w ]*?(?:\([^)]*\))?(?:\[\])?)(?=\s|$)/i.exec(rest)?.[1] ?? "").trim()
    .replace(/\s+(not|null|primary|unique|default|references|check|constraint|generated|collate|auto_increment|comment)\b.*$/i, "");
  const col = { name, type };
  if (/\bnot\s+null\b/.test(lower) || /\bprimary\s+key\b/.test(lower)) col.notNull = true;
  if (/\bprimary\s+key\b/.test(lower)) col.pk = true;
  if (/\bunique\b/.test(lower)) col.unique = true;
  const dflt = /\bdefault\s+((?:'[^']*'|\([^)]*\)|[^\s,]+)(?:::[\w\[\]]+)?)/i.exec(rest);
  if (dflt) col.default = dflt[1];
  const ref = new RegExp(`\\breferences\\s+(${IDENT})\\s*(?:\\(([^)]*)\\))?`, "i").exec(rest);
  if (ref) {
    col.ref = { table: qualified(ref[1]).key, column: ref[2] ? unquote(ref[2]) : "id" };
    ent.fks.push({ columns: [name], table: col.ref.table, refColumns: [col.ref.column] });
  }
  return col;
}

function tableConstraint(ent, item) {
  const s = item.replace(/^constraint\s+\S+\s+/i, "");
  let m;
  if ((m = /^primary\s+key\s*\(([^)]*)\)/i.exec(s))) {
    for (const c of parenList(m[1])) {
      const col = ent.columns.find((x) => x.name === c);
      if (col) { col.pk = true; col.notNull = true; }
    }
    return true;
  }
  if ((m = new RegExp(`^foreign\\s+key\\s*\\(([^)]*)\\)\\s*references\\s+(${IDENT})\\s*(?:\\(([^)]*)\\))?`, "i").exec(s))) {
    ent.fks.push({ columns: parenList(m[1]), table: qualified(m[2]).key, refColumns: m[3] ? parenList(m[3]) : ["id"] });
    return true;
  }
  if ((m = /^unique(?:\s+(?:key|index))?\s*(?:[`"\w]+\s*)?\(([^)]*)\)/i.exec(s))) {
    const cols = parenList(m[1]);
    ent.indexes.push({ name: null, columns: cols, unique: true });
    if (cols.length === 1) { const col = ent.columns.find((x) => x.name === cols[0]); if (col) col.unique = true; }
    return true;
  }
  if ((m = /^(?:key|index|fulltext\s+(?:key|index))\s+([`"\w]+)?\s*\(([^)]*)\)/i.exec(s))) {
    ent.indexes.push({ name: m[1] ? unquote(m[1]) : null, columns: parenList(m[2]), unique: false });
    return true;
  }
  return /^(check|exclude)\b/i.test(s);
}

// source = the CREATE statement only; later changes are listed separately, so an entity that is
// only ALTERed (created in a dashboard) stays recognisable.
const touch = (ent, src) => {
  if (ent.changedIn.at(-1) !== src.file && ent.source?.file !== src.file) ent.changedIn.push(src.file);
};

/** Applies one statement; returns false when it was not understood (and not ignorable). */
export function applyStatement(model, stmt, src) {
  const sql = stripComments(stmt).replace(/\s+/g, " ").trim();
  let m;
  // A ${…} placeholder where a table name belongs: a log message or dynamic DDL — nothing to model.
  if (/^(create|alter|drop)\s+table\s+(if\s+(not\s+)?exists\s+)?\?/i.test(sql)) return true;

  // CREATE TABLE
  if ((m = new RegExp(`^create\\s+(?:or\\s+replace\\s+)?(?:(?:global\\s+|local\\s+)?(?:temporary|temp|unlogged)\\s+)?table\\s+(?:if\\s+not\\s+exists\\s+)?(${IDENT})\\s*\\(`, "i").exec(sql))) {
    const open = sql.indexOf("(", m[0].length - 1);
    const close = closeParen(sql, open);
    const key = qualified(m[1]).key;
    // CREATE TABLE IF NOT EXISTS on an existing table does nothing (setup scripts repeat the schema).
    if (/\bif\s+not\s+exists\b/i.test(m[0]) && model.entities[key]?.source) return true;
    const ent = entity(model, key);
    ent.columns = [];
    ent.fks = [];
    ent.indexes = ent.indexes.filter((i) => i.name); // keep separately created indexes
    ent.source = src;
    for (const item of splitTopLevel(sql.slice(open + 1, close))) {
      if (!tableConstraint(ent, item)) ent.columns.push(column(ent, item));
    }
    return true;
  }

  // ALTER TABLE
  if ((m = new RegExp(`^alter\\s+table\\s+(?:if\\s+exists\\s+)?(?:only\\s+)?(${IDENT})\\s+(.*)$`, "i").exec(sql))) {
    const key = qualified(m[1]).key;
    const ent = entity(model, key);
    touch(ent, src);
    let ok = true;
    for (const action of splitTopLevel(m[2])) {
      let a;
      if (/^enable\s+row\s+level\s+security/i.test(action)) ent.rls = true;
      else if (/^disable\s+row\s+level\s+security/i.test(action)) ent.rls = false;
      else if (/^force\s+row\s+level\s+security/i.test(action)) ent.rls = true;
      else if ((a = /^add\s+(?:column\s+)?(?:if\s+not\s+exists\s+)?(.*)$/i.exec(action)) && !/^add\s+(constraint|primary|foreign|unique|index|key|check)\b/i.test(action)) {
        const col = column(ent, a[1]);
        ent.columns = ent.columns.filter((c) => c.name !== col.name).concat(col);
      } else if ((a = /^add\s+(.*)$/i.exec(action))) tableConstraint(ent, a[1]);
      else if ((a = /^drop\s+(?:column\s+)?(?:if\s+exists\s+)?([`"\w]+)/i.exec(action)) && !/^drop\s+(constraint|primary|foreign|index|key|default|not)\b/i.test(action)) {
        const name = unquote(a[1]);
        ent.columns = ent.columns.filter((c) => c.name !== name);
        ent.fks = ent.fks.filter((f) => !f.columns.includes(name));
      } else if ((a = /^rename\s+(?:column\s+)?([`"\w]+)\s+to\s+([`"\w]+)/i.exec(action)) && !/^rename\s+to\b/i.test(action)) {
        const col = ent.columns.find((c) => c.name === unquote(a[1]));
        if (col) col.name = unquote(a[2]);
      } else if ((a = new RegExp(`^rename\\s+to\\s+(${IDENT})`, "i").exec(action))) {
        const next = qualified(a[1]).key;
        delete model.entities[key];
        ent.name = next;
        model.entities[next] = ent;
      } else if ((a = /^(?:alter|modify|change)\s+(?:column\s+)?([`"\w]+)\s+(.*)$/i.exec(action))) {
        const col = ent.columns.find((c) => c.name === unquote(a[1]));
        const rest = a[2];
        if (col) {
          if (/^set\s+not\s+null/i.test(rest)) col.notNull = true;
          else if (/^drop\s+not\s+null/i.test(rest)) delete col.notNull;
          else if (/^(set\s+data\s+)?type\s+/i.test(rest)) col.type = rest.replace(/^(set\s+data\s+)?type\s+/i, "").split(/\s+using\b/i)[0].trim();
          else if (/^set\s+default\s+/i.test(rest)) col.default = rest.replace(/^set\s+default\s+/i, "");
          else if (/^drop\s+default/i.test(rest)) delete col.default;
        }
      } else if (/^(convert\s+to|(default\s+)?(character\s+set|charset|collate)\b|engine\s*=|auto_increment\s*=|comment\s*=|row_format|drop\s+(constraint|default|index|key|foreign|primary)|owner\s+to|set\s|reset\s|replica\s|alter\s+constraint|validate\s+constraint|cluster|inherit|no\s+inherit|attach|detach|enable\s+trigger|disable\s+trigger)/i.test(action)) {
        // no structural effect we track
      } else ok = false;
    }
    return ok;
  }

  // DROP TABLE / VIEW
  if ((m = /^drop\s+(table|view|materialized\s+view)\s+(?:if\s+exists\s+)?(.*?)(?:\s+(?:cascade|restrict))?$/i.exec(sql))) {
    for (const t of splitTopLevel(m[2])) delete model.entities[qualified(t).key];
    return true;
  }

  // CREATE INDEX
  if ((m = new RegExp(`^create\\s+(unique\\s+)?index\\s+(?:concurrently\\s+)?(?:if\\s+not\\s+exists\\s+)?([\\w"\`]+)?\\s*on\\s+(?:only\\s+)?(${IDENT})\\s*(?:using\\s+\\w+\\s*)?\\(`, "i").exec(sql))) {
    const open = sql.indexOf("(", m[0].length - 1);
    const ent = entity(model, qualified(m[3]).key);
    ent.indexes.push({ name: m[2] ? unquote(m[2]) : null, columns: parenList(sql.slice(open + 1, closeParen(sql, open))), unique: Boolean(m[1]) });
    return true;
  }
  if ((m = /^drop\s+index\s+(?:concurrently\s+)?(?:if\s+exists\s+)?([\w."`]+)(?:\s+on\s+([\w."`]+))?/i.exec(sql))) {
    const name = qualified(m[1]).name;
    for (const ent of Object.values(model.entities)) ent.indexes = ent.indexes.filter((i) => i.name !== name);
    return true;
  }

  // RLS policies
  if ((m = new RegExp(`^create\\s+policy\\s+("[^"]+"|[\\w]+)\\s+on\\s+(${IDENT})(.*)$`, "i").exec(sql))) {
    const ent = entity(model, qualified(m[2]).key);
    const rest = m[3];
    const cmd = /\bfor\s+(all|select|insert|update|delete)\b/i.exec(rest)?.[1]?.toLowerCase() ?? "all";
    const roles = /\bto\s+([\w, ]+?)(?=\s+(?:using|with)\b|$)/i.exec(rest)?.[1]?.trim() ?? null;
    const usingIdx = rest.search(/\busing\s*\(/i);
    const check = rest.search(/\bwith\s+check\s*\(/i);
    const expr = (idx) => { if (idx < 0) return null; const open = rest.indexOf("(", idx); return rest.slice(open + 1, closeParen(rest, open)).trim(); };
    ent.policies = ent.policies.filter((p) => p.name !== unquote(m[1]));
    ent.policies.push({ name: unquote(m[1]), cmd, roles, using: expr(usingIdx), check: expr(check), source: src });
    return true;
  }
  if ((m = new RegExp(`^drop\\s+policy\\s+(?:if\\s+exists\\s+)?("[^"]+"|[\\w]+)\\s+on\\s+(${IDENT})`, "i").exec(sql))) {
    const ent = model.entities[qualified(m[2]).key];
    if (ent) ent.policies = ent.policies.filter((p) => p.name !== unquote(m[1]));
    return true;
  }
  if ((m = new RegExp(`^alter\\s+policy\\s+("[^"]+"|[\\w]+)\\s+on\\s+(${IDENT})`, "i").exec(sql))) return true;

  // Views and functions: names only (functions are what supabase.rpc() calls)
  if ((m = new RegExp(`^create\\s+(?:or\\s+replace\\s+)?(?:materialized\\s+)?view\\s+(?:if\\s+not\\s+exists\\s+)?(${IDENT})`, "i").exec(sql))) {
    const ent = entity(model, qualified(m[1]).key, "view");
    ent.kind = "view";
    ent.source = src;
    const from = [...sql.matchAll(new RegExp(`\\b(?:from|join)\\s+(${IDENT})`, "gi"))].map((x) => qualified(x[1]).key);
    ent.reads = [...new Set(from)];
    return true;
  }
  if ((m = new RegExp(`^create\\s+(?:or\\s+replace\\s+)?function\\s+(${IDENT})\\s*\\(`, "i").exec(sql))) {
    const ent = entity(model, qualified(m[1]).key, "function");
    ent.kind = "function";
    ent.source = src;
    const body = stmt;
    ent.reads = [...new Set([...body.matchAll(new RegExp(`\\b(?:from|join|update|into)\\s+(${IDENT})`, "gi"))].map((x) => qualified(x[1]).key))].filter((k) => k !== ent.name);
    return true;
  }
  if ((m = new RegExp(`^drop\\s+function\\s+(?:if\\s+exists\\s+)?(${IDENT})`, "i").exec(sql))) {
    delete model.entities[qualified(m[1]).key];
    return true;
  }

  // Statements with no structural effect on tables/columns.
  if (/^(create|drop|alter)\s+(extension|schema|type|domain|trigger|sequence|role|user|publication|subscription|event\s+trigger|collation|aggregate|operator)\b/i.test(sql)) return true;
  if (/^(grant|revoke|comment\s+on|insert|update|delete|select|set|begin|commit|rollback|do|notify|analyze|vacuum|refresh|truncate|lock|use|reset|start\s+transaction|savepoint|release|call|perform)\b/i.test(sql)) return true;
  if (/^alter\s+(default\s+privileges|function|view|database|sequence|index|role)\b/i.test(sql)) return true;
  return false;
}

/** Applies a whole SQL file; unparsed statements are recorded with their location. */
export function applySql(model, text, file, lineOffset = 0) {
  for (const { sql, line } of splitStatements(text)) {
    const ok = applyStatement(model, sql, { file, line: line + lineOffset });
    if (!ok) model.unparsed.push({ file, line: line + lineOffset, sql: sql.replace(/\s+/g, " ").slice(0, 80) });
  }
  return model;
}
