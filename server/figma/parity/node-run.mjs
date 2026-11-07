// Runs every parity case through the legacy Node modules and writes one result per case.
// Usage: node parity/node-run.mjs <cases.json> <out.json>
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const legacy = join(here, "../../../legacy-node");
const load = (m) => import(pathToFileURL(join(legacy, m)).href);

const mods = {
  codegen: await load("codegen.mjs"),
  lottie: await load("lottie.mjs"),
  motion: await load("motion.mjs"),
  "motion-figma": await load("motion-figma.mjs"),
  "motion-handoff": await load("motion-handoff.mjs"),
  figjam: await load("figjam.mjs"),
  pdf: await load("pdf.mjs"),
  path: (await import("node:path")).default,
};

// A few calls need glue: functions that mutate their input, take callbacks, or return buffers.
const special = {
  "codegen.annotateComponents": ([root, map]) => {
    const unmapped = mods.codegen.annotateComponents(root, map);
    return { unmapped, root };
  },
  "codegen.writeComponentMap": async ([dir, map]) => {
    const file = await mods.codegen.writeComponentMap(dir, map);
    return { file: file.replace(dir, "<write>"), text: readFileSync(file, "utf8") };
  },
  "lottie.svgLayers": ([text, opts]) => mods.lottie.svgLayers(text, opts),
  "motion-handoff.pipeline": ([read, context]) => {
    const h = mods["motion-handoff"];
    const warnings = [];
    const rows = h.classify(read, warnings, { context });
    const lints = h.lint(rows, read, { context });
    const tk = h.tokens(rows);
    const cssWarnings = [];
    const css = h.toCss(rows, tk, cssWarnings);
    return { rows, warnings, lints, tokens: tk, css, cssWarnings, framer: h.toFramer(rows, tk), check: h.checkText(read, rows, lints, context) };
  },
  "motion.markSentToFigma": ([dir, path, info]) => {
    const out = mods.motion.markSentToFigma(dir, path, info);
    const written = JSON.parse(readFileSync(join(dir, path), "utf8"));
    // sentAt is the clock: blank it in both runs
    out.figma.sentAt = "<now>";
    written.meta.wara.figma.sentAt = "<now>";
    return { out, written };
  },
  "pdf.mergePdfs": async ([files, title]) => {
    const buf = await mods.pdf.mergePdfs(files.map((f) => readFileSync(f)), title);
    return { base64: buf.toString("base64") };
  },
  "pdf.designPdf": async ([nodes, ids]) => {
    // the plugin round trips, answered from the case: get_node → nodes[id], export_assets → that frame's PDF file
    const asked = [];
    const send = async (cmd, args, timeout) => {
      asked.push([cmd, args, timeout ?? null]);
      if (cmd === "get_node") return structuredClone(nodes[args.nodeId]);
      const file = nodes.__pdf[args.items[0].nodeId] ?? nodes.__pdf.default;
      return [{ base64: readFileSync(file).toString("base64") }];
    };
    const out = await mods.pdf.designPdf(send, ids);
    return { title: out.title, pages: out.pages, warnings: out.warnings, asked, base64: out.pdf.toString("base64") };
  },
};

const cases = JSON.parse(readFileSync(process.argv[2], "utf8"));
const outDir = dirname(process.argv[2]);
const place = (a) => (a === "<copy>" ? join(outDir, "motion-project-node") : a === "<write>" ? join(outDir, "codegen-write-node") : a);
const results = [];
for (const c of cases) {
  const key = `${c.module}.${c.fn}`;
  try {
    const fn = special[key] ?? ((args) => mods[c.module][c.fn](...args));
    const value = await fn(structuredClone(c.args).map(place));
    results.push({ id: c.id, ok: JSON.stringify(value === undefined ? null : value) });
  } catch (e) {
    results.push({ id: c.id, error: String(e?.message ?? e) });
  }
}
writeFileSync(process.argv[3], JSON.stringify(results, null, 1));
console.log(`node: ${results.length} cases`);
