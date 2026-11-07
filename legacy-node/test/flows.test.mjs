// Flow files (docs/flows/*.md): parse ⇄ format, safe saving, and refs checked against the code graph.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { FlowConflict, checkFlow, formatFlow, hashText, parseFlow, saveFlowFile, slugify } from "../codegraph/flows.mjs";

const SAMPLE = `# Flow: Checkout
entry: screen /checkout

## Steps
1. [user] presses "Pay" on \`/checkout\`
2. [web] POST \`/api/checkout\` → \`createPayment\`
3. [⏳ payment] waits for \`/api/payments/webhook\`
4. [server] \`setPaymentStatus\`
   - paid → step 5a
   - failed → end
5a. [db] writes \`payments, orders\`
5b. [email] sends the receipt
6. [user] sees "Paid"

## States
\`payments.status\`: pending → paid | failed · paid → refunded (admin only)

## Notes
Webhooks can arrive twice.
`;

test("parse reads steps, actors, waits, refs, branches, parallel groups, states and notes", () => {
  const f = parseFlow(SAMPLE);
  assert.equal(f.title, "Checkout");
  assert.equal(f.entry, "screen /checkout");
  assert.deepEqual(f.steps.map((s) => s.id), ["1", "2", "3", "4", "5a", "5b", "6"]);
  assert.equal(f.steps[2].wait, true);
  assert.equal(f.steps[2].actor, "payment");
  assert.deepEqual(f.steps[1].refs, ["/api/checkout", "createPayment"]);
  assert.deepEqual(f.steps[3].branches, [{ label: "paid", to: "5a" }, { label: "failed", to: "end" }]);
  assert.equal(f.steps[4].group, "5");
  assert.equal(f.steps[5].group, "5");
  assert.deepEqual(f.states[0].transitions, [
    { from: "pending", to: "paid", note: null },
    { from: "pending", to: "failed", note: null },
    { from: "paid", to: "refunded", note: "admin only" },
  ]);
  assert.equal(f.notes, "Webhooks can arrive twice.");
});

test("format → parse keeps the flow; reordering renumbers steps and branch targets follow", () => {
  const f = parseFlow(SAMPLE);
  assert.deepEqual(parseFlow(formatFlow(f)).steps, f.steps);
  // Move the email step before the db step: still one parallel group, letters swap.
  const moved = { ...f, steps: [f.steps[0], f.steps[1], f.steps[2], f.steps[3], f.steps[5], f.steps[4], f.steps[6]] };
  const again = parseFlow(formatFlow(moved));
  assert.deepEqual(again.steps.map((s) => [s.id, s.actor]), [["1", "user"], ["2", "web"], ["3", "payment"], ["4", "server"], ["5a", "email"], ["5b", "db"], ["6", "user"]]);
  assert.deepEqual(again.steps[3].branches[0], { label: "paid", to: "5b" }); // still points at the db step
  // Drop a step: later numbers shift down.
  const dropped = parseFlow(formatFlow({ ...f, steps: f.steps.filter((s) => s.id !== "2") }));
  assert.deepEqual(dropped.steps.map((s) => s.id), ["1", "2", "3", "4a", "4b", "5"]);
});

test("saving refuses to overwrite a file that changed since it was opened, and odd names", () => {
  const root = mkdtempSync(join(tmpdir(), "wara-flows-"));
  const first = saveFlowFile(root, "checkout.md", SAMPLE, null);
  assert.equal(first.file, "docs/flows/checkout.md");
  assert.throws(() => saveFlowFile(root, "checkout.md", SAMPLE, null), FlowConflict); // exists already
  writeFileSync(join(root, "docs/flows/checkout.md"), `${SAMPLE}\nedited elsewhere\n`);
  assert.throws(() => saveFlowFile(root, "checkout.md", "mine", first.hash), (e) => e instanceof FlowConflict && e.current.includes("edited elsewhere"));
  const current = readFileSync(join(root, "docs/flows/checkout.md"), "utf8");
  saveFlowFile(root, "checkout.md", "mine", hashText(current));
  assert.equal(readFileSync(join(root, "docs/flows/checkout.md"), "utf8"), "mine");
  assert.throws(() => saveFlowFile(root, "../escape.md", "x", null), /Invalid flow file name/);
  assert.equal(slugify("Sign up / Verify!"), "sign-up-verify");
});

test("refs are checked against routes, symbols, files and tables; missing ones are stale", () => {
  const model = { stores: [{ entities: { payments: { name: "payments", columns: [{ name: "status" }] }, orders: { name: "orders", columns: [] } } }] };
  const file = { id: 1, path: "app/api/checkout/route.ts" };
  const sym = { id: 1, name: "setPaymentStatus", kind: "function", file, line: 4 };
  const g = {
    db: { prepare: () => ({ get: () => ({ value: JSON.stringify(model) }) }) },
    routes: [{ kind: "route", path: "/api/checkout", file_id: 1, file }, { kind: "page", path: "/checkout", file_id: 2, file: { id: 2, path: "app/checkout/page.tsx" } }],
    symbolsOf: new Map([[1, [{ name: "POST", exported: 1 }]]]),
    symbolById: new Map([[1, sym]]),
    fileByPath: (p) => (p === file.path ? file : null),
  };
  const { stale } = checkFlow(g, parseFlow(SAMPLE));
  assert.deepEqual(stale.map((s) => s.ref), ["createPayment", "/api/payments/webhook"]);
});
