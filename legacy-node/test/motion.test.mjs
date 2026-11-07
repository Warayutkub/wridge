import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { compileLottie } from "../lottie.mjs";
import { contentHash, listLottie, markSentToFigma, readLottie } from "../motion.mjs";

const SVG = `<svg width="40" height="40" viewBox="0 0 40 40"><circle id="Dot" cx="20" cy="20" r="10" fill="#2563EB"/></svg>`;

function project() {
  const root = mkdtempSync(join(tmpdir(), "wara-motion-"));
  mkdirSync(join(root, "assets/lottie"), { recursive: true });
  mkdirSync(join(root, "node_modules/pkg"), { recursive: true });
  const { lottie } = compileLottie({ name: "pop", duration: 500, svg: SVG, tracks: [{ layer: "Dot", prop: "scale", from: 0, to: 1 }] });
  lottie.meta = { g: "wara-bridge", wara: { spec: { duration: 500, tracks: [] } } };
  writeFileSync(join(root, "assets/lottie/pop.json"), JSON.stringify(lottie));
  writeFileSync(join(root, "node_modules/pkg/anim.json"), JSON.stringify(lottie)); // skipped folder
  writeFileSync(join(root, "assets/data.json"), JSON.stringify({ layers: [], fr: "x", other: "not lottie".repeat(30) }));
  writeFileSync(join(root, "package.json"), "{}");
  return root;
}

test("finds Lottie files by content, skips node_modules and other JSON", () => {
  const root = project();
  try {
    const files = listLottie(root);
    assert.deepEqual(files.map((f) => f.path), ["assets/lottie/pop.json"]);
    assert.equal(files[0].wara, true);
    assert.equal(files[0].durationMs, 500);
    assert.equal(files[0].figma, null);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("paths outside the project are refused", () => {
  const root = project();
  try {
    assert.throws(() => readLottie(root, "../secret.json"), /inside the project/);
    assert.throws(() => readLottie(root, "assets/data.json"), /not a Lottie/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("sent to Figma: remembered, and stale once the animation changes", () => {
  const root = project();
  try {
    const sent = markSentToFigma(root, "assets/lottie/pop.json", { componentId: "1:2", file: "wara-bridge" });
    assert.deepEqual([sent.figma.componentId, sent.figma.stale], ["1:2", false]);
    const path = join(root, "assets/lottie/pop.json");
    const j = JSON.parse(readFileSync(path, "utf8"));
    assert.equal(j.meta.wara.spec.duration, 500, "recipe kept");
    j.op += 6; // the animation changed (e.g. create_lottie again)
    writeFileSync(path, JSON.stringify(j));
    assert.equal(listLottie(root)[0].figma.stale, true);
    assert.notEqual(contentHash(j), j.meta.wara.figma.hash);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
