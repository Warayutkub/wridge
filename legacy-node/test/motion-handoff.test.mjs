import { test } from "node:test";
import assert from "node:assert/strict";
import { checkText, classify, lint, toCss, toFramer, tokens } from "../motion-handoff.mjs";

const vec = { text: false, instances: 0, autoLayout: false, vectorOnly: true, image: false, component: null };
const ui = { ...vec, vectorOnly: false, autoLayout: true, text: true };
const key = (t, v, ease) => ({ t, v, ...(ease ? { ease } : {}) });
const field = (name, keys, op = "SET", base = 1) => ({ field: name, base, tracks: [{ op, keys }] });

// an illustration: ring scales (simple), check draws (path) → the whole art is one Lottie
const art = {
  root: { id: "A", name: "Success" }, timelines: [{ duration: 1.2 }],
  animated: [
    { id: "A1", name: "Ring", parentId: "A", nth: 1, hint: vec, styles: [], tracks: [field("SCALE_X", [key(0, 0, "BOUNCY"), key(0.5, 1)]), field("SCALE_Y", [key(0, 0, "BOUNCY"), key(0.5, 1)])] },
    { id: "A2", name: "Check", parentId: "A", nth: 1, hint: vec, styles: [], tracks: [field("PATH_TRIM_END", [key(0.3, 0, "cubic(0.2,0,0,1)"), key(0.7, 1)])] },
  ],
};
// a screen: title + list card slide/fade in, a Lottie/ component, a slow card that animates width
const screen = {
  root: { id: "S", name: "Home" }, timelines: [{ duration: 2 }],
  animated: [
    { id: "S1", name: "Title", parentId: "S", nth: 1, hint: ui, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.2, 1)]), field("TRANSLATION_Y", [key(0, 16, "EASE_OUT"), key(0.2, 0)], "SET", 24)] },
    { id: "S2", name: "Card", parentId: "S", nth: 1, hint: { ...ui, component: "Card" }, styles: [], tracks: [field("OPACITY", [key(0.1, 0, "EASE_OUT"), key(0.4, 1)]), field("WIDTH", [key(0.1, 200, "EASE_OUT"), key(0.8, 320)], "SET", 320)] },
    { id: "S3", name: "Confetti", parentId: "S", nth: 1, hint: { ...vec, component: "Lottie/Confetti" }, styles: [], tracks: [field("OPACITY", [key(0, 0), key(1.5, 1)])] },
  ],
};

test("an illustration goes to Lottie as one piece", () => {
  const rows = classify(art);
  assert.deepEqual(rows.map((r) => [r.name, r.to]), [["Ring", "lottie"], ["Check", "lottie"]]);
  assert.match(rows[0].why, /same illustration/);
  assert.match(checkText(art, rows, lint(rows, art)), /motion_to_lottie on Success \(A\)/);
});

test("a screen: UI to code, Lottie/ components to Lottie, lint", () => {
  const rows = classify(screen);
  assert.deepEqual(rows.map((r) => [r.name, r.to]), [["Title", "code"], ["Card", "code"], ["Confetti", "lottie"]]);
  const l = lint(rows, screen).join("\n");
  assert.match(l, /Card: 700ms of UI motion/);
  assert.match(l, /width\/height/);
  assert.match(l, /mixes Lottie art and UI layers/);
});

