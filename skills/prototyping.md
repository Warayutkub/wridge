---
name: prototyping
description: Required after designing 2+ screens or any dialog/sheet/toast (workflow stage 6), and whenever adding interactions, flows, transitions or scroll behavior.
area: design
---
# Prototyping

Tools: `get_prototype`, `add_interaction`, `remove_interactions`, `set_flow_start`, `set_scroll` (call via `call_tool` if not listed).

## Rules
- A design is not ready for review until it is wired: every button/tab/card that leads somewhere, every back, every dialog/sheet opening (`OVERLAY`) and closing, and the main flow from its start to its end.
- Start with `check_flows`: dead ends, orphan screens, broken links, and buttons/tabs with no interaction show what is missing.
- Read existing interactions first (`get_prototype`); never remove the user's connections unless asked.
- Connect from the **interactive element** (button, tab, card), not from whole screens. Destinations are screens (frames directly on the page or in a section).
- Every screen needs a way forward and a way back: back buttons use `action: "BACK"`; tab bars link every tab on every screen.
- Each flow gets a starting point (`set_flow_start`) with a clear name (`Onboarding`, `Checkout`).
- Finish with `check_flows` again: zero dead ends, orphans and broken links (or say why one is intended).

## Transitions by meaning
| Change | Transition |
|---|---|
| Switching tabs / same-level content | `INSTANT` or `DISSOLVE` 0.2s |
| Going deeper (list → detail) | `PUSH` or `SLIDE_IN` direction `LEFT`; back with `BACK` |
| Modal / bottom sheet | `OVERLAY` + `MOVE_IN` direction `TOP`; close with `CLOSE` |
| Same element changes state | `SMART_ANIMATE` (layer names must match on both screens) |

Duration 0.2–0.3 s, easing `EASE_OUT`; never above 0.5 s.

## Scrolling
Long screens: keep header / tab bar fixed with `set_scroll` (`fixedNodeIds`); inside auto layout set them `absolute: true` (`set_props`) first.

## Limits
The API can't set the prototype device or overlay position/background — tell the user to set those in Figma.
