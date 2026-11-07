// The Node side of parity/wave2.mjs: runs a list of calls [{ fn: "<module>.<jsName>", args }] against
// legacy-node/codegraph (brain, memory, notes, knowledge, flows, design, query, export) and prints one
// JSON line per call: { ok: result } or { error: message }. Objects returned by reference (graph,
// componentLinks, tokenTable) are printed as the projections examples/wave2.rs prints.
// usage: node node-wave2.mjs <calls.json>
import { readFileSync } from "node:fs";
import { openGraph, projectRoot } from "../../../legacy-node/codegraph/build.mjs";
import * as brain from "../../../legacy-node/codegraph/brain.mjs";
import * as memory from "../../../legacy-node/codegraph/memory.mjs";
import * as knowledge from "../../../legacy-node/codegraph/knowledge.mjs";
import * as flows from "../../../legacy-node/codegraph/flows.mjs";
import * as notes from "../../../legacy-node/codegraph/notes.mjs";
import * as design from "../../../legacy-node/codegraph/design.mjs";
import * as exp from "../../../legacy-node/codegraph/export.mjs";
import { graph, runQuery } from "../../../legacy-node/codegraph/query.mjs";

process.removeAllListeners("warning");

async function withGraph(dir, fn) {
  const { db } = await openGraph(dir);
  try {
    return await fn(graph(db), db);
  } finally {
    db.close();
  }
}

const symId = (s) => (s ? s.id : null);

function graphProjection(g) {
  const refs = [...g.refsOf.values()].flat().sort((a, b) => a.id - b.id);
  const symbols = [...g.symbolById.values()];
  return {
    files: g.files.size,
    symbols: symbols.map((s) => [s.id, s.kind]),
    routes: g.routes.length,
    bindings: refs.map((r) => {
      const b = g.bindingTarget(r.file_id, r.local);
      if (!b) return null;
      return { external: b.external ?? null, imported: b.imported ?? null, unresolved: b.unresolved ?? null, file: b.file?.path ?? null, namespace: Boolean(b.namespace), symbol: symId(b.symbol) };
    }),
    referencesTo: symbols.map((s) => g.referencesTo(s).map((r) => [r.id, symId(r.owner)])),
  };
}

function linksProjection(l) {
  return {
    linked: l.linked.map((x) => ({ figmaName: x.figmaName, figma: x.figma, entry: x.entry, symbol: symId(x.symbol), classes: x.classes ? { defined: x.classes.defined, missing: x.classes.missing, files: x.classes.files } : null })),
    suggested: l.suggested.map((x) => ({ figma: x.figma, symbol: x.symbol.id, exact: x.exact })),
    figmaOnly: l.figmaOnly,
    codeOnly: l.codeOnly.map((s) => s.id),
  };
}

function tableProjection(t) {
  return {
    rows: t.rows.map((r) => ({
      name: r.name,
      defs: r.defs.map((d) => ({ name: d.name, value: d.value, line: d.line, path: d.path })),
      figma: r.figma,
      uses: r.uses.map((u) => ({ form: u.form, name: u.name, line: u.line, count: u.count, path: u.path })),
      files: [...r.files], count: r.count, resolved: r.resolved, mismatch: r.mismatch,
    })),
    hardcoded: t.hardcoded.map((h) => ({ name: h.name, n: h.n, files: h.files })),
    hardFiles: t.hardFiles.map((h) => ({ path: h.path, n: h.n })),
  };
}