test("tokens + CSS + Framer Motion", () => {
  const rows = classify(screen);
  const tk = tokens(rows);
  assert.deepEqual(tk.duration, { fast: 200, base: 700 });
  assert.deepEqual(Object.keys(tk.easing), ["out"]);
  assert.match(tk.css, /--motion-duration-fast: 200ms;/);
  const css = toCss(rows, tk);
  assert.match(css, /@keyframes title \{/);
  assert.match(css, /0% \{ transform: translate\(0px, 16px\); opacity: 0; animation-timing-function: var\(--motion-easing-out\); \}/);
  assert.match(css, /\.title \{\n  animation: title var\(--motion-duration-fast\) both;/);
  assert.match(css, /animation: card var\(--motion-duration-base\) 100ms both;/);
  assert.match(css, /prefers-reduced-motion/);
  assert.doesNotMatch(css, /confetti/);
  const jsx = toFramer(rows, tk);
  assert.match(jsx, /opacity: \[0, 1\], y: \[16, 0\]/);
  assert.match(jsx, /opacity: \{ duration: 0.3, delay: 0.1, times: \[0, 1\], ease: \["easeOut"\] \}/);
});

test("a list appearing one by one → one animation + delay by index; named curves come back named", () => {
  const card = (id, nth, t) => ({ id, name: "Card", parentId: "H", nth, hint: ui, styles: [], tracks: [
    field("OPACITY", [key(t, 0, "cubic(0.2,0,0,1)"), key(t + 0.3, 1)]),
    field("TRANSLATION_Y", [key(t, 24, "cubic(0.2,0,0,1)"), key(t + 0.3, 0)], "SET", 76 + nth * 72),
  ] });
  const home = { root: { id: "H", name: "Home" }, timelines: [{ duration: 0.8 }], animated: [card("H1", 1, 0.1), card("H2", 2, 0.16), card("H3", 3, 0.22)] };
  const rows = classify(home);
  const tk = tokens(rows);
  assert.deepEqual(Object.keys(tk.easing), ["standard"]);
  const css = toCss(rows, tk);
  assert.equal(css.match(/@keyframes/g).length, 1);
  assert.match(css, /3 items, 60ms apart/);
  assert.match(css, /animation: card var\(--motion-duration-base\) calc\(100ms \+ var\(--i, 0\) \* 60ms\) both;/);
  assert.match(css, /0% \{ transform: translate\(0px, 24px\)/);
  const jsx = toFramer(rows, tk);
  assert.match(jsx, /\{items\.map\(\(item, i\) => \(/);
  assert.match(jsx, /delay: 0\.1 \+ i \* 0\.06/);
});

test("AI smell: identical layers are flagged, a led composition is not", () => {
  const item = (id, t, dist, dur, ease) => ({ id, name: `Item ${id}`, parentId: "P", nth: 1, hint: ui, styles: [], tracks: [
    field("OPACITY", [key(t, 0, ease), key(t + dur, 1)]),
    field("TRANSLATION_Y", [key(t, dist, ease), key(t + dur, 0)], "SET", 100),
  ] });
  const read = (animated) => ({ root: { id: "P", name: "Page" }, timelines: [{ duration: 2 }], animated });
  const flat = read(["a", "b", "c", "d"].map((id) => item(id, 0, 20, 0.3, "EASE_IN")));
  const smell = lint(classify(flat), flat).filter((l) => l.startsWith("AI smell")).join("\n");
  assert.match(smell, /Item a, Item b, Item c, Item d appear with in/);
  assert.match(smell, /all 4 layers last 300ms/);
  assert.match(smell, /one easing/);
  assert.match(smell, /everything starts at once/);
  assert.match(smell, /fades in and moves from \[0,20\]/);
  const led = read([item("a", 0, 32, 0.42, "cubic(0.16,1,0.3,1)"), item("b", 0.12, 12, 0.26, "EASE_OUT"), item("c", 0.16, 12, 0.26, "EASE_OUT")]);
  assert.deepEqual(lint(classify(led), led).filter((l) => l.startsWith("AI smell")), []);
});

test("landing context: art is part of the scroll scene, long moments are allowed", async () => {
  const { motionContext } = await import("../motion-handoff.mjs");
  assert.equal(motionContext("Eyewear Atelier landing · Storyboard · Draft"), "landing");
  assert.equal(motionContext("Checkout · Prototype · Draft"), "ui");
  const ui = classify(art), landing = classify(art, [], { context: "landing" });
  assert.ok(ui.every((x) => x.to === "lottie"));
  assert.ok(landing.every((x) => x.to === "code" && /scroll scene/.test(x.why)));
  const slow = { root: { id: "L", name: "Hero" }, timelines: [{ duration: 1.5 }], animated: [
    { id: "L1", name: "Glasses", parentId: "L", nth: 1, hint: vec, styles: [], tracks: [field("OPACITY", [key(0, 0, "EASE_OUT"), key(0.9, 1)])] },
  ] };
  assert.match(lint(classify(slow), slow).join("\n"), /feels slow/);
  assert.doesNotMatch(lint(classify(slow, [], { context: "landing" }), slow, { context: "landing" }).join("\n"), /feels slow|usable within/);
});
