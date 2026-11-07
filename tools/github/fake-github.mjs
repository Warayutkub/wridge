// A fake GitHub for checking the wridge GitHub page by hand. Port 4710.
import http from "node:http";
const T0 = Date.now();
const iso = (s) => new Date(T0 + s * 1000).toISOString();
let polls = 0, installed = false;
const now = () => (Date.now() - T0) / 1000;
// run 130 runs for 40 s, then fails
function runs() {
  const done = now() > 40;
  const base = [
    { id: 130, run_number: 130, name: "CI", display_title: "Add the GitHub page", status: done ? "completed" : "in_progress", conclusion: done ? "failure" : null, head_branch: "feat/github", head_sha: "a1f9c2e44", event: "push", actor: { login: "Warayutkub" }, triggering_actor: { login: "Warayutkub" }, created_at: iso(-5), run_started_at: iso(-5), updated_at: done ? iso(40) : new Date().toISOString(), html_url: "https://github.com/Warayutkub/wridge/actions/runs/130" },
    { id: 129, run_number: 129, name: "CI", display_title: "Merge pull request #12 from feat/picker", status: "completed", conclusion: "success", head_branch: "main", head_sha: "7c21d0b11", event: "push", actor: { login: "Warayutkub" }, created_at: iso(-3600), run_started_at: iso(-3600), updated_at: iso(-3475), html_url: "https://github.com/Warayutkub/wridge/actions/runs/129" },
    { id: 128, run_number: 31, name: "Deploy site", display_title: "Website: real screenshots", status: "completed", conclusion: "failure", head_branch: "main", head_sha: "3e8812a00", event: "push", actor: { login: "mint" }, created_at: iso(-7200), run_started_at: iso(-7200), updated_at: iso(-7152), html_url: "https://github.com/Warayutkub/wridge/actions/runs/128" },
    { id: 127, run_number: 127, name: "CI", display_title: "Per-file queues", status: "completed", conclusion: "cancelled", head_branch: "feat/queue", head_sha: "0aa31c9ff", event: "pull_request", actor: { login: "Warayutkub" }, created_at: iso(-18000), run_started_at: iso(-18000), updated_at: iso(-17988), html_url: "https://github.com/Warayutkub/wridge/actions/runs/127" },
    { id: 126, run_number: 12, name: "Release", display_title: "v1.5.0", status: "completed", conclusion: "success", head_branch: "v1.5.0", head_sha: "5d1e07a33", event: "release", actor: { login: "Warayutkub" }, created_at: iso(-172800), run_started_at: iso(-172800), updated_at: iso(-172580), html_url: "https://github.com/Warayutkub/wridge/actions/runs/126" },
  ];
  return base;
}
function jobs(id) {
  const r = runs().find((x) => x.id === id) ?? runs()[1];
  const t = now(), done = r.status === "completed", bad = r.conclusion === "failure";
  const step = (n, name, a, b, concl) => ({ number: n, name, status: b <= t || done ? "completed" : a <= t ? "in_progress" : "queued", conclusion: b <= t || done ? concl : null, started_at: a <= t ? iso(a) : null, completed_at: b <= t || done ? iso(b) : null });
  if (id !== 130) {
    return [{ id: id * 10 + 1, name: "build", status: "completed", conclusion: bad ? "failure" : "success", started_at: r.run_started_at, completed_at: r.updated_at, labels: ["ubuntu-latest"],
      steps: [step(1, "Set up job", -9e9, -9e9, "success"), step(2, "Checkout", -9e9, -9e9, "success"), step(3, "Build", -9e9, -9e9, bad ? "failure" : "success"), step(4, "Deploy", -9e9, -9e9, bad ? "skipped" : "success")] }];
  }
  return [
    { id: 1301, name: "lint", status: t > 8 ? "completed" : "in_progress", conclusion: t > 8 ? "success" : null, started_at: iso(-5), completed_at: t > 8 ? iso(8) : null, labels: ["ubuntu-latest"], steps: [step(1, "Set up job", -5, -3, "success"), step(2, "Checkout", -3, -2, "success"), step(3, "cargo clippy", -2, 8, "success")] },
    { id: 1302, name: "test", status: t > 40 ? "completed" : t > 8 ? "in_progress" : "queued", conclusion: t > 40 ? "failure" : null, started_at: t > 8 ? iso(8) : null, completed_at: t > 40 ? iso(40) : null, labels: ["windows-latest"],
      steps: [step(1, "Set up job", 8, 10, "success"), step(2, "Checkout", 10, 11, "success"), step(3, "Install Rust", 11, 20, "success"), step(4, "cargo test", 20, 40, "failure"), step(5, "Upload results", 1e9, 1e9, "skipped")] },
    { id: 1303, name: "build", status: t > 40 ? "completed" : "queued", conclusion: t > 40 ? "skipped" : null, labels: ["windows-latest"], steps: [] },
    { id: 1304, name: "publish", status: t > 40 ? "completed" : "queued", conclusion: t > 40 ? "skipped" : null, labels: ["ubuntu-latest"], steps: [] },
  ];
}
const LOG = [
  "##[group]Run cargo test", "cargo test --manifest-path server/app/Cargo.toml", "##[endgroup]",
  "   Compiling wridge-server v1.6.0", "    Finished test profile in 41.2s", "     Running tests/github.rs",
  "test device_flow_signs_in_and_lists_repos ... ok", "test etag_makes_the_second_read_free ... ok", "test failed_runs_notify_once ... FAILED",
  "", "failures:", "---- failed_runs_notify_once stdout ----", "thread 'failed_runs_notify_once' panicked at tests/github.rs:233:5:", "assertion `left == right` failed: only my run that just failed",
  "  left: 2", " right: 1", "", "test result: FAILED. 6 passed; 1 failed", "##[error]Process completed with exit code 101.",
].map((l, i) => `2026-11-07T08:00:${String(i).padStart(2, "0")}.0000000Z ${l}`).join("\n");

