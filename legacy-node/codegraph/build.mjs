// Build / incrementally update <project>/.wara/code-graph.db.
import { mkdirSync, readFileSync, statSync } from "node:fs";
import { extname, isAbsolute, join, posix, resolve } from "node:path";
import { buildDataModel, dataSources, sourcesStamp } from "./data/model.mjs";
import { DATA_SCAN_VERSION, dataAccessStyles, scanDartData, scanData, sqlDialect } from "./data/usage.mjs";
import { getMeta, openDb, setMeta, transaction } from "./db.mjs";
import { WARA_DIR, ensureGitignored, listFiles } from "./files.mjs";
import { parseDartFiles } from "./parse-dart.mjs";
import { parseTs } from "./parse-ts.mjs";
import { DART_TOKEN_CLASS, normColor, scanStyles } from "./styles.mjs";
import { createResolver } from "./resolve-ts.mjs";
import { nextRoutes } from "./routes-next.mjs";

export const graphFile = (root) => join(root, WARA_DIR, "code-graph.db");

export function projectRoot(projectDir) {
  if (!projectDir || !isAbsolute(projectDir)) throw new Error(`projectDir must be an absolute path, got "${projectDir}"`);
  const root = resolve(projectDir);
  if (!statSync(root, { throwIfNoEntry: false })?.isDirectory()) throw new Error(`Not a folder: ${root}`);
  return root;
}

