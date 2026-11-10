---
name: seo
description: Being found and shared — titles and descriptions, headings, Open Graph previews (LINE, Facebook, X), sitemap and robots, structured data (business, event, product), Thai-language pages, indexable rendering, Search Console. Load when building or reviewing a public website or landing page, when the user wants it on Google, asks why the link preview in LINE looks wrong, or before launch. Not needed for logged-in apps and admin tools.
area: code
---
# SEO and link previews

## 0. Context
Ask once and save as `project_knowledge` facts: the site's **domain**, the main **audience and language** (Thai only? Thai + English?), and what people search for (2–5 phrases in the words customers use, e.g. "ติดตั้งกระจกนิรภัย นนทบุรี"). No invented keywords.

## 1. Every public page
- `<html lang="th">` (or the page's language); one `<title>` ≈ 50–60 characters, unique per page, the important words first, brand last ("กระจกนิรภัย ติดตั้งถึงบ้าน · ชื่อร้าน").
- `<meta name="description">` ≈ 120–160 characters, written for people (it's the snippet under the title).
- `<link rel="canonical">` to the page's one URL (no duplicates via `?utm=` or trailing slashes).
- One `<h1>` that says what the page is; headings in order; real text in HTML (not baked into images); `alt` on meaningful images.
- Readable URLs: short, lowercase; English or transliterated slugs share better than percent-encoded Thai (`/glass-shower` rather than `/%E0%B8…`).
- Internal links between related pages with descriptive text (not "คลิกที่นี่").

## 2. Link previews (LINE, Facebook, X, Discord)
- Open Graph on every page: `og:title`, `og:description`, `og:image` (1200×630, under ~1 MB, absolute https URL), `og:url`, `og:type`, `og:site_name`; plus `twitter:card="summary_large_image"`.
- LINE and Facebook cache previews: after changing them, re-scrape with Facebook's Sharing Debugger; LINE refreshes on its own after a while — test with a new URL or query string.
- The preview image is a design asset: readable text at small size, brand visible, not just a logo on white.

## 3. Crawling and indexing
- Content must be in the HTML the server sends (SSR/SSG/static). A client-only SPA shows Google an empty page more often than not.
- `sitemap.xml` with every public page (frameworks generate it), `robots.txt` pointing to it; staging and preview sites send `noindex` (meta or `X-Robots-Tag`) so they never compete with the real site.
- Correct status codes: 404 for missing pages (not a 200 "not found" page), 301 for moved pages; no redirect chains.
- Speed and mobile layout count a little for ranking and a lot for visitors: `web-performance`.

## 4. Structured data (JSON-LD)
Pick the type that matches the page and fill real values only; validate with Google's Rich Results Test.
- A shop or service: `LocalBusiness` (name, address, phone, opening hours, geo, url, image).
- An event, concert, fair: `Event` (name, startDate with time zone `+07:00`, location with address, offers with price/currency/availability/url, organizer, image, eventStatus).
- A product: `Product` with `offers`; articles: `Article`; FAQ only for real FAQ content.
- `Organization` with logo and `sameAs` (Facebook, LINE OA, Instagram) on the home page.

## 5. After launch
Verify the domain in Google Search Console, submit the sitemap, check coverage and errors there; a Google Business Profile for a local business matters more than any meta tag. Rankings take weeks — say so.

## 6. Before handing over
List per page type: title/description pattern, OG image, structured data type, sitemap/robots/noindex on staging, and the Search Console steps the user must do themselves (they own the account).
