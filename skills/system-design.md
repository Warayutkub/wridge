---
name: system-design
description: Before building a feature that adds or changes data, APIs, statuses, money, background jobs or external services — draft the flow and state, check the design against this list, and stop for the user's approval. Also when asked how a system works or should work. Can be done on a FigJam board (figjam_diagram / board_map).
area: both
---
# System design — flow first, then code

Design the behaviour before the screens' details and before code. Keep it small: the goal is a flow the user can read in a minute, not a document.

## 0. Know what exists (no file reading yet)
- `docs/flows/` has a flow for this area → read only that file; it is the agreed behaviour.
- `code_graph find <thing>` for tables, routes and functions you might reuse; `code_graph table <name>` for a table you will touch. Never add a table or column that already exists under a similar name.
- `project_memory find <topic>` for earlier decisions.

## 1. Draft the flow — who does what, in order
Write it to `docs/flows/<name>.md` (one flow per file; edit the existing file, never a copy — git keeps history):
```
# Flow: <name in the user's words>
entry: screen /signup · route POST /api/signup · webhook … · cron …
## Steps
1. [user] fills the form on `/signup`
2. [web] POST `/api/signup` → `createSchool`
3. [server] writes `schools`, `signup_tokens` — in one transaction
4. [⏳ email] user clicks the link → GET `/api/signup/verify`
   - expired → show "link expired", allow resend
## States
`plan_payments.status`: pending → paid | rejected · paid → refunded (admin only)
## Notes
why the IP limit exists, manual steps, open questions
```
- Actors in brackets: user, web, server, db, a named external service, cron, webhook, admin. Mark every wait (⏳) — webhooks, emails, jobs, humans.
- Code names in backticks (routes, functions, tables): they are checked against the code graph; free text is not.
- Every branch says what the user sees, and ends with `→ end` or `→ step N`.
- Steps that run at the same time share a number with letters (`5a`, `5b`); the next number waits for all of them. Use it only when the work really runs in parallel (jobs, queues), not for readability.

## 2. Check the design
- **Data:** reuse before adding; every query scoped by owner/tenant; soft delete or real delete (decide); history/audit needed for money, grades, permissions.
- **States:** each `status` column lists its allowed moves and who may make them; the code refuses any other move (one function changes the status, not scattered updates).
- **Now or later:** the request does only what the user waits for; slow or retryable work goes to a job/queue (`reliability`). Webhooks and callbacks can arrive twice or out of order → idempotent.
- **Consistency:** writes that must succeed together share a transaction; a failure halfway leaves nothing half-done (or a state that says so).
- **Boundaries:** the web talks to the server through a small, typed set of endpoints/actions; one error shape for the whole API; secrets and checks stay on the server (`backend-security`).
- **Load:** lists paginate; heavy reads/reports are checked with `backend-performance`.
- **Simple first:** no new service, framework or abstraction without a reason the user agrees with.

## 3. Stop for approval ⏸
Show the flow (and states) in chat, short; list the choices you made and the open questions. Wait for the user. Record lasting choices with `project_memory decide` (e.g. "payments are soft-deleted", "status changes only through `setPaymentStatus`").

## 3b. Designing on a FigJam board (when the user wants to design there)
The plugin can be open in a Figma file and a FigJam board at once: `get_document_info` lists both, `use_file figjam` / `use_file <name>` switches (then check the file again before writing). The board is where the user thinks and edits; the flow file stays the source the code follows.
- **Draw with `figjam_diagram`, never shape by shape.** One section per diagram, named `"<feature> · System design · Draft"`; the server lays it out (ELK) and places it clear of existing work.
  - `type: "flow"` — lanes = actors (user, web, server, external services, staff), steps in order; `decision` diamonds with labelled exits; `wait` (⏳) for anything that waits on a person or a service; one `start`, an `end` per way out.
  - `type: "state"` — one per `status` column: states as nodes, every allowed move as an edge labelled with its event.
  - `type: "er"` — the tables the feature touches (from `code_graph data` / `table` when there is code): 🔑 PK, → FK.
  - `type: "sequence"` — calls between web, server and outside services in time order, replies dashed: use it for anything with a third party (payments, slip checks, LINE).
  - `status` per node/table: existing (grey), new (blue), changed (amber), removed (red).
  - `notes`: open questions (yellow), risks (red), what is already agreed (green) — the checklist in §2 becomes these stickies.
- **Keep the board readable:** ≤ ~20 boxes per diagram (split a long flow into two), short box text (what happens, not how), lanes only for actors that act.
- **Reading back:** after the user edits, `board_map` (one section with `sectionId`). Ids from the diagram show which step is which; items without an id were added by the user. Summarise what changed and answer the stickies before redrawing anything.
- **Never redraw over the user's edits:** before replacing a section, check `board_map` for items without a diagram id or changed text; if there are any, draw the new version beside it and ask.
- **Approval:** rename the section to `· In review` when it is ready, and to `· Approved` only after the user agrees. Then write the agreed flow into `docs/flows/<name>.md` (and decisions with `project_memory decide`) — the code follows that file, not the board.

## 4. Then build — and keep the flow true
- Screens follow the flow (each wait and branch has a screen state).
- Code follows the flow; when the code ends up different, update the flow file in the same change.
- Existing flow and code disagree → ask which one is right before changing either.
