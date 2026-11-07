# Project decisions

Agreed choices and rules for this project, newest last. Each entry: `## D-n · date · title`, then Tags / Decision / Why (and Supersedes when it replaces an older one). Edit freely; the AI reads it through wridge `project_memory`.

## D-1 · 2026-10-13 · Motion taste for wara-bridge's own UI
Tags: motion, design
Decision: wara-bridge's UI motion is lively (back/spring eases, a little overshoot) at a medium pace (~600–1200 ms for Lottie art). Scope for now: Lottie art only (loading, success, plugin connected, empty states, logo intro) — no new code-driven UI transitions yet. Every animation shows its last frame under reduced motion.
Why: User's answer to the motion skill's taste questions (2026-10-14).

## D-2 · 2026-10-13 · Motion taste for wara-bridge's own UI (v2)
Tags: motion, design
Decision: Lottie art for wara-bridge is detailed and dimensional, in the 3D graph's visual language (glossy spheres with light and shade, glowing blue links, deep-blue space glow), with Duolingo-like energy: vivid, bouncy, squash & stretch, particles, overlapping beats; medium pace. Lottie only for now; reduced motion shows the last frame.
Why: The first flat set looked plain; user picked: more detail/dimension, match the 3D graph, more motion, reference Duolingo (2026-10-14).
Supersedes: D-1

## D-3 · 2026-11-07 · Each AI session has its own Figma file; one queue per file
Decision: The Rust server keeps a file per session (MCP Mcp-Session-Id from initialize, X-Wridge-Session header for /api/tool, WRIDGE_SESSION for the CLI; none = the shared session = old behaviour) and a file check per session. Commands to one Figma window run one at a time in arrival order (run_batch = one turn, 120 s wait then a "stayed busy" error); different files run side by side. The file guard re-reads the open file after any window closes or switch (epoch), in Rust and legacy Node.
Why: Two AI sessions at once: one use_file moved the other's script into another file mid-run (2026-11-07), and commands to one file could interleave in the plugin. No new port or plugin change needed.

## D-4 · 2026-11-07 · wridge goes public: fresh history, use-only license, stickers as brand
Decision: Public GitHub repo "wridge" starts from a single cleaned commit (personal paths and build output removed); the full history stays only in the local repo. License: custom "free to use, no modification, no redistribution or sale" (not MIT; GitHub shows "Other"). The character stickers are published as wridge's brand/watermark. A wridge website (designed in Figma first) will be the GitHub App homepage.
Why: User wants a backup, a homepage for the GitHub App, and others to use wridge — but not modify it; old history contains their local paths.

## D-5 · 2026-11-07 · Reports without Figma (design in review)
Decision: AI sends report blocks (header, stats, compare, chart, board, table, timeline, callout, checklist, flow, text, image) via a `report` tool; wridge lays them out in its own style with no Figma, stores docs/reports/*.json, shows them on a Reports page under DESIGN, exports Copy image / PNG / PDF. Thai labels use IBM Plex Sans Thai (Plex Mono has no Thai). Figma file wara-bridge: sections "wridge Reports · Report blocks · In review" and "wridge Reports · Prototype · In review". Proposed 2026-11-07, waiting for approval.

## D-6 · 2026-11-07 · Tasks: one shared board per project (design in review)
Decision: General-purpose, not tied to book checking. One board per project in docs/tasks/*.json that the user and the AI both edit (add, move, tick). Columns To do / Doing / Review / Done; a task has title, description (markdown), subtasks, labels, links (file:line, flow, Figma node, PR, URL), who, due, activity/notes. An optional "suggested change" (old → new + where) makes it an AI suggestion that waits in Review for Accept / Edit first / Skip (skip asks a reason so later checks don't raise it again). AI checks are batches of cards ("AI check · date") you can filter. Menu item Tasks sits under Overview. Reports can show the board as a snapshot. Figma: "wridge Tasks · Prototype · In review" in wara-bridge. Proposed 2026-11-07.

## D-7 · 2026-11-07 · Report tables, tasks block and colors (design in review)
Decision: Table block has presets basic / status / numbers / progress / compare / grouped, with column kinds text, num, chip, bar, delta, check, link and options total, groups, compact, zebra, header soft|ink|none. A `tasks` block shows Tasks board items live by filter (no retyping). Colors: every token has a default (page, card, line, soft, title, text, muted, faint, accent, ok, warn, bad, before, after, series, grid, radius, font); override at report.theme → block.style (any token + the block's own parts) → item.color, closest wins; themes can be saved in docs/reports/themes/<name>.json and used by name. The AI sends nothing about looks unless asked. Figma: frames Report/Tables and Report/Theme in "wridge Reports · Report blocks · In review".
