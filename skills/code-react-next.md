---
name: code-react-next
description: Writing React / Next.js (App Router) + Tailwind CSS v4 UI code — from a Figma design (load design-to-code first) or without one. Also load interaction-code.
area: code
---
# React / Next.js + Tailwind v4

## Setup
- Tokens: `get_tokens {format: "tailwind", outFile: "<project>/app/tokens.css"}` and `@import "./tokens.css";` after `@import "tailwindcss";` in `globals.css`. Tokens become utilities (`--color-peach-500` → `bg-peach-500`, `--spacing-4` → `p-4`, `--radius-md` → `rounded-md`, `--text-heading-h1` → `text-heading-h1`, `--shadow-card` → `shadow-card`). Use the returned `map` for names: a text style or shadow named like a color is renamed (`--text-type-tile`, `--shadow-depth-card`) so `text-*`/`shadow-*` stay unambiguous. After regenerating tokens, restart `next dev` — Turbopack can keep serving the old imported `tokens.css`.
- Fonts: `next/font/google` (e.g. `IBM_Plex_Sans_Thai`, subsets `["thai","latin"]`), exposed as a CSS variable used by `--font-*`.
- Assets: `export_assets {outDir: "<project>/public/assets"}`; images via `next/image` with width/height or `fill` + sized parent; SVG icons as React components (`components/icons/`) using `currentColor`.
- Check the project's Next.js version/docs (`node_modules/next/dist/docs`, `AGENTS.md`) before using APIs — newer versions change conventions.

## Mapping
| Design context | Tailwind |
|---|---|
| `direction: row/column` | `flex flex-row` / `flex flex-col` |
| `gap`, `padding` | `gap-*`, `p-*`/`px-*`/`py-*` (token or 4px scale; arbitrary `[13px]` only if unavoidable) |
| `justify: between/center/end`, `align: center/end/baseline` | `justify-between`… / `items-center`… |
| `wrap` + `rowGap` | `flex-wrap gap-y-*` |
| `display: grid`, columns `["1fr","1fr","1fr"]` | `grid grid-cols-3`; mixed tracks → `grid-cols-[240px_1fr]` |
| child `grid.columnSpan: 2` | `col-span-2` |
| `width: "fill"` | parent row → `flex-1 min-w-0`; parent column → `self-stretch`/`w-full` |
| `width: "hug"` | nothing (or `w-fit`) |
| fixed width | `w-[320px]` only when truly fixed; prefer `max-w-*` |
| `position` + `pin: {x: "right", y: "bottom"}` | parent `relative`; child `absolute right-* bottom-*` |
| `radius: "full"` | `rounded-full` |
| `style.overflow` | `overflow-hidden` |
| `responsive.md` / `.lg` | `md:` / `lg:` prefixes (mobile-first); `hiddenAt: ["lg"]` → `lg:hidden`; `onlyAt: ["lg"]` → `hidden lg:flex` |
| `states["State=Hover"]` | `hover:`; Pressed → `active:`; Disabled → `disabled:` + `aria-disabled`; Focus → `focus-visible:` |
| `link.action: navigate` | `<Link href="/route">` (derive route from the destination frame name) |
| `link.action: overlay` | dialog/sheet component with open state |
| `text.spans` | nested `<span className=…>` |
| `repeat` | `items.map((item) => <Card key={item.id} {...item} />)` with a typed data array |

## Code rules
- Server Components by default; `"use client"` only for interactivity (state, handlers, effects).
- Components in `components/` (`ui/` for primitives like Button), PascalCase files, typed props (`type Props = {…}`), no `any`.
- Variants via a small map or `cva`-style object, not long ternaries. Merge classes with `clsx`/`cn` if the project has it.
- Semantic HTML: `header/nav/main/section/footer`, `h1–h3` in order, `button` for actions, `a`/`Link` for navigation, `ul/li` for lists, `label` for inputs.
- Images: meaningful `alt`; decorative `alt=""`.
- Keep page files thin: compose sections (`HeroSection`, `FeatureGrid`) and keep data in constants or props.

## Structure — layers (MVC idea, Next.js shape)
| Layer | Where | Rule |
|---|---|---|
| Model / data | `lib/db/`, `lib/models/` | types + queries only; the only place that touches the DB |
| Logic | `lib/services/<domain>.ts` | business rules (pricing, payroll, permissions); pure where possible, no React, no `Request` |
| Controller | `app/api/**/route.ts`, Server Actions | parse + validate input, check auth, call a service, return; no business rules |
| View | `app/**/page.tsx`, `components/` | render; get data from services (server) or the API (client) — never query the DB directly |
- One concern per file; split files past ~300 lines (check `code_graph impact` before moving code).
- Shared UI in `components/ui/`, feature UI in `app/<route>/_components/`; reuse before writing (`code_graph find`).
- Existing projects that mix layers: don't rewrite; when touching a part, move that part's logic into its service first.

## Performance
- Fetch on the server (Server Components) and in parallel (`Promise.all`); no request waterfalls, no N+1 queries — one query with a join/`IN`.
- Small client bundles: `"use client"` on leaves, not pages; `next/dynamic` for heavy, rarely used UI (charts, editors, PDF).
- `next/image` with sizes; `next/font`; no layout shift (reserve space for images/skeletons).
- Lists > ~100 rows: paginate or virtualize. Memoize only measured hot spots.
- Cache what is stable (`revalidate`, `cache`), never per-user data in shared caches.
