import assert from "node:assert/strict";
import { test } from "node:test";
import { compileLottie, parseColor, parsePath, svgLayers } from "../lottie.mjs";

const CHECK = `<svg width="120" height="120" viewBox="0 0 120 120" fill="none" xmlns="http://www.w3.org/2000/svg">
<g clip-path="url(#c)">
<circle id="Circle" cx="60" cy="60" r="50" fill="#22C55E"/>
<path id="Check" d="M38 62l14 14 30-32" stroke="white" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/>
<rect id="Dot" x="10" y="10" width="6" height="6" rx="3" fill="#ffffff"/>
<rect id="Dot_2" x="104" y="10" width="6" height="6" rx="3" fill="#ffffff"/>
</g>
<defs><clipPath id="c"><rect width="120" height="120"/></clipPath></defs>
</svg>`;

test("path data: relative lines, arcs and closing", () => {
  const [sp] = parsePath("M10 10 h20 v20 h-20 z");
  assert.equal(sp.closed, true);
  assert.deepEqual(sp.pts, [[10, 10], [30, 10], [30, 30], [10, 30]]);
  assert.equal(sp.segs.length, 4);
  const [arc] = parsePath("M0 0 A10 10 0 0 1 20 0");
  assert.ok(arc.segs.length >= 2, "half circle → 2 cubic pieces");
  assert.deepEqual(arc.pts.at(-1).map((v) => Math.round(v) + 0), [20, 0]);
  const [q] = parsePath("M0 0 Q10 10 20 0 T40 0");
  assert.deepEqual(q.pts.at(-1), [40, 0]);
});

test("colours", () => {
  assert.deepEqual(parseColor("#fff"), [1, 1, 1, 1]);
  assert.deepEqual(parseColor("none"), "none");
  assert.deepEqual(parseColor("rgba(255,0,0,0.5)"), [1, 0, 0, 0.5]);
});

test("Figma-style SVG: wrapping group unwrapped, layers named by id", () => {
  const { layers, warnings, width } = svgLayers(CHECK);
  assert.equal(width, 120);
  assert.deepEqual(layers.map((l) => l.name), ["Circle", "Check", "Dot", "Dot_2"]);
  assert.ok(warnings.some((w) => /clip-path/.test(w)));
  assert.equal(layers[1].items[0].fill, null, "fill=none inherited from <svg>");
  assert.deepEqual(layers[1].items[0].stroke, [1, 1, 1, 1]);
});

test("compile: tracks become keyframes on the right layers", () => {
  const { lottie, summary, warnings } = compileLottie({
    name: "success", duration: 900, fps: 60, svg: CHECK,
    tracks: [
      { layer: "Circle", prop: "scale", from: 0, to: 1, start: 0, end: 400, ease: "back" },
      { layer: "Check", prop: "draw", from: 0, to: 1, start: 300, end: 700, ease: "out" },
      { layer: "dot*", prop: "opacity", from: 0, to: 1, start: 500, duration: 200, stagger: 100 },
    ],
  });
  assert.deepEqual(warnings.filter((w) => !/clip-path/.test(w)), []);
  assert.equal(lottie.op, 54);
  assert.equal(lottie.layers.length, 4);
  const byName = Object.fromEntries(lottie.layers.map((l) => [l.nm, l]));
  // layer order: last SVG element on top = first Lottie layer
  assert.equal(lottie.layers[0].nm, "Dot_2");
  const scale = byName.Circle.ks.s;
  assert.equal(scale.a, 1);
  assert.deepEqual(scale.k[0].s, [0, 0, 100]);
  assert.deepEqual(scale.k[1].s, [100, 100, 100]);
  assert.equal(scale.k[1].t, 24);
  assert.deepEqual(scale.k[0].o.y, [1.56], "back ease overshoots");
  // pivot = centre of the art
  assert.deepEqual(byName.Circle.ks.p.k, [60, 60, 0]);
  const trim = byName.Check.shapes[0].it.find((x) => x.ty === "tm");
  assert.equal(trim.e.k[0].t, 18);
  assert.equal(byName.Dot_2.ks.o.k[0].t, 36, "stagger: 2nd dot starts 100ms later");
  assert.match(summary, /Circle: scale/);
});

