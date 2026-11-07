// node wara.mjs memory brief|overview|find … — project memory without the server.
import { resolve } from "node:path";

async function readStdin() {
  const chunks = [];
  for await (const c of process.stdin) chunks.push(c);
  return Buffer.concat(chunks).toString("utf8");
}

export async function memoryCli(argv) {
  process.removeAllListeners("warning");
  process.on("warning", (w) => { if (w.name !== "ExperimentalWarning") console.warn(w.message); });
  const { overview, find, findProjectRoot } = await import("./memory.mjs");
  const opts = {};
  const pos = [];
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--stdin") opts.stdin = true;
    else if (argv[i].startsWith("--")) opts[argv[i].slice(2)] = argv[++i];
    else pos.push(argv[i]);
  }
  const [cmd, ...rest] = pos;

  if (cmd === "brief") {
    // Hook mode: never fail loudly — a broken hook must not block the session.
    try {
      let cwd = opts.dir ?? process.cwd();
      if (opts.stdin) cwd = JSON.parse((await readStdin()) || "{}").cwd ?? cwd;
      const root = findProjectRoot(cwd);
      if (!root) return;
      const text = overview(root, { brief: true });
      if (text) console.log(`${text}\n(Injected by the wridge project-memory hook after compaction/resume — this is the agreed state; continue from it.)`);
    } catch {
      // no memory for this folder
    }
    return;
  }

  const dir = resolve(opts.dir ?? findProjectRoot(process.cwd()) ?? process.cwd());
  if (cmd === "overview") console.log(overview(dir));
  else if (cmd === "find" && rest.length) console.log(find(dir, rest.join(" ")));
  else console.log("node wara.mjs memory overview [--dir D] | find <topic> [--dir D] | brief [--stdin]");
}
