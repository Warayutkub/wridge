---
name: responsive
description: Every page design — build mobile and desktop (and tablet when the layout changes a lot) so code can be responsive without guessing.
area: design
---
# Responsive design

Design every page responsive from the start, so developers never have to invent the mobile or desktop version.

## Frames per page
- Always: `Page/Mobile` (390) and `Page/Desktop` (1440). Add `Page/Tablet` (768) when the layout changes meaningfully between them.
- Same page name prefix, placed side by side (mobile left → desktop right), top-aligned.
- **Matching layer names across sizes** (`Header`, `Hero`, `Card/Product`…). `get_design_context` merges breakpoints by layer name — mismatched names break the merge.
- Use the same components in every size; change props/variants, don't redraw.

## Build fluid, not fixed
- All containers auto layout; content widths FILL; text wraps (fixed width or FILL with HUG height).
- Desktop: content wrapper with `maxWidth` ~1200 centered — don't stretch text to 1440.
- No fixed heights on content containers (HUG) so longer text never clips.
- Images keep their ratio; decide crop behavior (`FILL`) per breakpoint.

## Typical adaptations (decide and design them explicitly)
| Element | Mobile | Desktop |
|---|---|---|
| Navigation | top bar + hamburger or bottom tabs | full horizontal nav / sidebar |
| Hero | stacked (text above image), 32–40px heading | side by side, 48–64px heading |
| Card grids | 1 column (2 for small cards) | 3–4 columns (`wrap` or GRID layout) |
| Section spacing | 32–48 | 64–96 |
| Side padding | 16–24 | auto margins with max width |
| Tables | stacked cards or horizontal scroll | full table |
| Primary action | full-width button near the bottom | inline, hug width |
| Secondary content | collapsed / moved below | sidebar or columns |

Elements that exist only on one size (hamburger vs nav menu) are fine — keep their names distinct and clear.

## Mobile web vs mobile app — ask which one when unclear
They share Foundations (same brand) but not the same patterns:
| | Responsive web on a phone | Native/Flutter app |
|---|---|---|
| Frames | `Page/Mobile` + `Page/Desktop` of the same page | `Screen/App` (390) per screen; desktop only if the app also targets tablets/desktop |
| Navigation | header + hamburger/drawer, or a sticky bottom CTA; browser back exists | bottom tab bar + top app bar with back; gestures |
| Layout | must reflow between sizes | fixed to phone widths (360–430), safe areas (status bar, home indicator) |
| Hover states | yes on desktop | none — pressed states instead |
| Inputs | browser keyboards, autofill | native pickers, sheets, haptics |
| Code | breakpoints in CSS/Tailwind | widgets adapt with `LayoutBuilder` only for tablets |
A product with both (e.g. customer app + admin web) gets both sets, each following its platform.

## Self-check
1. Temporarily widen/narrow a frame with `move_resize` (e.g. mobile 360 and 430), `export_node_image`, look for overflow, overlaps, awkward wraps; restore the size.
2. Long Thai strings and big numbers still fit.
3. Both frames tell the same story in the same order.

## Handoff
Note for code: breakpoint widths used, what changes at each, and which frames to pass to `get_design_context` as `breakpoints: {base: mobileId, md: tabletId, lg: desktopId}`.
