---
name: reports-tasks
description: The project's Tasks board (shared with the person) and Reports (good-looking summaries without Figma) — when work is planned, checked, tracked or summed up: break a feature into tasks, record what a check found, pick up the next task, or turn results into a report the person can copy as an image or PDF. Every block's fields, table presets and theme overrides.
area: both
---
# Tasks and reports

Two tools, both files in the project (they travel with git), both shown in the wridge window:
- `tasks` → **Tasks** board, `docs/tasks/board.json`. The person drags cards, ticks subtasks, writes notes, accepts or skips your suggestions — you see the same board.
- `report` → **Reports**, `docs/reports/<name>.json`. You send content; wridge lays it out and exports PNG / PDF / a copied image. No Figma involved.

Every call needs `projectDir` (the project this session works in). Results are short text — don't `get` what `list` already told you.

## Tasks

Columns: `todo` → `doing` → `review` → `done` (+ `skipped`, hidden from `list` unless asked). Ids look like `T-12`.

| Do | Call |
|---|---|
| See the board | `list` (filters: `status`, `label`, `from`, `query`; `status: "all"` includes skipped) |
| One task in full | `get id` (description, numbered subtasks, links, change, notes) |
| Plan work | `add tasks: [{title, description?, labels?, subtasks?, links?, due?, who?}]` — many at once |
| Record what a check found | `add` with `change: {where, old, new}` and `from: "<check> · <date>"` → it lands in **review** |
| Start / finish | `move id status: "doing"` … `move id status: "done"` |
| Progress | `tick id subtask: 2` (numbers from `get`; `done: false` unticks) |
| Tell the person something | `note id text` (they see it in Activity) |
| Change fields | `update id set: {title?, description?, labels?, subtasks?, links?, due?, who?, change?, status?}` |

**The suggestion loop** (any kind of check — code, copy, a book, a config):
1. While checking, `add` each finding with `change` (+ `where` like `src/api/slip.ts:88` or `book/ch03.md:42`) and the same `from` for the whole batch. Say *why* in `description` (1–2 sentences).
2. Stop. The person accepts or skips in the window (or tells you in chat — then `accept` / `skip` with their reason yourself).
3. Accepted → status is `doing`: make the change, then `move … done`. Skipped → leave it; before raising findings again, `list status: "skipped"` and don't repeat those (their reason is in `get`).

Rules
- Before starting work the person may have planned, `list` first; continue `doing` tasks before taking new ones.
- One task = one thing someone can finish. Long plans → subtasks, not paragraphs.
- `who`: "AI" when you will do it, the person's name when they will. Don't move or edit the person's tasks unless asked.
- `links`: `path:line`, `docs/flows/<name>.md`, `figma:12:34`, a PR or https URL — they open from the card.
- Never invent progress: tick only what you finished.

## Reports

`write` with `report: {title, subtitle?, eyebrow?, status?, meta?, theme?, footer?, rows: [...]}`. A row is one block, or a list of 1–3 blocks shown side by side (`span: 2` makes one wider). Without a `header` block the title becomes the header. `name` defaults to the title (`mode: "replace"` to overwrite). Then tell the person: it's in wridge → Reports, where **Copy image** pastes straight into LINE/Slack. `export name format: "png" | "pdf"` writes `docs/reports/exports/<name>.<ext>` if they want a file.

**Send content, not looks.** Every color, size and spacing has a default; leave `theme`, `style` and `color` out unless the person asks for colors. Short text everywhere — a report is read in 30 seconds.

### Blocks (common optional fields: `eyebrow`, `title`, `caption`, `style`)
| type | fields |
|---|---|
| `header` | `title`, `subtitle?`, `eyebrow?` ("PROJECT · REPORT · 7 NOV 2026"), `status?: {text, kind}`, `meta?: {key: value}` |
| `stats` | `items: [{label, value, unit?, delta?, kind?, note?, bar? (0–1), color?}]` — 2–4 items |
| `compare` | `rows: [{label, before, after, beforeText?, afterText?, change?, better?}]`, `unit?`, `better?: "lower"\|"higher"` (default lower), `labels?: ["before","after"]` — numbers drive the bars; % change is computed |
| `chart` | `kind?: "line"\|"bars"`, `x: [...]`, `series: [{name, values, color?}]`, `badge?: {text, kind}`, `note?`, `noteLabel?`, `highlight?` (bar index) |
| `board` | `columns: [{title, kind?, cards: [{title, body?, status?: {text, kind}}]}]` |
| `table` | `columns: [name \| {name, kind?, width?}]`, `rows: [[cells] \| "Group name"]`, `preset?`, `total?: [cells]`, `compact?`, `zebra?`, `header?: "soft"\|"ink"\|"none"` |
| `timeline` | `items: [{time, text, kind?}]` |
| `callout` | `kind: ok\|warn\|bad\|info`, `title?`, `text` — agreed / risk / decide / note |
| `checklist` | `items: [{text, done, note?}]` |
| `flow` | `steps: [{actor?, text, kind?}]` — left to right |
| `text` | `text` (Markdown: paragraphs, `-` lists, `**bold**`, `` `code` ``, links) |
| `image` | `src` (project-relative path or https), `caption?` |
| `tasks` | `filter: {from?, label?, status?}` (or a string = from), `view?: "table"\|"board"\|"list"` — the board's tasks, live; nothing retyped |
| `divider` | — |

`kind` everywhere: `ok` (done, better), `warn` (wait, decide), `bad` (failed, worse), `info`, `neutral`.

### Table presets
`basic` (text) · `status` (a chip column) · `numbers` (right-aligned + `total`) · `progress` (0–1 → bars) · `compare` (before, after, change chip) · grouped rows with a string row. Column `kind` is inferred from the cells; set it only when it guesses wrong: `text`, `num`, `chip`, `bar`, `delta`, `check`, `link`. A chip cell is `{text, kind}`; a check cell is `true/false`.

### Colors — only when asked
The closest setting wins: `item.color` → `block.style` → `report.theme` → saved theme → default.
- Tokens: `page card line soft title text muted faint accent ok warn bad info neutral before after series grid radius font mono width`. A color value may name another token (`"after": "accent"`).
- `report.theme`: `{"accent": "#7A3BC4"}` or a saved theme by name `"theme": "flowklass"` (or `{"use": "flowklass", "ok": "#00AA00"}`).
- Save a theme once with `theme name: "flowklass" theme: {...}` — then every report sends one word.
- `block.style`: any token, plus the block's parts — stats `bar value`; compare `before after`; chart `series[] grid area: "none"`; board `columns: [{dot, bg}]`; table `head headText zebra rule total row`; timeline `dot` (a color, or `{ok, info, warn, bad}`) `line time`; callout `bg border text`; checklist `on`; flow `step border arrow actor`; chips `chip: {ok: {bg, dot, text}}`.

### Good reports
- Lead with what changed or what's decided (`header` status + `stats`), details after.
- Numbers you measured, not guesses; say how (`caption: "เฉลี่ย 5 ครั้ง"`).
- Findings already on the board → a `tasks` block, not a retyped table.
- Thai text is fine everywhere (labels switch to the Thai font by themselves).
