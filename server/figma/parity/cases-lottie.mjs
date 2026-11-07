// Parity cases for lottie.mjs: the unit-test fixtures, the repo's own Lottie specs (web/lottie/*.json
// meta.wara.spec) on matching art, edge cases and seeded random paths (arcs, transforms) to catch float drift.
import { readFileSync } from "node:fs";
import { join } from "node:path";

export function lottieCases({ here, repo, rng }) {
  const fx = (f) => readFileSync(join(here, "fixtures", f), "utf8");
  const specOf = (f) => JSON.parse(readFileSync(join(repo, "web/lottie", f), "utf8")).meta.wara.spec;
  const cases = [];
  const add = (fn, ...args) => cases.push({ module: "lottie", fn, args });

  const CHECK = `<svg width="120" height="120" viewBox="0 0 120 120" fill="none" xmlns="http://www.w3.org/2000/svg">
<g clip-path="url(#c)">
<circle id="Circle" cx="60" cy="60" r="50" fill="#22C55E"/>
<path id="Check" d="M38 62l14 14 30-32" stroke="white" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/>
<rect id="Dot" x="10" y="10" width="6" height="6" rx="3" fill="#ffffff"/>
<rect id="Dot_2" x="104" y="10" width="6" height="6" rx="3" fill="#ffffff"/>
</g>
<defs><clipPath id="c"><rect width="120" height="120"/></clipPath></defs>
</svg>`;
  const GRAD = `<svg width="100" height="100" viewBox="0 0 100 100"><defs>
<radialGradient id="ball" cx="0.35" cy="0.3" r="0.7"><stop offset="0" stop-color="#93C5FD"/><stop offset="1" stop-color="#1E3A8A"/></radialGradient>
<linearGradient id="glow" x1="0" y1="0" x2="0" y2="100" gradientUnits="userSpaceOnUse"><stop offset="0%" stop-color="#2563EB" stop-opacity="0.6"/><stop offset="100%" stop-color="#2563EB" stop-opacity="0"/></linearGradient></defs>
<circle id="Ball" cx="50" cy="50" r="20" fill="url(#ball)"/><rect id="Glow" width="100" height="100" fill="url(#glow)"/></svg>`;
  const CARD = `<svg width="40" height="40"><g id="Card"><rect id="Bg" width="40" height="40" fill="#ffffff"/><rect id="Line" x="5" y="5" width="30" height="4" fill="#0000ff"/></g><circle id="Dot" cx="2" cy="2" r="1"/></svg>`;
  const DOTS = `<svg width="40" height="40"><circle id="Dot" cx="5" cy="5" r="2"/><circle id="Dot" cx="15" cy="5" r="2"/><circle id="Dot" cx="25" cy="5" r="2"/></svg>`;
  const GLASSES = `<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><g id="Glasses"><rect id="Lens L" x="0" y="10" width="40" height="30" fill="#ccc"/><rect id="Lens R" x="60" y="10" width="40" height="30" fill="#ccc"/></g><rect id="Shadow" x="0" y="50" width="100" height="4" fill="#000"/></svg>`;
  const BOX = `<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect id="Box" width="10" height="10" fill="#f00"/></svg>`;
  const ODD = `<?xml version="1.0"?><!DOCTYPE svg><!-- a comment <g> --><svg viewBox="0 0 64 48" style="fill:#333"><title>t</title>
<g id="Wrap" opacity="0.5" transform="rotate(30 32 24)"><g data-name="Inner &amp; more" style="stroke: #f00; stroke-width: 2; opacity: .5">
<polyline points="1,2 10,20 30 5 40,40 44" fill="none"/><line x1="0" y1="0" x2="10" y2="10"/><rect x="2" y="2" width="10" height="5" rx="20"/>
<ellipse cx="20" cy="20" rx="10" ry="4" transform="skewX(10) scale(1.5,0.5)"/><path d="M5 5 Q10 0 15 5 T25 5 S35 10 40 5 C45 0 50 10 55 5 Z m3 3 h5 v5 h-5 z" fill-rule="evenodd"/>
<path d="M 1e1 5 L 20,5e0 L30 1.5.5" /><use href="#x"/><text>hi</text><image href="a.png"/>
<rect id="Hidden" width="5" height="5" display="none"/><g filter="url(#f)" mask="url(#m)"><circle r="3" cx="9" cy="9" fill="rgba(10, 20, 30, 50%)"/></g>
<circle r="4" cx="40" cy="30" fill="url(#Missing)" stroke="currentColor" stroke-opacity="0.25"/><circle r="4" cx="50" cy="30" fill="#abc" fill-opacity="0.3"/>
<circle r="4" cx="60" cy="30" fill="#abcd"/><circle r="4" cx="10" cy="40" fill="#12345"/><circle r="4" cx="20" cy="40" fill="rgb(1 2 3 / 0.5)"/>
</g></g></svg>`;
  const SHARED = `<svg width="60" height="20"><defs><linearGradient id="base"><stop offset="20%" style="stop-color:#ff0000;stop-opacity:0.5"/><stop offset="1.2" stop-color="blue"/></linearGradient>
<linearGradient id="ref" href="#base" x1="10%" x2="90%" gradientTransform="rotate(90)"/><radialGradient id="r2" xlink:href="#ref" fx="1"/></defs>
<rect id="A" width="20" height="20" fill="url(#ref)"/><rect id="B" x="20" width="20" height="20" stroke="url(#r2)" stroke-width="3" fill="none"/><rect id="C" x="40" width="20" height="20" fill="url('#base')"/></svg>`;

  // ---- unit-level functions
  for (const d of [
    "M10 10 h20 v20 h-20 z", "M0 0 A10 10 0 0 1 20 0", "M0 0 Q10 10 20 0 T40 0", "M0 0 C 5 5 10 5 15 0 S 25 -5 30 0",
    "m10,10 l5,5 5-5 z m0 20 l3 3", "M1e5 2L3 4", "M10 10 20 20 30 10", "M0 0L10 10ZM20 20", "L5 5 10 10", "M0 0 A0 5 0 0 0 10 0",
    "M0 0 a25 25 -30 1 1 50 -25", "M0 0 A5 5 0 1 0 0 0.0001", "M 10 10 A 30 50 45 0 1 50 60 A 30 50 -45 1 0 90 20",
    "M0,0 l.5.5.5-.5", "M10 10 H 0 V 0 Z", "x M1 1 L2 2", "M0 0 L1", "M5 5 l-1.5e-3 2E2 Z", "M0 0 q5 5 10 0 t10 0 t10 0z",
  ]) add("parsePath", d);
  for (const c of [
    "#fff", "#FFFFFF", "#ff000080", "#abcd", "#12345", "#1234567", "none", "", "  Red ", "transparent", "currentColor",
    "rgba(255,0,0,0.5)", "rgb(10 20 30)", "rgb(50%, 0, 100%)", "rgba(1,2,3,40%)", "url(#Grad)", "url(foo)", "hsl(0,0%,0%)", null,
  ]) add("parseColor", c);
  for (const svg of [CHECK, GRAD, CARD, DOTS, GLASSES, BOX, ODD, SHARED]) add("svgLayers", svg, {});
  add("svgLayers", GLASSES, { split: ["Glasses"] });
  add("svgLayers", ODD, { split: ["wrap", "inner & more"] });
  for (const f of ["logo-intro.svg", "loading.svg", "success.svg", "plugin-connected.svg"]) add("svgLayers", fx(f), {});
  for (const f of ["brand/mark.svg", "brand/mark-dark.svg", "brand/app-icon.svg"]) add("svgLayers", readFileSync(join(repo, f), "utf8"), {});
  add("svgLayers", "<g/>", {});
  add("svgLayers", '<svg><text x="&#X41;">a</text></svg>', {});

  // ---- compileLottie: the unit-test specs
  add("compileLottie", { name: "success", duration: 900, fps: 60, svg: CHECK, tracks: [
    { layer: "Circle", prop: "scale", from: 0, to: 1, start: 0, end: 400, ease: "back" },
    { layer: "Check", prop: "draw", from: 0, to: 1, start: 300, end: 700, ease: "out" },
    { layer: "dot*", prop: "opacity", from: 0, to: 1, start: 500, duration: 200, stagger: 100 },
  ] });
  add("compileLottie", { duration: 600, svg: CHECK, tracks: [{ layer: "Circle", prop: "scale", from: 0, to: 1, end: 600, ease: "spring" }] });
  add("compileLottie", { duration: 500, svg: CHECK, tracks: [{ layer: "tick", prop: "draw", from: 0, to: 1 }] });
  add("compileLottie", { duration: 400, svg: DOTS, tracks: [{ layer: "Dot*", prop: "scale", from: 0, to: 1, ease: "spring" }] });
  add("compileLottie", { duration: 300, svg: GRAD, tracks: [] });
  add("compileLottie", { duration: 100, svg: CARD, tracks: [] });
  add("compileLottie", { svg: BOX, duration: 1000, tracks: [{ layer: "Box", prop: "opacity", keys: [[0, 0, "hold"], [500, 1]] }] });
  add("compileLottie", { svg: GLASSES, duration: 1000, tracks: [{ layer: "Lens L", prop: "move", keys: [[0, [0, 50], "out"], [500, [0, 0]]] }], split: ["Glasses"] });

  // ---- the repo's Lottie files, rebuilt from their recipes on matching art
  for (const [svg, file] of [["logo-intro.svg", "logo-intro.json"], ["loading.svg", "loading.json"], ["success.svg", "success.json"], ["plugin-connected.svg", "plugin-connected.json"]]) {
    add("compileLottie", { ...specOf(file), svg: fx(svg) });
  }
  // the logo intro as a from/to spec: move/scale/opacity/rotate tracks with stagger, every ease
  add("compileLottie", { name: "logo-intro-fromto", duration: 1600, fps: 30, svg: fx("logo-intro.svg"), tracks: [
    { layer: "*Head", prop: "move", from: [0, 30], to: [0, 0], start: 0, end: 420, ease: "emphasized", stagger: 60 },
    { layer: "*Head", prop: "opacity", from: 0, to: 1, duration: 200, ease: "out", stagger: 60 },
    { layer: "*Ear", prop: "rotate", from: -25, to: 0, start: 200, end: 700, ease: "back", stagger: 80 },
    { layer: "*Ear", prop: "scale", from: [0.6, 1.2], to: 1, start: 200, end: 640, ease: "bounce" },
    { layer: "Bridge", prop: "scale", from: [0, 1], start: 400, end: 900, ease: "spring" },
    { layer: "Eye*", prop: "scale", keys: [[700, 0.01, "anticipate"], [900, 1.1, "in-out"], [1000, 1, "hold"], [1300, 1, "ease"], [1360, [1, 0.1], "accelerate"], [1440, 1]], stagger: 40 },
    { layer: "Eye*", prop: "fill", from: "#FBFAF8", to: "#2F6BFF", start: 900, end: 1500, ease: "spring", stagger: 40 },
    { layer: "LeftHead", prop: "fill", keys: [[0, "#111111"], [800, "rgb(47,107,255)", "decelerate"], [1600, "#111"]] },
    { layer: "all", prop: "opacity", keys: [[1500, 1, "linear"], [1600, 0.98]] },
    { layer: "RightEar", prop: "move", from: 12, to: 0, end: 500, ease: [0.1, 0.9, 0.2, 1] },
    { layer: "Bridge", prop: "rotate", from: 0, to: 360, start: 900, end: 1500, ease: "cubic(0.65,0,0.35,1)" },
    { layer: "Bridge", prop: "rotate", from: 0, to: 10, start: 1500, end: 1600, ease: "wobble" },
  ] });
  add("compileLottie", { name: "scaled", width: 140, height: 95, duration: 1000, svg: fx("success.svg"), tracks: [
    { layer: "CheckInk", prop: "draw", from: 0, to: 1, start: 200, end: 700, ease: "standard" },
    { layer: "CheckInk", prop: "stroke", from: "#ffffff", to: "#0f172a", start: 300, end: 900, ease: "bounce" },
    { layer: "Spark*", prop: "move", from: [0, 10], to: [0, -10], start: 100, end: 500, stagger: 50 },
    { layer: "Disc", prop: "scale", from: 0.5, end: 400, ease: "back-in" },
  ] });
  add("compileLottie", { duration: 750, fps: 24, svg: fx("plugin-connected.svg"), split: ["Badge"], tracks: [{ layer: "Badge*", prop: "opacity", from: 0, to: 1 }, { layer: "*", prop: "rotate", to: 5 }] });
  add("compileLottie", { duration: 500, svg: ODD, tracks: [{ layer: "Wrap", prop: "draw", from: 0 }] });
  add("compileLottie", { duration: 500, svg: SHARED, tracks: [{ layer: "B", prop: "stroke", from: "red", to: "blue" }] });
  // errors
  add("compileLottie", { duration: 0, svg: CHECK, tracks: [] });
  add("compileLottie", { duration: 500, svg: '<svg width="10" height="10"><rect width="0" height="0" fill="none"/></svg>', tracks: [] });
  add("compileLottie", { duration: 500, svg: CHECK, tracks: [{ layer: "Circle", prop: "skew", from: 0, to: 1 }] });
  add("compileLottie", { duration: 500, svg: CHECK, tracks: [{ layer: "Circle", prop: "fill", to: "#fff" }] });
  add("compileLottie", { duration: 500, svg: CHECK, tracks: [{ layer: "Circle", prop: "fill", from: "nope", to: "#fff" }] });
  add("compileLottie", { duration: 500, svg: "<html/>", tracks: [] });
  add("compileLottie", { duration: 500, tracks: [] });

  // ---- seeded random art: arcs with rotation, rounded rects, ellipses under transforms (float drift)
  const r = (a, b) => Math.round((a + rng() * (b - a)) * 100) / 100;
  for (let k = 0; k < 12; k++) {
    const shapes = [];
    for (let i = 0; i < 6; i++) {
      const tf = `translate(${r(-20, 20)} ${r(-20, 20)}) rotate(${r(-180, 180)} ${r(0, 100)} ${r(0, 100)}) scale(${r(0.5, 2)} ${r(0.5, 2)}) skewY(${r(-20, 20)})`;
      const kind = i % 3;
      if (kind === 0) shapes.push(`<path id="P${i}" transform="${tf}" d="M${r(0, 100)} ${r(0, 100)} A${r(1, 60)} ${r(1, 60)} ${r(-90, 90)} ${i % 2} ${(i >> 1) % 2} ${r(0, 100)} ${r(0, 100)} a${r(1, 30)} ${r(1, 30)} ${r(0, 360)} 1 0 ${r(-50, 50)} ${r(-50, 50)}z" fill="#${Math.floor(rng() * 0xffffff).toString(16).padStart(6, "0")}" stroke="#000" stroke-width="${r(0.5, 4)}"/>`);
      if (kind === 1) shapes.push(`<rect id="R${i}" transform="${tf}" x="${r(0, 50)}" y="${r(0, 50)}" width="${r(5, 60)}" height="${r(5, 60)}" rx="${r(0, 20)}" ry="${r(0, 20)}" fill="rgba(${Math.floor(rng() * 255)},${Math.floor(rng() * 255)},${Math.floor(rng() * 255)},${r(0, 1)})"/>`);
      if (kind === 2) shapes.push(`<ellipse id="E${i}" transform="${tf}" cx="${r(0, 100)}" cy="${r(0, 100)}" rx="${r(1, 40)}" ry="${r(1, 40)}" fill="url(#g${k})"/>`);
    }
    const svg = `<svg width="${r(50, 300)}" height="${r(50, 300)}" viewBox="${r(-10, 10)} ${r(-10, 10)} ${r(80, 120)} ${r(80, 120)}"><defs><linearGradient id="g${k}" x1="${r(0, 1)}" y1="${r(0, 1)}" x2="${r(0, 1)}" y2="${r(0, 1)}" gradientTransform="rotate(${r(0, 90)})"><stop offset="0" stop-color="#f00"/><stop offset="${r(0, 1)}" stop-color="#0f0" stop-opacity="${r(0, 1)}"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>${shapes.join("")}</svg>`;
    add("compileLottie", { duration: r(200, 3000), fps: [24, 30, 60][k % 3], svg, tracks: [
      { layer: "P*", prop: "rotate", from: r(-90, 0), to: r(0, 90), start: r(0, 200), end: r(300, 900), ease: ["spring", "back", "standard"][k % 3], stagger: r(0, 80) },
      { layer: "R*", prop: "scale", from: [r(0, 1), r(0, 1)], to: [r(1, 2), r(1, 2)], duration: r(100, 800), ease: ["bounce", "in-out", "emphasized"][k % 3] },
      { layer: "E*", prop: "move", from: [r(-40, 40), r(-40, 40)], ease: "spring", stagger: r(0, 100) },
      { layer: "all", prop: "opacity", keys: [[0, r(0, 1)], [r(100, 400), r(0, 1), "out"], [r(500, 900), 1]] },
    ] });
  }
  return cases;
}
