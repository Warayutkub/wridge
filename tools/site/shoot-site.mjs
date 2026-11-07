// Full-page screenshots of the website for a look: node tools/site/shoot-site.mjs <url> <outDir>
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_PATH ?? "playwright");
const [url = "http://localhost:5179/", out = "."] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH });
for (const [name, w, h, mobile] of [["desktop", 1440, 900, false], ["mobile", 390, 844, true]]) {
  const page = await browser.newPage({ viewport: { width: w, height: h }, deviceScaleFactor: 1, isMobile: mobile, hasTouch: mobile });
  await page.goto(url, { waitUntil: "networkidle" });
  // scroll through once so lazy images load and stickers pop
  for (let y = 0; y < (await page.evaluate(() => document.body.scrollHeight)); y += h / 2) { await page.evaluate((v) => scrollTo(0, v), y); await page.waitForTimeout(60); }
  await page.evaluate(() => scrollTo(0, 0)); await page.waitForTimeout(600);
  await page.screenshot({ path: `${out}/site-${name}.png`, fullPage: true });
  console.log("shot", name);
}
await browser.close();
