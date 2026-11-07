// Projects added in the "กราฟโค้ด" page. The web view may only read graphs of folders listed here.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const FILE = join(dirname(fileURLToPath(import.meta.url)), "..", "code-projects.json");
const key = (dir) => resolve(dir).replaceAll("\\", "/").toLowerCase();

export function listProjects() {
  try {
    return JSON.parse(readFileSync(FILE, "utf8"));
  } catch {
    return [];
  }
}

export function addProject(dir) {
  const list = listProjects().filter((p) => key(p.dir) !== key(dir));
  const entry = { dir: resolve(dir).replaceAll("\\", "/"), addedAt: new Date().toISOString() };
  writeFileSync(FILE, `${JSON.stringify([entry, ...list], null, 2)}\n`);
  return entry;
}

export function removeProject(dir) {
  writeFileSync(FILE, `${JSON.stringify(listProjects().filter((p) => key(p.dir) !== key(dir)), null, 2)}\n`);
}

export const isKnownProject = (dir) => listProjects().some((p) => key(p.dir) === key(dir));