const send = (res, code, body, headers = {}) => { res.writeHead(code, { "Content-Type": typeof body === "string" ? "text/plain" : "application/json", ...headers }); res.end(typeof body === "string" ? body : JSON.stringify(body)); };
http.createServer((req, res) => {
  let body = ""; req.on("data", (c) => (body += c)); req.on("end", () => {
    const u = new URL(req.url, "http://x"), p = u.pathname;
    console.log(req.method, p);
    if (p === "/login/device/code") { polls = 0; return send(res, 200, { device_code: "dev", user_code: "WDGE-7Q4K", verification_uri: "https://github.com/login/device", expires_in: 900, interval: 2 }); }
    if (p === "/login/oauth/access_token") { if (body.includes("refresh_token=")) return send(res, 200, { access_token: "tok", expires_in: 28800, refresh_token: "ref" }); return send(res, 200, ++polls < 4 ? { error: "authorization_pending" } : { access_token: "tok", expires_in: 28800, refresh_token: "ref", refresh_token_expires_in: 15000000 }); }
    if (p === "/user") return send(res, 200, { login: "Warayutkub" });
    if (p === "/user/installations") { if (!installed) { installed = true; return send(res, 200, { installations: [] }); } return send(res, 200, { installations: [{ id: 1 }] }); }
    if (p === "/user/installations/1/repositories") return send(res, 200, { repositories: [
      { full_name: "Warayutkub/wridge", name: "wridge", owner: { login: "Warayutkub" }, private: false, html_url: "https://github.com/Warayutkub/wridge" },
      { full_name: "Warayutkub/book-lock", name: "book-lock", owner: { login: "Warayutkub" }, private: true, html_url: "https://github.com/Warayutkub/book-lock" },
      { full_name: "Warayutkub/empty-repo", name: "empty-repo", owner: { login: "Warayutkub" }, private: true, html_url: "https://github.com/Warayutkub/empty-repo" } ] });
    let m;
    if ((m = p.match(/^\/repos\/Warayutkub\/(wridge|book-lock)\/actions\/runs$/))) return send(res, 200, { total_count: 5, workflow_runs: runs() }, { "x-ratelimit-remaining": "4800" });
    if (p.match(/^\/repos\/Warayutkub\/empty-repo\/actions\/runs$/)) return send(res, 200, { total_count: 0, workflow_runs: [] });
    if ((m = p.match(/\/actions\/runs\/(\d+)\/jobs$/))) return send(res, 200, { jobs: jobs(Number(m[1])) });
    if ((m = p.match(/\/actions\/runs\/(\d+)$/))) return send(res, 200, runs().find((r) => r.id === Number(m[1])) ?? {});
    if (p.match(/\/actions\/jobs\/\d+\/logs$/)) return send(res, 200, LOG);
    send(res, 404, { message: "Not Found" });
  });
}).listen(4710, "127.0.0.1", () => console.log("fake github on 4710"));
