---
name: accessibility
description: When checking or designing for accessibility — contrast, target sizes, reading order, color independence, text scaling, Thai text.
area: both
---
# Accessibility (WCAG 2.1 AA, practical)

## Check these in every design
- **Contrast:** normal text ≥ 4.5:1; large text (≥ 24px, or ≥ 18.66px bold) ≥ 3:1; icons, input borders, focus rings ≥ 3:1 against neighbours. Placeholder text is not a label and still needs ~4.5:1 if it carries meaning.
- **Not color alone:** errors, status, selection, links also use an icon, text, underline or shape.
- **Targets:** ≥ 44×44 on touch, ≥ 24×24 minimum on desktop, with spacing between adjacent targets.
- **Text:** body ≥ 16px web / ≥ 14px mobile minimum; line height ≥ 1.5 for body (Thai ≥ 1.5 — tone marks and vowels above/below get clipped otherwise); avoid all-caps paragraphs; left-aligned body text (no justify for Thai).
- **Text resizing:** layouts must survive 200% text — use HUG heights and wrapping, no fixed-height text boxes.
- **Focus:** design a visible focus state for interactive components (2px outline, ≥ 3:1).
- **Reading order:** layer order matches visual order (auto layout guarantees this — keep it).
- **Headings:** one H1 per page, logical H2/H3 nesting, visually distinct.
- **Forms:** visible labels, instructions before the field, errors next to the field and in text.
- **Images:** decide meaningful vs decorative; meaningful images need alt text in the handoff (put it in the layer name or a note).
- **Motion:** no flashing > 3/s; large motion should have a reduced-motion alternative.

## How to check with tools
- Read colors with `get_node`/`get_design_context` and compute contrast (relative luminance) for each text/background pair; report pairs that fail with the ratio.
- Measure interactive elements' sizes from `get_node`.
- Export with `export_node_image` at small size to judge hierarchy and legibility.

## Report format
List each issue: element (name + id), problem, measured value vs target, fix. Fix them directly when asked to design; otherwise propose.
