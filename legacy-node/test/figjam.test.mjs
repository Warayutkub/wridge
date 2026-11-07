import assert from "node:assert/strict";
import { test } from "node:test";
import { boardText, layoutDiagram } from "../figjam.mjs";

const BOOKING = {
  title: "จองบูธ · System design · Draft",
  lanes: ["ร้านค้า", "server"],
  nodes: [
    { id: "s", kind: "start", lane: "ร้านค้า" },
    { id: "pick", text: "เลือกบูธ", lane: "ร้านค้า" },
    { id: "hold", text: "ล็อกบูธ 15 นาที", lane: "server" },
    { id: "ok", text: "สลิปตรงไหม", kind: "decision", lane: "server" },
    { id: "ticket", text: "ออกตั๋ว", lane: "server" },
    { id: "e", kind: "end", lane: "ร้านค้า" },
  ],
  edges: [
    { from: "s", to: "pick" }, { from: "pick", to: "hold" }, { from: "hold", to: "ok" },
    { from: "ok", to: "ticket", label: "ตรง" }, { from: "ok", to: "pick", label: "ไม่ตรง" }, { from: "ticket", to: "e" },
  ],
  notes: [{ text: "ร้านแก้สลิปได้กี่ครั้ง?", kind: "question" }],
};

test("columns follow the arrows, lanes follow the actor, loops are dashed", async () => {
  const d = await layoutDiagram(BOOKING);
  const at = Object.fromEntries(d.nodes.map((n) => [n.id, n]));
  assert.ok(at.s.x < at.pick.x && at.pick.x < at.hold.x && at.hold.x < at.ok.x && at.ok.x < at.ticket.x && at.ticket.x < at.e.x);
  const lane = (n) => d.lanes.findIndex((l) => n.y >= l.y && n.y + n.height <= l.y + l.height);
  assert.equal(lane(at.pick), 0);
  assert.equal(lane(at.hold), 1);
  assert.equal(at.ok.shape, "DIAMOND");
  const back = d.edges.find((e) => e.from === "ok" && e.to === "pick");
  assert.equal(back.dashed, true, "going back is a loop");
  assert.equal(d.edges.find((e) => e.to === "ticket").dashed, undefined);
  const sticky = d.nodes.find((n) => n.kind === "sticky");
  assert.ok(sticky.x > d.lanes[0].width, "stickies sit beside the diagram");
  assert.ok(d.section.width >= sticky.x + 200);
});

test("bad input says what is wrong", async () => {
  await assert.rejects(() => layoutDiagram({ nodes: [{ id: "a", text: "x" }], edges: [{ from: "a", to: "b" }] }), /unknown node/);
  await assert.rejects(() => layoutDiagram({ lanes: ["A"], nodes: [{ id: "a", text: "x", lane: "B" }] }), /lane "B" is not in lanes/);
  await assert.rejects(() => layoutDiagram({ nodes: [{ id: "a", text: "x", kind: "question" }] }), /sticky/);
});

test("board text: steps with lanes, arrows by diagram id, stickies", async () => {
  const map = {
    file: "book-lock board", editorType: "figjam", loose: [],
    sections: [{
      id: "1:1", name: "จองบูธ · System design · Draft", lanes: [{ name: "ร้านค้า", x: 0, y: 72, w: 1100, h: 160 }, { name: "server", x: 0, y: 232, w: 1100, h: 160 }],
      items: [
        { id: "2:1", kind: "shape", role: "step", waraId: "pick", x: 300, y: 100, w: 220, h: 96, text: "เลือกบูธ" },
        { id: "2:2", kind: "shape", role: "step", waraId: "hold", x: 600, y: 260, w: 220, h: 96, text: "ล็อกบูธ 15 นาที" },
        { id: "2:3", kind: "sticky", role: "question", x: 1200, y: 72, w: 200, h: 200, text: "แก้สลิปได้กี่ครั้ง?" },
        { id: "3:1", kind: "connector", from: "2:1", to: "2:2", label: "ยืนยัน" },
      ],
    }],
  };
  const t = boardText(map);
  assert.match(t, /pick · step @ร้านค้า · "เลือกบูธ"/);
  assert.match(t, /hold · step @server/);
  assert.match(t, /pick → hold \[ยืนยัน\]/);
  assert.match(t, /sticky question · "แก้สลิป/, "a sticky beside the lanes has no lane");
});

test("state: nodes default to states, transitions keep their labels", async () => {
  const d = await layoutDiagram({ type: "state", title: "booking", nodes: [{ id: "hold", text: "hold" }, { id: "paid", text: "paid" }], edges: [{ from: "hold", to: "paid", label: "slip ok" }] });
  assert.ok(d.nodes.every((n) => n.role === "state" && n.shape === "ELLIPSE"));
  assert.equal(d.edges[0].label, "slip ok");
});

test("er: referenced tables sit left, FK arrows point at them, columns listed", async () => {
  const d = await layoutDiagram({ type: "er", tables: [
    { name: "bookings", columns: [{ name: "id", type: "uuid", pk: true }, { name: "booth_id", type: "uuid", fk: "booths.id" }] },
    { name: "booths", columns: [{ name: "id", type: "uuid", pk: true }, { name: "label", type: "text" }] },
  ] });
  const at = Object.fromEntries(d.nodes.map((n) => [n.id, n]));
  assert.ok(at.booths.x < at.bookings.x);
  assert.deepEqual(d.edges.map((e) => [e.from, e.to, e.label]), [["bookings", "booths", "booth_id"]]);
  assert.match(at.bookings.text, /🔑 id {2}uuid/);
  assert.match(at.bookings.text, /→ booth_id {2}uuid {2}\(booths\.id\)/);
  await assert.rejects(() => layoutDiagram({ type: "er", tables: [{ name: "a", columns: [{ name: "x_id", fk: "x.id" }] }] }), /table "x" is not in tables/);
});

test("sequence: a lifeline per participant, messages top to bottom between them", async () => {
  const d = await layoutDiagram({ type: "sequence", participants: ["web", "server", "Slip2Go"], messages: [
    { from: "web", to: "server", text: "upload slip" }, { from: "server", to: "Slip2Go", text: "verify" }, { from: "Slip2Go", to: "server", text: "result", kind: "reply" },
  ] });
  const lifelines = d.edges.filter((e) => e.id.startsWith("life"));
  const msgs = d.edges.filter((e) => e.id.startsWith("m"));
  assert.equal(lifelines.length, 3);
  assert.ok(lifelines.every((l) => l.noArrow && l.fromPoint.x === l.toPoint.x));
  assert.ok(msgs[0].fromPoint.y < msgs[1].fromPoint.y && msgs[1].fromPoint.y < msgs[2].fromPoint.y);
  assert.equal(msgs[2].dashed, true);
  assert.equal(msgs[0].label, "1. upload slip");
  await assert.rejects(() => layoutDiagram({ type: "sequence", participants: ["a", "b"], messages: [{ from: "a", to: "c", text: "x" }] }), /c is not a participant/);
});
