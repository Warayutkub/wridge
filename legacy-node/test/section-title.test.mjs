// Section names carry kind and review status: "<Name> · <Kind> · <Status>".
// The parser lives in the plugin; this reads it from the compiled code.js (run `npm run build` first).
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const codeJs = fileURLToPath(new URL("../../code.js", import.meta.url));
const block = existsSync(codeJs) ? /\/\/ <section-title>[^\n]*\n([\s\S]*?)\/\/ <\/section-title>/.exec(readFileSync(codeJs, "utf8"))?.[1] : null;
const parse = block ? new Function(`${block}; return parseSectionTitle;`)() : null;
const skip = parse ? false : "build the plugin first (npm run build in wara-bridge)";

const p = (name) => {
  const { kind, status, prototype } = parse(name);
  return { kind, status, prototype };
};

test("new form: name · kind · status", { skip }, () => {
  assert.deepEqual(p("Checkout · Prototype · In review"), { kind: "prototype", status: "review", prototype: true });
  assert.deepEqual(p("Bakery Pitch · Slides · Draft"), { kind: "slides", status: "draft", prototype: false });
  assert.deepEqual(p("Bakery Storyboard - Storyboard - Approved"), { kind: "storyboard", status: "approved", prototype: false });
  assert.deepEqual(p("Sale · Poster Set · Draft"), { kind: "poster set", status: "draft", prototype: false });
});

test("older forms count as Prototype", { skip }, () => {
  assert.deepEqual(p("Teacher · Approved"), { kind: null, status: "approved", prototype: true });
  assert.deepEqual(p("Bakery Storyboard · Draft"), { kind: null, status: "draft", prototype: true });
  assert.deepEqual(p("Checkout"), { kind: null, status: null, prototype: true });
});

test("a known kind without a status; hyphens inside words are not separators", { skip }, () => {
  assert.deepEqual(p("Bakery Pitch · Slides"), { kind: "slides", status: null, prototype: false });
  assert.deepEqual(p("Sign-up · Draft"), { kind: null, status: "draft", prototype: true });
  assert.deepEqual(p("Sign-up flow · Mobile"), { kind: "mobile", status: null, prototype: true });
  assert.equal(parse("Sign-up · Prototype · Draft").title, "Sign-up");
});
