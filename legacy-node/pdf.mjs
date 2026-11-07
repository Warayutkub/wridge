// Figma frames → one PDF, a page per frame, like a document or a book.
// Order: the number at the start of the frame name ("1 Cover", "02 Intro", "100 …"), compared as numbers so
// no zero-padding is needed; frames without a number follow, top-to-bottom then left-to-right.
import { PDFDocument } from "pdf-lib";

const PAGE_TYPES = new Set(["FRAME", "COMPONENT", "INSTANCE"]);

export const pageNumber = (name) => {
  const m = /^\s*(\d+)(?!\d)/.exec(String(name));
  return m ? Number(m[1]) : null;
};

// children: get_node children of a section (or page) → { pages, skipped, warnings, issues }
// issues: { duplicates: [{ n, names }], unnumbered: [name], hidden: [name] } — the same as warnings, for a UI to draw
export function orderPages(children) {
  const byPlace = (a, b) => a.y - b.y || a.x - b.x;
  const candidates = (children ?? []).filter((c) => PAGE_TYPES.has(c.type));
  const skipped = candidates.filter((c) => c.visible === false).map((c) => c.name);
  const shown = candidates.filter((c) => c.visible !== false).map((c) => ({ ...c, n: pageNumber(c.name) }));
  const numbered = shown.filter((c) => c.n !== null).sort((a, b) => a.n - b.n || byPlace(a, b));
  const loose = shown.filter((c) => c.n === null).sort(byPlace);
  const warnings = [];
  const seen = new Map();
  for (const c of numbered) seen.set(c.n, [...(seen.get(c.n) ?? []), c.name]);
  const duplicates = [...seen].filter(([, names]) => names.length > 1).map(([n, names]) => ({ n, names }));
  const unnumbered = numbered.length ? loose.map((c) => c.name) : [];
  for (const { n, names } of duplicates) warnings.push(`Page number ${n} is used twice: ${names.join(", ")} (ordered by position)`);
  if (unnumbered.length) warnings.push(`No number at the start of the name, so they go last: ${unnumbered.join(", ")}`);
  if (skipped.length) warnings.push(`Hidden, left out: ${skipped.join(", ")}`);
  const pages = [...numbered, ...loose].map(({ id, name, width, height }) => ({ id, name, width, height }));
  return { pages, skipped, warnings, issues: { duplicates, unnumbered, hidden: skipped } };
}

// One-page (or more) PDFs from Figma, in order → one PDF.
export async function mergePdfs(buffers, title) {
  const out = await PDFDocument.create();
  for (const buf of buffers) {
    const src = await PDFDocument.load(buf);
    for (const page of await out.copyPages(src, src.getPageIndices())) out.addPage(page);
  }
  if (title) out.setTitle(title);
  out.setProducer("wridge");
  return Buffer.from(await out.save());
}

export const pdfFileName = (name) => `${String(name).replace(/[\\/:*?"<>|]+/g, "-").replace(/\s+/g, " ").trim() || "design"}.pdf`;

// Sections (their frames as pages, in order) or single frames, one after another → { pdf, title, pages, warnings }.
// ids: one node id or a list; the document follows the list's order. send = the bridge's command sender.
export async function designPdf(send, ids) {
  const list = Array.isArray(ids) ? ids : [ids];
  const pages = [], warnings = [], titles = [];
  for (const nodeId of list) {
    const node = await send("get_node", { nodeId, depth: 1 });
    let order;
    if (node.type === "SECTION") order = orderPages(node.children);
    else if (PAGE_TYPES.has(node.type)) order = { pages: [node], warnings: [] };
    else throw new Error(`${node.name} is a ${node.type}: pick a section (its frames become the pages) or a frame`);
    if (!order.pages.length) throw new Error(`No frames in ${node.name} to use as pages`);
    pages.push(...order.pages);
    warnings.push(...order.warnings);
    titles.push(String(node.name).split(" · ")[0].trim());
  }
  const buffers = [];
  for (const page of pages) {
    const [asset] = await send("export_assets", { items: [{ nodeId: page.id, format: "pdf", scale: 1 }] }, 120_000);
    buffers.push(Buffer.from(asset.base64, "base64"));
  }
  const title = titles.length > 1 ? `${titles[0]} +${titles.length - 1}` : titles[0];
  return { pdf: await mergePdfs(buffers, title), title, pages, warnings };
}
