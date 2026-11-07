---
name: interaction-patterns
description: Whenever designing screens in Figma — navigation, buttons, forms, feedback, and the frames every product needs beyond the happy path (loading/empty/error states, dialogs, toasts, sheets, 404/500/offline pages). For UI code use interaction-code.
area: design
---
# Interaction patterns

## Actions & buttons
- One **primary** action per screen (filled, accent color). Others are secondary (outline/tonal) or tertiary (text).
- Labels are verbs that state the outcome: "บันทึก", "ส่งคำขอ", "สร้างงาน" — not "ตกลง"/"OK".
- Destructive actions: separate from the primary, red/danger styling, confirmation with the object named ("ลบโปรเจกต์ X?") and an undo when possible.
- Every button needs states: default, hover (web), pressed, disabled, loading. Make them variants of one component.
- Touch targets ≥ 44×44 (mobile), ≥ 8px apart.

## Navigation
- Mobile: bottom tab bar for 3–5 top destinations (icon + label), top app bar with title and back.
- Desktop: top nav or sidebar; current item clearly highlighted.
- Always show where the user is and how to go back. Breadcrumbs for deep hierarchies on web.
- Don't hide primary destinations in a hamburger on desktop.

## Forms
- One column. Labels above fields (not placeholder-only). Mark optional fields, not required ones, when most are required.
- Group related fields with headings; ask only what's necessary; smart defaults.
- Right input type: date picker, segmented control for 2–4 options, dropdown for many, toggle for instant on/off.
- Validation: inline, after leaving the field or on submit — never while the user is still typing the first time. Error text under the field says how to fix it.
- Submit button stays visible; disable only with a visible reason, otherwise validate on submit.
- Long forms: steps with a progress indicator; keep entered data when going back.

## Feedback & system status
- Every action gets feedback within ~100 ms (pressed state), and a result: toast/snackbar for light confirmations, inline message for context-specific results, dialog only when a decision is needed.
- Loading: skeletons for content areas, spinners only for short waits in small places, progress bars for known durations.
- Saved/sent states are explicit ("บันทึกแล้ว ✓"), with the time if useful.

## States, overlays & system pages — draw them, don't leave them to code
A design with only the "happy" screen is **not finished**. Real products spend much of their time loading, empty, failing, confirming. Draw these as actual frames and components, not as notes.

### 1. Feedback components (build once, in the Foundations/Components area, before screens)
| Component | Spec |
|---|---|
| `Dialog/Confirm`, `Dialog/Destructive` | Width 400 desktop / full width − 32 mobile, padding 24, radius lg; title (what will happen) + body (consequence) + actions right-aligned: secondary "ยกเลิก", primary verb ("ลบโปรเจกต์"). Destructive = danger button. |
| `Dialog/Form` | Same shell with 1–3 fields; longer forms become a page or sheet. |
| `Sheet/Bottom` (mobile) | Full width, top radius xl, handle bar, max ~90% height. |
| `Toast/Success`, `Toast/Error`, `Toast/Info` | Icon + one line ("บันทึกแล้ว") + optional action ("เลิกทำ"); bottom-center mobile, bottom-right desktop; error toasts stay until dismissed. |
| `Banner/Warning`, `Banner/Error` | Full-width inline strip at the top of content for persistent states (offline, trial ending). |
| `Skeleton/Line`, `Skeleton/Block`, `Skeleton/Avatar` | surface-2 fill, same radius as the real content. |
| `Spinner`, `Progress/Bar` | Spinner for < 2 s waits in small places; bar for known durations. |
| `EmptyState` | Icon/illustration slot + title + one sentence + primary action. |
| `Field/Error` | Input with danger border + help text under it saying how to fix. |
| Button `…/Loading`, `…/Disabled` | Same size as default; spinner replaces or precedes the label. |

Name them with slashes so they group in the Assets panel. If the file already has some, reuse them.

### 2. State frames per screen (next to the main screen, same row)
For every main screen, add the states that can really happen, named `<Screen>/<Breakpoint> · <State>` (e.g. `Home/Mobile · Loading`). Build each with `duplicate_node` of the finished screen, then change only the content area — header/nav stay identical.
| State | Design |
|---|---|
| Loading | Skeletons in the shape of the final layout (not a lone spinner) |
| Empty (first use) | `EmptyState`: what goes here + primary action to create the first item |
| Empty (no results) | Nothing matched, how to widen the search, "ล้างตัวกรอง" |
| Error | What happened in plain words + "ลองอีกครั้ง"; keep the user's input |
| Success | Confirm the outcome and offer the next step |
| Long / partial data | Very long names, many items, missing images |
| No permission / offline | Explain and offer the way out (`Banner` or full state) |

Forms also get: field errors shown, submitting (button loading), submit failed.

### 3. Overlay frames (show the moment, not just the component)
For every destructive action, important confirmation and key feedback, add a frame `<Screen> · Dialog <name>` / `· Toast <name>`: a duplicate of the screen + a full-frame scrim (text color at 40–50%) + the dialog instance centered (desktop) or a sheet at the bottom (mobile). Toasts need no scrim. Link the trigger button to it in the prototype (`OVERLAY`, see `prototyping`).

### 4. System pages (once per product, mobile + desktop)
`System/404` (not found + way home), `System/500` (our fault + retry + contact), `System/Offline`, `System/Session expired` (sign in again, keep their place), `System/Maintenance` if the product has downtime. Same header/brand as the product, one illustration or icon, one clear action.

### 5. Definition of done
Before handing over, list in your reply which states/overlays/system pages exist and which you intentionally skipped and why (e.g. "no search, so no no-results state"). Offer to draw the skipped ones.

## Microcopy
- Short, specific, friendly; the user's language and tone (Thai UI: polite, natural Thai).
- Errors never blame the user; avoid codes alone.
- Numbers, dates, currency in local format (฿1,250, 5 ต.ค. 2569 or as the product uses).

## Motion (in prototypes)
- 150–300 ms, ease-out entering, ease-in leaving. Motion explains change (where something came from/went), never decoration. Durations and curves come from the motion tokens; animated art (splash, success, loading) is a Lottie — load `motion`.
