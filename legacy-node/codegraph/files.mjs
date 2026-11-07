// Which files belong to the graph, and keeping .wara/ out of git.
import { spawnSync } from "node:child_process";
import { appendFileSync, existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, extname, join, relative } from "node:path";

export const WARA_DIR = ".wara";

const CODE = { ".ts": "ts", ".tsx": "ts", ".js": "ts", ".jsx": "ts", ".mjs": "ts", ".cjs": "ts", ".mts": "ts", ".cts": "ts", ".dart": "dart" };
const CONFIG_NAMES = new Set(["package.json", "pubspec.yaml"]);
const isConfig = (name) => CONFIG_NAMES.has(name) || /^[tj]sconfig(\..+)?\.json$/.test(name);
// Other source languages are kept at file level so the graph has no blind spots.
const OTHER = new Set([
  ".py", ".go", ".rs", ".java", ".kt", ".kts", ".swift", ".m", ".mm", ".c", ".h", ".cc", ".cpp", ".hpp", ".cs", ".rb", ".php",
  ".sql", ".prisma", ".graphql", ".gql", ".proto", ".css", ".scss", ".sass", ".less", ".html", ".vue", ".svelte", ".astro", ".sh", ".ps1",
]);
const SKIP_DIRS = new Set(["node_modules", ".git", ".next", ".turbo", ".vercel", ".dart_tool", "build", "dist", "out", "coverage", ".wara", "Pods", ".gradle", ".idea", "vendor", ".gomodcache", "target", "__pycache__", ".venv"]);
const MAX_BYTES = 1_000_000; // bigger files are generated/minified; parsing them only adds noise

export function classify(path) {
  const name = basename(path);
  if (isConfig(name)) return "config";
  if (/\.(min|bundle)\.[cm]?js$/.test(name) || /\.(g|freezed|mocks)\.dart$/.test(name)) return null; // generated
  const ext = extname(name).toLowerCase();
  return CODE[ext] ?? (OTHER.has(ext) ? ext.slice(1) : null);
}

function git(root, args) {
  const r = spawnSync("git", ["-C", root, ...args], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, windowsHide: true });
  return r.status === 0 ? r.stdout : null;
}

export const gitTop = (root) => git(root, ["rev-parse", "--show-toplevel"])?.trim() || null;

// Project files that git would track (respects every .gitignore), or a plain walk outside git.
export function listFiles(root) {
  const out = git(root, ["ls-files", "--cached", "--others", "--exclude-standard", "-z"]);
  const paths = out !== null ? out.split("\0").filter(Boolean) : walk(root);
  const files = [];
  for (const path of paths) {
    const lang = classify(path);
    if (!lang || path.split("/").some((seg) => SKIP_DIRS.has(seg))) continue;
    let st;
    try {
      st = statSync(join(root, path));
    } catch {
      continue; // deleted but still in the git index
    }
    if (!st.isFile() || st.size > MAX_BYTES) continue;
    files.push({ path, lang, mtime: st.mtimeMs, size: st.size });
  }
  return files;
}

function walk(root, dir = "", acc = []) {
  for (const entry of readdirSync(join(root, dir), { withFileTypes: true })) {
    if (entry.name.startsWith(".") && entry.name !== ".github") continue;
    const rel = dir ? `${dir}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) walk(root, rel, acc);
    } else acc.push(rel);
  }
  return acc;
}

// Must run before .wara/ is created: the graph is generated, machine-specific data that never belongs in git.
// Adds ".wara/" to the repo's root .gitignore when git does not ignore it yet. Outside git it does nothing.
export function ensureGitignored(root) {
  const top = gitTop(root);
  if (!top) return { git: false };
  const probe = join(root, WARA_DIR, "code-graph.db");
  const check = spawnSync("git", ["-C", root, "check-ignore", "-q", probe], { windowsHide: true });
  if (check.status === 0) return { git: true, gitignore: null, added: false };
  const file = join(top, ".gitignore");
  const rel = relative(top, root).replaceAll("\\", "/");
  const pattern = rel ? `/${rel}/${WARA_DIR}/` : `/${WARA_DIR}/`;
  const block = `\n# wridge code graph (generated, rebuilt from source)\n${pattern}\n`;
  if (existsSync(file)) {
    const current = readFileSync(file, "utf8");
    appendFileSync(file, (current.endsWith("\n") || current === "" ? "" : "\n") + block);
  } else {
    writeFileSync(file, block.trimStart());
  }
  return { git: true, gitignore: file.replaceAll("\\", "/"), added: true, pattern };
}
