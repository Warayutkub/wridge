---
name: vector-shapes
description: Bending and reshaping shapes like Figma's vector edit mode (Enter) — curve a side, waves, trapezoids, cut corners, move / add / remove points, paint closed areas — and every stroke setting (dashes, end caps, arrows, joins, taper / variable width, dynamic wobble, brushes). Load before reshaping a shape or styling a line beyond color and weight.
area: design
---
# Vector shapes and strokes

Three tools do what Figma's vector edit mode (select a shape, press Enter) does, without a mouse; `set_stroke` covers the whole Stroke settings panel.

| Want | Use |
|---|---|
| A common bend (curved side, wave, trapezoid, cut / rounded corner) | `bend_shape` — one call |
| See the points first, or an exact edit | `vector_read` → `vector_edit` |
| Line style: dashes, arrows, taper, hand-drawn, brush | `set_stroke` |

Rectangles, ellipses, polygons, stars and lines **become a vector** the moment you edit their points (Figma does the same on Enter). The id changes — use the id the tool returns for the next call. They keep their name, place and fill, but lose their shape-only controls (a rectangle's corner radius field, a polygon's point count). `vector_read` on a shape reads a copy and changes nothing.

## bend_shape — ready-made
`{nodeId, effect, side? | corner?, amount, count?}` — amount in px.

| effect | does | example |
|---|---|---|
| `bulge` | a side curves outward (negative: inward); amount = how far its middle moves | ticket edge: `side: "bottom", amount: 24` · concave: `amount: -16` |
| `wave` | a side becomes `count` waves of height `amount` | wavy banner bottom: `side: "bottom", amount: 8, count: 6` |
| `pinch` | a side's two ends move toward each other (trapezoid); negative spreads | `side: "top", amount: 30` |
| `skew` | a side slides along itself (parallelogram) | `side: "top", amount: 20` |
| `chamfer` | cuts corners off diagonally | `corner: "all", amount: 12` (octagon-ish) |
| `round` | rounds chosen corners only (radius = amount) | `corner: "top-left", amount: 24` |

`side`: top · bottom · left · right · all. `corner`: top-left · top-right · bottom-right · bottom-left · all. A side needs a straight edge lying on that side of the shape's box — on an ellipse use `vector_edit` instead.

## vector_read
Returns `points [{i, x, y, corner?, cap?}]` (x, y inside the layer), `segments [{i, from, to, curve?}]` (`curve.start` / `curve.end` are the handles, relative to each end point), `regions [{i, loops, fill?}]` (closed areas), `sides {top: {points, segments}, …}` and `corners {top-left: i, …}`.

## vector_edit
`{nodeId, ops: [...]}` — ops run in order, one save at the end. Points / segments are picked by index, a list, a side, a corner or `"all"`.

| op | fields | what it is in Figma |
|---|---|---|
| `move` | `points`, `dx` / `dy` (or `x` / `y` for one point) | Move tool: drag points |
| `curve` | `segments`, `amount` (px at the middle; + outward) | Bend tool |
| `handles` | `segment`, `start: [x, y]`, `end: [x, y]` | dragging the bezier handles |
| `straighten` | `segments` | double-click a curve |
| `split` | `segment`, `at` 0–1 (default 0.5) | click on a segment to add a point |
| `cut` | `segment`, `at` | Cut tool — opens the path there |
| `remove` | `points` or `segments` | Delete / Erase. A point between two segments: its neighbours join up. A segment: the path opens |
| `connect` | `from`, `to` | Pen from one point to another |
| `corner` | `points`, `radius` | corner radius of single points |
| `cap` | `points`, `cap` | end point style on open paths |
| `mirror` | `points`, `mode` NONE · ANGLE · ANGLE_AND_LENGTH | handle mirroring |
| `paint` | `region`, `color` (or "none") | Paint tool — fill one closed area |

Indexes shift after `split`, `cut` and `remove` (new points go last) — read again before picking by index, or pick by side / corner.

Example — a speech-bubble tail on a rectangle: `vector_edit` ops `[{op: "split", segments: "bottom", at: 0.2}, {op: "split", segment: <the new segment>, at: 0.15}, …]` gets fiddly by index; read first, then `move` the one new middle point down. For simple looks prefer `bend_shape` — e.g. a ticket: `bulge` side left and right with a negative amount.

## set_stroke — Stroke settings
`color`, `weight`, `align` as before, plus:

- **Basic:** `dash: [8, 4]` (dash, gap px) or `"solid"`; `cap` both ends — NONE · ROUND · SQUARE · ARROW_LINES · ARROW_EQUILATERAL · DIAMOND_FILLED · TRIANGLE_FILLED · CIRCLE_FILLED; `startCap` / `endCap` for one end of an open vector (an arrow only at the end); `join` MITER · BEVEL · ROUND; `miterAngle` degrees.
- **Width profile:** `widthProfile` UNIFORM · WEDGE · TAPER · QUARTER_TAPER · EYE · MIRRORED_TAPER, or `widthPoints: [{position: 0–1, width: ×weight}]` for your own (e.g. thick middle: `[{position: 0, width: 0.3}, {position: 0.5, width: 2}, {position: 1, width: 0.3}]`). Not on branching paths, and not together with dynamic.
- **Dynamic** (hand-drawn wobble): `dynamic: {frequency 0.01–20, wiggle ≥ 0, smoothen 0–1}`; missing values keep what's there (defaults 0.75 / 0.3 / 0.5).
- **Brush:** `brush: {name, direction?}` — stretch brushes HEIST · BLOCKBUSTER · GRINDHOUSE · BIOPIC · SPAGHETTI_WESTERN · SLASHER · HARDBOILED · VERITE · EPIC · SCREWBALL · ROM_COM · NOIR · PROPAGANDA · MELODRAMA · NEW_WAVE (`direction` FORWARD / BACKWARD); scatter brushes BUBBLEGUM · WITCH_HOUSE · SHOEGAZE · HONKY_TONK · SCREAMO · DRONE · DOO_WOP · SPOKEN_WORD · VAPORWAVE · OI (`gap` ≥ 0.25, `wiggle`, `sizeJitter` 0–3, `angularJitter` / `rotation` ±180). Custom brushes can't be set by plugins.
- `kind: "basic"` turns dynamic / brush off again.

Brushes and dynamic strokes are for illustration and hand-made accents — UI lines (dividers, borders, connectors) stay basic.
