// node wara.mjs code-graph … — works without the wara-bridge server (reads/writes <project>/.wara directly).
import { resolve } from "node:path";

export const CODE_GRAPH_USAGE = `wara code-graph — ask a project's code graph (no server needed)

  node wara.mjs code-graph update [--dir D] [--force]     build/refresh <D>/.wara/code-graph.db
  node wara.mjs code-graph overview [--dir D]             files, routes, most used components, packages
  node wara.mjs code-graph find <text>                    symbols / files / routes containing text
  node wara.mjs code-graph file <path>                    what a file declares, imports, and who imports it
  node wara.mjs code-graph used_by <target>               every place that uses it
  node wara.mjs code-graph uses <target>                  what it depends on
  node wara.mjs code-graph impact <target> [--depth 3]    everything affected by changing it
  node wara.mjs code-graph route </url> [--depth 3]       route files + the component tree it renders (Flutter: route ScreenWidget)
  node wara.mjs code-graph design                         Figma components ↔ code components (sync first: node wara.mjs code_design_sync projectDir=…)
  node wara.mjs code-graph tokens [token]                 design tokens: usage, unused, raw colors, values that differ from Figma
  node wara.mjs code-graph notes                          AI-written notes: stale ones, and where a note is missing
  node wara.mjs code-graph data                           data model: tables, RLS, who reads/writes, drift from migrations
  node wara.mjs code-graph table <name>                   columns, indexes, relations, policies, queries using it
  node wara.mjs code-graph impact <table>.<column>        queries + functions + routes affected by a schema change
  node wara.mjs code-graph note <target> "text"           save a note (purpose/rules, 1-3 sentences); "" deletes

  target: Button | components/ui/button.tsx | components/ui/button.tsx#Button | /jobs/[id]
  --dir defaults to the current folder; --limit N caps list length`;

export async function codeGraphCli(argv) {
  // node:sqlite prints an ExperimentalWarning on first use; it is noise for a CLI.
  process.removeAllListeners("warning");
  process.on("warning", (w) => { if (w.name !== "ExperimentalWarning") console.warn(w.message); });
  const { updateGraph, openGraph } = await import("./build.mjs");
  const { runQuery } = await import("./query.mjs");

  const opts = { dir: process.cwd() };
  const pos = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--force") opts.force = true;
    else if (a.startsWith("--")) opts[a.slice(2)] = argv[++i];
    // Git Bash rewrites "/api/jobs/1" into "C:/Program Files/Git/api/jobs/1" before node sees it.
    else pos.push(a.replace(/^[A-Za-z]:[\\/](?:[^\\/]+[\\/])*?Git(?:[\\/]usr)?(?=[\\/])/i, "").replaceAll("\\", "/"));
  }
  const [query, ...rest] = pos;
  if (!query || query === "help") return console.log(CODE_GRAPH_USAGE);
  const dir = resolve(opts.dir);
  if (query === "update") {
    const info = await updateGraph(dir, { force: opts.force });
    if (info.gitignore?.added) console.log(`added ${info.gitignore.pattern} to ${info.gitignore.gitignore}`);
    console.log(`${info.db}\n${info.files} files · parsed ${info.parsed} · removed ${info.removed} · unresolved imports ${info.unresolvedImports} · ${info.ms} ms`);
    for (const e of info.parseErrors) console.log(`parse error: ${e}`);
    return;
  }
  const { db, info } = await openGraph(dir);
  try {
    if (query === "note") {
      const { writeNote } = await import("./notes.mjs");
      const { graph } = await import("./query.mjs");
      const [target, ...words] = rest;
      console.log(JSON.stringify(writeNote(db, graph(db), target, words.join(" "))));
      return;
    }
    if (info.gitignore?.added) console.log(`(added ${info.gitignore.pattern} to ${info.gitignore.gitignore})`);
    console.log(runQuery(db, query, rest.join(" ") || undefined, { depth: opts.depth ? Number(opts.depth) : undefined, limit: opts.limit ? Number(opts.limit) : 40, root: dir }));
  } finally {
    db.close();
  }
}
