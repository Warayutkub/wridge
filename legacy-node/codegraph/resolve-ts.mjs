// Module resolution for TS/JS imports, against the files already in the graph (no disk access per import).
// Covers relative paths, tsconfig/jsconfig "paths" + "baseUrl" (with "extends"), workspace packages and index files.
import { readFileSync } from "node:fs";
import { join, posix } from "node:path";

const EXTS = [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];

// tsconfig allows comments and trailing commas.
export function readJsonc(file) {
  const text = readFileSync(file, "utf8")
    .replace(/\\"|"(?:\\"|[^"])*"|(\/\/.*|\/\*[\s\S]*?\*\/)/g, (m, comment) => (comment ? "" : m))
    .replace(/,(\s*[}\]])/g, "$1");
  return JSON.parse(text);
}

const norm = (p) => posix.normalize(p.replaceAll("\\", "/")).replace(/^\.\//, "");

export function createResolver(root, filePaths) {
  const files = new Set(filePaths);
  const configs = loadTsconfigs(root, filePaths);
  const workspaces = loadWorkspaces(root, filePaths);

  function tryFile(base) {
    base = norm(base);
    if (base.startsWith("../")) return null;
    if (files.has(base)) return base;
    // ESM-style "./x.js" that really points at x.ts
    const swapped = base.replace(/\.(m|c)?jsx?$/, "");
    for (const b of swapped !== base ? [swapped, base] : [base]) {
      for (const ext of EXTS) if (files.has(b + ext)) return b + ext;
      for (const ext of EXTS) if (files.has(`${b}/index${ext}`)) return `${b}/index${ext}`;
    }
    return null;
  }

  // Closest tsconfig/jsconfig above the importing file.
  function configFor(file) {
    let dir = posix.dirname(file);
    for (;;) {
      const c = configs.get(dir === "." ? "" : dir);
      if (c) return c;
      if (dir === "." || dir === "") return null;
      dir = posix.dirname(dir);
    }
  }

  return function resolve(fromFile, spec) {
    if (spec.startsWith(".")) return { file: tryFile(posix.join(posix.dirname(fromFile), spec)) };
    if (spec.startsWith("/")) return { file: null };
    const config = configFor(fromFile);
    if (config) {
      for (const [pattern, targets] of config.paths) {
        const star = pattern.indexOf("*");
        let rest = null;
        if (star < 0 && spec === pattern) rest = "";
        else if (star >= 0 && spec.startsWith(pattern.slice(0, star)) && spec.endsWith(pattern.slice(star + 1))) {
          rest = spec.slice(star, spec.length - (pattern.length - star - 1));
        }
        if (rest === null) continue;
        for (const t of targets) {
          const hit = tryFile(posix.join(config.pathsBase, t.replace("*", rest)));
          if (hit) return { file: hit };
        }
      }
      if (config.baseUrl !== null) {
        const hit = tryFile(posix.join(config.baseUrl, spec));
        if (hit) return { file: hit };
      }
    }
    const pkg = packageName(spec);
    const ws = workspaces.get(pkg);
    if (ws) {
      const sub = spec.slice(pkg.length).replace(/^\//, "");
      const hit = sub ? tryFile(posix.join(ws.dir, sub)) ?? tryFile(posix.join(ws.dir, "src", sub)) : ws.entries.map(tryFile).find(Boolean);
      if (hit) return { file: hit };
    }
    return { file: null, external: pkg };
  };
}

export const packageName = (spec) => (spec.startsWith("@") ? spec.split("/").slice(0, 2).join("/") : spec.split("/")[0]);

function loadTsconfigs(root, paths) {
  const out = new Map();
  for (const p of paths) {
    if (!/(^|\/)[tj]sconfig\.json$/.test(p)) continue;
    try {
      out.set(posix.dirname(p) === "." ? "" : posix.dirname(p), compilerPaths(root, p, new Set()));
    } catch {
      // unreadable config: imports from that folder fall back to relative/package resolution
    }
  }
  return out;
}

// Effective "paths"/"baseUrl" after following relative "extends".
function compilerPaths(root, rel, seen) {
  if (seen.has(rel)) return { paths: [], pathsBase: "", baseUrl: null };
  seen.add(rel);
  const json = readJsonc(join(root, rel));
  const dir = posix.dirname(rel) === "." ? "" : posix.dirname(rel);
  let base = { paths: [], pathsBase: dir, baseUrl: null };
  for (const ext of [json.extends ?? []].flat()) {
    if (!ext.startsWith(".")) continue; // package configs (e.g. @tsconfig/next) rarely carry paths
    const target = norm(posix.join(dir, ext.endsWith(".json") ? ext : `${ext}.json`));
    try {
      base = compilerPaths(root, target, seen);
    } catch {
      // missing base config
    }
  }
  const co = json.compilerOptions ?? {};
  const baseUrl = co.baseUrl !== undefined ? norm(posix.join(dir, co.baseUrl)) : base.baseUrl;
  if (co.paths) {
    return { paths: Object.entries(co.paths), pathsBase: baseUrl ?? dir, baseUrl };
  }
  return { ...base, baseUrl };
}

// package.json "name" → folder, so imports between packages of a monorepo stay inside the graph.
function loadWorkspaces(root, paths) {
  const out = new Map();
  for (const p of paths) {
    if (!p.endsWith("package.json")) continue;
    try {
      const json = JSON.parse(readFileSync(join(root, p), "utf8"));
      if (!json.name) continue;
      const dir = posix.dirname(p) === "." ? "" : posix.dirname(p);
      const entries = [json.source, json.types, json.module, json.main, typeof json.exports === "string" ? json.exports : json.exports?.["."]?.import ?? json.exports?.["."]?.default ?? json.exports?.["."]]
        .filter((x) => typeof x === "string")
        .map((x) => posix.join(dir, x.replace(/^\.\/(dist|build|lib)\//, "./src/")));
      entries.push(posix.join(dir, "src/index"), posix.join(dir, "index"));
      out.set(json.name, { dir, entries });
    } catch {
      // invalid package.json
    }
  }
  return out;
}

