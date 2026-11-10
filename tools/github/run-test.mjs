// Check the window pages by hand (GitHub, Tasks, Reports, Settings → Load testing): node tools/github/run-test.mjs, then open
// http://127.0.0.1:4756/github, /tasks?dir=<project> or /reports?dir=<project>, or /app#/settings (the window page, without the Tauri parts)
// (build the debug server first: cargo build --manifest-path server/app/Cargo.toml)
// fake GitHub (4710) + a fake k6 release (4720) + the debug wridge server on 4755/4756 pointed at them
// (memory store, no browser opening, downloaded tools in a temp folder)
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { join } from "node:path";
import "./fake-github.mjs";
import "../loadtest/fake-k6-release.mjs";
const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const exe = `${ROOT}server/app/target/debug/wridge-server.exe`;
const env = { ...process.env, WARA_PORT: "4755", WARA_HTTP_PORT: "4756", WRIDGE_GITHUB_API: "http://127.0.0.1:4710", WRIDGE_GITHUB_WEB: "http://127.0.0.1:4710", WRIDGE_GITHUB_STORE: "memory", WRIDGE_GITHUB_OPEN: "log",
  WRIDGE_K6_RELEASES: "http://127.0.0.1:4720/releases/latest", WRIDGE_DATA_DIR: join(tmpdir(), "wridge-test-data"),
  // its own projects list, so a test never edits yours (set WRIDGE_PROJECTS_FILE to choose another)
  WRIDGE_PROJECTS_FILE: process.env.WRIDGE_PROJECTS_FILE ?? join(tmpdir(), "wridge-test-projects.json") };
const c = spawn(exe, [], { env, cwd: ROOT, stdio: "inherit" });
c.on("exit", (code) => { console.log("server exited", code); process.exit(code ?? 0); });
