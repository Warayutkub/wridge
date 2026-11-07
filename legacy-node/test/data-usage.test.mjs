// Where code touches data: supabase-js, raw SQL strings (TS and Dart), sqflite, Prisma.
import assert from "node:assert/strict";
import { test } from "node:test";
import { applyPrisma, scanPrisma } from "../codegraph/data/prisma.mjs";
import { emptyModel } from "../codegraph/data/sql-ddl.mjs";
import { dartSqlLiterals, parseDml, scanSqflite, sqlLiterals } from "../codegraph/data/sql-usage.mjs";
import { dataAccessStyles, parseSelect, scanSupabase } from "../codegraph/data/usage.mjs";

const pick = (refs, ...keys) => refs.map((r) => Object.fromEntries(keys.map((k) => [k, r[k] ?? null])));

// ---------- supabase-js ----------

test("supabase select list: columns, aliases, casts and embedded tables", () => {
  assert.deepEqual(parseSelect("id, total::int, author:profiles!fk(username, avatar_url), items(*)"), { columns: ["id", "total"], embeds: ["profiles", "items"] });
});

test("supabase calls: read/write, written columns from object literals, filters; Buffer/storage.from are not tables", () => {
  const refs = scanSupabase(`const { data } = await supabase
  .from("orders")
  .update({ status: "paid", paid_at: new Date(), meta: { a: 1 } })
  .eq("id", id);
await supabase.from("cart_items").insert([{ order_id, product_id: p.id, qty }]);
const b = Buffer.from("abc"); supabase.storage.from("avatars").upload(x);
await sb.from("products").select().eq("slug", s);
await supabase.rpc("redeem_code", { c });`);
  assert.deepEqual(pick(refs, "entity", "op", "columns", "filters"), [
    { entity: "orders", op: "write", columns: ["status", "paid_at", "meta"], filters: ["id"] },
    { entity: "cart_items", op: "write", columns: ["order_id", "product_id", "qty"], filters: null },
    { entity: "products", op: "read", columns: ["*"], filters: ["slug"] },
    { entity: "redeem_code", op: "call", columns: null, filters: null },
  ]);
});

// ---------- raw SQL ----------

test("raw SQL: aliases and JOINs attribute columns and filters to the right table", () => {
  const [students, courses] = parseDml("SELECT s.id, s.first_name, c.name AS course FROM students s JOIN courses c ON c.id = s.course_id WHERE s.status = ?");
  assert.deepEqual([students.entity, students.columns, students.filters], ["students", ["id", "first_name"], ["status"]]);
  assert.deepEqual([courses.entity, courses.columns], ["courses", ["name"]]);
});

test("raw SQL: ON DUPLICATE KEY UPDATE is neither a table nor a filter", () => {
  const refs = parseDml("INSERT INTO line_users (line_user_id, display_name) VALUES (?, ?) ON DUPLICATE KEY UPDATE display_name = VALUES(display_name)");
  assert.deepEqual(pick(refs, "entity", "op", "columns", "filters"), [{ entity: "line_users", op: "write", columns: ["line_user_id", "display_name"], filters: null }]);
});

test("raw SQL: a function around a column still counts as a filter (DATE(col) = …)", () => {
  assert.deepEqual(parseDml("SELECT COUNT(*) AS n FROM attendance WHERE DATE(check_in) = CURDATE()")[0].filters, ["check_in"]);
});

test("raw SQL: UPDATE … SET columns and DELETE filters", () => {
  assert.deepEqual(pick(parseDml("UPDATE line_link_tokens SET used = 1 WHERE id = ?"), "op", "columns", "filters"), [{ op: "write", columns: ["used"], filters: ["id"] }]);
  assert.deepEqual(pick(parseDml("DELETE FROM sessions WHERE user_id = ?"), "op", "filters"), [{ op: "write", filters: ["user_id"] }]);
});

test("UI text that starts like SQL yields no table", () => {
  assert.deepEqual(parseDml("Select a course"), []);
});

test("TS literals: ${…} becomes ?, escaped MySQL backticks are unescaped, DDL is flagged", () => {
  const code = "const a = await query(`\n  SELECT id FROM users\n  WHERE role = ${r}`);\nconst t = `CREATE TABLE x (\n  \\`key\\` VARCHAR(10))`;\nconst label = 'Hello';";
  const lits = sqlLiterals(code);
  assert.equal(lits.length, 2);
  assert.match(lits[0].sql, /WHERE role = \?/);
  assert.equal(lits[1].ddl, true);
  assert.match(lits[1].sql, /`key` VARCHAR/);
});

