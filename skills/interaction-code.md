---
name: interaction-code
description: Whenever writing UI in code (with or without a Figma design) — buttons, forms, navigation, feedback, and every state beyond the happy path (loading/empty/error/offline, dialogs, toasts). The checklist to pass before handing over UI code.
area: code
---
# Interaction in code

Same rules as the design side (`interaction-patterns`), written for code. Apply them even when nobody drew the states.

## Actions & buttons
- One primary action per screen; others secondary/tertiary. Labels are verbs that state the outcome ("บันทึก", "สร้างงาน"), not "ตกลง"/"OK".
- One button component with states: hover (web), pressed, disabled (with a visible reason), loading (spinner, same size, no double submit).
- Touch targets ≥ 44×44 on mobile, ≥ 8px apart.
- Destructive: separate from the primary, danger style, **your own** confirm dialog that names the object ("ลบโปรเจกต์ X?"); Esc / backdrop / Cancel do nothing and focus starts on Cancel; offer undo when possible.

## Navigation & forms
- Current location always visible; a way back everywhere. Mobile: bottom tabs for 3–5 top destinations.
- Forms: one column, labels above fields, the right input type, validate on blur/submit (not while typing the first time), error text under the field saying how to fix it, keep what the user typed after an error.
- Motion: durations and curves only from the motion tokens; animated art (splash, success, loading) is a Lottie — load `motion`. Respect reduced motion.

## Feedback — every async action
Pressed state at once → loading on the trigger → result: toast for light confirmations, inline message for context-specific results, dialog only when a decision is needed. Errors say what happened in plain words and offer a retry. Saved/sent is explicit ("บันทึกแล้ว ✓").

## States — every data view
| State | Build |
|---|---|
| Loading | Skeleton shaped like the result (spinner only for short waits in small places) |
| Empty (first use) | What goes here + the primary action to create the first item |
| Empty (no results) | Nothing matched + how to widen the search + "ล้างตัวกรอง" |
| Error | Plain words + "ลองอีกครั้ง"; keep user input |
| Offline / server down | Explain, retry automatically or on a button; don't lose work |
| Long / missing data | Long names truncate or wrap cleanly; missing images have a placeholder |

## Never
- `alert()`, `confirm()`, `prompt()` or bare platform default dialogs — use the product's Dialog/Toast/Banner/Skeleton/EmptyState (`code_graph find Dialog`, `find Toast`). If it has none, build them once from its tokens, then reuse.
- Browser/OS permission prompts for things the app can do itself: custom-protocol links (`vscode://`, `slack://`), popup windows, permission requests on page load. Do it server-side (e.g. the server runs `code -g file:line`) or ask first in your own UI, then trigger it.

## Microcopy
Short, specific, friendly, in the user's language (Thai UI: polite, natural Thai). Errors never blame the user; no bare error codes. Local number/date/currency formats (฿1,250, 5 ต.ค. 2569).

## Before handing over
Run the app and trigger each state yourself: slow/failed requests, empty data, stop the server, the destructive confirm, mobile width. List which states exist and anything you skipped and why.
