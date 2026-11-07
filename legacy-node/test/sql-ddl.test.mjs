// SQL DDL reader: statements applied in order give the latest schema.
import assert from "node:assert/strict";
import { test } from "node:test";
import { applySql, emptyModel, splitStatements } from "../codegraph/data/sql-ddl.mjs";

const model = (...files) => {
  const m = emptyModel();
  files.forEach((sql, i) => applySql(m, sql, `m${i + 1}.sql`));
  return m;
};
const cols = (ent) => ent.columns.map((c) => c.name);

test("splits statements without breaking $$ bodies, quotes or comments", () => {
  const parts = splitStatements(`
    -- a comment; with a semicolon
    create table a (id int, note text default 'x;y');
    create function f() returns trigger language plpgsql as $$ begin new.x = 1; return new; end $$;
    /* block; comment */ select 1;`);
  assert.equal(parts.length, 3);
  assert.match(parts[1].sql, /\$\$ begin new\.x = 1; return new; end \$\$/);
  assert.equal(parts[0].line, 3);
});

test("CREATE then ALTER add / drop / rename column, and DROP TABLE removes the table", () => {
  const m = model(
    "create table public.profiles (id uuid primary key references auth.users(id), username citext unique not null, resume_url text);",
    "alter table public.profiles add column plan text not null default 'free', drop column if exists resume_url; alter table profiles rename column username to handle;",
    "create table projects (id uuid primary key); drop table if exists public.projects;",
  );
  assert.deepEqual(cols(m.entities.profiles), ["id", "handle", "plan"]);
  assert.equal(m.entities.profiles.columns[0].ref.table, "auth.users");
  assert.equal(m.entities.projects, undefined);
  assert.deepEqual(m.unparsed, []);
});

test("CREATE TABLE IF NOT EXISTS on an existing table changes nothing (setup scripts repeat the schema)", () => {
  const m = model("create table settings (id int primary key, `key` varchar(100), value text);", "CREATE TABLE IF NOT EXISTS settings (id INT)");
  assert.deepEqual(cols(m.entities.settings), ["id", "key", "value"]);
  assert.equal(m.entities.settings.source.file, "m1.sql");
});

test("MySQL: backticks, ENUM, inline KEY / UNIQUE KEY, AUTO_INCREMENT, table options", () => {
  const m = model(`CREATE TABLE IF NOT EXISTS users (
      id INT AUTO_INCREMENT PRIMARY KEY,
      \`role\` ENUM('admin','teacher') NOT NULL DEFAULT 'teacher',
      phone VARCHAR(20),
      KEY idx_phone (phone),
      UNIQUE KEY uq_role_phone (\`role\`, phone)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`);
  const u = m.entities.users;
  assert.deepEqual(cols(u), ["id", "role", "phone"]);
  assert.equal(u.columns[1].type, "ENUM('admin','teacher')");
  assert.deepEqual(u.indexes.map((i) => [i.name, i.columns, i.unique]), [["idx_phone", ["phone"], false], [null, ["role", "phone"], true]]);
});

test("MySQL charset / engine ALTERs are understood, not reported", () => {
  const m = model("create table attendance (id int);", "ALTER TABLE attendance CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci; ALTER TABLE attendance ENGINE=InnoDB;");
  assert.deepEqual(m.unparsed, []);
});

test("a ${…} placeholder as table name (log messages, dynamic DDL) is ignored, not unparsed", () => {
  const m = model("CREATE TABLE ? failed: ?");
  assert.deepEqual(m.unparsed, []);
  assert.deepEqual(Object.keys(m.entities), []);
});

test("unparsed statements keep the line offset of the string they came from", () => {
  const m = emptyModel();
  applySql(m, "frobnicate the table", "lib/x.ts", 47);
  assert.deepEqual(m.unparsed.map((u) => [u.file, u.line]), [["lib/x.ts", 48]]);
});

test("RLS: enable, policies with command / roles / using, drop policy", () => {
  const m = model(`create table orders (id uuid primary key, profile_id uuid);
    alter table orders enable row level security;
    create policy orders_read_own on public.orders for select to authenticated using (profile_id = auth.uid());
    create policy tmp on orders for delete using (false);
    drop policy if exists tmp on public.orders;`);
  const o = m.entities.orders;
  assert.equal(o.rls, true);
  assert.deepEqual(o.policies.map((p) => [p.name, p.cmd, p.roles, p.using]), [["orders_read_own", "select", "authenticated", "profile_id = auth.uid()"]]);
});

test("indexes, views and functions (what supabase.rpc calls)", () => {
  const m = model(`create table orders (id int, status text);
    create index orders_status_idx on public.orders (status);
    create or replace view open_orders as select * from orders where status = 'open';
    create or replace function redeem_code(c text) returns void language sql as $$ update orders set status = 'paid' $$;`);
  assert.deepEqual(m.entities.orders.indexes.map((i) => i.columns), [["status"]]);
  assert.equal(m.entities.open_orders.kind, "view");
  assert.deepEqual(m.entities.open_orders.reads, ["orders"]);
  assert.equal(m.entities.redeem_code.kind, "function");
});
