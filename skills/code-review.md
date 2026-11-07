---
name: code-review
description: Final quality gate before handing over or opening a PR — a short check across correctness, security, performance, reliability, maintainability and usability, pointing to the topic skill when something fails. Also for reviewing someone else's diff. Design reviews use critique.
area: code
---
# Code review gate

Review the **diff**, not the whole project (`git diff main...`, `code_graph impact <changed>` for what it can break). Fix what you find, or list it. Load a topic skill only when its row fails.

| Area | Check | Deeper |
|---|---|---|
| Correctness | does what was asked; edge cases (empty, boundaries, time zone, money rounding); no dead/debug code | — |
| Security | every new endpoint/action: who may call it, checked on the server, scoped to owner/tenant; input validated; placeholders in SQL; no secrets in code/logs; rate limit where it costs | `backend-security` |
| Performance | only used columns; paginated lists; no N+1 / queries in loops; indexes for new filters; slow work off the request; cache has TTL + invalidation | `backend-performance` |
| Reliability | timeouts on external calls; safe retries; idempotent charge/send/create; errors handled once, logged with context, safe messages | `reliability` |
| Tests | unit for logic, integration for endpoints incl. 401/403/other-tenant; bug fix has a regression test; CI green; coverage gate on logic holds | `testing-ci` |
| Structure | right layer (view / controller / service / data); files < ~300 lines; reused existing components/services instead of new copies; names say what things are | platform skill, `code-graph` |
| Data & docs | schema change = new migration (expand/contract); `.env.example`, README/API docs updated | `docs-migrations` |
| UI/UX (if UI changed) | matches the design/tokens; loading/empty/error/success states; own dialogs/toasts; mobile width; accessible labels/focus | `interaction-code`, `design-to-code` |
| Git | branch + Conventional Commits; PR small with test plan; nothing generated/secret staged | `git-team` |

## Output
Short report: ✅ passed areas in one line; for each problem — file:line, what is wrong, the fix (applied or proposed), severity (must fix / should fix / nit). Never mark an area passed that you did not check; say "not checked" instead.
