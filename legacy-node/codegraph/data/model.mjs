// The project's data model in one shape, whatever stores it: entities (tables/views/functions) with
// columns, keys, indexes, relations and access policies. Each store kind has its own reader.
// Stores: Supabase migrations, Prisma schema, plain SQL (.sql files + CREATE/ALTER strings in TS/Dart code).
import { readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { applySql, emptyModel } from "./sql-ddl.mjs";
import { applyPrisma, prismaProvider } from "./prisma.mjs";
import { dartSqlLiterals, sqlLiterals } from "./sql-usage.mjs";

const SUPABASE_MIGRATION = /(^|\/)supabase\/migrations\/[^/]+\.sql$/;

/**
 * Schema source files per store kind. ddlCodeFiles = code files whose strings hold CREATE/ALTER TABLE
 * (found while scanning, e.g. lib/schema/tenant.ts or a migrate route).
 */
// Tests build old or fake schemas on purpose (migration tests, fixtures): never a schema source.
const isTest = (p) => /(^|\/)(test|tests|__tests__|spec|specs|e2e|fixtures?|integration_test)\//.test(p) || /(_test\.dart|\.(test|spec)\.[cm]?[jt]sx?)$/.test(p);

export function dataSources(allPaths, ddlCodeFiles = []) {
  const paths = allPaths.filter((p) => !isTest(p));
  ddlCodeFiles = ddlCodeFiles.filter((p) => !isTest(p));
  const supabase = paths.filter((p) => SUPABASE_MIGRATION.test(p)).sort();
  // Generated copies (prisma-client output) are not sources.
  const prisma = paths.filter((p) => p.endsWith(".prisma") && !/(^|\/)(generated|node_modules|\.prisma)\//.test(p)).sort();
  const sql = [
    // prisma/migrations/*.sql are generated from schema.prisma: the Prisma store already covers them.
    ...paths.filter((p) => p.endsWith(".sql") && !p.split("/").includes("supabase") && !(prisma.length && /(^|\/)prisma\/migrations\//.test(p))),
    ...ddlCodeFiles.filter((p) => !p.split("/").includes("supabase")),
  ].sort();
  return { supabase, prisma, sql };
}

// Database dumps (phpMyAdmin / mysqldump / pg_dump) are snapshots, not the schema's history: skipped.
const isDump = (text) => /^--\s*(phpMyAdmin SQL Dump|MySQL dump|MariaDB dump|PostgreSQL database dump)/im.test(text.slice(0, 2000)) || (text.match(/^\s*insert\s+into\b/gim)?.length ?? 0) > 50;

/** Cheap fingerprint of every schema source, to rebuild the model only when one changed. */
// Bump when the readers change, so cached models are rebuilt even if no migration changed.
const MODEL_VERSION = 9;

export function sourcesStamp(root, sources) {
  return `v${MODEL_VERSION}|` + Object.entries(sources)
    .flatMap(([kind, files]) => files.map((f) => {
      const st = statSync(join(root, f), { throwIfNoEntry: false });
      return `${kind}:${f}:${st?.mtimeMs ?? 0}:${st?.size ?? 0}`;
    }))
    .join("|");
}

export function buildDataModel(root, sources, { dialect = "SQL" } = {}) {
  const stores = [];
  if (sources.supabase.length) {
    const model = emptyModel();
    for (const f of sources.supabase) applySql(model, readFileSync(join(root, f), "utf8"), f);
    finish(model);
    stores.push({ kind: "supabase", label: "Supabase (Postgres)", sources: sources.supabase, ...model });
  }
  if (sources.prisma.length) {
    const model = emptyModel();
    let provider = null;
    for (const f of sources.prisma) {
      const text = readFileSync(join(root, f), "utf8");
      provider ??= prismaProvider(text);
      applyPrisma(model, text, f);
    }
    finish(model);
    stores.push({ kind: "prisma", label: `Prisma${provider ? ` (${provider})` : ""}`, sources: sources.prisma, ...model });
  }
  if (sources.sql.length) {
    const model = emptyModel();
    const skipped = [];
    // Each source → its SQL chunks (whole .sql file, or each DDL string of a code file with its line).
    const chunks = [];
    for (const f of sources.sql) {
      const text = readFileSync(join(root, f), "utf8");
      if (f.endsWith(".sql")) {
        if (isDump(text)) { skipped.push(f); continue; }
        chunks.push({ f, sql: text, offset: 0 });
      } else {
        const lits = f.endsWith(".dart") ? dartSqlLiterals(text) : sqlLiterals(text);
        for (const l of lits.filter((x) => x.ddl)) chunks.push({ f, sql: l.sql, offset: l.line - 1 });
      }
    }
    // Order: schema files (*.sql, schema/, db…) → other code that creates tables (migrate/setup scripts,
    // whose CREATE TABLE IF NOT EXISTS then adds nothing) → numbered migrations → ALTER-only code.
    const creates = new Set(chunks.filter((c) => /\bcreate\s+table\b/i.test(c.sql)).map((c) => c.f));
    const schemaLike = (f) => f.endsWith(".sql") || /(^|\/)(schemas?|db|database|models?)(\/|[-_.])|schema/i.test(f);
    const rank = (f) => (!creates.has(f) ? 3 : /(^|\/)migrations?\//.test(f) ? 2 : schemaLike(f) ? 0 : 1);
    const files = [...new Set(chunks.map((c) => c.f))].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
    // Within a file, CREATEs before ALTERs (sqflite's onUpgrade often sits above onCreate).
    const isCreate = (c) => /\bcreate\s+table\b/i.test(c.sql);
    for (const f of files) {
      const own = chunks.filter((x) => x.f === f);
      for (const c of [...own.filter(isCreate), ...own.filter((x) => !isCreate(x))]) applySql(model, c.sql, f, c.offset);
    }
    finish(model);
    if (Object.keys(model.entities).length || skipped.length) {
      stores.push({ kind: "sql", label: `${dialect} (schema in code/SQL files)`, sources: files, skipped, ...model });
    }
  }
  return { stores, builtAt: new Date().toISOString() };
}

// Entities only referenced (policies on storage.objects, FKs to auth.users) are platform tables.
function finish(model) {
  for (const ent of Object.values(model.entities)) {
    // Never created here: either a platform table (only referenced) or one made outside migrations
    // (dashboard) that migrations later altered — its columns and RLS state are incomplete.
    if (!ent.source) {
      if (ent.columns.length || ent.changedIn.length) ent.outsideMigrations = true;
      else ent.external = true;
    }
    // Indexes on columns no migration defines = schema drift (changed in a dashboard, not in code).
    const names = new Set(ent.columns.map((c) => c.name));
    if (!ent.external && !ent.outsideMigrations && ent.kind === "table") {
      const missing = [...new Set(ent.indexes.flatMap((i) => i.columns).filter((c) => /^\w+$/.test(c) && !names.has(c)))];
      if (missing.length) ent.unknownColumns = missing;
    }
  }
}

/** Entity by name across stores ("profiles", "public.profiles", "auth.users"). */
export function findEntity(dataModel, name) {
  const key = String(name).replace(/^public\./, "");
  const stores = dataModel?.stores ?? [];
  for (const store of stores) {
    const ent = store.entities[key];
    if (ent) return { store, ent };
  }
  // Prisma: the mapped table name ("users" for model User), or the client accessor ("orderItem").
  const lower = key.toLowerCase();
  for (const store of stores) {
    const ent = Object.values(store.entities).find((e) => e.table === key || e.name.toLowerCase() === lower || e.table?.toLowerCase() === lower);
    if (ent) return { store, ent };
  }
  return null;
}
