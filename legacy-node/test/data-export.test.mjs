// The JSON behind the "โครงสร้างข้อมูล" page: schema stores as entity lists, plus code refs with file paths.
import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { openDb, setMeta } from "../codegraph/db.mjs";
import { exportData } from "../codegraph/export.mjs";

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "wara-data-"));
  const db = openDb(join(dir, "code-graph.db"));
  db.prepare("INSERT INTO files (id, path, lang) VALUES (1, 'app/orders/page.tsx', 'ts'), (2, 'supabase/migrations/1.sql', 'sql')").run();
  const add = db.prepare("INSERT INTO data_refs (file_id, form, entity, op, columns, line) VALUES (?, ?, ?, ?, ?, ?)");
  add.run(1, "supabase", "orders", "read", JSON.stringify(["id", "status"]), 12);
  add.run(2, "sql", "orders", "ddl", null, 1);
  return { dir, db };
}

test("stores become entity lists; ddl refs are left out; refs carry the file path", () => {
  const { dir, db } = fixture();
  setMeta(db, "data_model", JSON.stringify({
    stores: [{ kind: "supabase", label: "Supabase (Postgres)", sources: ["supabase/migrations/1.sql"], unparsed: [{ sql: "x" }],
      entities: { orders: { name: "orders", kind: "table", columns: [{ name: "id", pk: true }], fks: [], indexes: [], policies: [], rls: true } } }],
  }));
  const out = exportData(db, dir);
  db.close();
  assert.equal(out.stores.length, 1);
  assert.equal(out.stores[0].unparsed, 1);
  assert.deepEqual(out.stores[0].entities.map((e) => e.name), ["orders"]);
  assert.deepEqual(out.refs, [{ entity: "orders", op: "read", form: "supabase", columns: ["id", "status"], filters: null, line: 12, path: "app/orders/page.tsx" }]);
});

test("a project without a data model gives no stores", () => {
  const { dir, db } = fixture();
  const out = exportData(db, dir);
  db.close();
  assert.deepEqual(out.stores, []);
});
