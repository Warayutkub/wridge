// GraphQL answers for the fake GitHub (phase 2 tabs): by operation name. Warayutkub/wridge has a bit of everything;
// the other repos are empty, to see the empty states.
const T0 = Date.now();
const ago = (min) => new Date(T0 - min * 60000).toISOString();
const me = "Warayutkub";
const roll = (state) => ({ nodes: [{ commit: { statusCheckRollup: state ? { state } : null } }] });

const PRS = [
  { number: 14, title: "Pull requests tab", isDraft: false, state: "OPEN", createdAt: ago(120), headRefName: "feat/github-pr", baseRefName: "main", author: { login: "mint" }, additions: 420, deletions: 38, comments: { totalCount: 4 }, reviewDecision: "REVIEW_REQUIRED", reviewRequests: { nodes: [{ requestedReviewer: { login: me } }] }, commits: roll("SUCCESS") },
  { number: 13, title: "Bend shapes on the website", isDraft: false, state: "OPEN", createdAt: ago(1500), headRefName: "feat/site-shapes", baseRefName: "main", author: { login: "ploy" }, additions: 348, deletions: 61, comments: { totalCount: 9 }, reviewDecision: "CHANGES_REQUESTED", reviewRequests: { nodes: [] }, commits: roll("FAILURE") },
  { number: 11, title: "Offline fonts", isDraft: true, state: "OPEN", createdAt: ago(4300), headRefName: "feat/fonts", baseRefName: "main", author: { login: me }, additions: 88, deletions: 12, comments: { totalCount: 1 }, reviewDecision: null, reviewRequests: { nodes: [] }, commits: roll("PENDING") },
];
const MERGED = [
  { number: 12, title: "Figma file picker", isDraft: false, state: "MERGED", createdAt: ago(3000), mergedAt: ago(60), mergedBy: { login: me }, headRefName: "feat/picker", baseRefName: "main", author: { login: "mint" }, additions: 210, deletions: 18, comments: { totalCount: 6 }, reviewDecision: "APPROVED", reviewRequests: { nodes: [] }, commits: roll("SUCCESS") },
  { number: 10, title: "Fix save on leave race", isDraft: false, state: "MERGED", createdAt: ago(4000), mergedAt: ago(2900), mergedBy: { login: me }, headRefName: "fix/save-race", baseRefName: "main", author: { login: me }, additions: 14, deletions: 9, comments: { totalCount: 2 }, reviewDecision: "APPROVED", reviewRequests: { nodes: [] }, commits: roll("SUCCESS") },
];
const CLOSED = [{ number: 9, title: "Try Electron instead of Tauri", isDraft: false, state: "CLOSED", createdAt: ago(20000), closedAt: ago(18000), headRefName: "exp/electron", baseRefName: "main", author: { login: me }, additions: 1200, deletions: 300, comments: { totalCount: 3 }, reviewDecision: null, reviewRequests: { nodes: [] }, commits: roll(null) }];
const url = (x) => ({ ...x, url: `https://github.com/Warayutkub/wridge/pull/${x.number}` });

const commit = (msg, min, login, state, sha) => ({ oid: sha.padEnd(40, "0"), abbreviatedOid: sha.slice(0, 7), messageHeadline: msg, committedDate: ago(min), url: `https://github.com/Warayutkub/wridge/commit/${sha}`, author: { name: login, user: { login } }, statusCheckRollup: state ? { state } : null });
const MAIN = [
  commit("Add the GitHub page", 4, me, "PENDING", "a1f9c2e"), commit("Merge pull request #12 from feat/picker", 60, me, "SUCCESS", "7c21d0b"),
  commit("Website: real screenshots", 130, "mint", "FAILURE", "3e8812a"), commit("Per-file queues", 1500, me, "SUCCESS", "0aa31c9"),
  commit("Fix save on leave race", 2900, me, "SUCCESS", "c0ffee1"), commit("Tune the tray icon", 3100, "ploy", "SUCCESS", "8d2f310"),
  commit("v1.5.0", 8700, me, "SUCCESS", "5d1e07a"),
];
const BRANCH = (name, min, msg, login, state, behind, ahead, pr) => ({ name, target: { messageHeadline: msg, committedDate: ago(min), author: { name: login, user: { login } }, statusCheckRollup: state ? { state } : null }, compare: { aheadBy: behind, behindBy: ahead }, associatedPullRequests: { nodes: pr ? [{ number: pr }] : [] } });
const BRANCHES = [
  BRANCH("main", 4, "Add the GitHub page", me, "PENDING", 0, 0),
  BRANCH("feat/github-pr", 110, "Rows for pull requests", "mint", "SUCCESS", 0, 5, 14),
  BRANCH("feat/site-shapes", 300, "Sunset scene v3", "ploy", "FAILURE", 2, 9, 13),
  BRANCH("feat/fonts", 4300, "Ship IBM Plex", me, "PENDING", 6, 3, 11),
  BRANCH("fix/tray-dot", 2000, "Red dot only for my runs", me, "SUCCESS", 1, 1),
  BRANCH("exp/electron", 60 * 24 * 60, "Try Electron", me, null, 48, 2),
  BRANCH("old/ui-v1", 60 * 24 * 90, "Old menu layout", "mint", null, 120, 0),
];

