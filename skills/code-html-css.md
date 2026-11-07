---
name: code-html-css
description: Writing plain HTML + CSS (no framework) UI — from a Figma design (load design-to-code first) or without one. Also load interaction-code.
area: code
---
# Plain HTML + CSS

## Setup
- Tokens: `get_tokens {format: "css", outFile: "<project>/css/tokens.css"}` → CSS custom properties (`var(--color-peach-500)`), text styles as `.text-…` classes, other modes under `[data-theme="dark"]`.
- Files: `index.html`, `css/tokens.css`, `css/base.css` (reset, fonts, body), `css/components.css` (one block per component), `assets/`. Record each component class in `wara.components.json` with `map_component {element, className, variants}` so the next screen reuses it.
- Fonts: Google Fonts `<link>` with `display=swap` (Thai fonts: include the Thai subset), `font-family` from the token.
- Assets: `export_assets {outDir: "<project>/assets"}`; inline small SVG icons with `fill="currentColor"`, `<img>` for photos with `width`/`height` and `loading="lazy"`.

## Mapping
| Design context | CSS |
|---|---|
| `direction: row/column` | `display:flex; flex-direction:row/column` |
| `gap`, `padding` | `gap`, `padding` with `var(--spacing-…)` |
| `justify` / `align` | `justify-content` / `align-items` (`between` → `space-between`) |
| `display: grid` + tracks | `display:grid; grid-template-columns: 1fr 1fr 1fr` (or `repeat(auto-fit, minmax(…))` for card grids) |
| child grid span | `grid-column: span 2` |
| `width: "fill"` | in a row: `flex:1; min-width:0`; in a column: `align-self:stretch` |
| `width: "hug"` | default (no width) |
| fixed width | `max-width` preferred; `width` only when truly fixed |
| `position` + `pin` | parent `position:relative`; child `position:absolute; right:…; bottom:…` |
| `responsive.md` / `.lg` | `@media (min-width: 768px)` / `(min-width: 1024px)` — mobile-first; `hiddenAt` → `display:none` in that query |
| `states` Hover / Pressed / Disabled / Focus | `:hover`, `:active`, `:disabled`/`[aria-disabled="true"]`, `:focus-visible` |
| `link` navigate / url | `<a href="…">`; overlay → `<dialog>` |
| `repeat` | one component class (`.card`) used `count` times (or generated from data with a small script if dynamic) |

## Code rules
- Class names by role in kebab-case, optionally BEM (`.card`, `.card__title`, `.card--featured`); no styling by IDs, no inline styles.
- Semantic HTML: landmarks, ordered headings, `button` vs `a`, `label for`, `alt` text, `lang="th"` on `<html>` for Thai pages.
- Every value from tokens; no repeated literal colors.
- Mobile-first media queries only (`min-width`), breakpoints from the design.
- No JavaScript unless the design needs interaction (menus, dialogs); then small, unobtrusive scripts with `defer`.
