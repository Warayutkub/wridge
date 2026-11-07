// Dart/Flutter files → symbols, imports, references and routes, using the official Dart analyzer
// (codegraph/dart/bin/parse.dart). It is compiled once to an exe so each update starts instantly.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { hash } from "./parse-ts.mjs";

const DIR = join(dirname(fileURLToPath(import.meta.url)), "dart");
const EXE = join(DIR, "build", process.platform === "win32" ? "wara-dart-parse.exe" : "wara-dart-parse");
const SOURCES = [join(DIR, "bin", "parse.dart"), join(DIR, "pubspec.yaml"), join(DIR, "pubspec.lock")];
const BATCH = 400;

const run = (cmd, args) => spawnSync(cmd, args, { cwd: DIR, encoding: "utf8", shell: process.platform === "win32", windowsHide: true, maxBuffer: 64 * 1024 * 1024 });

// Build the parser if it is missing or older than its source. Needs the Dart SDK (comes with Flutter).
function ensureParser() {
  const built = existsSync(EXE) ? statSync(EXE).mtimeMs : 0;
  if (built && SOURCES.every((f) => !existsSync(f) || statSync(f).mtimeMs <= built)) return { built: false };
  const version = run("dart", ["--version"]);
  if (version.status !== 0) throw new Error("Dart SDK not found — install Flutter or Dart (and put it on PATH) to graph .dart files");
  const get = run("dart", ["pub", "get"]);
  if (get.status !== 0) throw new Error(`dart pub get failed: ${(get.stderr || get.stdout).trim().split("\n").pop()}`);
  mkdirSync(dirname(EXE), { recursive: true });
  const compile = run("dart", ["compile", "exe", "bin/parse.dart", "-o", EXE]);
  if (compile.status !== 0) throw new Error(`Compiling the Dart parser failed: ${(compile.stderr || compile.stdout).trim().split("\n").pop()}`);
  return { built: true };
}

// rel paths → { [rel]: { directive, symbols, imports, refs, routes } | { error } }
export function parseDartFiles(root, relPaths) {
  if (!relPaths.length) return { results: {}, built: false };
  const { built } = ensureParser();
  const results = {};
  for (let i = 0; i < relPaths.length; i += BATCH) {
    const chunk = relPaths.slice(i, i + BATCH);
    const abs = chunk.map((p) => join(root, p).replaceAll("\\", "/"));
    const r = spawnSync(EXE, [], { input: JSON.stringify({ files: abs }), encoding: "utf8", windowsHide: true, maxBuffer: 256 * 1024 * 1024 });
    if (r.status !== 0) throw new Error(`Dart parser failed: ${(r.stderr || "").trim().split("\n").pop()}`);
    const out = JSON.parse(r.stdout);
    chunk.forEach((rel, j) => {
      const res = out[abs[j]];
      if (!res || res.error) { results[rel] = { error: res?.error ?? "no result" }; return; }
      const source = readFileSync(join(root, rel), "utf8");
      results[rel] = {
        directive: null,
        symbols: res.symbols.map((s) => ({ ...s, hash: hash(source.slice(s.start, s.end)) })),
        imports: res.imports.map((x) => ({ ...x, typeOnly: 0 })),
        refs: res.refs,
        routes: res.routes,
      };
    });
  }
  return { results, built };
}
