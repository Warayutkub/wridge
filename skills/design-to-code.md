---
name: design-to-code
description: When writing UI code from a Figma design (any platform) — workflow, reading get_design_context, tokens, components, assets, clean-code rules. Then load the platform skill.
area: both
---
# Design → code

Then load the platform skill: `code-react-next`, `code-html-css`, or `code-flutter`.

## Workflow
0. **Approved only:** `design_map` → the screens' `status` must be `approved` (section `"<Flow> · Approved"`). Draft / review / no section → stop and ask the user to review the design first (show images, list states). Code built from an unapproved design gets redone.
1. **Tokens (first time per project, or when the design system changes):** `get_tokens` with the platform format (`tailwind` / `css` / `dart`) and `outFile` inside the project. Keep the returned `map` — it translates token names in design context to code identifiers.
2. **Read the design:** `get_design_context` with `projectDir`.
   - Responsive page: pass `breakpoints: {base: <mobile frame>, md: <tablet>, lg: <desktop>}` (only the ones that exist).
   - Big pages: read section by section (`nodeId` of each section) rather than one huge call.
3. **Components:** instances with `code` → import and use that component; `code` with `className` (projects whose UI is shared CSS classes) → that element with exactly those classes, no new styles. Names in `unmappedComponents` → search the project (`code_graph design` lists name-based matches; `code_graph find <Name>` otherwise); if found, record it with `map_component`; if not, create one component and record it. Load `code-graph` for the rest of the code work.
4. **Assets:** `export_assets` for the ids in `assets` (`platform: "flutter"` for Flutter) into the project's asset folder. SVG icons become icon components/widgets.
5. **Write the code** (rules below + platform skill).
6. **Verify:** compare `export_node_image` with the running result; fix differences.

## Reading the context
- `layout`: `direction` row/column (flex), or `display: "grid"` with `columns`/`rows` tracks (`"1fr"`, px number, `"auto"`). Missing gap/padding/justify/align = defaults (0 / start).
- `padding`: `16` | `[y, x]` | `[top, right, bottom, left]` — CSS shorthand order.
- `width`/`height`: `"fill"` (stretch along parent), `"hug"` (fit content), number (fixed px — avoid hard-coding unless truly fixed).
- `position` + `pin`: absolutely positioned child and which edges it sticks to (`right`, `bottom`, `left-right`, `center`, `scale`).
- `grid` on a child: row/column (1-based) and spans.
- `{ token, value }`: use the token via the `map`, never the raw value.
- `kind: "repeat"`: same structure `count` times → one component + a data array; `items[].diff` holds what changes per item (text, colors, props).
- `kind: "lottie"`: an instance of a `Lottie/<name>` component — play the file in `lottie` (`Lottie.asset(path)` / `lottie-react`), never redraw the GIF preview as shapes. See `motion`.
- `kind: "instance"` + `states`: other variants such as `State=Hover` / `State=Disabled` and what changes in each → implement as hover/pressed/disabled styles.
- `link`: interaction on click → route/navigation (`navigate` to `to`), `back`, `overlay` (modal/sheet), `url`.
- `responsive[bp]` (breakpoints mode): properties that change from that breakpoint up, mobile-first; `hiddenAt` / `onlyAt` control visibility.
- `text.spans`: mixed styles inside one text.
- `layout: "absolute"` on a frame means the designer didn't use auto layout there: reinterpret as flex/grid; only overlays/badges stay absolute.

## Clean-code rules (all platforms)
- **No magic values:** tokens for colors, spacing, radius, type. Values without a token snap to the spacing scale.
- **Components for repetition** (every `repeat`, every instance), data via props/parameters; one component per file.
- **Semantic structure:** real headings, buttons, links, lists, landmarks (or the platform's equivalents with semantics).
- **Meaningful English names** by role (`ProductCard`, not `Frame12`).
- **Responsive/adaptive from the start** following the `responsive` data; never lock whole layouts to the design's pixel width.
- **Accessibility:** alt text / semantics labels, labels for inputs, focus styles, contrast as designed.
- **Thai text:** Thai font loaded, line height from tokens, no fixed heights that clip.
- **Don't invent:** no extra features, animations or states not in the design; if states are missing (loading/empty/error), say so or ask.
