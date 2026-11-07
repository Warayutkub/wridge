// A private Node server (legacy-node/index.mjs) that cannot touch the user's files: legacy-node/ is copied
// into a temp folder (node_modules is a directory junction to the real one), next to junctions to web/ and
// skills/ and a copy of AI_GUIDE.md, as index.mjs reads them from "..". Its code-projects.json is the
// copy's own (starts as []), so the real legacy-node/code-projects.json — the live list of the running
// app — is never read or written.
//   const node = makePrivateNode();   node.dir (run index.mjs here), node.projectsFile, node.cleanup()
import { copyFileSync, cpSync, existsSync, lstatSync, mkdtempSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
export const ROOT = resolve(HERE, "..", "..", "..");
const LEGACY = join(ROOT, "legacy-node");
export const REAL_PROJECTS_FILE = join(LEGACY, "code-projects.json");

/** Throws unless `file` is somewhere other than the real legacy-node/code-projects.json. */
export function assertNotRealProjects(file) {
  if (resolve(file).toLowerCase() === REAL_PROJECTS_FILE.toLowerCase()) throw new Error(`refusing the real projects list ${file}`);
}

export function makePrivateNode(base = mkdtempSync(join(tmpdir(), "wridge-private-node-"))) {
  const dir = join(base, "legacy-node");
  const skip = new Set([join(LEGACY, "node_modules"), REAL_PROJECTS_FILE, join(LEGACY, "wara.log")].map((p) => p.toLowerCase()));
  // the Dart parser's exe keeps its time, so it is not rebuilt in the copy
  cpSync(LEGACY, dir, { recursive: true, preserveTimestamps: true, filter: (src) => !skip.has(resolve(src).toLowerCase()) });
  const junctions = [
    [join(LEGACY, "node_modules"), join(dir, "node_modules")],
    [join(ROOT, "web"), join(base, "web")],
    [join(ROOT, "skills"), join(base, "skills")],
  ];
  for (const [target, link] of junctions) symlinkSync(target, link, "junction");
  copyFileSync(join(ROOT, "AI_GUIDE.md"), join(base, "AI_GUIDE.md"));
  const projectsFile = join(dir, "code-projects.json");
  writeFileSync(projectsFile, "[]\n");
  return {
    base,
    dir,
    projectsFile,
    // Junctions go first (and must be gone) before anything is deleted, so nothing behind them is touched.
    cleanup() {
      for (const [, link] of junctions) {
        try { if (lstatSync(link).isSymbolicLink()) unlinkSync(link); } catch { /* not there */ }
        if (existsSync(link)) {
          console.log(`left ${base} in place: could not remove the junction ${link}`);
          return;
        }
      }
      rmSync(base, { recursive: true, force: true });
    },
  };
}

