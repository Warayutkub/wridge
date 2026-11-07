import { readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = `http://127.0.0.1:${process.env.WARA_HTTP_PORT ?? 3156}`;

const USAGE = `wara — control Figma through the wara-bridge server

  node wara.mjs guide                    how to use wara-bridge (read this first)
  node wara.mjs skills                   list skills (design / coding methods)
  node wara.mjs skill <name>             print one skill, e.g. skill ui-visual
  node wara.mjs status                   server + plugin status
  node wara.mjs list                     all tools and their parameters
  node wara.mjs <tool> key=value ...     run a tool (values are parsed as JSON when possible)
  node wara.mjs <tool> '{"json":true}'   run a tool with a JSON object
  node wara.mjs <tool> @args.json        run a tool with arguments from a JSON file (best for run_batch)
  node wara.mjs code-graph …             ask a code project's graph (works without the server; see code-graph help)
  node wara.mjs memory overview|find <topic> [--dir D]   project memory: open tasks + decisions (no server needed)

examples
  node wara.mjs get_selection depth=2
  node wara.mjs set_image_fill nodeId=12:34 source=C:/Users/me/pic.jpg scaleMode=FIT
  node wara.mjs copy_fills fromNodeId=1:2 'toNodeIds=["3:4","5:6"]'
  node wara.mjs export_node_image nodeId=12:34        (saves a PNG and prints its path)`;

function parseArgs(rest) {
  if (rest.length === 0) return {};
  if (rest[0].startsWith("@")) return JSON.parse(readFileSync(rest[0].slice(1), "utf8"));
  if (rest[0].trimStart().startsWith("{")) return JSON.parse(rest.join(" "));
  const args = {};
  for (const pair of rest) {
    const i = pair.indexOf("=");
    if (i < 1) throw new Error(`Expected key=value, got "${pair}"`);
    const raw = pair.slice(i + 1);
    let value = raw;
    try {
      value = JSON.parse(raw);
    } catch {
      // not JSON — keep as a plain string (node ids like 12:34, paths, text)
    }
    args[pair.slice(0, i)] = value;
  }
  return args;
}

async function request(path, body) {
  try {
    return await fetch(BASE + path, body === undefined ? {} : {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch {
    console.error(`wridge server is not running on ${BASE}.\nAsk the user to start it (open the wridge app and press Start server).`);
    process.exit(2);
  }
}

const call = async (path, body) => (await request(path, body)).json();

const [command, ...rest] = process.argv.slice(2);

if (!command || command === "help" || command === "--help") {
  console.log(USAGE);
} else if (command === "code-graph") {
  // Local only: reads the project's .wara/code-graph.db, so it works while the server is off.
  const { codeGraphCli } = await import("./codegraph/cli.mjs");
  await codeGraphCli(rest);
} else if (command === "memory") {
  // Local only. "memory brief --stdin" is the Claude Code SessionStart hook: reads the hook JSON (cwd)
  // from stdin and prints the project's open tasks + decision titles — or nothing outside a project.
  const { memoryCli } = await import("./codegraph/memory-cli.mjs");
  await memoryCli(rest);
} else if (command === "guide") {
  console.log(await (await request("/guide")).text());
} else if (command === "skills" || command === "skill") {
  const result = await call("/api/tool", { name: "get_skill", args: rest[0] ? { name: rest[0] } : {} });
  console.log(result.content?.[0]?.text ?? "");
  if (result.isError) process.exit(1);
} else if (command === "status") {
  const s = await call("/api/status");
  console.log(
    `server: running (pid ${s.pid})\nplugin: ${s.pluginConnected ? "connected" : "NOT connected — run the wridge plugin in Figma"}\npaused: ${s.paused}\ntools: ${s.toolCount}`,
  );
} else if (command === "list") {
  for (const t of await call("/api/tools")) {
    const props = t.params.properties ?? {};
    const required = new Set(t.params.required ?? []);
    const params = Object.entries(props)
      .map(([k, v]) => `${k}${required.has(k) ? "" : "?"}${v.default !== undefined ? `=${JSON.stringify(v.default)}` : ""}`)
      .join(" ");
    console.log(`${t.name} ${params}\n    ${t.description}`);
  }
} else {
  const result = await call("/api/tool", { name: command, args: parseArgs(rest) });
  for (const item of result.content ?? []) {
    if (item.type === "image") {
      const file = join(tmpdir(), `wara-${command}-${Date.now()}.png`);
      writeFileSync(file, Buffer.from(item.data, "base64"));
      console.log(`image saved: ${file}`);
    } else {
      console.log(item.text);
    }
  }
  if (result.isError) process.exit(1);
}
