---
name: motion-landing
description: Creative motion for landing pages, product pages, launches and brand sites — finding the one motion idea, the signature moment, scroll storyboards, and the technique palette (scroll-scrubbed scenes, pinned stories, kinetic type, masked reveals, image-to-page transitions, cursor play, 2D physics, WebGL) with GSAP + ScrollTrigger + Lenis or CSS scroll-driven animations. For motion inside apps load motion-craft.
area: both
---
# Motion for landing & product pages

Award-level pages don't win with many effects; they win with **one idea expressed in motion**, a moment people remember, and quiet everywhere else. The Lando Norris site (Awwwards Site of the Year 2025) is "you are inside a racing driver's world" — a 3D helmet that turns as you read. Ponpon Mania (Site of the Month, Oct 2025) is 2D illustration with physics, no 3D at all. Lusion's own site is an experience instead of a project list. In each, the technique follows the idea, never the other way round.

## 0. Brief (ask, don't assume)
- Product, audience, and **the one feeling** the page must leave (fast? trustworthy? playful? premium? alive?).
- References the user likes — and what exactly they like in them.
- Constraints: stack (default: Next.js/React + GSAP + ScrollTrigger + Lenis; CSS scroll-driven animations for simple scroll effects), devices (mobile share), performance budget, brand rules, and whether 3D/WebGL is welcome.
- Record the answers in `project_memory decide` — they are the taste for every later page.

## 1. Find the motion idea ⏸
Turn what the product *does* into a physical metaphor, then let motion act it out. Propose **3 concepts that are genuinely different**, each with:
- **Name + one sentence** ("The floor plan fills up": booths light up and get claimed as you scroll).
- **Metaphor** (what the page behaves like), **signature moment** (the 1–2 seconds people will remember), **techniques** it needs, **cost** (build effort, performance, mobile risk).
Wait for the user to choose or mix. Generic concepts ("smooth fade-ins", "modern parallax") are not concepts.

## 2. Scroll storyboard ⏸
A table, one row per section: scroll range → what the user sees → what moves and why → feeling. Mark:
- **the signature moment** (gets the budget: pin, scrub, 3D, physics),
- **breathing sections** (static or one small reveal — contrast is what makes the moment land),
- **the CTA** (motion points at it; nothing animates *away* from it).
For key moments, also draw frames in Figma (start / middle / end of the scrub) so the user can approve the look before code.

## 3. Technique palette (pick from the idea, not the list)
| Technique | Says | Notes |
|---|---|---|
| **Pinned scroll story** (section sticks, scroll drives a timeline) | "walk through it step by step" | ScrollTrigger `pin` + `scrub`; keep each pin ≤ 2–3 screens of scroll; give a progress cue |
| **Scroll-scrubbed object** (product rotates / assembles with scroll) | "look at it from every side", precision | sticky + image sequence drawn to canvas (not video `currentTime`); adjacent frames differ 1–3°; preload nearest frames first; static fallback |
| **Masked line reveal** (text slides up from behind a mask) | confidence, editorial | split into lines after fonts load; overflow-hidden wrapper; 60–90ms between lines; headlines only |
| **Kinetic / variable type** (weight, width, tracking move) | voice, energy | variable fonts; hero and key statements only, never body or nav |
| **Clip-path / shape wipes** | transition, chapter change | `clip-path` circle / inset from the trigger point |
| **Image → page (shared element)** | continuity, "go deeper" | FLIP or View Transitions; the thumbnail becomes the hero |
| **Cursor play** (magnetic buttons, spotlight, trails) | tactile, playful | desktop only, subtle; never move what the user is trying to read |
| **2D physics** (things fall, stack, can be thrown) | play, abundance | matter.js / custom; cap bodies; pause when off-screen |
| **WebGL scenes / shader hovers / distortion** | immersion, premium | three.js / OGL; only when the idea needs it; a designed static poster for low-power and reduced motion |
| **Number / data in motion** (counters, charts drawing) | proof, scale | animate once; the final number readable without motion |
| **Marquee / infinite strips** | momentum, community (logos, quotes) | slow, pausable on hover, no more than one per page |
| **Lottie / Rive accents** | character | from `motion-craft`; Rive when it must react (state machine) |

## 4. Making it feel crafted (not template)
- **Contrast:** loud signature moment, quiet everything else. If every section reveals, none does.
- **Scroll is a timeline, not a trigger:** prefer scrubbed motion tied to scroll position over "play when 20% visible" for story sections; use one-shot reveals only for small content.
- **Easing has a voice:** one hero curve for the page (e.g. expo-out for confident, a soft in-out for calm) plus a quiet one for small reveals. Never default `ease`.
- **Choreograph sections like shots:** establish (wide) → detail (close) → payoff; vary rhythm — a fast section after a slow pinned one.
- **Text is read, not watched:** reveals finish before the user reaches the line; never animate words they are halfway through reading.
- **Respect the scroll:** smooth scroll (Lenis) only lightly (lerp ~0.1), never scroll-jacking that fights the wheel; anchor links and keyboard still work.
- **AI tells to avoid:** fade-up on every block, parallax on every image, `whileInView` everywhere, gradient blobs drifting for no reason, three different effects for the same job.

## 5. Build and performance rules
- Build the static page first (it must work and look finished with no motion), then layer motion.
- Animate `transform` / `opacity` / `clip-path` / `filter` (sparingly) only — no layout properties in scroll handlers.
- GSAP + Lenis: drive Lenis from `gsap.ticker` (seconds → ms), call `ScrollTrigger.update` on Lenis scroll, `lagSmoothing(0)`, no `scrollerProxy`.
- Pause everything off-screen (IntersectionObserver), cap devicePixelRatio for canvas/WebGL at 2, lazy-load heavy scenes after first paint.
- Mobile: shorter pins, fewer frames, no cursor effects, physics/WebGL simplified or replaced by the poster.
- `prefers-reduced-motion`: skip Lenis, replace scrubs with the key frames shown statically, keep crossfades short.
- Check Core Web Vitals (INP, LCP) — a beautiful page that stutters reads as cheap.

## 6. Hand-off
Per section: the storyboard row, the technique, the timeline (durations / scroll distances / curves as tokens), assets (Lottie via `motion_to_lottie` or `create_lottie`, image sequences, posters), and the reduced-motion and mobile versions. Load `motion-craft` for the timing and principles inside each moment.
