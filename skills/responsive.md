---
name: responsive
description: Every web page design — four frames, each designed for its screen: Mobile, Tablet (iPad), Tablet Pro (iPad 13"), Desktop — so code can be responsive without guessing. Also mobile web vs app.
area: design
---
# Responsive design

Design every web page for four screens from the start, so developers never have to invent a size — and each size can look different where that serves it better.

## Four frames per web page — always
| Frame | Size | Gets this layout (CSS width) | Screens |
|---|---|---|---|
| `Page/Mobile` | 390 × 844 | below 744 | phones |
| `Page/Tablet` | 820 × 1180 | 744–1023 | iPad mini (744/756), iPad and iPad Air 11" (820), iPad Pro 11" (834) — upright; Android tablets upright |
| `Page/Tablet Pro` | 1032 × 1376 | 1024–1279 | iPad Pro / Air 13" upright (1024–1032); iPad and iPad Pro 11" sideways (1180–1210) |
| `Page/Desktop` | 1440 × 900 | 1280 and up | laptops and desktops; iPad 13" sideways (1366–1376) |

- **Each frame is designed for its screen**, not a squeezed or stretched copy of another. Tablet Pro is not "Tablet but wider" and not "Desktop but narrower".
- Same page name prefix, side by side left → right: Mobile, Tablet, Tablet Pro, Desktop; top-aligned.
- **Matching layer names across sizes** (`Header`, `Hero`, `Card/Product`…). `get_design_context` merges breakpoints by layer name — mismatched names break the merge.
- Use the same components in every size; change props/variants, don't redraw.
- Code switches layouts at **744 / 1024 / 1280** (min-width), not at the frame widths. `get_tokens` writes these into the token file.
- Pages designed before this rule with fewer frames keep them; offer to add the missing sizes, don't do it unasked. New pages always get all four.

## What each size is for
- **Mobile:** one hand, thumb zone at the bottom, one column.
- **Tablet (820 × 1180):** touch, two hands or on a stand. Two columns, navigation as top tabs or a side rail (not a phone bottom bar stretched wide), bigger type than mobile. The screen is tall (about 3:2): content fills the height — no content bunched at the top with an empty bottom.
- **Tablet Pro (1032 × 1376):** a big touch screen almost as wide as a laptop and very tall (4:3). Room for list + detail side by side, 2–3 columns, a side rail with labels. Still touch: targets ≥ 44, nothing hover-only. The same layout also shows on an 11" iPad held sideways (1180 × 820 — short), so never depend on the height: fill the tall frame with content that also scrolls fine on a short screen (no `100vh` heroes; min/max heights).
- **Desktop (1440 × 900):** mouse and keyboard; content max ~1200 centered. Hover may add things but never hide them — an iPad 13" held sideways gets this layout too.

## Build fluid, not fixed
- All containers auto layout; content widths FILL; text wraps (fixed width or FILL with HUG height).
- Content wrapper with `maxWidth` on the large frames (Tablet Pro ~960–1000, Desktop ~1200) centered — don't stretch text across the screen.
- No fixed heights on content containers (HUG) so longer text never clips.
- Images keep their ratio; decide crop behavior (`FILL`) per size.

## Typical adaptations (decide and design them explicitly)
| Element | Mobile | Tablet | Tablet Pro | Desktop |
|---|---|---|---|---|
| Navigation | top bar + hamburger, or bottom tabs | top tabs or icon+label side rail | side rail with labels, or full top nav | full top nav / sidebar |
| Hero | stacked, 32–40px heading | stacked or 50/50, 40–48px | side by side, 48–56px | side by side, 48–64px |
| Card grids | 1 column (2 for small cards) | 2 columns | 3 columns | 3–4 columns |
| List + detail | separate screens | separate, or split if short | split view | split view |
| Section spacing | 32–48 | 48–64 | 56–80 | 64–96 |
| Side padding | 16–24 | 32 | 40 + max width | auto margins + max width |
| Tables | stacked cards or horizontal scroll | key columns, details on tap | full table | full table |
| Primary action | full-width button near the bottom | hug width, bottom or inline | inline | inline |
| Secondary content | collapsed / moved below | below or second column | side column | sidebar or columns |
| Dialogs | bottom sheet | centered dialog | centered dialog | centered dialog |

Elements that exist only on some sizes (hamburger vs nav menu) are fine — keep their names distinct and clear.

## Mobile web vs mobile app — ask which one when unclear
They share Foundations (same brand) but not the same patterns:
| | Responsive web | Native/Flutter app |
|---|---|---|
| Frames | the four frames above | `Screen/App` (390) per screen; `Screen/Tablet` (820 × 1180) and `Screen/Tablet Pro` (1032 × 1376) only if the app also targets iPads/tablets (else the phone layout is shown scaled) |
| Navigation | header + hamburger/drawer, or a sticky bottom CTA; browser back exists | bottom tab bar + top app bar with back; on tablets a navigation rail |
| Layout | must reflow between sizes | phone widths (360–430), safe areas (status bar, home indicator); tablets in both orientations and split screen |
| Hover states | desktop only, never the only way | none — pressed states instead |
| Inputs | browser keyboards, autofill | native pickers, sheets, haptics |
| Code | breakpoints 744 / 1024 / 1280 in CSS/Tailwind | `LayoutBuilder` by width (`AppBreakpoints` from `get_tokens`) |
A product with both (e.g. customer app + admin web) gets both sets, each following its platform.

## Self-check
1. Each frame at the edges of its range: temporarily resize with `move_resize`, `export_node_image`, look for overflow, overlaps, awkward wraps, empty bottoms; restore the size.
   - Mobile 360 and 430 · Tablet 744 and 1000 · Tablet Pro 1024 and 1210 × 820 (an 11" iPad sideways) · Desktop 1280 and 1920.
2. Long Thai strings and big numbers still fit.
3. All four frames tell the same story in the same order; nothing reachable only by hover.

## Handoff
`design_map` lists each page's frames ready for code (`base` Mobile, `md` Tablet, `lg` Tablet Pro, `xl` Desktop) and says which are missing. Note for code: what changes at each size, and pass the four frames to `get_design_context` as `breakpoints: {base, md, lg, xl}`.
