---
name: ui-visual
description: For any visual design work in Figma — structure, spacing, typography, color, components, depth, images, naming.
area: design
---
# UI visual design

## Taste comes from the project, never from you
The rules below are craft (alignment, spacing, contrast). The *look* — colors, fonts, radius, density, mood, imagery — belongs to the project and its developer. Before any visual choice, take it from, in order:
1. `Foundations` in the file (and its `Brief` frame) → follow it.
2. Existing screens (`scan_design_values`) or the code project (theme, CSS, logo) → match it.
3. `project_memory` decisions about the look.
4. Nothing found → **ask the developer first, before drawing**: brand colors/logo, 2–3 products or sites they like (and what about them), mood in a few words, what they dislike, density, dark mode. Then follow `design-direction`.

Never fill a gap with your own defaults (blue on white, generic gradients, the same card style every time) and present it as decided. If you must suggest something, label it a suggestion and ask. Record the agreed look in the `Brief` frame and as a `project_memory` decision so later sessions keep it.

## Structure: auto layout everywhere
- Every container is a frame with auto layout; never place siblings by x/y. Groups only for decoration that needs no layout.
- Sizing: children that should stretch → `widthSizing: "FILL"`; buttons/tags/chips → `HUG`; screens → `FIXED`.
- Nest by meaning: Screen → Section → Container → Content. Sections are vertical; rows are horizontal.
- Child order = visual order; use `move_to_parent` with `index` to reorder.
- `absolute: true` only for floating things (badges, close buttons, decorative shapes).

## Spacing (4/8 grid)
- Only 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 80, 96. Never 13, 17, 22.
- Proximity: inside an element < between elements < between sections.
- Typical: screen edge 16–24 (mobile), card padding 16–24, section gap 32–48 (mobile) / 64–96 (desktop).
- Same padding for the same kind of container across the page.

## Layout & sizes
- Frames: Mobile 390 (375/402 fine), Tablet 768, Desktop 1440.
- Desktop content max ~1200 (`maxWidth`) and centered; one consistent left edge.
- Body text line length 45–75 characters (`width`/`maxWidth` on text).

## Typography
- ≤ 2 font families. Thai text: IBM Plex Sans Thai, Noto Sans Thai, Prompt, Kanit, Sarabun — never Inter for Thai.
- Scale: 12, 14, 16, 18, 20, 24, 32, 40, 48, 64. Body 16 (≥ 14 on mobile), captions 12–14.
- Line height: body 150% (Thai 150–160%), headings 110–130%.
- Weights: 2–3 per page (400/500/600/700). Large headings: letter spacing −1% to −2%.
- Hierarchy = size + weight + color together, not size alone.
- If the file has text styles, apply them (`apply_style`) instead of raw settings.

## Color
- Order of preference: variables → paint styles → a small new palette.
- New palette: 1 primary (+ tints/shades), 5–7 neutrals, semantic success/warning/error/info.
- 60-30-10: neutral backgrounds 60, secondary 30, accent 10. One accent color draws the eye — use it for the primary action.
- Contrast (WCAG AA): text 4.5:1, large text (≥ 24px or ≥ 18.66px bold) and UI parts 3:1.
- Reused colors become styles (`create_paint_style`, names like `Color/Primary/500`).

## Components & consistency
- Check `get_components` before drawing buttons, inputs, cards. Reuse via `create_instance`.
- Anything used ≥ 2 times becomes a component (`create_component`); states (default/hover/pressed/disabled) are variants of one component: name each `State=Hover, Type=Primary`, then `combine_as_variants`; labels/icons as `add_component_property`.
- Same size of a thing looks identical everywhere (buttons, inputs, radius, shadows).
- `component_usage` shows where a component is used (impact of a change); `find_duplicates` finds layers drawn separately that should be one component, and near-copies whose spacing/radius/font size drifted — propose merging them, ask before converting.

## Radius & depth
- Radius scale 4, 8, 12, 16, 24, 999 (pill). Nested: inner radius = outer radius − padding.
- 2–3 shadow levels: low (y 1–2, blur 2–4), medium (y 4, blur 12), high (y 12, blur 32); black at 8–16% only. Shadows express layering, not decoration.

## Images & icons
- Images: `set_image_fill` on a frame with a fixed ratio (16:9, 4:3, 1:1, 3:4); `FILL` for photos, `FIT` for logos. Text over images needs an overlay/gradient that passes contrast.
- Icons: `create_from_svg` or an icon component, sizes 16/20/24/32, colored like adjacent text. Never build icons from primitive shapes.
- Prefer `boolean_shapes` over `flatten` (editable); circular crops via `cornerRadius`/ellipse before masks.

## Naming
English, by role: `Header`, `Hero`, `Card/Product`, `Button/Primary`, `Text/Title`. Slash-group component and style names. No `Frame 123` left behind.
