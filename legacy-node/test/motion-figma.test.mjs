import { test } from "node:test";
import assert from "node:assert/strict";
import { presetsToItems, toEasing, tracksToItems } from "../motion-figma.mjs";

test("from/to track: ms → s, ease on the start key, rotate in radians", () => {
  const [it] = tracksToItems([{ layer: "Box", prop: "rotate", from: 0, to: 90, start: 200, end: 1000, ease: "back" }]);
  assert.deepEqual(it.field, { type: "PROPERTY", name: "ROTATION" });
  assert.deepEqual(it.keys.map((k) => k.t), [0.2, 1]);
  assert.equal(it.keys[1].value.value, Math.PI / 2);
  assert.deepEqual(it.keys[0].easing, { type: "EASE_OUT_BACK" });
  assert.equal(it.keys[1].easing, undefined);
});

test("move and scale split into X/Y fields; fill becomes a colour", () => {
  const items = tracksToItems([{ prop: "move", from: [0, 20], to: [0, 0] }, { prop: "scale", from: 0.5 }, { prop: "fill", from: "#ff0000", to: "#00ff0080" }]);
  assert.deepEqual(items.map((i) => i.field.name ?? i.field.collection), ["TRANSLATION_X", "TRANSLATION_Y", "SCALE_X", "SCALE_Y", "fills"]);
  assert.equal(items[1].keys[0].value.value, 20);
  assert.equal(items[3].keys[1].value.value, 1); // scale rests at 1
  assert.equal(items[4].keys[1].value.value.a, 128 / 255);
});

test("keys form: each ease leaves its key", () => {
  const [it] = tracksToItems([{ prop: "opacity", keys: [[0, 0, "hold"], [300, 1], [600, 0.5]], ease: "out" }]);
  assert.deepEqual(it.keys.map((k) => k.easing?.type), ["HOLD", "EASE_OUT", undefined]);
});

test("easing names, cubic() and errors", () => {
  assert.deepEqual(toEasing("cubic(0.2,0,0,1)"), toEasing("standard"));
  assert.throws(() => toEasing("wobble"), /Unknown ease/);
  assert.throws(() => tracksToItems([{ prop: "skew" }]), /prop "skew"/);
});

test("presets: matched by short name, delay/duration in seconds", () => {
  const [it] = presetsToItems([{ layer: "Box", style: "opacity", delay: 500, duration: 800, stagger: 100 }], [{ styleId: "Opacity", name: "motion.preset_name.opacity" }]);
  assert.equal(it.styleId, "Opacity");
  assert.equal(it.config.duration, 0.8);
  assert.equal(it.config.props.delay, 0.5);
  assert.equal(it.stagger, 0.1);
  assert.throws(() => presetsToItems([{ style: "Wiggle" }], []), /no animation style/);
});

test("motionToSpec: Figma tracks → create_lottie spec (units, ops, axes, eases, scope)", async () => {
  const { motionToSpec } = await import("../motion-figma.mjs");
  const read = {
    root: { id: "1:1", name: "Card" }, timelines: [{ id: "t", duration: 1.5 }],
    animated: [
      { id: "1:1", name: "Card", parentId: "0:1", nth: 1, tracks: [{ field: "OPACITY", base: 1, tracks: [{ op: "SET", keys: [{ t: 0, v: 0 }, { t: 1, v: 1 }] }] }], styles: [] },
      { id: "1:2", name: "Dot", parentId: "1:1", nth: 2, tracks: [
        { field: "ROTATION", base: 0, tracks: [{ op: "SET", keys: [{ t: 0.2, v: 0, ease: "EASE_OUT_BACK" }, { t: 1, v: Math.PI / 2, ease: "cubic(0.5,0,0.5,1)" }] }] },
        { field: "OPACITY", base: 1, tracks: [{ op: "SCALE", keys: [{ t: 0, v: 0, ease: "HOLD" }, { t: 0.8, v: 1 }] }] },
        { field: "TRANSLATION_Y", base: 0, tracks: [{ op: "SET", keys: [{ t: 0, v: 20, ease: "GENTLE" }, { t: 0.5, v: 0 }] }] },
      ], styles: [] },
      { id: "1:3", name: "Inner", parentId: "1:2", nth: 1, tracks: [{ field: "OPACITY", base: 1, tracks: [{ op: "SET", keys: [{ t: 0, v: 0 }, { t: 1, v: 1 }] }] }], styles: [] },
    ],
  };
  const { duration, tracks, warnings } = motionToSpec(read);
  assert.equal(duration, 1500);
  const rot = tracks.find((t) => t.prop === "rotate");
  assert.equal(rot.layer, "Dot_2");
  assert.deepEqual(rot.keys, [[200, 0, "back"], [1000, 90, "cubic(0.5,0,0.5,1)"]]);
  assert.deepEqual(tracks.find((t) => t.prop === "opacity").keys, [[0, 0, "hold"], [800, 1]]);
  assert.deepEqual(tracks.find((t) => t.prop === "move").keys, [[0, [0, 20], "decelerate"], [500, [0, 0]]]);
  assert.ok(warnings.some((w) => /frame itself/.test(w)));
  assert.ok(warnings.some((w) => /Inner.*nested layer/.test(w)));
  assert.ok(warnings.some((w) => /GENTLE/.test(w)));
});

