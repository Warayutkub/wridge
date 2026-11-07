---
name: motion-craft
description: Making motion feel designed by a senior motion designer, not generated — for UI motion in apps and for animated art (Lottie, Figma Motion). The thinking (purpose, weight, leading element), timing and spacing, choreography, the animation principles that matter on screens, and the "AI smell" checklist. Load with `motion` (which covers the tools) before animating anything; for landing / product / brand pages load motion-landing.
area: both
---
# Motion craft

Generated motion looks generated because every choice is the default: fade + rise 20px, 300ms, `ease`, everything at once or with one even stagger, everywhere. A motion designer makes **a decision per element** — what it is, what it weighs, what it is saying — and the defaults disappear on their own.

## 0. Taste first, then craft
Never impose a feel. Read the project's motion profile (Foundations / `project_memory`); if there is none, ask once and record it:
- **Register:** quiet (Linear: ~100ms, almost no overshoot) · polite (Stripe: ~150–250ms ease-out) · lively (Duolingo: overshoot, squash, personality) — or "like <reference>".
- **Where it may show off:** which 1–3 moments are *expressive* (first success, onboarding, empty → filled); everything else is *productive* (IBM Carbon's split: productive = efficient and out of the way, expressive = significant moments).
The craft below is how to reach that register well — it never overrides it.

## 1. Before keyframes: three questions per element
1. **What is it saying?** Motion is behaviour, not decoration: where did this come from, what caused it, what is related to what, what changed. If it says nothing → no motion (or the shortest fade).
2. **How often is it seen?** 100×/day (shortcuts, typing, tab switches) → none. 10×/day (menus, sheets, toasts) → short and quiet, ≤ 250ms. Rare (first run, success, milestone) → this is where character goes.
3. **What does it weigh?** Small & light (chip, icon, toast) moves fast and short; large & heavy (sheet, page, hero art) moves longer, starts slower, settles softer. Same element → same physics everywhere.

## 2. Timing and spacing
- **Durations scale with distance and size,** not one number: 120–180ms for small state changes, 200–300ms for surfaces, 300–450ms for full-screen moves. Exits ≈ 70–80% of the enter — leaving should not hold the user.
- **Ease-out to enter, ease-in only to leave the screen, standard (in-out) for moving within it.** Never ease-in on something appearing (it feels late); never linear except for progress/rotation loops and colour.
- **Shape the curve; don't accept `ease`.** A strong ease-out (`cubic(0.16, 1, 0.3, 1)` / expo-out) reads crisp and confident; `cubic(0.2, 0, 0, 1)` reads calm. Pick one or two per project and own them.
- **Distance is a choice:** 4–8px for a nudge, 12–24px for an entrance, full-edge only when the element really comes from off-screen. Large travel with short duration = whip; small travel with long duration = float. Both are deliberate looks — never accidental.
- **Springs for anything the user can touch** (drag, swipe, sheets, toggles): they start from the current value and velocity, so motion can be interrupted and reversed. A UI that can't be grabbed mid-motion feels like a video, not an object.

## 3. Choreography (several things moving)
- **One leader.** The element that caused the change, or the most important one, moves first and most; the rest follow as a group. If everything moves equally, nothing is important.
- **Stagger with intent:** 30–60ms between siblings, follow reading order, cap at ~5–6 items (the rest arrive with the last one). Total entrance ≤ ~600ms — content must be usable fast.
- **Vary inside the group:** the leader travels farther / overshoots; followers travel less and settle sooner. Uniform copies are the clearest AI tell.
- **Continuity:** an element that exists in both states moves (shared-element / FLIP), it does not fade out and fade in again. Things come from where they were triggered (a menu grows from its button — origin-aware).
- **Don't animate what didn't change.** Only the part of the screen that is new or moved gets motion.

## 4. The animation principles that matter on screens
For UI use them lightly; for illustrations / Lottie / mascots they are the job.
| Principle | UI | Art / Lottie |
|---|---|---|
| **Anticipation** | a press scales to 0.97 before the action; hover hints | wind-up before the jump (squash down, then go) |
| **Follow-through & overlap** | children settle 30–60ms after the parent | tips, tails, sparks lag 2–4 frames behind the body; parts have different drag by mass |
| **Overshoot & settle** | only in lively registers, ≤ 5% | the norm: pop past, come back, small second wobble |
| **Squash & stretch** | a toggle knob stretches while travelling | keep volume (scale X up ⇒ Y down); pivot at the contact point, not the centre |
| **Arcs** | rarely | living things move on curves: ease X and Y differently (fast-out X, slow-out Y) |
| **Secondary action** | a check draws after the button turns green | blink, breath, a spark — never louder than the main action |
| **Holds** | — | hold the key pose 4–10 frames so it reads; no hold = mush |
| **Exaggeration** | no | yes, then pull back 20% |
Lottie / Figma Motion also: pose-to-pose (key poses first, then timing, then overlap), test at the real size (a 24px icon needs bigger, simpler moves), a finished-looking poster frame.

## 5. Process — passes, like an animator
1. **Concept:** one line per moment — "the card drops onto the table", "the check is drawn by hand". Offer 2 contrasting options when the moment is expressive.
2. **Storyboard:** timestamps + who moves (`0 press → 80 card leads → 260 overshoot → 380 settle; list follows 40ms apart`). Show it before building.
3. **Blocking:** key poses and timing only (motion_animate / create_lottie).
4. **Spacing:** curves per segment — never the same curve on every key.
5. **Polish:** overlap, follow-through, secondary action, holds.
6. **Review** with the checklist below and `motion_check`; watch it at 0.25× (on /motion) — problems show up slowed down.

## 6. The AI-smell checklist (fix before showing)
- [ ] Everything enters with fade + rise of the same distance.
- [ ] One duration and one easing for every element.
- [ ] Even stagger over a long list, or everything at once.
- [ ] No leader — nothing moves first or more.
- [ ] `ease` / `ease-in-out` / linear on entrances; ease-in on something appearing.
- [ ] Motion on things that didn't change, or on every scroll into view.
- [ ] Bouncy everywhere (or nowhere, in a lively project).
- [ ] Art moves as one rigid block — no overlap, no holds, centre pivots.
- [ ] A fade-out + fade-in where the element could have moved.
- [ ] No reduced-motion version (reduced motion = crossfade or the final frame, not "nothing works").
`motion_check` flags several of these automatically ("AI smell"); the rest are judgement — say which ones you checked.

## 7. Calibrate on real work
When the user names a reference (an app, a site, a video), describe its motion in these terms (register, leader, durations, curves, overlap) before copying the feel — and copy the decisions, not the surface.
