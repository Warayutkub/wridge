// The real projects the parity checks read (read-only; graph-wiring copies them before touching anything).
// They live on the developer's computer, so the list is in server/parity-projects.json (git-ignored).
// Copy server/parity-projects.example.json to that name and fill in five folders, in this order:
//   1. a brain with notes (a game or a book)   2. a large app (e.g. Laravel + TypeScript, with tables and flows)
//   3. a mid-sized web app   4. a small web app   5. a Node project with web/lottie files
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const FILE = join(dirname(fileURLToPath(import.meta.url)), "parity-projects.json");

function load() {
  if (!existsSync(FILE)) {
    throw new Error(`Parity needs your test projects: copy server/parity-projects.example.json to ${FILE} and fill it in.`);
  }
  const cfg = JSON.parse(readFileSync(FILE, "utf8"));
  if (!Array.isArray(cfg.projects) || cfg.projects.length !== 5) throw new Error(`${FILE}: "projects" must list exactly five folders`);
  return cfg;
}

const cfg = load();
/** The five project folders, in the order above. */
export const PROJECTS = cfg.projects;
/** A project with web/lottie files for the motion checks (defaults to the fifth project). */
export const LOTTIE_PROJECT = cfg.lottieProject ?? cfg.projects[4];
/** Optional: a real multi-page PDF for the merge check. */
export const REPORT_PDF = cfg.reportPdf ?? null;