test("Lottie hold ease → h:1 keyframe", async () => {
  const { compileLottie } = await import("../lottie.mjs");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect id="Box" width="10" height="10" fill="#f00"/></svg>`;
  const { lottie } = compileLottie({ svg, duration: 1000, tracks: [{ layer: "Box", prop: "opacity", keys: [[0, 0, "hold"], [500, 1]] }] });
  const op = lottie.layers[0].ks.o;
  assert.equal(op.k[0].h, 1);
});

test("nested layers: their groups are opened in the SVG so they animate on their own", async () => {
  const { motionToSpec } = await import("../motion-figma.mjs");
  const { compileLottie } = await import("../lottie.mjs");
  const read = { root: { id: "F", name: "Scene" }, timelines: [{ duration: 1 }], animated: [
    { id: "F2", name: "Lens L", parentId: "G", nth: 1, path: ["Glasses"], tracks: [{ field: "TRANSLATION_Y", base: 10, tracks: [{ op: "SET", keys: [{ t: 0, v: 50, ease: "EASE_OUT" }, { t: 0.5, v: 0 }] }] }], styles: [] },
  ] };
  const spec = motionToSpec(read);
  assert.deepEqual(spec.split, ["Glasses"]);
  assert.deepEqual(spec.tracks[0].keys, [[0, [0, 50], "out"], [500, [0, 0]]]);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><g id="Glasses"><rect id="Lens L" x="0" y="10" width="40" height="30" fill="#ccc"/><rect id="Lens R" x="60" y="10" width="40" height="30" fill="#ccc"/></g><rect id="Shadow" x="0" y="50" width="100" height="4" fill="#000"/></svg>`;
  const { lottie, summary } = compileLottie({ svg, duration: 1000, tracks: spec.tracks, split: spec.split });
  assert.deepEqual(lottie.layers.map((l) => l.nm).sort(), ["Lens L", "Lens R", "Shadow"]);
  assert.match(summary, /Lens L: move/);
});

test("plugin never reads an instance's main component synchronously (dynamic-page throws)", async () => {
  const { readFileSync } = await import("node:fs");
  const src = readFileSync(new URL("../../figma-plugin/code.ts", import.meta.url), "utf8");
  const sync = src.split("\n").map((l, i) => [i + 1, l]).filter(([, l]) => /\.mainComponent\b/.test(l));
  assert.deepEqual(sync, [], "use await node.getMainComponentAsync() instead");
});

test("move goes to Figma as offsets (no layer position added) and reads back the same", async () => {
  const { motionToSpec } = await import("../motion-figma.mjs");
  const items = tracksToItems([{ layer: "Fold1", prop: "move", keys: [[0, [0, 0]], [1300, [-283, 0]]] }]);
  assert.deepEqual(items.map((i) => [i.field.name, i.relative, i.keys.map((k) => k.value.value)]), [["TRANSLATION_X", undefined, [0, -283]], ["TRANSLATION_Y", undefined, [0, 0]]]);
  const read = { root: { id: "F", name: "Folds" }, timelines: [{ duration: 2 }], animated: [
    { id: "F1", name: "Fold1", parentId: "F", nth: 1, path: [], tracks: [
      { field: "TRANSLATION_X", base: 480, tracks: [{ op: "SET", keys: [{ t: 0, v: 0 }, { t: 1.3, v: -283 }] }] },
    ], styles: [] },
  ] };
  assert.deepEqual(motionToSpec(read).tracks[0].keys, [[0, [0, 0]], [1300, [-283, 0]]]);
});
