import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { brainGraph, brainSummary, emptyTrash, findNotes, linkTarget, listTrash, parseNote, readNote, restoreNote, trashNote, writeNote } from "../codegraph/brain.mjs";
import { find, overview } from "../codegraph/memory.mjs";

const project = () => mkdtempSync(join(tmpdir(), "wara-brain-"));
const put = (root, rel, text) => { mkdirSync(join(root, "brain", rel, ".."), { recursive: true }); writeFileSync(join(root, "brain", rel), text); };

test("links: notes, other projects, and things outside the brain", () => {
  assert.deepEqual(linkTarget("Nathee"), { kind: "note", target: "Nathee" });
  assert.deepEqual(linkTarget("figma:12:34"), { kind: "figma", target: "12:34" });
  assert.deepEqual(linkTarget("code:src/Boss.lua#attack"), { kind: "code", target: "src/Boss.lua#attack" });
  assert.deepEqual(linkTarget("book-lock:สีหลัก"), { kind: "project", target: "book-lock:สีหลัก" });
});

test("parse: title, Thai tags, links with lines, front-matter relations", () => {
  const n = parseNote("---\ntags: [character, ch1]\naliases:\n  - ครุฑ\nboss_of: \"[[CH1]]\"\n---\n# นที\n\nไกด์ที่จริงคือ [[ครุฑ]] #สปอยล์\n## หัวข้อ #ไม่ใช่แท็ก\nดู [[figma:1:2|เฟรม]]", "characters/nathee.md");
  assert.equal(n.title, "นที");
  assert.equal(n.name, "nathee");
  assert.deepEqual(n.aliases, ["ครุฑ"]);
  assert.deepEqual(n.tags.sort(), ["ch1", "character", "สปอยล์"].sort());
  assert.deepEqual(n.links.map((l) => [l.kind, l.target, l.rel]), [["note", "ครุฑ", null], ["figma", "1:2", null], ["note", "CH1", "boss_of"]]);
  assert.equal(n.links[0].line, 9);
});

test("find returns one line per note; read gives links and backlinks; missing links are flagged", () => {
  const root = project();
  put(root, "characters/nathee.md", "---\ntags: [character]\n---\n# นที\n\nไกด์ใน CH1\nความลับ: เป็นครุฑ จะแปลงร่างตอนบอส [[CH1]]\n[[ยังไม่เขียน]]");
  put(root, "CH1.md", "# CH1\n\nบทแรก [[roblox:ServerScriptService.Boss]]");
  const hits = findNotes(root, "ครุฑ");
  assert.equal(hits.length, 1);
  assert.match(hits[0].line, /ครุฑ/);
  const r = readNote(root, "นที");
  assert.deepEqual(r.links.map((l) => [l.target, Boolean(l.missing)]), [["CH1", false], ["ยังไม่เขียน", true]]);
  assert.deepEqual(readNote(root, "ch1").backlinks, ["นที (nathee)"]);
  assert.equal(brainSummary(root).broken, 1);
  const g = brainGraph(root);
  assert.ok(g.nodes.some((n) => n.kind === "roblox"));
  assert.ok(g.nodes.some((n) => n.kind === "missing" && n.label === "ยังไม่เขียน"));
});

test("index follows edits made outside wara (Obsidian, by hand)", () => {
  const root = project();
  put(root, "a.md", "# A\nold");
  assert.equal(findNotes(root, "new").length, 0);
  writeFileSync(join(root, "brain", "a.md"), "# A\nnew text, longer than before");
  assert.equal(findNotes(root, "new").length, 1);
});

