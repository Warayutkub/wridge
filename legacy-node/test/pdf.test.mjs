import { test } from "node:test";
import assert from "node:assert/strict";
import { PDFDocument } from "pdf-lib";
import { designPdf, mergePdfs, orderPages, pageNumber, pdfFileName } from "../pdf.mjs";

const frame = (name, x = 0, y = 0, extra = {}) => ({ id: name, name, type: "FRAME", x, y, width: 595, height: 842, ...extra });

test("page number: leading digits as a number, no padding needed", () => {
  assert.equal(pageNumber("01 Cover"), 1);
  assert.equal(pageNumber("100 ภาคผนวก"), 100);
  assert.equal(pageNumber("  7"), 7);
  assert.equal(pageNumber("Cover"), null);
  assert.equal(pageNumber("A1"), null);
});

test("pages sort as numbers: 1, 2, 10, 100, 1000 (not as text)", () => {
  const { pages, warnings } = orderPages([frame("1000 z"), frame("10 c"), frame("2 b", 900), frame("100 d"), frame("01 a", 2000)]);
  assert.deepEqual(pages.map((p) => p.name), ["01 a", "2 b", "10 c", "100 d", "1000 z"]);
  assert.deepEqual(warnings, []);
});

test("unnumbered frames go last by position; hidden and non-frames are left out", () => {
  const { pages, warnings, skipped } = orderPages([
    frame("note", 0, 900), frame("2 b"), frame("cover", 0, 0), frame("1 a"),
    frame("3 hidden", 0, 0, { visible: false }), { id: "t", name: "4 text", type: "TEXT", x: 0, y: 0 },
  ]);
  assert.deepEqual(pages.map((p) => p.name), ["1 a", "2 b", "cover", "note"]);
  assert.deepEqual(skipped, ["3 hidden"]);
  assert.equal(warnings.length, 2);
});

test("duplicate numbers keep position order and warn", () => {
  const { pages, warnings } = orderPages([frame("2 right", 700), frame("2 left", 0), frame("1")]);
  assert.deepEqual(pages.map((p) => p.name), ["1", "2 left", "2 right"]);
  assert.match(warnings[0], /number 2 is used twice/);
});

async function onePage(w, h) {
  const d = await PDFDocument.create();
  d.addPage([w, h]);
  return Buffer.from(await d.save());
}

test("merge keeps every page in order", async () => {
  const merged = await PDFDocument.load(await mergePdfs([await onePage(595, 842), await onePage(842, 595), await onePage(100, 200)], "Book"));
  assert.deepEqual(merged.getPages().map((p) => [Math.round(p.getWidth()), Math.round(p.getHeight())]), [[595, 842], [842, 595], [100, 200]]);
  assert.equal(merged.getTitle(), "Book");
});

test("designPdf: section → ordered export, title without kind/status", async () => {
  const pdf = await onePage(595, 842);
  const asked = [];
  const send = async (cmd, args) => {
    if (cmd === "get_node") return { id: "s", name: "Laravel DOC 1 · Doc · Draft", type: "SECTION", children: [frame("2 b"), frame("1 a")] };
    asked.push(args.items[0].nodeId);
    assert.equal(args.items[0].format, "pdf");
    return [{ base64: pdf.toString("base64") }];
  };
  const out = await designPdf(send, "s");
  assert.deepEqual(asked, ["1 a", "2 b"]);
  assert.equal(out.title, "Laravel DOC 1");
  assert.equal((await PDFDocument.load(out.pdf)).getPageCount(), 2);
  assert.equal(pdfFileName('a/b: "c"'), "a-b- -c-.pdf");
});

test("designPdf refuses nodes that are not a section or frame", async () => {
  await assert.rejects(designPdf(async () => ({ name: "x", type: "TEXT" }), "1"), /pick a section/);
  await assert.rejects(designPdf(async () => ({ name: "empty", type: "SECTION", children: [] }), "1"), /No frames/);
});