const pr = (n) => {
  const base = [...PRS, ...MERGED, ...CLOSED].find((x) => x.number === n);
  if (!base) return null;
  const run = (name, wf, conclusion, s) => ({ __typename: "CheckRun", name, status: conclusion ? "COMPLETED" : "IN_PROGRESS", conclusion, startedAt: ago(s / 60 + 3), completedAt: conclusion ? ago(3) : null, detailsUrl: "https://github.com/Warayutkub/wridge/actions", checkSuite: { workflowRun: { workflow: { name: wf } } } });
  const failing = base.commits.nodes[0].commit.statusCheckRollup?.state === "FAILURE";
  return {
    ...url(base), merged: base.state === "MERGED", mergeable: base.state === "OPEN" ? (n === 13 ? "CONFLICTING" : "MERGEABLE") : "UNKNOWN",
    bodyText: n === 14 ? "Shows open pull requests with their checks and reviews, and a detail page per PR.\nReads everything with one GraphQL query; no new permissions." : "Short description of the change.",
    changedFiles: 12,
    commits: { totalCount: 5, nodes: [commit("Read PRs with GraphQL", 180, base.author.login, "SUCCESS", "6a0c2d1"), commit("Rows for pull requests", 110, base.author.login, failing ? "FAILURE" : "SUCCESS", "91bd4f0")].map((c) => ({ commit: c })) },
    checks: { nodes: [{ commit: { statusCheckRollup: { state: failing ? "FAILURE" : "SUCCESS", contexts: { nodes: [run("lint", "CI", "SUCCESS", 31), run("test", "CI", failing ? "FAILURE" : "SUCCESS", 100), { __typename: "StatusContext", context: "Deploy preview", state: "SUCCESS", targetUrl: "https://example.com" }] } } } }] },
    reviewRequests: base.reviewRequests,
    latestReviews: { nodes: n === 14 ? [{ author: { login: "ploy" }, state: "APPROVED" }] : n === 13 ? [{ author: { login: me }, state: "CHANGES_REQUESTED" }] : [] },
    files: { totalCount: 12, nodes: [{ path: "web/github.html", additions: 264, deletions: 20 }, { path: "server/app/src/github.rs", additions: 131, deletions: 4 }, { path: "tools/github/fake-graphql.mjs", additions: 25, deletions: 14 }] },
  };
};

const ISSUES_OPEN = [
  { number: 57, title: "Tray dot stays red after a re-run passes", createdAt: ago(4300), author: { login: "mint" }, comments: { totalCount: 6 }, labels: { nodes: [{ name: "bug", color: "d73a4a" }, { name: "tray", color: "0e8a16" }] }, assignees: { nodes: [{ login: me }] } },
  { number: 61, title: "Export cuts the last row of a long table", createdAt: ago(1440), author: { login: "ploy" }, comments: { totalCount: 2 }, labels: { nodes: [{ name: "bug", color: "d73a4a" }, { name: "reports", color: "1d76db" }] }, assignees: { nodes: [{ login: me }] } },
  { number: 55, title: "Dark mode for the Reports page", createdAt: ago(9000), author: { login: me }, comments: { totalCount: 0 }, labels: { nodes: [{ name: "ui", color: "fbca04" }] }, assignees: { nodes: [{ login: "ploy" }] } },
  { number: 52, title: "Brushes: let the AI pick by mood", createdAt: ago(20000), author: { login: "mint" }, comments: { totalCount: 11 }, labels: { nodes: [{ name: "idea", color: "a2eeef" }, { name: "figma", color: "5319e7" }] }, assignees: { nodes: [] } },
  { number: 49, title: "Code graph misses Dart part files", createdAt: ago(30000), author: { login: me }, comments: { totalCount: 3 }, labels: { nodes: [{ name: "code-graph", color: "c5def5" }] }, assignees: { nodes: [{ login: me }] } },
].map((i) => ({ ...i, url: `https://github.com/Warayutkub/wridge/issues/${i.number}` }));
const ISSUES_CLOSED = [{ number: 50, title: "Copy image fails inside the window", createdAt: ago(40000), closedAt: ago(30000), author: { login: me }, comments: { totalCount: 4 }, labels: { nodes: [{ name: "bug", color: "d73a4a" }] }, assignees: { nodes: [{ login: me }] }, url: "https://github.com/Warayutkub/wridge/issues/50" }];