test("write: create refuses to overwrite, append adds, replace keeps the old text in the trash", () => {
  const root = project();
  assert.deepEqual(writeNote(root, "rules/Crime", { text: "ห้ามขโมย", tags: ["rule"] }), { saved: "brain/rules/Crime.md", mode: "create" });
  assert.match(readFileSync(join(root, "brain/rules/Crime.md"), "utf8"), /^---\ntags: \[rule\]\n---\n# Crime\n\nห้ามขโมย/);
  assert.throws(() => writeNote(root, "Crime", { text: "x" }), /already exists/);
  writeNote(root, "Crime", { text: "ห้ามฆ่า", mode: "append" });
  assert.match(readFileSync(join(root, "brain/rules/Crime.md"), "utf8"), /ห้ามขโมย\n\nห้ามฆ่า/);
  writeNote(root, "Crime", { text: "# Crime\nใหม่ [[ไม่มี]]", mode: "replace" });
  assert.equal(listTrash(root).length, 1);
  assert.equal(readFileSync(join(root, "brain/rules/Crime.md"), "utf8"), "# Crime\nใหม่ [[ไม่มี]]\n");
});

test("trash and restore; only empty-trash removes for good", () => {
  const root = project();
  put(root, "a.md", "# A\n[[B]]");
  put(root, "B.md", "# B");
  const t = trashNote(root, "B");
  assert.deepEqual(t.brokenLinksFrom, ["A"]);
  assert.ok(!existsSync(join(root, "brain/B.md")));
  assert.equal(findNotes(root, "B").length, 1); // A still mentions [[B]] — but B itself is gone
  assert.throws(() => readNote(root, "B"), /No note/);
  put(root, "B.md", "# B again");
  assert.throws(() => restoreNote(root, t.restore), /exists again/);
  trashNote(root, "B");
  assert.equal(listTrash(root).length, 2);
  assert.deepEqual(restoreNote(root, "B (2).md"), { restored: "brain/B.md" });
  assert.deepEqual(emptyTrash(root), { removed: 1 });
  assert.equal(listTrash(root).length, 0);
  assert.throws(() => emptyTrash(root, "../a.md"), /Not in the trash/);
});

test("project_memory overview and find include the brain, for a project without code", () => {
  const root = project();
  put(root, "Buki.md", "---\ntags: [mascot]\n---\n# Buki\nมาสคอตสีส้ม");
  assert.match(overview(root), /## Brain \(1 notes in brain\/\)\nTags: mascot 1/);
  assert.match(overview(root, { brief: true }), /Brain/);
  assert.match(find(root, "มาสคอต"), /## Brain notes[\s\S]*- Buki \[mascot\] — มาสคอตสีส้ม/);
});

test("code_graph answers point at brain notes about that code; missing code links are flagged", async () => {
  const { updateGraph, openGraph } = await import("../codegraph/build.mjs");
  const { runQuery } = await import("../codegraph/query.mjs");
  const { notesAbout, codeLinkMissing } = await import("../codegraph/brain.mjs");
  const root = project();
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(join(root, "src/boss.ts"), "export function attack() { return 1 }\nexport const hp = 10\n");
  writeFileSync(join(root, "src/arena.ts"), "import { attack } from './boss'\nexport const go = () => attack()\n");
  put(root, "นที.md", "# นที\n\nบอส CH1 ห้ามหนีออกจากฉาก [[code:src/boss.ts#attack]]\nเก่า [[code:src/gone.ts]] [[code:src/boss.ts#heal]]");
  await updateGraph(root);
  assert.deepEqual(notesAbout(root, { path: "src/boss.ts" }).map((n) => n.note), ["นที"]);
  assert.deepEqual(notesAbout(root, { name: "attack" }).map((n) => n.note), ["นที"]);
  assert.deepEqual(notesAbout(root, { path: "src/arena.ts" }), []);
  assert.equal(codeLinkMissing(root, "src/boss.ts#attack"), false);
  assert.equal(codeLinkMissing(root, "src/gone.ts"), true);
  assert.equal(codeLinkMissing(root, "src/boss.ts#heal"), true);
  const r = readNote(root, "นที");
  assert.deepEqual(r.links.filter((l) => l.missing).map((l) => l.target), ["src/gone.ts", "src/boss.ts#heal"]);
  assert.equal(brainSummary(root).broken, 2);
  const { db } = await openGraph(root);
  try {
    assert.match(runQuery(db, "file", "src/boss.ts", { root }), /## Brain notes linking here \(1\)[\s\S]*- นที — บอส CH1 ห้ามหนีออกจากฉาก code:src\/boss\.ts#attack/);
    assert.match(runQuery(db, "used_by", "attack", { root }), /Brain notes linking here \(1\)/);
    assert.doesNotMatch(runQuery(db, "file", "src/arena.ts", { root }), /Brain notes/);
    assert.match(runQuery(db, "find", "ห้ามหนี", { root }), /## Brain notes \(1\)/);
  } finally {
    db.close();
  }
});

test("a project without brain/ gets no brain index from code_graph find", async () => {
  const root = project();
  assert.deepEqual(findNotes(root, "x"), []);
  assert.equal(existsSync(join(root, ".wara", "brain.db")), false);
});