test("TS literals: quotes inside regex literals and comments do not start strings", () => {
  const code = [
    "// don't parse this: it's a comment",
    "const re = /[`\"\\w]+\\s*'/i; const ok = s.match(/'/g);",
    "/* it's \"quoted\" */ const q = `SELECT id FROM users WHERE a = ${x}`;",
    "const r = a / b / c; const msg = 'done';",
  ].join("\n");
  const lits = sqlLiterals(code);
  assert.equal(lits.length, 1);
  assert.match(lits[0].sql, /^SELECT id FROM users/);
  assert.equal(lits[0].line, 3);
});

test("TS literals: prose that mentions ALTER TABLE is not DDL; a real multi-statement string is", () => {
  const lits = sqlLiterals("const a = 'Read: .sql files and CREATE/ALTER TABLE strings in code';\nconst b = `CREATE TABLE x (id int);\nALTER TABLE x ADD y int`;");
  assert.deepEqual(lits.map((l) => l.ddl), [true]);
});

// ---------- Dart / sqflite ----------

test("Dart literals: ''' strings, $x / ${x} interpolation, CREATE INDEX counts as DDL", () => {
  const code = "await db.execute('''\n  CREATE TABLE pockets(\n    id INTEGER PRIMARY KEY\n  )\n''');\nawait db.execute('CREATE INDEX idx_p ON pockets(name)');\nfinal q = 'SELECT * FROM pockets WHERE id = \$id';";
  const lits = dartSqlLiterals(code);
  assert.deepEqual(lits.map((l) => l.ddl), [true, true, false]);
  assert.match(lits[2].sql, /id = \?/);
  assert.equal(lits[0].line, 1);
});

test("sqflite helpers: table, columns, where filters, and map keys for writes", () => {
  const refs = scanSqflite(`final rows = await db.query('categories', columns: ['name'], where: 'kind = ? AND archived = 0');
batch.insert('pockets', {'name': n, 'icon': i});
await _db.update('pockets', values, where: 'id = ?', whereArgs: [id]);`);
  assert.deepEqual(pick(refs, "entity", "op", "columns", "filters"), [
    { entity: "categories", op: "read", columns: ["name"], filters: ["kind", "archived"] },
    { entity: "pockets", op: "write", columns: ["name", "icon"], filters: null },
    { entity: "pockets", op: "write", columns: null, filters: ["id"] },
  ]);
});

// ---------- Prisma ----------

test("Prisma schema: @map / @@map, relations become FKs (not columns), enums, @@index", () => {
  const m = emptyModel();
  applyPrisma(m, `enum Status { draft done }
model User {
  id        String   @id @default(uuid())
  avatarUrl String?  @map("avatar_url")
  projects  Project[]
  @@map("users")
}
model Project {
  id     String @id
  userId String @map("user_id")
  status Status @default(draft)
  user   User   @relation(fields: [userId], references: [id])
  @@index([userId, status])
}`, "prisma/schema.prisma");
  const u = m.entities.User;
  assert.equal(u.table, "users");
  assert.deepEqual(u.columns.map((c) => [c.name, c.db ?? null, Boolean(c.notNull)]), [["id", null, true], ["avatarUrl", "avatar_url", false]]);
  const p = m.entities.Project;
  assert.deepEqual(p.columns.map((c) => c.name), ["id", "userId", "status"]);
  assert.deepEqual(p.fks, [{ columns: ["userId"], table: "User", refColumns: ["id"] }]);
  assert.equal(p.columns[2].type, "Status (enum)");
  assert.deepEqual(p.indexes[0].columns, ["userId", "status"]);
});

test("Prisma client calls: model from accessor, select / data / where keys, include as embedded read", () => {
  const refs = scanPrisma(`await prisma.orderItem.findMany({ where: { orderId: id, NOT: { qty: 0 } }, select: { id: true, qty: true } });
await db.user.create({ data: { email, name: n } });
await prisma.project.findUnique({ where: { id }, include: { scenes: true } });`);
  assert.deepEqual(pick(refs, "entity", "op", "columns", "filters"), [
    { entity: "OrderItem", op: "read", columns: ["id", "qty"], filters: ["orderId"] },
    { entity: "User", op: "write", columns: ["email", "name"], filters: null },
    { entity: "Scene", op: "read", columns: null, filters: null },
    { entity: "Project", op: "read", columns: ["*"], filters: ["id"] },
  ]);
});

// ---------- access styles ----------

test("access styles come from package.json and pubspec.yaml", () => {
  assert.deepEqual(dataAccessStyles({ dependencies: { "@supabase/supabase-js": "2", mysql2: "3", "@prisma/client": "6" } }), ["supabase", "sql", "prisma"]);
  assert.deepEqual(dataAccessStyles(null, "dependencies:\n  flutter:\n    sdk: flutter\n  sqflite: ^2.4.2\n"), ["sqflite"]);
  assert.deepEqual(dataAccessStyles({ dependencies: { next: "16" } }), []);
});