// Parses only files whose mtime/size changed since the last run; re-resolves imports and routes when anything changed.
export async function updateGraph(projectDir, { force = false } = {}) {
  const root = projectRoot(projectDir);
  const started = Date.now();
  const gitignore = ensureGitignored(root); // before .wara/ exists, so the db is never picked up by git
  mkdirSync(join(root, WARA_DIR), { recursive: true });
  const db = openDb(graphFile(root));
  try {
    const listed = listFiles(root);
    const known = new Map(db.prepare("SELECT id, path, mtime, size FROM files").all().map((r) => [r.path, r]));
    const listedPaths = new Set(listed.map((f) => f.path));
    const changed = listed.filter((f) => {
      const k = known.get(f.path);
      return force || !k || k.mtime !== f.mtime || k.size !== f.size;
    });
    const removed = [...known.keys()].filter((p) => !listedPaths.has(p));

    // Data access styles come from the packages; when they change, every source file is rescanned for data use.
    const pkgJson = readJsonSafe(join(root, "package.json"));
    const pubspec = readTextSafe(join(root, "pubspec.yaml"));
    const dataStyles = dataAccessStyles(pkgJson, pubspec);
    // The scanner version is part of the key, so improving a scanner rescans existing graphs once.
    const stylesKey = `${DATA_SCAN_VERSION}:${dataStyles.join(",")}`;
    const rescanData = (getMeta(db, "data_styles") ?? "") !== stylesKey;

    const parsed = [];
    // Dart goes to the analyzer-based parser in one batch (it may build itself on first use).
    const dartFiles = changed.filter((f) => f.lang === "dart");
    let dart = { results: {}, built: false };
    let dartError = null;
    try {
      dart = parseDartFiles(root, dartFiles.map((f) => f.path));
    } catch (err) {
      dartError = String(err.message ?? err);
    }
    for (const f of changed) {
      if (f.lang === "dart") {
        const r = dart.results[f.path];
        const data = scanDartData(readFileSync(join(root, f.path), "utf8"), dataStyles);
        parsed.push(r && !r.error ? { f, r, data } : { f, error: dartError ?? r?.error ?? "not parsed", data });
        continue;
      }
      // Stylesheets/markup: only token definitions and usages.
      const styled = f.lang === "ts" || /\.(css|scss|sass|less|html|vue|svelte|astro)$/.test(f.path);
      const text = styled ? readFileSync(join(root, f.path), "utf8") : null;
      const styles = styled ? scanStyles(text, f.path) : null;
      if (f.lang !== "ts") { parsed.push({ f, styles }); continue; }
      const data = scanData(text, dataStyles);
      try {
        parsed.push({ f, r: await parseTs(text, extname(f.path)), styles, data });
      } catch (err) {
        parsed.push({ f, error: String(err.message ?? err).slice(0, 300), styles, data });
      }
    }

    transaction(db, () => {
      const del = db.prepare("DELETE FROM files WHERE path = ?");
      for (const p of removed) del.run(p);
      const upsert = db.prepare(`INSERT INTO files (path, lang, mtime, size, directive, parse_error) VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(path) DO UPDATE SET lang = excluded.lang, mtime = excluded.mtime, size = excluded.size,
        directive = excluded.directive, parse_error = excluded.parse_error RETURNING id`);
      const clear = ["symbols", "imports", "refs", "routes", "declared_routes", "style_defs", "style_refs", "data_refs"].map((t) => db.prepare(`DELETE FROM ${t} WHERE file_id = ?`));
      const addData = db.prepare("INSERT INTO data_refs (file_id, form, entity, op, detail, columns, filters, via, line) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)");
      const putData = (fileId, refs) => {
        for (const d of refs ?? []) addData.run(fileId, d.form, d.entity, d.op, d.detail ?? null, d.columns ? JSON.stringify(d.columns) : null, d.filters ? JSON.stringify(d.filters) : null, d.via ?? null, d.line);
      };
      const addSymbol = db.prepare("INSERT INTO symbols (file_id, name, kind, exported, is_default, line, end_line, props, hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id");
      const addImport = db.prepare("INSERT INTO imports (file_id, kind, source, imported, local, type_only, line) VALUES (?, ?, ?, ?, ?, ?, ?)");
      const addRef = db.prepare("INSERT INTO refs (file_id, symbol_id, local, member, kind, line, count) VALUES (?, ?, ?, ?, ?, ?, ?)");
      const addDeclared = db.prepare("INSERT INTO declared_routes (file_id, path, widget, kind, line) VALUES (?, ?, ?, ?, ?)");
      const addDef = db.prepare("INSERT INTO style_defs (file_id, name, value, line) VALUES (?, ?, ?, ?)");
      const addUse = db.prepare("INSERT INTO style_refs (file_id, form, name, line, count) VALUES (?, ?, ?, ?, ?)");
      for (const { f, r, error, styles, data } of parsed) {
        const { id } = upsert.get(f.path, f.lang, f.mtime, f.size, r?.directive ?? null, error ?? null);
        for (const c of clear) c.run(id);
        putData(id, data);
        for (const d of styles?.defs ?? []) addDef.run(id, d.name, d.value, d.line);
        for (const u of styles?.refs ?? []) addUse.run(id, u.form, u.name, u.line, u.count);
        if (!r) continue;
        if (f.lang === "dart") {
          // Dart tokens: statics of AppColors-like classes are definitions; AppColors.x references are uses.
          for (const sym of r.symbols) for (const st of sym.props?.statics ?? []) addDef.run(id, `${sym.name}.${st.name}`, st.value, st.line);
          for (const x of r.refs) {
            if (x.kind === "hex") addUse.run(id, "hex", normColor(x.member) ?? x.member, x.line, x.count);
            else if (x.member && DART_TOKEN_CLASS.test(x.local)) addUse.run(id, "dart", `${x.local}.${x.member}`, x.line, x.count);
          }
          r.refs = r.refs.filter((x) => x.kind !== "hex");
        }
        const ids = r.symbols.map((s) => addSymbol.get(id, s.name, s.kind, s.exported, s.isDefault, s.line, s.endLine, s.props ? JSON.stringify(s.props) : null, s.hash).id);
        for (const i of r.imports) addImport.run(id, i.kind, i.source, i.imported ?? null, i.local ?? null, i.typeOnly, i.line);
        for (const x of r.refs) addRef.run(id, x.owner >= 0 ? ids[x.owner] : null, x.local, x.member ?? null, x.kind, x.line, x.count);
        for (const rt of r.routes ?? []) addDeclared.run(id, rt.path, rt.widget ?? null, rt.kind, rt.line);
      }
      // Access style changed (e.g. supabase-js just added): rescan unchanged sources for data use too.
      if (rescanData) {
        const changedPaths = new Set(changed.map((f) => f.path));
        const fileRow = db.prepare("SELECT id FROM files WHERE path = ?");
        const clearData = db.prepare("DELETE FROM data_refs WHERE file_id = ?");
        for (const f of listed) {
          if ((f.lang !== "ts" && f.lang !== "dart") || changedPaths.has(f.path)) continue;
          const row = fileRow.get(f.path);
          if (!row) continue;
          clearData.run(row.id);
          const text = readFileSync(join(root, f.path), "utf8");
          putData(row.id, f.lang === "dart" ? scanDartData(text, dataStyles) : scanData(text, dataStyles));
        }
        setMeta(db, "data_styles", stylesKey);
      }
      // The data model is rebuilt only when a schema source (migration…) changed.
      const ddlFiles = db.prepare("SELECT DISTINCT f.path FROM data_refs d JOIN files f ON f.id = d.file_id WHERE d.op = 'ddl'").all().map((x) => x.path);
      const sources = dataSources(listed.map((f) => f.path), ddlFiles);
      const stamp = sourcesStamp(root, sources);
      if (getMeta(db, "data_stamp") !== stamp) {
        setMeta(db, "data_model", JSON.stringify(buildDataModel(root, sources, { dialect: sqlDialect(pkgJson, pubspec) })));
        setMeta(db, "data_stamp", stamp);
      }
      if (changed.length || removed.length) {
        relink(db, root);
        setMeta(db, "updated_at", new Date().toISOString());
      }
    });

    const count = (sql) => db.prepare(sql).get().n;
    return {
      root: root.replaceAll("\\", "/"),
      db: graphFile(root).replaceAll("\\", "/"),
      gitignore,
      files: listed.length,
      parsed: parsed.filter((p) => p.r).length,
      changed: changed.length,
      removed: removed.length,
      parseErrors: dartError ? [`${dartFiles.length} Dart file(s): ${dartError}`, ...parsed.filter((p) => p.error && p.f.lang !== "dart").map((p) => `${p.f.path}: ${p.error}`)] : parsed.filter((p) => p.error).map((p) => `${p.f.path}: ${p.error}`),
      dartParserBuilt: dart.built || undefined,
      unresolvedImports: count("SELECT COUNT(*) AS n FROM imports WHERE target_file_id IS NULL AND external IS NULL"),
      ms: Date.now() - started,
    };
  } finally {
    db.close();
  }
}