test("spring is sampled into keys that overshoot and settle on the target", () => {
  const { lottie } = compileLottie({ duration: 600, svg: CHECK, tracks: [{ layer: "Circle", prop: "scale", from: 0, to: 1, end: 600, ease: "spring" }] });
  const keys = lottie.layers.find((l) => l.nm === "Circle").ks.s.k;
  assert.ok(keys.length > 5);
  assert.ok(Math.max(...keys.map((k) => k.s[0])) > 100, "overshoots");
  assert.deepEqual(keys.at(-1).s, [100, 100, 100]);
});

test("unknown layer names are reported with the real ones", () => {
  const { warnings } = compileLottie({ duration: 500, svg: CHECK, tracks: [{ layer: "tick", prop: "draw", from: 0, to: 1 }] });
  assert.ok(warnings.some((w) => /no layer named "tick".*Circle, Check/.test(w)));
});

test("layers with the same name get _2, _3 and all reach a glob track; spring adds no ease warning", () => {
  const svg = `<svg width="40" height="40"><circle id="Dot" cx="5" cy="5" r="2"/><circle id="Dot" cx="15" cy="5" r="2"/><circle id="Dot" cx="25" cy="5" r="2"/></svg>`;
  const { lottie, warnings } = compileLottie({ duration: 400, svg, tracks: [{ layer: "Dot*", prop: "scale", from: 0, to: 1, ease: "spring" }] });
  assert.deepEqual(lottie.layers.map((l) => l.nm).reverse(), ["Dot", "Dot_2", "Dot_3"]);
  assert.ok(lottie.layers.every((l) => l.ks.s.a === 1));
  assert.deepEqual(warnings, []);
});

test("gradients: radial and linear fills keep their stops (with opacity) in the art's space", () => {
  const svg = `<svg width="100" height="100" viewBox="0 0 100 100"><defs>
<radialGradient id="ball" cx="0.35" cy="0.3" r="0.7"><stop offset="0" stop-color="#93C5FD"/><stop offset="1" stop-color="#1E3A8A"/></radialGradient>
<linearGradient id="glow" x1="0" y1="0" x2="0" y2="100" gradientUnits="userSpaceOnUse"><stop offset="0%" stop-color="#2563EB" stop-opacity="0.6"/><stop offset="100%" stop-color="#2563EB" stop-opacity="0"/></linearGradient></defs>
<circle id="Ball" cx="50" cy="50" r="20" fill="url(#ball)"/><rect id="Glow" width="100" height="100" fill="url(#glow)"/></svg>`;
  const { lottie, warnings } = compileLottie({ duration: 300, svg, tracks: [] });
  assert.deepEqual(warnings, []);
  const ball = lottie.layers.find((l) => l.nm === "Ball").shapes[0].it.find((x) => x.ty === "gf");
  assert.equal(ball.t, 2);
  assert.equal(ball.g.p, 2);
  assert.deepEqual(ball.s.k, [-6, -8], "centre at 35%/30% of the ball's box, relative to its pivot");
  const glow = lottie.layers.find((l) => l.nm === "Glow").shapes[0].it.find((x) => x.ty === "gf");
  assert.equal(glow.t, 1);
  assert.equal(glow.g.k.k.length, 8 + 4, "colour stops then opacity stops");
  assert.deepEqual(glow.g.k.k.slice(8), [0, 0.6, 1, 0]);
});

test("inside a layer, later SVG shapes stay on top (Lottie lists the top group first)", () => {
  const svg = `<svg width="40" height="40"><g id="Card"><rect id="Bg" width="40" height="40" fill="#ffffff"/><rect id="Line" x="5" y="5" width="30" height="4" fill="#0000ff"/></g><circle id="Dot" cx="2" cy="2" r="1"/></svg>`;
  const { lottie } = compileLottie({ duration: 100, svg, tracks: [] });
  const fills = lottie.layers.find((l) => l.nm === "Card").shapes.map((g) => g.it.find((x) => x.ty === "fl").c.k);
  assert.deepEqual(fills, [[0, 0, 1, 1], [1, 1, 1, 1]]);
});
