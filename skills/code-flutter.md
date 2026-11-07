---
name: code-flutter
description: Writing Flutter (Dart) UI code — from a Figma design (load design-to-code first) or without one. Also load interaction-code.
area: code
---
# Flutter

## Setup
- Tokens: `get_tokens {format: "dart", outFile: "<project>/lib/theme/tokens.dart"}` → `AppColors`, `AppSpacing`, `AppRadius`, `AppTextStyles`, `AppShadows`, `AppGradients` (other modes as `AppColorsDark`…). Build `ThemeData` (`ColorScheme`, `TextTheme`) from them in `lib/theme/app_theme.dart`; dark mode from the `Dark` classes.
- Assets: `export_assets {outDir: "<project>/assets/images", platform: "flutter"}` writes `name.png`, `2.0x/name.png`, `3.0x/name.png`; declare `assets/images/` in `pubspec.yaml`. SVG icons with `flutter_svg` (`SvgPicture.asset`, `colorFilter` for tint).
- Fonts: Thai fonts as assets in `pubspec.yaml` (or `google_fonts`), family names matching the tokens.
- Units: design px = logical pixels (dp) 1:1.

## Mapping
| Design context | Flutter |
|---|---|
| `direction: column / row` | `Column` / `Row` |
| `gap` | `spacing:` (Flutter ≥ 3.27) or `SizedBox` between children |
| `padding` | `Padding(padding: EdgeInsets.all / symmetric / fromLTRB(left, top, right, bottom))` — note LTRB order differs from CSS |
| `justify` | `mainAxisAlignment` (`between` → `spaceBetween`) |
| `align` | `crossAxisAlignment` (`center`, `end`, `baseline` + `textBaseline`) |
| `width: "fill"` (main axis) | `Expanded` / `Flexible` |
| `width: "fill"` (cross axis) | `crossAxisAlignment: CrossAxisAlignment.stretch` or `SizedBox(width: double.infinity)` |
| `width: "hug"` | `mainAxisSize: MainAxisSize.min` / intrinsic size |
| fixed size | `SizedBox(width:, height:)` only when truly fixed; `ConstrainedBox` for min/max |
| `wrap` | `Wrap(spacing:, runSpacing:)` |
| `display: grid` | `GridView` / `SliverGrid` with `SliverGridDelegateWithFixedCrossAxisCount` (or `LayoutGrid` package for mixed tracks) |
| `position` + `pin` | `Stack` + `Positioned(right:, bottom:)`; `left-right` → `left:` and `right:` |
| `style.background`, `border`, `radius`, `shadow` | `DecoratedBox`/`Container(decoration: BoxDecoration(color:, border: Border.all(), borderRadius:, boxShadow: AppShadows.x))` |
| `style.overflow` | `ClipRRect` |
| `text` | `Text('…', style: AppTextStyles.x)`; `spans` → `Text.rich(TextSpan(children: […]))` |
| `repeat` | a widget class + `List<Data>` → `ListView.builder` / `Column(children: items.map(…))` |
| `states` Pressed / Disabled | `ButtonStyle` with `WidgetStateProperty.resolveWith`; Hover for web/desktop |
| `link.action: navigate` | `Navigator.push` / `context.go('/route')` (go_router if the project uses it) |
| `link.action: overlay` | `showModalBottomSheet` / `showDialog` |
| `responsive.md` / `.lg`, `hiddenAt`, `onlyAt` | `LayoutBuilder` (or `MediaQuery.sizeOf`) with breakpoints from the design (e.g. 600 / 1024); switch layouts per breakpoint |

## Code rules
- `StatelessWidget` by default; state only where needed. `const` constructors everywhere possible.
- One widget per file in `lib/widgets/` (shared) or `lib/features/<feature>/widgets/`; screens in `lib/features/<feature>/<name>_screen.dart`. snake_case files, PascalCase classes.
- No magic numbers: `AppSpacing`, `AppRadius`, `AppColors`, `AppTextStyles` (or `Theme.of(context)`).
- Extract widgets instead of deeply nested builders; keep `build` methods short.
- Accessibility: `Semantics` labels for icon buttons and images, `tooltip` on `IconButton`, minimum tap size 48.
- Thai text: set `height` from tokens; never fix text container heights.

## Structure — layers (MVC idea, Flutter shape)
Feature-first: `lib/features/<feature>/{data,logic,widgets}` + `<name>_screen.dart`; shared in `lib/core/`, `lib/widgets/`, `lib/theme/`.
| Layer | Where | Rule |
|---|---|---|
| Model / data | `data/` (models, repositories) | the only place that talks to DB/API/files |
| Logic | `logic/` (controller / view-model / notifier) | state + business rules; no `BuildContext`, no widgets |
| View | screens, `widgets/` | render state, forward user actions to logic; no queries |
- Use the state approach the project already uses (Provider, Riverpod, Bloc…); don't add a second one.
- Split files past ~300 lines (check `code_graph impact` first); existing mixed code gets moved when touched.

## Performance
- `const` widgets; `ListView.builder` / slivers for long lists; never build large lists eagerly.
- Rebuild the smallest subtree: select only the state a widget needs; heavy work off `build` (and off the UI isolate with `compute` when long).
- Images sized (`cacheWidth`), `precacheImage` for the first screen; dispose controllers/streams.
