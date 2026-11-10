---
name: web-performance
description: How fast a web page feels in the browser — Core Web Vitals (LCP, INP, CLS), images, fonts (Thai fonts are heavy), JavaScript size, render blocking, caching headers and CDN. Load when building or reviewing a public page, landing page or web app, when a page "feels slow"/"โหลดช้า", before launch, or when Lighthouse/PageSpeed scores are low. Server speed lives in backend-performance; many users at once in scale-traffic.
area: code
---
# Web performance (in the browser)

## 0. Measure first, on a phone
- Targets (Core Web Vitals, 75th percentile of real visits): **LCP ≤ 2.5 s** (main content shows), **INP ≤ 200 ms** (responds to taps), **CLS ≤ 0.1** (nothing jumps).
- Lab: Lighthouse / PageSpeed Insights in mobile mode with throttling; DevTools Performance for long tasks. Field: PageSpeed's real-user data once the site has traffic, or the `web-vitals` library reporting to analytics.
- Write before → after numbers; fix the biggest item first.

## 1. LCP — the main thing shows fast
- Find the LCP element (usually the hero image or heading). That image: right size, modern format (AVIF/WebP), `fetchpriority="high"`, **never lazy-loaded**, served from the CDN; preload it if it's set from CSS.
- Server answers HTML fast (static/ISR/cached where possible; `backend-performance` for the rest). No client-side fetch for above-the-fold content when SSR/SSG can render it.
- Remove render-blocking: one CSS file early, non-critical CSS deferred, scripts `defer`/`async`, third-party tags (chat, analytics, pixels) after load or on interaction.

## 2. Images
- `width`/`height` (or `aspect-ratio`) on every image; `srcset` + `sizes` so phones get small files; `loading="lazy"` below the fold only.
- Compress (quality ~70–80 is usually invisible); SVG for icons and logos; no giant PNG photos. Frameworks: `next/image`, Astro `<Image>`, or an image CDN.
- Video: poster image, `preload="none"` unless it's the hero, muted autoplay only when small.

## 3. Fonts — Thai fonts are large
- Self-host WOFF2; only the weights used (often 400/600/700); subset by `unicode-range` (Thai + Latin), as Google Fonts and `next/font` do.
- `font-display: swap`; preload the one or two files used above the fold; match fallback metrics (`size-adjust`, `next/font` does it) so text doesn't jump (CLS).
- One family for Thai text is usually enough; every extra family and weight is another download.

## 4. JavaScript — ship less, run less
- Budget: keep the JS for a page small (check the build output / bundle analyzer); split by route; `import()` heavy widgets (maps, editors, charts) when needed.
- Static content stays static: server components / islands; don't hydrate what never changes.
- INP: no long tasks (> 50 ms) in tap handlers — show feedback first, do heavy work after (`requestAnimationFrame`, `setTimeout`, a worker); debounce input handlers; virtualize long lists.
- Remove unused libraries (moment → `Intl`/date-fns, lodash → native); watch what a new dependency adds.

## 5. CLS — nothing jumps
Reserve space for images, embeds and ads; skeletons the same size as the content; no banners or cookie bars pushed in above content (overlay them); fonts as in §3; animations with `transform`/`opacity` only.

## 6. Caching and delivery
- Hashed assets: `Cache-Control: public, max-age=31536000, immutable`. HTML: short or `no-cache` with revalidation. API responses: cache only what is public (see `backend-performance` §4).
- CDN in front of static files (and pages when possible); Brotli/gzip on; HTTP/2 or 3; few third-party origins (each costs a connection).
- A service worker only when offline/PWA is a goal — and with an update strategy.

## 7. Before handing over
Report the scores and vitals before → after (mobile), what changed (images, fonts, JS removed or split, caching), and anything left with its cost (e.g. "the chat widget costs 180 KB — load it on click?").
