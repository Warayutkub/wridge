# wridge — Figma for AI

You can read and edit the user's Figma file through wridge. This is the core guide; detailed methods live in **skills** (listed at the end) — load the matching one with `get_skill` before that kind of work.

## Hard rules
1. **Never write your own helper** (JSON-RPC/curl/`mcp.mjs` scripts). Everything is reachable as below.
2. **Missing capability → tell the user.** Don't hack around it; new tools belong in this project's `server/tools.mjs` + `code.ts`.
3. **Never delete or overwrite the user's work** unless asked. Build new things next to existing ones. (Enforced: `delete_node` refuses nodes not made through wridge unless `confirm: true`, which needs the user's yes.)
4. **Server down** (connection refused / `wridge status` fails) → ask the user to open the wridge app (the cat icon in the tray) and press **Start server** — or turn on Settings → Startup → *Start the server when wridge opens*. Never start it yourself.
5. **Plugin not connected** → ask the user to run Figma → Plugins → Development → wridge.
6. **Check the file name** (`get_document_info`) before writing, and say which file you changed. (Enforced: writes are refused until a check, and again if another file gets opened.)
7. **Existing code project → ask `code_graph` before writing** — every session, also mid-task and for bug fixes:
   - **Reuse:** `find <thing>` lists matching components, files, routes, **tables and columns**. Use what exists; never add a component, table or column that already exists under a similar name (try synonyms).
   - **Effect:** `impact <symbol|file>` before changing shared code; `impact <table>.<column>` (and `table <name>`) before any schema change.
   - Then open only the lines it points to. Grep is for what the graph doesn't hold: strings and UI text, error messages, comments, CSS values, env names, unparsed languages (e.g. Go).

## How to call
| Situation | Use |
|---|---|
| Tool listed in this session | call it directly |
| Tool not listed (see "More tools") | `call_tool {name, args}`; `describe_tool {name}` shows parameters |
| Many steps (a section, a page) | `run_batch` — one call, steps reference earlier results |
| No MCP tools in this session | CLI: the `wridge` program (installed: `%LOCALAPPDATA%\wridge\bin\wridge.exe`; its exact path is `cliPath` in `GET http://127.0.0.1:3156/api/status`) — `"<cliPath>" <tool> key=value …` (`guide`, `skill <name>`, `list`, `status`, `<tool> @args.json`) |
| Loops / heavy logic | A small script (any language) that POSTs `{"name": "<tool>", "args": {…}}` to `http://127.0.0.1:3156/api/tool` in a loop, with a header `X-Wridge-Session: <any short name>` — or one `run_batch` |

**Other AI sessions may be working at the same time.** Each session has its own file: `use_file` switches only yours, and your file check is yours. A script without the `X-Wridge-Session` header (or a CLI run without the `WRIDGE_SESSION` env var) shares one session with every other such caller, so name it. Commands to one file take turns (a `run_batch` is one turn); a "stayed busy" error means another session is working in that file — wait a moment or ask the user.

`run_batch` example — `"$hero"` is the id of the step named `hero`; `"$hero.width"` a property; `"$0"` by index:
```json
{ "steps": [
  { "tool": "create_frame", "as": "hero", "args": { "name": "Hero", "parentId": "1:3", "widthSizing": "FILL", "heightSizing": "HUG", "padding": 24, "itemSpacing": 16 } },
  { "tool": "create_text", "args": { "parentId": "$hero", "text": "สวัสดี", "fontFamily": "IBM Plex Sans Thai", "fontStyle": "Bold", "fontSize": 32, "widthSizing": "FILL" } },
  { "tool": "select_nodes", "args": { "nodeIds": ["$hero"] } }
]}
```
Prefer `run_batch` over many single calls: every round trip re-sends the whole conversation.

## Workflow — stages with stops (⏸ = stop and wait for the user)
1. **Explore:** `design_map` (screens, flows, components, review status), then `get_styles` / `get_variables` / `get_components` — the file's design system comes first.
2. **Plan together ⏸:** before drawing, brainstorm in chat: who uses it and when, the goal, the flow step by step, the screens and their states, open questions; propose, ask, refine. Draw nothing until the user agrees on the plan.
3. **System design ⏸ — when the work adds or changes data, APIs, statuses, money, jobs or external services:** load `system-design`; draft the flow (+ states) in `docs/flows/<name>.md` — or on a FigJam board when the user designs there (`figjam_diagram`, `board_map`) — check it, stop for approval. Pure visual changes skip this. The approved flow decides the screens and their states.
4. **Foundations:** a `Foundations` page/section exists → read it (`get_tokens {format:"json"}`) and use only its values. If not → `design-direction` (new product: propose a direction ⏸; existing code: derive from the code).
5. **Design:** load `ux-foundations`, `ui-visual`, `interaction-patterns` (+ `responsive`). Build with `run_batch`, check each part with `export_node_image` (small `maxSize`). Screens + their states + dialogs/toasts. Put each flow's screens in a section `"<Flow> · Prototype · Draft"` (`create_section`).
6. **Prototype — always when there are 2+ screens or any overlay:** load `prototyping`, wire every flow, `check_flows` until no dead ends/orphans/broken links.
7. **Self-review:** `critique`; fix.
8. **Hand over for approval ⏸:** small images + which flows/states exist + what was skipped; rename the section to `"<Flow> · Prototype · In review"`. Changes requested → back to 5–7. The user approves → rename to `"… · Approved"`.
9. **Code only Approved screens** (see Code work). Enforced: `get_design_context` refuses unapproved screens, and renaming a Prototype section to In review / Approved refuses an unwired prototype; the `userApproved` / `allowFlowIssues` overrides need the user's explicit words. "Design and code it" in one request still stops at 8 (and at 3 when it applies), unless the user explicitly said not to ask.

