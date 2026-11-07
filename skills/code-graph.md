---
name: code-graph
description: Before reading, changing or reviewing code in a project — ask the code graph (structure, routes/screens, impact, design links, tokens) instead of opening files, and keep its notes (what each part is for) current.
area: code
---
# Code graph

Each code project gets `<project>/.wara/code-graph.db`: files, components/widgets, hooks, functions, imports, who-uses-what, routes (Next.js) and screens (Flutter), design tokens, links to Figma, the data model (Supabase migrations, schema.prisma, SQL files, CREATE/ALTER TABLE strings in TS/Dart code; tables, columns, RLS, which code reads/writes what — supabase-js, Prisma client, sqflite and raw SQL strings), and notes written by AI. It refreshes itself (only changed files are re-read) before every answer.

How to call: `code_graph {projectDir, query, target?, depth?, limit?}` (MCP profile `code`, or `call_tool`), or the `wridge` CLI, which works while the server is off: `"<cliPath>" code-graph <query> <target> --dir <project>` (installed: `%LOCALAPPDATA%\wridge\bin\wridge.exe`; `cliPath` is in `GET http://127.0.0.1:3156/api/status`).

## First time in a project
1. `overview` — size, routes/screens, most used components, packages, unresolved imports, notes.
2. If it printed `(added /.wara/ to …/.gitignore)`, tell the user: `.gitignore` changed and should be committed. Never commit `.wara/` or edit that line away.
3. `notes` — read what earlier sessions wrote; it also lists notes that went stale.

## Which query for which question
| Question | Query |
|---|---|
| What's in this project / where do I start? | `overview`, then `notes` |
| Where is X? | `find X` (symbols, files, routes, tokens) |
| What does this file declare/import, who imports it? | `file path/to/file.tsx` |
| Who uses X? | `used_by X` |
| What does X depend on? | `uses X` |
| What breaks if I change X? | `impact X` (`depth` 1–8, default 3) — **always before editing shared code** |
| What does page `/orders/[id]` / screen `TaxScreen` render? | `route /orders/42` or `route TaxScreen` |
| Which Figma component is this, which are missing? | `design` |
| Where is a color/spacing token used, which are unused, raw colors? | `tokens` / `tokens primary` |
| What tables exist, RLS, which ones code uses, drift from migrations? | `data` |
| Columns, indexes, relations, policies of a table + every query on it | `table orders` |
| What breaks if I change a table/column? | `impact orders.status` (queries → the functions running them → their callers/routes) — **before any schema change** |
| How does feature X work (steps, actors, waits, states)? | `flows` → `flows checkout` — the agreed behaviour from `docs/flows/`; ⚠ marks steps whose code is gone |
| What does this endpoint/function really do (calls, tables, packages)? | `trace POST /api/checkout` or `trace createOrder` |

Targets: a name (`Button`), a path (`components/ui/button.tsx`), `path#Name` when a name is declared twice, or a route. Answers are short text — quote them instead of opening files. Open a file only for the lines you will change, and use the line numbers the graph gives you.

## Changing code
1. `impact` on what you'll touch; list the affected routes/screens (⚑) in your plan.
2. Prefer existing components: `find` / `used_by` before creating a new one. If Figma has a component the code lacks (`design` → "In Figma, no code component"), build it once and map it.
3. Use tokens, not raw values: `tokens` shows the names in use (`--color-primary`, `AppColors.primary`). New raw colors show up in `tokens` → "Raw colors in code".
4. After the change, run the query again to confirm nothing unexpected now depends on it.

## Design ↔ code
- `code_design_sync {projectDir}` copies the open Figma file's components and tokens into the graph (needs the plugin; the user must have the project's design file open). Do it once per session when design matters.
- `design` lists linked pairs (from `wara.components.json`), suggested pairs by name, and gaps. Confirm a suggested pair only after checking both sides: `map_component {projectDir, figmaName, import, component}` (the import path is the one the project already uses, e.g. `@/components/ui/button`). Class-based UI (no component yet): `map_component {projectDir, figmaName: "Button", element: "button", className: "btn", variants: {"Type=Primary": "btn-p"}}`; `design` then flags classes missing from the CSS. New UI still becomes a real component.
- `tokens` flags values that differ from Figma (`≠`). Fix in code, or regenerate the token file with `get_tokens`.

## Notes — the graph's memory of *why*
The structure comes from the code; the meaning comes from you. Write a note with `code_graph_note {projectDir, target, text}` (CLI: `code-graph note <target> "text"`):
- **When:** you understood a folder, entry point or widely used component well enough to save the next session from re-reading it; `notes` lists the best candidates ("Worth a note").
- **What:** purpose, rules and caveats — 1–3 sentences, in the user's language. Not a summary of the code (the graph already has that).
  - Good: `dir:lib/features/finance` → "Money tracking: transactions, pockets, slips and Thai personal income tax. Data stays on the device — never add cloud sync."
  - Good: `AppCard` → "Base card for every screen; use it instead of Material Card so radius/shadow stay consistent."
  - Bad: "This file exports a StatelessWidget named AppCard with a child parameter."
- **Stale notes:** a note is marked stale when its code changed. When you see `STALE`, check the code; rewrite the note (or delete it with empty text) as part of your change.
- Never invent intent. If you're unsure why something exists, ask the user or leave it without a note.

## Limits
- Dart has no named imports: when two imported files declare the same name, the first is used.
- Generated code (`*.g.dart`, Prisma client, build output) is skipped; imports of it appear as "unresolved".
- Other languages appear at file level only (no symbols).
