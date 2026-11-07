// Real screenshots of the wridge window for the website: a dev server on :3256 (WRIDGE_PROJECTS_FILE listing the
// demo made by make-brewly.mjs, three fake plugin windows "Brewly app/marketing/flows"), 1280×820 at 2×.
// Then convert to WebP into site/assets/img (see the commit that added them).
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_PATH ?? "playwright");
const OUT = process.env.OUT ?? fileURLToPath(new URL("../../site/assets/", import.meta.url));
const DIR = process.env.DEMO_DIR ?? "C:/demo/brewly";
const only = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH });
const ctx = await browser.newContext({ viewport: { width: 1280, height: 820 }, deviceScaleFactor: 2, colorScheme: "light" });
await ctx.addInitScript((dir) => { localStorage.setItem("wridge-project", dir); localStorage.setItem("wridge-theme", "light"); localStorage.setItem("wridge-figma", "kA1"); }, DIR);
const page = await ctx.newPage();
const shots = [
  ["app-overview", "overview", 2500],
  ["app-brain-graph", "graph", 5000],
  ["app-brain-notes", "notes", 3000],
  ["app-code-graph", "code", 5000],
  ["app-data", "data", 4000],
  ["app-flows", "flows", 3000],
];
for (const [name, route, wait] of shots) {
  if (only.length && !only.includes(name)) continue;
  await page.goto(`http://127.0.0.1:3256/app#/${route}`);
  await page.waitForTimeout(wait);
  if (route === "code" || route === "graph") { const f = page.frameLocator("#frame"); try { await f.getByRole("button", { name: /^Fit$/ }).first().click({ timeout: 2000 }); await page.waitForTimeout(1200); } catch {} }
  await page.screenshot({ path: `${OUT}/${name}.png` });
  console.log("shot", name);
}
// the Figma file picker, open
if (!only.length || only.includes("app-figma-picker")) {
  await page.goto("http://127.0.0.1:3256/app#/overview"); await page.waitForTimeout(2500);
  await page.click("#fig-btn"); await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/app-figma-picker.png` }); console.log("shot app-figma-picker");
}
await browser.close();
