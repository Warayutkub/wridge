// Whole-project data model: which files are schema sources, in which order, and what gets flagged.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";
import { buildDataModel, dataSources, findEntity } from "../codegraph/data/model.mjs";

const dirs = [];
after(() => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));

function project(files) {
  const root = mkdtempSync(join(tmpdir(), "wara-data-"));
  dirs.push(root);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return { root, paths: Object.keys(files) };
}

test("sources: tests, generated Prisma copies and Prisma's own migrations are not schema sources", () => {
  const s = dataSources(
    ["supabase/migrations/0001_init.sql", "supabase/tests/rls.sql", "prisma/schema.prisma", "lib/generated/prisma/schema.prisma", "prisma/migrations/2026_init/migration.sql", "db/schema.sql", "test/fixtures/old.sql"],
    ["lib/schema/tenant.ts", "test/migration_test.dart", "lib/data/repo.test.ts"],
  );
  assert.deepEqual(s, { supabase: ["supabase/migrations/0001_init.sql"], prisma: ["prisma/schema.prisma"], sql: ["db/schema.sql", "lib/schema/tenant.ts"] });
});

test("database dumps are skipped and reported", () => {
  const { root, paths } = project({
    "database/backup.sql": "-- phpMyAdmin SQL Dump\nCREATE TABLE old_stuff (id int);",
    "db/schema.sql": "create table students (id int primary key);",
  });
  const dm = buildDataModel(root, dataSources(paths), { dialect: "MySQL" });
  const store = dm.stores[0];
  assert.deepEqual(store.skipped, ["database/backup.sql"]);
  assert.deepEqual(Object.keys(store.entities), ["students"]);
  assert.equal(store.label.startsWith("MySQL"), true);
});

test("order: the schema file defines a table even if a migrate route repeats CREATE TABLE IF NOT EXISTS earlier in path order", () => {
  const { root, paths } = project({
    "app/api/migrate/route.ts": "await q(`CREATE TABLE IF NOT EXISTS settings (id INT)`);\nawait q(`ALTER TABLE settings ADD COLUMN IF NOT EXISTS note TEXT`);",
    "lib/schema/tenant.ts": "export const t = { settings: `CREATE TABLE IF NOT EXISTS settings (id INT PRIMARY KEY, \\`key\\` VARCHAR(100))` };",
  });
  const dm = buildDataModel(root, dataSources([], paths));
  const settings = dm.stores[0].entities.settings;
  assert.equal(settings.source.file, "lib/schema/tenant.ts");
  assert.deepEqual(settings.columns.map((c) => c.name), ["id", "key", "note"]);
});

test("order inside one file: CREATE before ALTER (sqflite onUpgrade sits above onCreate)", () => {
  const { root, paths } = project({
    "lib/data/repo.dart": "onUpgrade: (db, o, n) async { await db.execute('ALTER TABLE pockets ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0'); },\nstatic Future<void> create(Database db) async {\n  await db.execute('''\n    CREATE TABLE pockets(\n      id INTEGER PRIMARY KEY AUTOINCREMENT,\n      name TEXT NOT NULL\n    )\n  ''');\n}",
  });
  const pockets = buildDataModel(root, dataSources([], paths)).stores[0].entities.pockets;
  assert.deepEqual(pockets.columns.map((c) => c.name), ["id", "name", "pinned"]);
  assert.equal(pockets.source.line, 4);
});

test("flags: tables only ALTERed (made in a dashboard), indexes on undefined columns, platform tables", () => {
  const { root, paths } = project({
    "supabase/migrations/0001.sql": `create table profiles (id uuid primary key references auth.users(id));
      create index profiles_expiry_idx on profiles (plan_expires_at);
      alter table payments add column note text;
      create policy p on storage.objects for select using (true);`,
  });
  const e = buildDataModel(root, dataSources(paths)).stores[0].entities;
  assert.deepEqual(e.profiles.unknownColumns, ["plan_expires_at"]);
  assert.equal(e.payments.outsideMigrations, true);
  assert.equal(e["storage.objects"].external, true);
});

test("findEntity: by name, by Prisma table name, and by client accessor", () => {
  const dm = { stores: [{ kind: "prisma", entities: { OrderItem: { name: "OrderItem", table: "order_items", columns: [] } } }] };
  for (const q of ["OrderItem", "order_items", "orderItem", "public.order_items"]) assert.equal(findEntity(dm, q)?.ent.name, "OrderItem", q);
  assert.equal(findEntity(dm, "orders"), null);
});
