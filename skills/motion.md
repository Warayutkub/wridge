---
name: motion
description: Any animation in the product — UI motion (page transitions, sheets, dialogs, lists, buttons, toasts, skeletons) and animated art (splash/logo, success or error, loading, empty states, onboarding, animated icons). Which one goes in code with motion tokens and which one becomes a Lottie file, how to make the Lottie with create_lottie, designing animation on the Figma Motion timeline (motion_animate → motion_check → motion_tokens / motion_to_code / motion_to_lottie), and the rules both follow.
area: both
---
# Motion

This skill is the *how* (tools, code vs Lottie, tokens). For the *quality* load **motion-craft** (timing, choreography, animation principles, the AI-smell checklist); for landing / product / brand pages load **motion-landing** (the motion idea, scroll storyboard, technique palette).

## 0. Ask for the project's taste first
Never pick the feel yourself. Ask the developer once and write the answer into Foundations / `project_memory decide`:
- calm and soft, or lively and bouncy? (decides standard vs back/spring)
- fast or relaxed? (decides the duration scale)
- anything that must not move (brand rules, older users, kiosks)?

## 1. Code or Lottie?
| What | How |
|---|---|
| Page/route change, sheet, dialog, drawer, menu, toast, tab indicator, list items appearing, pressed buttons, skeleton shimmer, accordion | **Code** with motion tokens (CSS / Framer Motion / Flutter animations). It must follow real sizes and data. |
| Splash / logo, success ✓ / error ✕, branded loading, empty-state art, onboarding art, icons that animate when tapped (heart, star, bell) | **Lottie** made with `create_lottie`. Art that moves as one drawing. |

If a Lottie needs live data (a counter, a user's name, a progress value) → code, or Lottie only for the decoration around it.

## 2. Motion tokens (Foundations)
Put them in Foundations next to colors and spacing, then use only these in code — never type a raw ms or curve.

| Token | Default | Use |
|---|---|---|
| `duration.instant` | 100 ms | pressed state, toggles |
| `duration.fast` | 150 ms | small things: chips, icons, tooltips |
| `duration.base` | 250 ms | dialogs, sheets, most UI |
| `duration.slow` | 400 ms | page transitions, large surfaces |
| `ease.standard` | cubic(0.2, 0, 0, 1) | moving within the screen |
| `ease.enter` | cubic(0, 0, 0, 1) | things appearing |
| `ease.exit` | cubic(0.3, 0, 1, 1) | things leaving (leave faster than enter) |
| `ease.bounce` | cubic(0.34, 1.56, 0.64, 1) | only if the project's taste is lively |

Adjust the defaults to the answers in step 0. Figma prototypes use the same values (Smart Animate, same duration and curve).

## 3. Making a Lottie (`create_lottie`)
1. Draw the art in Figma first, in a frame the size of the animation. **Each part that moves separately is its own top-level layer with a clear name** (`Circle`, `Check`, `Dot`, `Dot`…). Outline text (Flatten) — text and images are not supported. Linear and radial gradients work (use them for light and shade); blur, shadows and masks are dropped — draw a soft shadow as a shape with a see-through gradient instead.
2. Check with the user what moves, in what order — a few lines like a storyboard (`0 ms circle pops in → 300 ms check draws → 500 ms dots appear one by one`).
3. Call `create_lottie` with `nodeId` of the frame, `out` (`assets/lottie/<name>.json` in Flutter, `public/lottie/<name>.json` on the web), `duration` and tracks. Example:
   ```
   tracks: [
     { layer: "Circle", prop: "scale", from: 0, to: 1, end: 450, ease: "spring" },
     { layer: "Check", prop: "draw", from: 0, to: 1, start: 300, end: 700, ease: "out" },
     { layer: "Dot*", prop: "opacity", from: 0, to: 1, start: 500, duration: 200, stagger: 80 },
     { layer: "Dot*", prop: "move", from: [0, 10], to: [0, 0], start: 500, duration: 300, stagger: 80, ease: "back" }
   ]
   ```
   Props: opacity 0–1, scale (1 = 100%), move [dx, dy] px from where the art is, rotate degrees, draw 0–1 (strokes), fill / stroke colors. Every layer pivots around its own centre. Loops that fade out at the end: set `poster` (ms of a full frame) — cards, reduced motion and the Figma GIF show that frame. Draw things where they end up (confetti already burst out) and animate them in from elsewhere, so the still frame looks finished. A layer name that matches nothing comes back as a warning listing the real names — fix the spec, don't guess.
4. Play it in code: `lottie-react` / `lottie-web` (web), `lottie` package (Flutter). Loop, speed and autoplay are player options.

## 3b. Designing the motion in Figma Motion (the default when the design lives in Figma)
Animate the real screens and art on their Figma Motion timeline, let the user watch and tweak it there, then hand off. Two ways to a Lottie: `create_lottie` (spec only) or `motion_to_lottie` (from the timeline) — prefer the timeline when the user wants to see and adjust it in Figma.
1. **Draft** with `motion_animate` on the frame: the same track language as `create_lottie` (`layer`, `prop`, `from`/`to`, `start`/`end`/`duration` ms, `ease`, `stagger`, or `keys [[ms, value, ease?]]`), plus `width`, `height`, `radius`; or Figma's presets (`presets: [{ layer, style: "Opacity" | "Position" | "Scale" | "Rotation" | "Size" | "Path" | "TextStagger", props, delay, duration }]` — `motion_read styles:true` lists each preset's props). `replace: true` redoes the layers it aims at. Tell the user to press play on the frame in Figma.
   - An ease shapes the segment **leaving** its key (`hold` keeps the value until the next key). Rotation is degrees in the spec (Figma stores radians — handled).
   - Use the project's motion tokens (§2) for durations and easings even in the draft.
2. **Classify ⏸** with `motion_check`: a table of every animated layer → **code + tokens** (text, components, auto-layout, images, simple transforms) or **Lottie** (vector art with path drawing / colour / multi-step motion, pieces of the same illustration, `Lottie/…` components), plus a lint (UI slower than ~400 ms, width/height animations, too many durations/easings, mixed frames, a screen still moving after ~1 s). Show the table, fix lint items with the user, and wait for approval.
3. **Hand off** after approval:
   - code: `motion_tokens` over all screens of the flow → Foundations / the project's tokens file (map onto existing tokens instead of adding new ones); `motion_to_code` (`css` or `framer-motion`) per screen → adapt into the real components. Flutter: translate the same tokens into `Duration`/`Curves` and `AnimationController` / implicit animations.
   - Lottie: art mixed into a screen goes in its own frame first (one frame = one Lottie); then `motion_to_lottie` into `public/lottie/` (web) or `assets/lottie/` (Flutter). Springs become curves (Lottie has no springs); only the frame's top-level layers animate.
4. `get_design_context` lists the frame's motion (`motion`: each layer, code or Lottie) so coding picks it up — never silently drop an animation that is in the design.

## 4. Rules for both
- Motion explains a change (where it came from, what happened). No decoration loops on screens people work in.
- Splash: ≤ 1.5–2 s, never delays the app — load data in parallel, skip the rest once ready. The OS splash (static) shows the same first frame, so the hand-off has no jump.
- Success/error art: ≤ 1 s, then the result text stays.
- Loading art only for waits > 1 s; under that, a skeleton or nothing.
- Respect reduced motion (`prefers-reduced-motion`, `MediaQuery.disableAnimations`): show the last frame of a Lottie, swap movement for a short fade in code.
- Nothing flashes more than 3 times per second.
