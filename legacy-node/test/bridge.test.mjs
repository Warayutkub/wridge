// Design → code mapping, token generation, project memory and stack detection.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { formatTokens, resolveEntry } from "../codegen.mjs";
import { detectStack } from "../codegraph/knowledge.mjs";
import { decide, find, handoff, note, overview } from "../codegraph/memory.mjs";

const dirs = [];
after(() => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));
const tempDir = () => {
  const d = mkdtempSync(join(tmpdir(), "wara-bridge-"));
  dirs.push(d);
  return d;
};

test("class mappings: base classes + the classes of each variant value", () => {
  const entry = { element: "button", className: "btn", variants: { "Type=Primary": "btn-p", "Type=Danger": "btn-d", "Size=Small": "btn-sm" } };
  assert.deepEqual(resolveEntry(entry, "Type=Danger, State=Hover, Size=Small"), { element: "button", className: "btn btn-d btn-sm" });
  assert.equal(resolveEntry({ import: "@/ui/button", component: "Button" }, "Type=Primary").component, "Button");
});

test("Tailwind tokens: a text style or shadow named like a color is renamed so text-*/shadow-* stay unambiguous", () => {
  const data = {
    file: "t",
    collections: [{ name: "c", defaultMode: "Light" }],
    variables: [{ token: "color-tile", type: "COLOR", collection: "c", values: { Light: "#ffffff" } }, { token: "color-card", type: "COLOR", collection: "c", values: { Light: "#eeeeee" } }],
    paintStyles: [],
    textStyles: [{ token: "text-tile", family: "Kanit", style: "Bold", size: 24, lineHeight: "130%", letterSpacing: "0px" }, { token: "text-body", family: "Kanit", style: "Regular", size: 16, lineHeight: "150%", letterSpacing: "0px" }],
    effectStyles: [{ token: "shadow-card", shadows: [{ x: 0, y: 2, blur: 4, spread: 0, color: "#0000001a" }] }],
  };
  const tw = formatTokens(data, "tailwind");
  assert.equal(tw.map["text-tile"], "--text-type-tile");
  assert.equal(tw.map["text-body"], "--text-body");
  assert.equal(tw.map["shadow-card"], "--shadow-depth-card");
  assert.match(tw.content, /Renamed to avoid clashing/);
  assert.equal(formatTokens(data, "css").map["shadow-card"], "--shadow-card");
});

test("project memory: decisions go to docs/decisions.md, replaced ones are hidden, notes close with their task", () => {
  const root = tempDir();
  execFileSync("git", ["init", "-q"], { cwd: root });
  const first = decide(root, { title: "Redis not used yet", decision: "Rate limits in memory.", tags: ["infra"] });
  assert.equal(first.id, "D-1");
  assert.match(first.gitignore ?? "", /\/\.wara\//);
  decide(root, { title: "Use Redis for rate limits", decision: "Two instances now.", tags: ["infra"], supersedes: "D-1" });
  note(root, { task: "Foundations", text: "Keep all 4 grays." });
  handoff(root, { task: "Foundations", stage: "5/8", next: "Input, Card" });
  const ov = overview(root);
  assert.match(ov, /D-2 · .* Use Redis/);
  assert.doesNotMatch(ov, /D-1 ·/);
  assert.match(ov, /Keep all 4 grays/);
  assert.match(find(root, "redis", { includeOld: true }), /\[replaced\]/);
  handoff(root, { task: "Foundations", close: true });
  assert.match(overview(root), /Open tasks: none/);
  assert.match(readFileSync(join(root, "docs/decisions.md"), "utf8"), /## D-2 · \d{4}-\d{2}-\d{2} · Use Redis/);
});

test("stack detection: package.json and go.mod one level down, notable packages only", () => {
  const root = tempDir();
  mkdirSync(join(root, "web"));
  mkdirSync(join(root, "backend"));
  writeFileSync(join(root, "web/package.json"), JSON.stringify({ dependencies: { next: "^16.1.0", zod: "^3.23.0", "left-pad": "1.0.0" } }));
  writeFileSync(join(root, "backend/go.mod"), "module x\n\ngo 1.22\n\nrequire (\n\tgithub.com/gofiber/fiber/v2 v2.52.5\n\tgithub.com/stretchr/testify v1.9.0\n)\n");
  const stack = detectStack(root).map((s) => `${s.tool}@${s.version}`).sort();
  assert.deepEqual(stack, ["github.com/gofiber/fiber@v2.52.5", "go@1.22", "next@16.1.0", "zod@3.23.0"]);
});