// Import targets and routes depend on other files (and on tsconfig/package.json), so they are recomputed as a whole.
function readTextSafe(file) {
  try {
    return readFileSync(file, "utf8");
  } catch {
    return "";
  }
}

function readJsonSafe(file) {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}

function relink(db, root) {
  const files = db.prepare("SELECT id, path FROM files").all();
  const idByPath = new Map(files.map((f) => [f.path, f.id]));
  const pathById = new Map(files.map((f) => [f.id, f.path]));
  const paths = files.map((f) => f.path);
  const resolveTs = createResolver(root, paths);
  const resolveDart = createDartResolver(root, paths);
  const set = db.prepare("UPDATE imports SET target_file_id = ?, external = ? WHERE id = ?");
  for (const imp of db.prepare("SELECT id, file_id, source FROM imports").all()) {
    const from = pathById.get(imp.file_id);
    const r = from.endsWith(".dart") ? resolveDart(from, imp.source) : resolveTs(from, imp.source);
    set.run(r.file ? idByPath.get(r.file) : null, r.file ? null : r.external ?? null, imp.id);
  }
  db.exec("DELETE FROM routes");
  const addRoute = db.prepare("INSERT INTO routes (file_id, framework, path, kind, root) VALUES (?, ?, ?, ?, ?)");
  for (const r of nextRoutes(root, paths)) addRoute.run(idByPath.get(r.file), r.framework, r.path, r.kind, r.root);
  flutterRoutes(db, addRoute);
}

// package:<name>/x.dart → <folder of the pubspec named <name>>/lib/x.dart; dart:… and other packages are external.
export function createDartResolver(root, paths) {
  const files = new Set(paths);
  const packages = new Map();
  for (const p of paths) {
    if (!p.endsWith("pubspec.yaml")) continue;
    try {
      const name = /^name:\s*([A-Za-z0-9_]+)/m.exec(readFileSync(join(root, p), "utf8"))?.[1];
      if (name) packages.set(name, posix.dirname(p) === "." ? "" : posix.dirname(p));
    } catch {
      // unreadable pubspec
    }
  }
  return (from, spec) => {
    if (spec.startsWith("dart:")) return { file: null, external: spec };
    const pkg = /^package:([^/]+)\/(.+)$/.exec(spec);
    if (pkg) {
      const dir = packages.get(pkg[1]);
      if (dir === undefined) return { file: null, external: pkg[1] };
      const target = posix.join(dir, "lib", pkg[2]);
      return { file: files.has(target) ? target : null };
    }
    const target = posix.normalize(posix.join(posix.dirname(from), spec));
    return { file: files.has(target) ? target : null };
  };
}

// Flutter has no file-based routes: screens are widgets that get pushed (MaterialPageRoute → "nav" refs),
// plus paths declared in GoRoute / MaterialApp(routes:, home:). Widget names are matched project-wide.
function flutterRoutes(db, addRoute) {
  const widgets = new Map();
  for (const s of db.prepare("SELECT s.name, s.file_id, f.path FROM symbols s JOIN files f ON f.id = s.file_id WHERE f.lang = 'dart' AND s.kind IN ('widget', 'class')").all()) {
    if (!widgets.has(s.name)) widgets.set(s.name, []);
    widgets.get(s.name).push(s);
  }
  const fileOf = (name, near) => {
    const list = widgets.get(name) ?? [];
    return (list.find((s) => s.file_id === near) ?? (list.length === 1 ? list[0] : null))?.file_id ?? null;
  };
  const seen = new Set();
  const add = (fileId, path, kind, widget) => {
    const key = `${path}|${kind}|${widget}`;
    if (!fileId || seen.has(key)) return;
    seen.add(key);
    addRoute.run(fileId, "flutter", path, kind, widget);
  };
  // Tests build throwaway MaterialApps; they are not the app's routes.
  const isTest = db.prepare("SELECT path FROM files WHERE id = ?");
  const inTests = (fileId) => /(^|\/)(test|integration_test|test_driver)\//.test(isTest.get(fileId)?.path ?? "");
  for (const r of db.prepare("SELECT * FROM declared_routes").all()) if (!inTests(r.file_id)) add(fileOf(r.widget, r.file_id) ?? r.file_id, r.path, r.kind, r.widget);
  for (const r of db.prepare("SELECT DISTINCT r.local, r.file_id FROM refs r JOIN files f ON f.id = r.file_id WHERE r.kind = 'nav' AND f.lang = 'dart' AND r.member IS NULL").all()) {
    if (!inTests(r.file_id)) add(fileOf(r.local, r.file_id), r.local, "screen", r.local);
  }
}

// Open for reading after bringing the graph up to date (cheap when nothing changed).
export async function openGraph(projectDir) {
  const info = await updateGraph(projectDir);
  return { db: openDb(graphFile(projectRoot(projectDir))), info };
}
