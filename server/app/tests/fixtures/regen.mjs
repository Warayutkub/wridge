// Regenerates node-api-tools.json and node-profiles.json from the Node server, after a tool or profile
// changed in both engines:  node server/app/tests/fixtures/regen.mjs
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const HERE = fileURLToPath(new URL(".", import.meta.url));
const ROOT = fileURLToPath(new URL("../../../../", import.meta.url));
const port = 4856;
const child = spawn(process.execPath, [`${ROOT}legacy-node/index.mjs`], { env: { ...process.env, WARA_PORT: "4855", WARA_HTTP_PORT: String(port) }, stdio: "ignore" });
const base = `http://127.0.0.1:${port}`;
try {
  for (let i = 0; i < 100; i++) { try { await fetch(`${base}/api/status`); break; } catch { await new Promise((r) => setTimeout(r, 150)); } }
  writeFileSync(`${HERE}node-api-tools.json`, await (await fetch(`${base}/api/tools`)).text());
  const profiles = {};
  for (const profile of ["core", "code", "figjam", "prototype", "design", "all", "bogus"]) {
    const post = async (body, sid) => {
      const r = await fetch(`${base}/mcp?profile=${profile}`, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...(sid ? { "mcp-session-id": sid } : {}) }, body: JSON.stringify(body) });
      const text = await r.text();
      const json = JSON.parse(text.startsWith("event:") || text.startsWith("data:") ? text.split("\n").find((l) => l.startsWith("data:")).slice(5) : text);
      return { sid: r.headers.get("mcp-session-id") ?? sid, json };
    };
    const init = await post({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "regen", version: "1" } } });
    await post({ jsonrpc: "2.0", method: "notifications/initialized" }, init.sid).catch(() => {});
    const list = await post({ jsonrpc: "2.0", id: 2, method: "tools/list" }, init.sid);
    profiles[profile] = list.json.result.tools.map((t) => t.name);
  }
  writeFileSync(`${HERE}node-profiles.json`, JSON.stringify(profiles, null, 1) + "\n");
  console.log("fixtures written:", Object.entries(profiles).map(([k, v]) => `${k} ${v.length}`).join(", "));
} finally {
  child.kill();
}