**Not an app** (slides, storyboards, posters, social posts, diagrams): same stops — plan ⏸ and review ⏸ — but no prototype and no code. Section `"<Name> · <Kind> · <Status>"` with the kind it is (`Slides`, `Storyboard`, `Graphic`…); such sections stay out of the design graph and skip the prototype check. Finish with exports (`export_node_image`) if the user wants files.

## Always
- Every container is an **auto-layout frame**; size children with FILL / HUG instead of x/y.
- Spacing on a **4/8 scale** (4, 8, 12, 16, 24, 32, 48, 64…).
- **Thai text needs a Thai font** (IBM Plex Sans Thai, Noto Sans Thai, Prompt, Kanit, Sarabun) and line height ≥ 150%. Never Inter for Thai.
- Use the file's **Foundations page, variables, styles and components** before raw values or new drawings (works on the free Figma plan — no variables needed).
- **Reuse before creating:** check `component_usage` before drawing a button/card/row; when you notice repeated or near-identical structures, run `find_duplicates` and *propose* turning them into one component (ask before converting). One design component = one code component later.
- **Responsive by default:** each page gets at least `Page/Mobile` (390) and `Page/Desktop` (1440) frames with matching layer names (see `responsive`).
- **Beyond the happy path:** a screen is not done until its loading/empty/error frames, the dialogs/toasts its actions trigger, and the product's system pages (404/500/offline) are drawn (see `interaction-patterns`).
- **English, meaningful layer names** (`Header`, `Card/Product`); no `Frame 123`.

## Code work — load before writing
- From a Figma design — only screens whose section is `Approved` (`design_map` → `status`); Draft/In review → ask the user first: `design-to-code` → platform skill (`code-react-next` / `code-html-css` / `code-flutter`) → `interaction-code`.
- UI with no design: platform skill → `interaction-code`. For a new screen of a product that has a Figma file, offer to design it first.
- New or changed behaviour without a design step (a backend feature, a job, a webhook): `system-design` first — flow ⏸, then code. Flows in `docs/flows/` describe how the project works: `code_graph flows` lists them (⚠ = steps whose code is gone, plus drafts for uncovered entry points); read the one for the area (`flows <name>`) and `trace <route|symbol>` before opening files.
- Server-side code (API routes, Server Actions, Go handlers, auth, queries, webhooks): `backend-security`; plus `backend-performance` for data-heavy features, lists/reports, slow paths, heavy jobs or launch prep.
- External calls, money/important writes, jobs, logging: `reliability`. Schema changes, env vars, setup/API docs: `docs-migrations`.
- New/changed logic or endpoints, bug fixes, CI: `testing-ci`. Branches, commits, PRs: `git-team`. Before handing over code or opening a PR: `code-review`.
- Topic skills are separate — load only the ones the task touches.
- In an existing project: also load the `code-graph` skill (hard rule 7). `code_graph` adds `/.wara/` to the repo's `.gitignore` first: never commit `.wara/`; tell the user when `.gitignore` changed.

## Project memory — what was agreed survives compaction
Agreements live in the project, not only in the chat (`project_memory`; in Claude Code a hook injects the overview after startup/clear/compact).
- **Start:** if no "Project memory" block is in context, `project_memory overview` for the project you work in. Continue open tasks from their handoff; ask `find <topic>` before deciding something that may already be settled. Never open `docs/decisions.md` whole.
- **Record when the user agrees, at once:** project-level choices (tech, rules, design direction, workflow) → `decide` (writes `docs/decisions.md`, committed — say so); task-level agreements → `note` with the task. One or two sentences + why. Not every message — only what a later session must not get wrong.
- **Handoff** at each stage change and before stopping (stage, done, next, what waits on the user). Task finished → `handoff close: true`, then suggest starting a new session for the next task (shorter context, nothing lost).
- A task-level note that turns out to be a project rule → propose moving it to `decide`.
- **No memory yet but history exists** (code, README/AGENTS.md, git log, or agreements earlier in this chat): offer a short list of the existing agreements; record only the ones the user picks.
- **Brain** = the project's knowledge as Markdown notes in `brain/`, which the user reads and edits too (also in Obsidian). Use it for any kind of project: game lore, characters, book chapters, vocabulary, research, and why things are the way they are. `find` first, then `read <note>`. Never open brain files whole.
  - **Writing:** use `write` (create / append / replace) only for what is settled or hard to find again, not chat trivia. Link related notes with `[[Note]]`. Point at real things with `[[code:path#Name]]`, `[[figma:id]]` and `[[roblox:Path]]`. Fix any links the result reports as missing.
  - **Deleting:** `trash` moves a note to `brain/.trash`, where `restore` brings it back. Only the user empties the trash.
  - **Signposts:** `code_graph` (find / file / used_by / uses / impact) ends with "Brain notes linking here" when notes point at that code: read the ones that matter before changing it. Links to code that no longer exists are reported as missing.
  - **Scope:** stay in the project this session works in. Read another project's brain only when the user asks. If the session's folder holds many projects, ask which one before writing.

## Limits
Figma's API can't set prototype device or overlay position/background, can't play Motion or export video. Tell the user to do those in Figma.
