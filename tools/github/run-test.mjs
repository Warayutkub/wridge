// Check the window pages by hand (GitHub, Tasks, Reports): node tools/github/run-test.mjs, then open
// http://127.0.0.1:4756/github, /tasks?dir=<project> or /reports?dir=<project> (/app only works inside the wridge window)
// (build the debug server first: cargo build --manifest-path server/app/Cargo.toml)
// fake GitHub (4710) + the debug wridge server on 4755/4756 pointed at it (memory store, no browser opening)
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { join } from "node:path";
import "./fake-github.mjs";
const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const exe = `${ROOT}server/app/target/debug/wridge-server.exe`;
const env = { ...process.env, WARA_PORT: "4755", WARA_HTTP_PORT: "4756", WRIDGE_GITHUB_API: "http://127.0.0.1:4710", WRIDGE_GITHUB_WEB: "http://127.0.0.1:4710", WRIDGE_GITHUB_STORE: "memory", WRIDGE_GITHUB_OPEN: "log",
  // its own projects list, so a test never edits yours (set WRIDGE_PROJECTS_FILE to choose another)
  WRIDGE_PROJECTS_FILE: process.env.WRIDGE_PROJECTS_FILE ?? join(tmpdir(), "wridge-test-projects.json") };
const c = spawn(exe, [], { env, cwd: ROOT, stdio: "inherit" });
c.on("exit", (code) => { console.log("server exited", code); process.exit(code ?? 0); });
