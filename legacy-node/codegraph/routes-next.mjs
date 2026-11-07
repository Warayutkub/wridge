// Next.js routes from the file system: App Router (app/, src/app/), Pages Router and proxy/middleware.
import { readFileSync } from "node:fs";
import { join, posix } from "node:path";

const APP_FILES = new Set(["page", "layout", "template", "loading", "error", "global-error", "not-found", "route", "default", "forbidden", "unauthorized"]);
const CODE = /\.(tsx|ts|jsx|js|mjs)$/;

// Folders holding a package.json that depends on next.
export function nextRoots(root, paths) {
  const roots = [];
  for (const p of paths) {
    if (!p.endsWith("package.json")) continue;
    try {
      const json = JSON.parse(readFileSync(join(root, p), "utf8"));
      if (json.dependencies?.next || json.devDependencies?.next) roots.push(posix.dirname(p) === "." ? "" : posix.dirname(p));
    } catch {
      // invalid package.json
    }
  }
  return roots;
}

export function nextRoutes(root, paths) {
  const routes = [];
  const join2 = (...parts) => parts.filter(Boolean).join("/");
  for (const r of nextRoots(root, paths)) {
    for (const base of ["app", "src/app"].map((d) => join2(r, d))) {
      for (const p of paths) {
        if (!p.startsWith(`${base}/`) || !CODE.test(p)) continue;
        const rel = p.slice(base.length + 1);
        const segs = rel.split("/");
        const kind = segs.pop().replace(CODE, "");
        if (!APP_FILES.has(kind)) continue;
        if (segs.some((s) => s.startsWith("_"))) continue; // private folder: not routable
        const url = segs
          .filter((s) => !/^\(.*\)$/.test(s) && !s.startsWith("@")) // route groups and parallel slots don't appear in the URL
          .map((s) => s.replace(/^(\(\.{1,3}\))+/, "")); // intercepting routes: (.)photo, (..)(..)photo
        routes.push({ file: p, framework: "next-app", path: `/${url.join("/")}`, kind, root: base });
      }
    }
    for (const base of ["pages", "src/pages"].map((d) => join2(r, d))) {
      for (const p of paths) {
        if (!p.startsWith(`${base}/`) || !CODE.test(p)) continue;
        const rel = p.slice(base.length + 1).replace(CODE, "");
        const name = rel.split("/").pop();
        if (["_app", "_document", "_error"].includes(name)) {
          routes.push({ file: p, framework: "next-pages", path: "*", kind: name.slice(1), root: base });
          continue;
        }
        const url = `/${rel.replace(/(^|\/)index$/, "")}`;
        routes.push({ file: p, framework: "next-pages", path: url, kind: rel.startsWith("api/") ? "route" : "page", root: base });
      }
    }
    for (const name of ["proxy", "middleware"]) {
      for (const dir of [r, join2(r, "src")]) {
        const p = paths.find((x) => CODE.test(x) && x.replace(CODE, "") === join2(dir, name));
        if (p) routes.push({ file: p, framework: "next-app", path: "*", kind: name === "proxy" ? "proxy" : "middleware", root: dir || "." });
      }
    }
  }
  return routes;
}

// "/jobs/[id]" matches "/jobs/42"; catch-alls match the rest.
export function routeMatches(pattern, url) {
  const p = pattern.split("/").filter(Boolean);
  const u = url.split("?")[0].split("/").filter(Boolean);
  for (let i = 0; i < p.length; i++) {
    if (/^\[\[\.\.\..+\]\]$/.test(p[i])) return true;
    if (/^\[\.\.\..+\]$/.test(p[i])) return u.length > i;
    if (i >= u.length) return false;
    if (!/^\[.+\]$/.test(p[i]) && p[i] !== u[i]) return false;
  }
  return p.length === u.length;
}
