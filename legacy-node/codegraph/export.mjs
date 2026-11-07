// Whole-graph JSON for the "กราฟโค้ด" web view. The page computes used-by / impact itself from these edges.
import { getMeta } from "./db.mjs";
import { componentLinks, designSnapshot, tokenTable } from "./design.mjs";
import { allNotes } from "./notes.mjs";
import { graph } from "./query.mjs";

// Schema + who reads/writes it, for the "โครงสร้างข้อมูล" web view (ER diagram and 3D).
export function exportData(db, root) {
  let model = null;
  try {
    model = JSON.parse(getMeta(db, "data_model") ?? "null");
  } catch {
    model = null;
  }
  const json = (v) => (v ? JSON.parse(v) : null);
  const refs = db
    .prepare("SELECT d.entity, d.op, d.form, d.columns, d.filters, d.line, f.path FROM data_refs d JOIN files f ON f.id = d.file_id WHERE d.op != 'ddl' ORDER BY f.path, d.line")
    .all()
    .map((r) => ({ ...r, columns: json(r.columns), filters: json(r.filters) }));
  const name = root.replaceAll("\\", "/");
  return {
    root: name,
    name: name.split("/").pop(),
    updatedAt: getMeta(db, "updated_at") ?? null,
    stores: (model?.stores ?? []).map((s) => ({
      kind: s.kind,
      label: s.label,
      sources: s.sources ?? [],
      unparsed: s.unparsed?.length ?? 0,
      entities: Object.values(s.entities ?? {}),
    })),
    refs,
  };
}

export function exportGraph(db, root) {
  const g = graph(db);
  const files = [...g.files.values()].map((f) => ({
    id: f.id,
    path: f.path,
    lang: f.lang,
    directive: f.directive ?? undefined,
    error: f.parse_error ?? undefined,
    routes: g.routes.filter((r) => r.file_id === f.id).map((r) => `${r.kind} ${r.path}`),
  }));
  const symbols = [...g.symbolById.values()]
    .filter((s) => s.kind !== "alias")
    .map((s) => ({
      id: s.id, file: s.file_id, name: s.name, kind: s.kind, line: s.line, end: s.end_line,
      exported: s.exported ? 1 : 0, isDefault: s.is_default ? 1 : 0,
      props: s.props ? JSON.parse(s.props) : undefined,
    }));

  // file → file (any import kind), weighted by how many names are imported
  const fileEdges = new Map();
  const externals = new Map(); // file id → { pkg: count }
  for (const [fileId, imps] of g.importsOf) {
    for (const i of imps) {
      if (i.target_file_id && i.target_file_id !== fileId) {
        const k = `${fileId}>${i.target_file_id}`;
        fileEdges.set(k, (fileEdges.get(k) ?? 0) + 1);
      } else if (i.external) {
        if (!externals.has(fileId)) externals.set(fileId, {});
        const e = externals.get(fileId);
        e[i.external] = (e[i.external] ?? 0) + 1;
      }
    }
  }

  // symbol → symbol uses ([from, to, kind, line]); from = owner symbol id, or -fileId for module-level code
  const symbolEdges = new Map();
  for (const [fileId, refs] of g.refsOf) {
    for (const r of refs) {
      const b = g.bindingTarget(fileId, r.local);
      let target = b?.symbol ?? null;
      if (b?.namespace && r.member) target = g.definition(b.file.id, r.member.split(".")[0]);
      if (!target || target.kind === "alias") continue;
      const from = r.symbol_id ?? -fileId;
      if (from === target.id) continue;
      const k = `${from}>${target.id}>${r.kind}`;
      if (!symbolEdges.has(k)) symbolEdges.set(k, [from, target.id, r.kind, r.line, r.count]);
      else symbolEdges.get(k)[4] += r.count;
    }
  }

  return {
    root: root.replaceAll("\\", "/"),
    name: root.replaceAll("\\", "/").split("/").pop(),
    updatedAt: getMeta(db, "updated_at") ?? null,
    files,
    symbols,
    fileEdges: [...fileEdges].map(([k, n]) => [...k.split(">").map(Number), n]),
    symbolEdges: [...symbolEdges.values()],
    externals: Object.fromEntries(externals),
    routes: g.routes.map((r) => ({ file: r.file_id, path: r.path, kind: r.kind, framework: r.framework, root: r.root })),
    ...designData(db, g, root),
    notes: allNotes(db, g).map((n) => ({ target: n.target, text: n.text, status: n.status, updatedAt: n.updated_at })),
  };
}

// Figma links (linked in wara.components.json, or suggested by name) and the token table.
function designData(db, g, root) {
  const snap = designSnapshot(db);
  const links = componentLinks(g, root, snap);
  const t = tokenTable(db, g, snap);
  return {
    design: {
      file: snap?.file ?? null,
      fetchedAt: snap?.fetchedAt ?? null,
      links: [
        ...links.linked.filter((l) => l.symbol).map((l) => ({ symbol: l.symbol.id, figma: l.figmaName, figmaId: l.figma?.id ?? null, status: "linked" })),
        ...links.suggested.map((l) => ({ symbol: l.symbol.id, figma: l.figma.name, figmaId: l.figma.id, status: "suggested" })),
      ],
      broken: links.linked.filter((l) => (l.classes ? l.classes.missing.length : !l.symbol)).map((l) => l.figmaName),
      classLinks: links.linked.filter((l) => l.classes).map((l) => ({ figma: l.figmaName, figmaId: l.figma?.id ?? null, element: l.entry.element ?? null, className: l.entry.className, variants: l.entry.variants ?? {}, files: l.classes.files, missing: l.classes.missing })),
      figmaOnly: links.figmaOnly.map((c) => c.name),
    },
    tokens: t.rows.map((r) => ({
      name: r.name.startsWith("figma:") ? r.figma.token : r.name,
      figma: r.figma ? { token: r.figma.token, value: r.figma.value } : null,
      value: r.defs[0]?.value ?? null,
      resolved: r.resolved && r.resolved !== r.defs[0]?.value ? r.resolved : null,
      defined: r.defs[0] ? `${r.defs[0].path}:${r.defs[0].line}` : null,
      count: r.count,
      files: [...r.files],
      mismatch: r.mismatch,
    })),
    hardcoded: t.hardFiles.map((f) => ({ path: f.path, count: f.n })),
  };
}
