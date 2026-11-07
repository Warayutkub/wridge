// The plugin WebSocket refuses real web origins (the plugin iframe sends Origin "null").
//   node server/app/parity/ws-guard.mjs <wsPort> [<otherWsPort>]   (never 3155/3055)
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "legacy-node", "package.json"));
const WebSocket = require("ws");
const ports = process.argv.slice(2).map(Number);
if (ports.some((p) => p === 3155 || p === 3055)) throw new Error("refusing the user's plugin ports");

function attempt(port, origin) {
  return new Promise((resolve) => {
    const ws = new WebSocket(`ws://localhost:${port}`, origin === undefined ? {} : { origin });
    ws.on("unexpected-response", (_req, res) => resolve(`refused ${res.statusCode}`));
    ws.on("message", (raw) => {
      const msg = JSON.parse(raw.toString());
      ws.close();
      resolve(`accepted, status ${JSON.stringify({ ...msg, dashboard: msg.dashboard.replace(/\d+$/, "<port>") })}`);
    });
    ws.on("error", (e) => resolve(`error ${e.message.replace(/\d{4}/g, "<port>")}`));
  });
}

const results = [];
for (const port of ports) {
  const r = [];
  for (const origin of [undefined, "null", "http://evil.example", "http://localhost:3256"]) r.push(`${origin ?? "(none)"}: ${await attempt(port, origin)}`);
  results.push(r);
  console.log(`:${port}\n  ${r.join("\n  ")}`);
}
if (results.length === 2) {
  const same = JSON.stringify(results[0]) === JSON.stringify(results[1]);
  console.log(same ? "ws guard: identical" : "ws guard: DIFFERENT");
  process.exit(same ? 0 : 1);
}
