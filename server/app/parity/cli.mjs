// CLI parity: `node legacy-node/wara.mjs …` against one server and the `wridge` binary against another,
// same arguments; compares stdout, stderr and exit codes.
//   node server/app/parity/cli.mjs <nodeHttpPort> <rustHttpPort>      (e.g. 3356 3256 — not 3156 for writes)
// Only read-only commands, plus run_batch / tool calls that fail before reaching Figma.
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..", "..", "..");
const [nodePort = 3356, rustPort = 3256] = process.argv.slice(2).map(Number);
const WRIDGE = join(HERE, "..", "target", "debug", process.platform === "win32" ? "wridge.exe" : "wridge");

const runs = [
  [], ["help"], ["guide"], ["skills"], ["skill", "motion"], ["skill", "nope"], ["list"],
  ["get_skill", "name=ui-visual"], ["describe_tool", "name=create_frame"], ["describe_tool", '{"name":"run_batch"}'],
  ["nope"], ["get_node"], ["create_frame", "width=-1"], ["use_file", "file=figjam"],
  ["run_batch", '{"steps":[{"tool":"nope"}]}'], ["call_tool", "name=get_skill"],
];

const norm = (s, port) => s.replaceAll(String(port), "<port>").replace(/\r\n/g, "\n");
// The usage text names its own program: legacy-node is `node wara.mjs`, the installed CLI is `wridge`.
const normUsage = (s) => s
  .replace("wara — control Figma through the wara-bridge server", "wridge — control Figma through the wridge server")
  .replace("how to use wara-bridge (read this first)", "how to use wridge (read this first)")
  .replaceAll("node wara.mjs ", "wridge ");
let same = 0;
for (const args of runs) {
  const a = spawnSync(process.execPath, [join(ROOT, "legacy-node", "wara.mjs"), ...args], { env: { ...process.env, WARA_HTTP_PORT: String(nodePort) }, encoding: "utf8" });
  const b = spawnSync(WRIDGE, args, { env: { ...process.env, WARA_HTTP_PORT: String(rustPort) }, encoding: "utf8" });
  const x = { code: a.status, out: normUsage(norm(a.stdout, nodePort)), err: normUsage(norm(a.stderr, nodePort)) };
  const y = { code: b.status, out: norm(b.stdout, rustPort), err: norm(b.stderr, rustPort) };
  if (JSON.stringify(x) === JSON.stringify(y)) same++;
  else console.log(`DIFF wara ${args.join(" ")}\n  node: ${JSON.stringify(x).slice(0, 600)}\n  rust: ${JSON.stringify(y).slice(0, 600)}`);
}
// server down: both explain and exit 2
const down = (cmd, args, env) => spawnSync(cmd, args, { env: { ...process.env, ...env }, encoding: "utf8" });
const a = down(process.execPath, [join(ROOT, "legacy-node", "wara.mjs"), "status"], { WARA_HTTP_PORT: "3999" });
const b = down(WRIDGE, ["status"], { WARA_HTTP_PORT: "3999" });
if (a.status === b.status && a.stderr === b.stderr) same++;
else console.log(`DIFF server down\n  node: ${a.status} ${JSON.stringify(a.stderr)}\n  rust: ${b.status} ${JSON.stringify(b.stderr)}`);
console.log(`cli: ${same}/${runs.length + 1} identical`);
process.exit(same === runs.length + 1 ? 0 : 1);
