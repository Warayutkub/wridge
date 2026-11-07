# Flow: GitHub in wridge
entry: screen /github · route POST /api/github/signin · poller every 5 s
## Steps
1. [user] opens GitHub in the wridge window and presses "Sign in with GitHub"
2. [web] POST `/api/github/signin`
3. [server] asks GitHub for a device code → `sign_in`
4. [github] answers user_code, verification_uri, expires_in, interval (device_code stays in server memory)
5. [web] shows the code, copies it, opens github.com/login/device in the browser
6. [⏳ user] types the code on github.com and approves wridge-app
   - presses Cancel in wridge → POST `/api/github/signin/cancel` → back to "Sign in" → end
   - denies on github.com → "Sign-in was cancelled on GitHub" + Try again → end
   - code expires (15 min) → "The code expired" + Get a new code → end
7. [server] polls the token endpoint every interval s (+5 s on slow_down) → `poll_device`
8. [server] saves tokens + login in Windows Credential Manager → `KeyringStore`
9. [web] GET `/api/github/status` shows "Signed in as @login"
   - no repos installed → "Pick the repos wridge may read" (opens the install page), waits → step 9
10. [user] picks a repo in the switcher
11. [web] GET `/api/github/repos`, then GET `/api/github/repo/{o}/{r}/runs`
12. [server] reads runs with the cached ETag (304 = free) → `runs`, remembers the repo to watch
13. [user] opens a run
14. [web] GET `/api/github/repo/{o}/{r}/runs/{id}` → jobs as flow nodes + steps; refreshes every 5 s while running
15. [web] a failed job → GET `/api/github/repo/{o}/{r}/jobs/{job}/log` → last lines around the first error
16a. [server] poller checks watched repos: 5 s while a run is going, 60 s otherwise → `poll_once`
16b. [server] one of my runs ended failed → prints `[wridge:notify] {json}` (once per run, never on the first look)
17. [tauri] shows a Windows toast and a red dot on the tray icon
18. [user] clicks the tray → window opens on that run, the dot clears → end
## States
`github.state`: signed_out → signing_in → ready · signing_in → signed_out · ready → offline → ready · ready → expired → signing_in · ready → signed_out · expired → signed_out
(signing_in → signed_out: cancel, denied or the code expired · offline: GitHub unreachable · expired: refresh failed · → signed_out: sign out)
## Notes
- Read-only GitHub App **wridge-app** (Client ID `Iv23liculOqtNY9MWvmv`): Actions, Checks, Contents, Issues, Metadata, Pull requests. Device flow = nothing secret ships in the app; each person signs in with their own account.
- Tokens: access 8 h, refresh 6 months; refreshed when < 5 min left (no secret needed for device-flow tokens). Refresh fails or 401 → `expired` banner, old data stays.
- The window never sees a token: only the server talks to GitHub. Sign out deletes the Credential Manager entry.
- Live step logs: GitHub serves a job log only after the job ends, so a running step shows name, status and timer.
- Rate limit: ETags; remaining < 200 → poll 4× slower.
- Notification rules (Settings → GitHub): run failed (on, my runs), run succeeded (off). Phase 2 adds review requested and PR merged.
- Rust engine only; the legacy Node engine answers "not available on this engine".

## Phases
1. Sign-in + Actions: device flow, keyring, repo switcher, Actions list, run detail, failed log tail, toast + tray dot, Settings → GitHub. Then a build to try.
2. Other tabs: Overview, Pull requests (+ detail), Branches, Commits, Issues, Releases; offline / expired screens.
3. AI tool `github` (read-only: status, runs, run, pulls, failing), All-repos view, pins, motion.

## Data per screen
| Screen | GitHub calls | Refresh |
|---|---|---|
| repo switcher | `/user/installations` → `/user/installations/{id}/repositories` | on open |
| Actions list | `/repos/{o}/{r}/actions/runs?per_page=30` | 5 s while a run is going, else 60 s |
| Run detail | `/actions/runs/{id}` + `/jobs` | 5 s while running |
| Failed log | `/actions/jobs/{id}/logs` (after the job ends) | once |
| Pull requests (phase 2) | `/pulls`, `/reviews`, `/files`, `/check-runs` | 60 s |
| Branches, Commits, Issues, Releases (phase 2) | `/branches`, `/commits`, `/issues`, `/releases` | 1–10 min |

## Tests
A fake GitHub (axum on a random port): device flow, refresh, ETag/304, notifications; the window checked in the browser against it.
