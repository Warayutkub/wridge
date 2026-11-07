---
name: critique
description: Before handing over any design, or when asked to review one — look at the render, score it against a rubric, fix, re-check.
area: design
---
# Design critique loop

Never hand over a design you haven't looked at. Run this loop at least once; stop when every area scores ≥ 4.

## 1. Look
`export_node_image` each new frame (maxSize ~800–1200). Look at it as a user would: what do you see first, second, third? Is that the intended order?

## 2. Score 1–5 per area (write the scores briefly)
| Area | 5 means |
|---|---|
| Purpose | The primary task and primary action are obvious within 3 seconds |
| Hierarchy | One clear focal point; headings/body/meta clearly distinct; nothing competes with the primary action |
| Layout & spacing | Aligned to one grid, consistent 4/8 spacing, related things grouped, generous whitespace |
| Typography | ≤ 2 families, consistent scale, readable sizes, Thai renders fully (no clipped marks), good line length |
| Color & contrast | Restrained palette, accent only where it matters, all text passes AA |
| Consistency | Reused components/styles, same things look the same, naming clean |
| States & edge cases | Loading/empty/error frames exist for each main screen; dialogs/toasts drawn for destructive and key actions; system pages (404/500/offline) exist; long text and missing images handled. Only the happy screen = score 1 |
| Responsive | Mobile and desktop both exist and work (see `responsive`) |
| Craft | No overlaps, no overflow, no stray layers, everything auto layout, names meaningful |

## Structural checks (tools)
- `find_duplicates` on the page: separate copies or drifted near-copies → consistency problem.
- `check_flows` when the work includes prototypes: dead ends, orphans, unlinked buttons.

## 3. Fix
Fix the lowest scores first, using `run_batch`. Typical fixes: remove an element, increase whitespace, reduce colors, strengthen one heading, align edges, make the primary button the only filled one.

## 4. Re-check
Export again and re-score the changed areas. Max 3 rounds; then report what remains and why.

## Reviewing someone else's design
Same rubric, but don't change their file unless asked. Report: what works (briefly), issues ranked by impact with element names/ids, and concrete fixes.
