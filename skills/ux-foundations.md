---
name: ux-foundations
description: Before designing a new screen, page or flow — define users, goals, flow and information structure, and apply core UX heuristics.
area: design
---
# UX foundations

Design the experience before the visuals. Spend a few minutes here; it decides whether the screen works.

## 1. Frame the problem (write it down in your reply, briefly)
- **Who** uses this screen, in what situation (on the go? at a desk? first time or daily?).
- **Job to be done:** the one thing they came to do. Every screen has one primary task.
- **Success:** what the user has achieved when they leave the screen.
- **Constraints:** platform, content that already exists, brand, data the system really has.
If the request is vague, make sensible assumptions and state them rather than stalling.

## 2. Map the flow
- List the steps from entry to success, including **entry points** (link, notification, search) and **exits**.
- Mark decision points and what happens on **errors, empty data, no permission, offline**.
- Remove steps: every extra screen or field costs users. Ask "can the system infer this?"
- Each screen = one step of the flow. If a screen serves two goals, split it or make one clearly primary.

## 3. Structure the information (IA)
- Group content by what users think about together, not by database tables.
- Order by importance and by the sequence users need it (what do I see → what can I do → details).
- Use familiar labels in the user's language (Thai UI → natural Thai, not translated jargon).
- Navigation depth ≤ 3 levels; the current location must always be obvious.

## 4. Heuristics to check every design against (Nielsen)
1. **Visibility of system status** — show loading, progress, saved/sent states.
2. **Match the real world** — user vocabulary, natural order, familiar metaphors.
3. **User control & freedom** — back, cancel, undo; no dead ends.
4. **Consistency & standards** — same thing looks and behaves the same everywhere; follow platform conventions.
5. **Error prevention** — constrain inputs, confirm destructive actions, sensible defaults.
6. **Recognition over recall** — show options and recent items; don't make users remember.
7. **Flexibility & efficiency** — shortcuts for frequent users without hurting beginners.
8. **Aesthetic & minimalist design** — every element must earn its place.
9. **Help recover from errors** — plain language, what happened, how to fix it.
10. **Help & documentation** — contextual hints over manuals.

## 5. Laws worth applying
- **Hick's law:** more choices → slower decisions. Limit primary choices; group and prioritize the rest.
- **Fitts's law:** important targets big and close to where the hand/cursor already is (thumb zone on mobile: bottom area).
- **Cognitive load:** chunk information (groups of 3–7), progressive disclosure for advanced options.
- **Jakob's law:** users expect your product to work like the others they use — reuse common patterns.
- **Peak-end rule:** make the key moment and the final step (success screen) feel good.

## 6. Output before drawing
A short plan: screens in the flow, the primary action of each, the sections of the current screen in order, the states needed (empty / loading / error / success), the dialogs/toasts each action triggers, and which system pages apply (404 / 500 / offline). Then load `ui-visual` + `interaction-patterns` and build.