const REL = (tag, min, extra = {}) => ({ name: `wridge ${tag.slice(1)}`, tagName: tag, isPrerelease: false, isLatest: false, isDraft: false, createdAt: ago(min), url: `https://github.com/Warayutkub/wridge/releases/tag/${tag}`, author: { login: me }, description: "", tagCommit: { abbreviatedOid: "5d1e07a" }, releaseAssets: { nodes: [{ name: `wridge_${tag.slice(1)}_x64-setup.exe`, size: 23601733, downloadCount: 38, downloadUrl: "https://github.com" }] }, ...extra });
const RELEASES = [
  REL("v1.9.0-beta.1", 30, { isPrerelease: true, description: "Trying GitHub phase 2 with a few people first.", releaseAssets: { nodes: [] } }),
  REL("v1.8.2", 300, { isLatest: true, description: "**Steadier shape edits, calmer pages**\n- The Reports list no longer pops on every check.\n- Vector edits try again when Figma's connection check times out.\n- A report no longer stays faded after it loads." }),
  REL("v1.8.0", 1500, { description: "**Bend shapes and every stroke setting**\n- bend_shape, vector_edit, set_stroke with brushes.\n- Delete and restore reports." }),
  REL("v1.5.0", 8700, { description: "First public release." }),
];

const empty = { totalCount: 0, nodes: [] };
export function graphql(body) {
  const { query = "", variables: v = {} } = JSON.parse(body || "{}");
  const op = /query (\w+)/.exec(query)?.[1];
  const full = v.owner === "Warayutkub" && v.name === "wridge";
  const repo = (x) => ({ data: { repository: x } });
  switch (op) {
    case "Overview":
      return { data: {
        repository: full ? { defaultBranchRef: { name: "main" }, pullRequests: { totalCount: PRS.length }, refs: { totalCount: BRANCHES.length, nodes: BRANCHES.map((b) => ({ target: b.target })) }, issues: { totalCount: ISSUES_OPEN.length }, newIssues: { totalCount: 2 }, latestRelease: RELEASES[1] }
          : { defaultBranchRef: { name: "main" }, pullRequests: { totalCount: 0 }, refs: { totalCount: 1, nodes: [] }, issues: { totalCount: 0 }, newIssues: { totalCount: 0 }, latestRelease: null },
        review: full ? { issueCount: 1, nodes: [{ number: 14, title: "Pull requests tab", createdAt: ago(120), author: { login: "mint" } }] } : { issueCount: 0, nodes: [] },
        assigned: full ? { issueCount: 3, nodes: ISSUES_OPEN.filter((i) => i.assignees.nodes.some((a) => a.login === me)).map((i) => ({ number: i.number, title: i.title, updatedAt: i.createdAt })) } : { issueCount: 0, nodes: [] },
      } };
    case "Pulls": {
      if (!full) return repo({ open: { totalCount: 0 }, merged: { totalCount: 0 }, closed: { totalCount: 0 }, list: empty, recent: empty });
      const st = v.states?.[0];
      return repo({ open: { totalCount: PRS.length }, merged: { totalCount: 24 }, closed: { totalCount: 5 }, list: { nodes: (st === "MERGED" ? MERGED : st === "CLOSED" ? CLOSED : PRS).map(url) }, recent: { nodes: MERGED.map(url) } });
    }
    case "Pull": return repo({ pullRequest: full ? pr(v.n) : null });
    case "Branches": return repo(full ? { defaultBranchRef: { name: "main", target: BRANCHES[0].target }, refs: { totalCount: BRANCHES.length, nodes: BRANCHES } } : { defaultBranchRef: { name: "main", target: null }, refs: { totalCount: 1, nodes: [BRANCH("main", 9000, "Initial commit", me, null, 0, 0)] } });
    case "Commits": return repo({ refs: { nodes: (full ? BRANCHES : [{ name: "main" }]).map((b) => ({ name: b.name })) }, ref: { name: v.ref?.replace("refs/heads/", ""), target: { history: full ? { totalCount: 128, nodes: v.ref === "refs/heads/main" ? MAIN : MAIN.slice(1, 4) } : { totalCount: 1, nodes: [commit("Initial commit", 9000, me, null, "f00dbad")] } } } });
    case "Issues": return repo(full ? { open: { totalCount: ISSUES_OPEN.length }, closed: { totalCount: 40 }, list: { nodes: v.states?.[0] === "CLOSED" ? ISSUES_CLOSED : ISSUES_OPEN } } : { open: { totalCount: 0 }, closed: { totalCount: 0 }, list: empty });
    case "Releases": return repo({ releases: full ? { totalCount: RELEASES.length, nodes: RELEASES } : empty });
    default: return { errors: [{ message: `fake GitHub: no answer for ${op}` }] };
  }
}
