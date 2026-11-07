// Skill files: front matter parsing and the size estimate shown on the skills page.
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { estimateTokens, loadSkills } from "../skills.mjs";

test("loadSkills reads front matter, file name and a size estimate, sorted by name", () => {
  const dir = mkdtempSync(join(tmpdir(), "wara-skills-"));
  writeFileSync(join(dir, "b.md"), "---\nname: beta\ndescription: When coding\narea: code\n---\n# Beta\nBody text");
  writeFileSync(join(dir, "a.md"), "# No front matter");
  writeFileSync(join(dir, "notes.txt"), "ignored");
  const skills = loadSkills(dir);
  assert.deepEqual(skills.map((s) => [s.name, s.area, s.file]), [["a", "", "a.md"], ["beta", "code", "b.md"]]);
  assert.equal(skills[1].body, "# Beta\nBody text");
  assert.ok(skills[1].tokens >= 10);
});

test("estimateTokens counts Thai heavier than Latin and rounds to tens", () => {
  assert.equal(estimateTokens("a".repeat(400)), 100);
  assert.equal(estimateTokens("ก".repeat(150)), 100);
  assert.equal(estimateTokens(""), 10);
});

test("a missing skills folder gives an empty list", () => {
  assert.deepEqual(loadSkills(join(tmpdir(), "wara-no-such-dir-xyz")), []);
});
