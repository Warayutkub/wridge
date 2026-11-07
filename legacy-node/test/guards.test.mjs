// The wrong-file guard: writes need a checked file, and that file must still be open.
import assert from "node:assert/strict";
import { test } from "node:test";
import { FIGMA_WRITES, createFileGuard } from "../guards.mjs";

function fakeFigma(fileName) {
  const calls = [];
  const state = { fileName };
  const send = async (command, args) => {
    calls.push(command);
    if (command === "get_document_info") return { fileName: state.fileName, pages: [] };
    if (command === "design_map") return { file: state.fileName, pages: [] };
    if (command === "file_name") return { fileName: state.fileName };
    return { ok: true, command, args };
  };
  return { send, calls, state };
}

test("a write before any file check is refused; reads pass", async () => {
  const f = fakeFigma("utils");
  const send = createFileGuard(f.send);
  await assert.rejects(send("create_frame", {}), /call get_document_info/);
  assert.deepEqual(await send("get_node", { nodeId: "1:2" }), { ok: true, command: "get_node", args: { nodeId: "1:2" } });
});

test("after get_document_info (or design_map) writes go through", async () => {
  for (const check of ["get_document_info", "design_map"]) {
    const f = fakeFigma("utils");
    const send = createFileGuard(f.send);
    await send(check, {});
    assert.equal((await send("create_frame", { name: "A" })).ok, true);
  }
});

test("another file opened since the check: refused, and a new check is required", async () => {
  let t = 0;
  const f = fakeFigma("utils");
  const send = createFileGuard(f.send, { ttlMs: 2000, now: () => t });
  await send("get_document_info", {});
  f.state.fileName = "flowklass";
  t = 5000;
  await assert.rejects(send("rename_node", {}), /changed from "utils" to "flowklass"/);
  t = 5001;
  await assert.rejects(send("rename_node", {}), /call get_document_info/);
  await send("get_document_info", {});
  assert.equal((await send("rename_node", {})).ok, true);
});

test("the open file is re-read at most once per ttl (run_batch stays fast)", async () => {
  let t = 0;
  const f = fakeFigma("utils");
  const send = createFileGuard(f.send, { ttlMs: 2000, now: () => t });
  await send("get_document_info", {});
  t = 3000;
  for (let i = 0; i < 5; i++) await send("create_text", {});
  assert.equal(f.calls.filter((c) => c === "file_name").length, 1);
});

test("every Figma write tool is guarded; code/data tools are not", () => {
  for (const name of ["create_frame", "delete_node", "rename_node", "set_props", "create_section", "add_interaction", "set_flow_start"]) assert.ok(FIGMA_WRITES.has(name), name);
  for (const name of ["get_design_context", "code_graph", "project_memory", "map_component", "export_assets", "select_nodes"]) assert.ok(!FIGMA_WRITES.has(name), name);
});
