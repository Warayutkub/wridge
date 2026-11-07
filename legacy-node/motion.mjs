// Lottie files in a code project, for the /motion page: found by what they contain (no registry),
// read and written only inside the project. A file made by create_lottie keeps its recipe in
// meta.wara (spec) and, once sent to Figma, the GIF component it fills (meta.wara.figma).
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";

const SKIP_DIRS = new Set(["node_modules", ".git", ".wara", ".next", ".nuxt", ".dart_tool", ".gradle", ".idea", ".vscode", "build", "dist", "out", "coverage", "Pods", "vendor", ".turbo", ".cache"]);
const MAX_FILES = 20_000, MAX_BYTES = 8 * 1024 * 1024;

export const isLottie = (j) => j && typeof j === "object" && typeof j.v === "string" && Number(j.fr) > 0 && Array.isArray(j.layers) && Number.isFinite(j.op) && Number(j.w) > 0;

// The animation itself, without the recipe: tells whether the GIF in Figma is older than the file.
export const contentHash = (j) => createHash("sha1").update(JSON.stringify({ ...j, meta: undefined })).digest("hex").slice(0, 16);

export function insideProject(root, path) {
  const abs = resolve(root, path);
  const rel = relative(resolve(root), abs);
  if (!rel || rel.startsWith("..") || isAbsolute(rel)) throw new Error(`Path must be inside the project: ${path}`);
  return abs;
}

function summary(root, abs, json, bytes) {
  const wara = json.meta?.wara ?? null;
  const frames = json.op - (json.ip ?? 0);
  const hash = contentHash(json);
  return {
    path: relative(root, abs).replaceAll("\\", "/"),
    name: json.nm || abs.split(/[\\/]/).pop().replace(/\.json$/i, ""),
    bytes, w: json.w, h: json.h, fps: json.fr, frames, durationMs: Math.round((frames / json.fr) * 1000),
    layers: json.layers.length,
    wara: Boolean(wara?.spec),
    figma: wara?.figma ? { componentId: wara.figma.componentId, file: wara.figma.file, sentAt: wara.figma.sentAt, stale: wara.figma.hash !== hash } : null,
  };
}

export function listLottie(dir) {
  const root = resolve(dir);
  const out = [];
  let seen = 0;
  const walk = (d) => {
    let entries;
    try { entries = readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of entries) {
      if (++seen > MAX_FILES) return;
      if (e.isDirectory()) { if (!SKIP_DIRS.has(e.name) && !e.name.startsWith(".")) walk(join(d, e.name)); continue; }
      if (!e.isFile() || !/\.json$/i.test(e.name) || /^(package|package-lock|tsconfig|composer|app|manifest)\.json$/i.test(e.name)) continue;
      const abs = join(d, e.name);
      const { size } = statSync(abs);
      if (size < 200 || size > MAX_BYTES) continue;
      const text = readFileSync(abs, "utf8");
      if (!/"layers"\s*:/.test(text) || !/"fr"\s*:/.test(text)) continue; // cheap check before parsing
      try { const j = JSON.parse(text); if (isLottie(j)) out.push(summary(root, abs, j, size)); } catch { /* not JSON */ }
    }
  };
  walk(root);
  return out.sort((a, b) => a.path.localeCompare(b.path));
}

export function readLottie(dir, path) {
  const abs = insideProject(dir, path);
  const json = JSON.parse(readFileSync(abs, "utf8"));
  if (!isLottie(json)) throw new Error(`${path} is not a Lottie file`);
  return json;
}

// After the GIF went to Figma: remember the component and which version of the animation it shows.
export function markSentToFigma(dir, path, { componentId, file }) {
  const abs = insideProject(dir, path);
  const json = JSON.parse(readFileSync(abs, "utf8"));
  json.meta = { ...(json.meta ?? {}), wara: { ...(json.meta?.wara ?? {}), figma: { componentId, file, sentAt: new Date().toISOString(), hash: contentHash(json) } } };
  writeFileSync(abs, JSON.stringify(json));
  return summary(resolve(dir), abs, json, Buffer.byteLength(JSON.stringify(json)));
}
