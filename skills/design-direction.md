---
name: design-direction
description: When a file has no Foundations page yet — a new product (propose a direction, brainstorm, build Foundations) or an existing code project being brought into Figma (derive Foundations and components from the code). Colors with light/dark, type, spacing, radius, shadows, base components.
area: design
---
# Design direction & Foundations

Each system deserves its own taste. Never impose one house style; derive it from the product, then agree on it with the user before designing screens.

## 0. Check first
`get_document_info` → is there a page named **Foundations**? If yes: read it (`get_tokens {format:"json"}` and `get_node` on its sections) and follow it — don't re-propose. If no and the product already exists in code: follow **Existing product** below. If no and it is new: continue at 1.

## Existing product — the code is the source of truth
Goal: a Figma base that matches what ships, so new work can be designed first. No new direction; the direction is in the code. Load `code-graph`.
1. **Survey (no file reading):** `code_graph overview` (component usage), `tokens` (defined + raw colors most used), `find` for shared UI. Shared UI may be React components **or CSS classes** (`.btn`, `.btn-p`, `.card` in `globals.css`) — read only the stylesheet sections you need.
2. **Plan rounds, show the user, then go one round per turn**, smallest first: (a) Foundations — every token plus the raw colors used often (name them, e.g. `slate-400`); list near-duplicates and ask before merging; (b) the 10–15 most used primitives; (c) domain components; (d) screens only when a screen is about to change. Stop after each round for review.
3. **Components mirror code exactly:** same name; each variant prop/class family becomes a variant property (`Type=Primary`, `Size=Small`, `State=Hover`); build each variant with `create_component`, then `combine_as_variants`; labels/icons via `add_component_property`. Values from the CSS/props, not taste. Only what exists in code — list missing pieces (no Dialog, no Toast…) as **gaps** and ask before designing them.
4. **Link at once:** `map_component` — a code component (`import` + `component`), or for class-based UI `element` + `className` + `variants` (`{"Type=Primary": "btn-p"}`) on the set name. Rule for later: new UI becomes a real component; class mappings are for what already exists.
5. **Check against the real app:** run it (or ask for screenshots) and compare with `export_node_image`.
6. A docs/showcase page, if wanted, uses **instances** of these components — never separate drawings.
7. Progress lives in `code_graph design` (linked / code without Figma), so the next session continues from there.

## 1. Read the context
The request, file name, existing screens (`scan_design_values` on them shows colors/fonts/spacing already in use), the code project if any (existing CSS/theme, logo, brand colors). Summarize what you understood in 2–3 lines.

## 2. Ask the developer's taste first (in chat, no drawing yet)
The direction is theirs, not yours. Before proposing anything, ask what step 1 didn't answer: existing logo/brand colors, 2–3 products or sites they like (and what about them), mood in a few words, what they dislike, audience, dark mode needed? Wait for the answers.

## 3. Propose — built from their answers
Give **one recommended direction** that traces each choice back to what they said (mark anything you filled in yourself as "my suggestion") and **1–2 short alternatives**. Keep it conversational, like a designer brainstorming:

```
Product: … (who, when, how often)
Recommended direction: "<3 personality words>"
- Color: mood + primary color (why) + how the accent is used
- Type: heading font + body font (Thai-capable), why
- Shape: radius level, borders vs shadows
- Density: airy / balanced / compact (per platform: app vs web/admin)
- Imagery & icons: photo / illustration / none; outline vs filled icons
- Voice: formal / friendly
Alternatives: "<words>" — one line each, what changes
Open questions: only what is still unclear
```
If the user wants to see it, draw small **style tiles** (one frame per direction: palette, type sample, button, card, mini screen) side by side, then let them pick or mix. Iterate until they say go.

## 4. Choosing colors
- **Primary** from brand first; otherwise from the mood: blue/teal = trust, calm; green = growth, money, health; orange/coral = warm, friendly, energetic; red = urgent, passion (careful: clashes with error); purple = creative, premium; black/neutral + one accent = premium/minimal. Check competitors to stand apart.
- **Scale 50–950** for primary (and one secondary if needed): even steps in lightness; keep hue, slightly reduce saturation at the extremes (pure white/black mixing looks muddy). Typical use: 50–100 backgrounds, 200–300 borders, 500–600 main/actions, 700–900 text on light.
- **Neutrals** tinted slightly toward the primary hue (e.g. warm grays for a coral brand), 50–950.
- **Semantic:** success (green), warning (amber), error (red), info (blue) — tuned to sit with the primary; if the primary is red/orange, make error clearly distinct.
- **Harmony:** analogous for calm brands, complementary accent only for small highlights; ≤ 1 accent hue besides primary.
- **Dark mode** (if needed): not an inversion. Background near-black tinted (not #000), higher surfaces lighter, primary shifted lighter/less saturated, text off-white; recheck contrast.
- **Contrast check** every real pair: text/background, text on primary buttons, links, borders — AA (4.5:1 text, 3:1 large/UI).

## 5. Choosing type
- Thai products: a Thai-capable pair. Examples: Prompt or Kanit (headings, character) + IBM Plex Sans Thai / Noto Sans Thai / Sarabun (body, readable). One family for both is fine for calm/minimal products.
- Scale from body 16: e.g. 12, 14, 16, 18, 20, 24, 32, 40, 48; Thai line height 150–160% body, 120–130% headings.

## 6. Build Foundations (works on the free Figma plan)
Create a **section named `Foundations`** (or a page with that name if the file has pages to spare — free files allow only 3) containing frames whose **names are the conventions** below — `get_tokens` reads layer names + properties directly, no variables or styles required:

| Frame name | Contents (layer name = token name) | Value read from |
|---|---|---|
| `Colors/Light` | swatches named `primary`, `primary-50`…`primary-900`, `surface`, `text`, `muted`, `border`, `success`… | fill |
| `Colors/Dark` | same names, dark values | fill |
| `Spacing` | rectangles named `xs`, `sm`, `md`, `lg`, `xl`… | width |
| `Radius` | rectangles named `sm`, `md`, `card`, `full` | corner radius |
| `Typography` | text layers named `display`, `heading-h1`, `heading-h2`, `body`, `caption`… set in the real font/size/line height | text properties |
| `Shadows` | rectangles named `sm`, `md`, `lg` | drop shadow |

- Name captions/labels inside these frames with a leading `_` (e.g. `_spec`) so they are never read as tokens.
- Use the same swatch names in Light and Dark. A swatch can be a frame named `primary` containing a rectangle + label text (the frame name is used when the rectangle has a default name).
- Add a `Brief` text frame with the agreed direction (personality, do/don't) so later sessions keep the taste.
- Then base components on the same page (or a `Components` page): Button (primary/secondary/ghost × default/hover/pressed/disabled/loading), Input (+ error), Card, Tag, Nav item — **plus the feedback set**: Dialog, Bottom sheet, Toast, Banner, Skeleton, Spinner, EmptyState (specs in `interaction-patterns`) — built from these values.
- If the file's plan supports it and the user wants Figma-native tokens, mirror colors into variables with `create_variable_collection` / `create_variables` (modes Light/Dark); otherwise the Foundations page alone is enough.
- Build it with `run_batch`; verify with `export_node_image`; then `get_tokens {format: "json"}` to confirm every token was picked up.

## 7. Then design
Every screen uses only Foundations values. When something new is needed (a new color or size), add it to Foundations first, then use it.