const CALLS = {
  "brain.parseFrontMatter": (t) => brain.parseFrontMatter(t),
  "brain.linkTarget": (r) => brain.linkTarget(r),
  "brain.parseNote": (t, p) => brain.parseNote(t, p),
  "brain.brainSummary": (d) => brain.brainSummary(d),
  "brain.findNotes": (d, q, o) => brain.findNotes(d, q, o),
  "brain.readNote": (d, n) => brain.readNote(d, n),
  "brain.brainGraph": (d) => brain.brainGraph(d),
  "brain.writeNote": (d, n, o) => brain.writeNote(d, n, o),
  "brain.trashNote": (d, n) => brain.trashNote(d, n),
  "brain.listTrash": (d) => brain.listTrash(d),
  "brain.restoreNote": (d, p) => brain.restoreNote(d, p),
  "brain.emptyTrash": (d, p) => brain.emptyTrash(d, p),
  "brain.codeLinkMissing": (d, t) => brain.codeLinkMissing(d, t),
  "brain.notesAbout": (d, o) => brain.notesAbout(d, o),
  "brain.codeLinks": (d) => brain.codeLinks(d),
  "memory.parseDecisions": (t) => memory.parseDecisions(t),
  "memory.overview": (d, o) => memory.overview(d, o),
  "memory.find": (d, q, o) => memory.find(d, q, o),
  "memory.decide": (d, o) => memory.decide(d, o),
  "memory.note": (d, o) => memory.note(d, o),
  "memory.handoff": (d, o) => memory.handoff(d, o),
  "memory.findProjectRoot": (c) => memory.findProjectRoot(c),
  "knowledge.major": (v) => knowledge.major(v),
  "knowledge.detectStack": (d) => knowledge.detectStack(projectRoot(d)),
  "knowledge.listKnowledge": (d, t) => knowledge.listKnowledge(d, t),
  "knowledge.getKnowledge": (d, t, n) => knowledge.getKnowledge(d, t, n),
  "knowledge.readFacts": (d) => knowledge.readFacts(d),
  "knowledge.setFact": (d, k, v) => knowledge.setFact(d, k, v),
  "knowledge.saveKnowledge": (d, o) => knowledge.saveKnowledge(d, o),
  "flows.hashText": (t) => flows.hashText(t),
  "flows.parseFlow": (t) => flows.parseFlow(t),
  "flows.formatFlow": (f) => flows.formatFlow(f),
  "flows.slugify": (n) => flows.slugify(n),
  "flows.readFlowFiles": (r) => flows.readFlowFiles(r),
  "flows.saveFlowFile": (r, n, t, h) => {
    try {
      return flows.saveFlowFile(r, n, t, h);
    } catch (e) {
      if (e instanceof flows.FlowConflict) return { conflict: e.message, current: e.current };
      throw e;
    }
  },
  "flows.readIgnored": (r) => [...flows.readIgnored(r)],
  "flows.ignoreDraft": (r, id) => (flows.ignoreDraft(r, id), null),
  "flows.checkRef": (d, ref) => withGraph(d, (g) => flows.checkRef(g, ref)),
  "flows.checkFlow": (d, f) => withGraph(d, (g) => flows.checkFlow(g, f)),
  "flows.trace": (d, t, o) => withGraph(d, (g) => flows.trace(g, t, o)),
  "flows.statusValues": (d, r, tables) => withGraph(d, (g) => flows.statusValues(g, r, new Set(tables))),
  "flows.draftFlows": (d, r, fl) => withGraph(d, (g) => flows.draftFlows(g, r, fl)),
  "flows.flowsOverview": (d) => withGraph(d, (g) => flows.flowsOverview(g, projectRoot(d))),
  "flows.flowsText": (d, r, t, l) => withGraph(d, (g) => flows.flowsText(g, r, t, l)),
  "flows.traceText": (d, t) => withGraph(d, (g) => flows.traceText(g, t)),
  "query.runQuery": async (d, q, t, o) => {
    const { db } = await openGraph(d);
    try {
      return runQuery(db, q, t ?? undefined, o);
    } finally {
      db.close();
    }
  },
  "query.graph": (d) => withGraph(d, (g) => graphProjection(g)),
  "notes.resolveNoteTarget": (d, t) => withGraph(d, (g) => notes.resolveNoteTarget(g, t)),
  "notes.writeNote": (d, t, text) => withGraph(d, (g, db) => notes.writeNote(db, g, t, text)),
  "notes.allNotes": (d) => withGraph(d, (g, db) => notes.allNotes(db, g)),
  "notes.missingNotes": (d, l) => withGraph(d, (g, db) => notes.missingNotes(db, g, l)),
  "notes.notesFor": (d, p) => withGraph(d, (g, db) => notes.notesFor(db, g, g.fileByPath(p))),
  "design.designSnapshot": (d) => withGraph(d, (g, db) => design.designSnapshot(db)),
  "design.componentLinks": (d, r) => withGraph(d, (g, db) => linksProjection(design.componentLinks(g, r, design.designSnapshot(db)))),
  "design.tokenTable": (d) => withGraph(d, (g, db) => tableProjection(design.tokenTable(db, g, design.designSnapshot(db)))),
  "design.importPathFor": (d, r, p) => withGraph(d, (g) => design.importPathFor(g, r, g.fileByPath(p))),
  "design.confirmLink": (d, r, n, id) => withGraph(d, (g) => design.confirmLink(g, r, n, id)),
  "design.syncDesign": async (d, info, components, data) => {
    const answers = [info, components, data];
    const sent = [];
    const { db } = await openGraph(d);
    try {
      const result = await design.syncDesign(db, async (cmd, params, timeout) => {
        sent.push([cmd, params, timeout ?? null]);
        return answers[sent.length - 1];
      });
      return { result, sent, snapshot: design.designSnapshot(db) };
    } finally {
      db.close();
    }
  },
  "export.exportData": async (d, r) => {
    const { db } = await openGraph(d);
    try {
      return exp.exportData(db, r);
    } finally {
      db.close();
    }
  },
  "export.exportGraph": async (d, r) => {
    const { db } = await openGraph(d);
    try {
      return exp.exportGraph(db, r);
    } finally {
      db.close();
    }
  },
  "http.brain": (d) => brain.brainGraph(projectRoot(d)),
  "http.codeLinks": (d) => brain.codeLinks(projectRoot(d)),
  "http.note": (d, n) => brain.readNote(projectRoot(d), n ?? ""),
  "http.trash": (d) => brain.listTrash(projectRoot(d)),
  "http.flows": (d) => withGraph(d, (g) => ({ name: projectRoot(d).replaceAll("\\", "/").split("/").pop(), ...flows.flowsOverview(g, projectRoot(d)) })),
  "http.flowsTrace": (d, t) => withGraph(d, (g) => flows.trace(g, t ?? "")),
};

const calls = JSON.parse(readFileSync(process.argv[2], "utf8"));
for (const c of calls) {
  let out;
  const t0 = Date.now();
  try {
    const fn = CALLS[c.fn];
    if (!fn) throw new Error(`unknown call ${c.fn}`);
    const v = await fn(...(c.args ?? []));
    out = { ok: v === undefined ? null : v };
  } catch (e) {
    out = { error: String(e?.message ?? e) };
  }
  if (process.env.WAVE2_TIMES) process.stderr.write(`${String(Date.now() - t0).padStart(6)} ms ${c.fn}\n`);
  console.log(JSON.stringify(out));
}
