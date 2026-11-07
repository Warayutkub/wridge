---
name: backend-performance
description: Server-side speed and load — queries (indexes, N+1, select only what is used), bottlenecks, caching (when and which layer), queues, rate limits/backpressure, stateless scaling, load testing with k6. Load for data-heavy features, lists/reports, slow pages/APIs, heavy jobs, or before launch. UI-side performance lives in the platform skill.
area: code
---
# Backend performance & load

## 0. Context first (ask once per project)
`project_knowledge {action: "list", topic: "performance"}` shows the stack and saved facts. Advice that depends on infrastructure needs the facts **`deploy`** (where/how it runs: VPS/Docker, number of instances, serverless/Vercel, shared hosting) and **`redis`** (available or not; queue service?). If missing, **ask the user before writing that code**, then save each answer with `action: "fact"`. Version-specific APIs (Next.js caching, DB driver pool options, Fiber/Gin middleware): read or research + save a note under topic `performance`, as in `backend-security`.

## 1. Measure, don't guess
- Set a target in numbers before optimizing (e.g. p95 < 300 ms at 50 req/s, report < 2 s).
- Find the bottleneck with tools: DB slow query log + `EXPLAIN` (look for full scans, filesort, rows examined ≫ rows returned); Node `--cpu-prof` / timing logs per step; Go `net/http/pprof`. Fix the biggest one, measure again.

## 2. Queries — the usual bottleneck
- **Select only what is used:** name the columns (no `SELECT *`), return only fields the caller shows; map rows to a response type. Server → client components get only the props they render (everything passed is serialized and shipped).
- **Pagination** for every list (cursor/keyset for big or infinite lists, `LIMIT/OFFSET` only for small ones); never return unbounded rows. Aggregate in SQL (`COUNT`, `SUM`, `GROUP BY`), not in JS/Go loops.
- **No N+1:** one query with `JOIN` or `WHERE id IN (…)` instead of a query per row; batch inserts/updates.
- **Indexes** for columns in `WHERE`, `JOIN`, `ORDER BY` (composite in the order used; tenant id first, e.g. `(school_id, date)`); check with `EXPLAIN`; don't index everything (writes slow down). `code_graph table <name>` lists every query on a table and the filtered columns that have no index.
- Keep transactions short; no network calls inside them. Avoid functions on indexed columns in `WHERE` (`DATE(created_at) = …` → range).
- Connection pool sized for the DB: instances × pool size < DB max connections; set timeouts; release connections on every path.

## 3. Work that should not block a request
- Calls to external APIs, emails/LINE messages, PDFs/exports, image processing, anything > ~1 s or batch-sized → a **queue/background job** (BullMQ with Redis, a DB-backed jobs table, Go worker pool) and return immediately with a status the UI can poll or be notified about.
- CPU-heavy work never on Node's event loop request path (worker thread or job). Go: bounded worker pools, `context` with timeouts, no unbounded goroutines.
- Run independent awaits in parallel (`Promise.all`, `errgroup`).

## 4. Cache — only when it pays
Use a cache when data is read often, changes rarely, and measurement shows it is slow or costly. Order of preference:
1. HTTP/CDN caching for public data (`Cache-Control`, static/ISR pages).
2. Framework cache (Next.js data cache / `revalidate` / tags — check the version's note; APIs change between majors).
3. In-process memory (small, per instance, short TTL) — fine on one instance.
4. Redis — shared across instances, for sessions/rate limits/hot lookups.
Rules: every cached value has a TTL **and** an invalidation on write (tag/key per tenant); never cache per-user or permission-dependent data under a shared key; prevent stampedes (single-flight / lock, slight TTL jitter); the app must still work when the cache is empty or down.

## 5. Under load
- **Stateless servers:** no sessions, uploads or job state on local disk/memory that another instance needs — DB, Redis or object storage instead; then add instances behind a load balancer.
- **Backpressure:** rate limits (see `backend-security`), request body size limits, timeouts on every outgoing call, a max concurrency for expensive endpoints; shed load with `429/503` instead of letting everything slow down.
- Payloads: compress responses, paginate, stream big exports.

## 6. Load testing (k6)
Use when it matters, not for every change: before launch, after changing a hot path (login, listing, checkout, attendance check-in at 08:00), or when a slowdown is reported.
- Script in `tests/load/<flow>.js` (k6): realistic flow + think time, ramp up to the expected peak and ~2× it, thresholds as pass/fail (`http_req_duration: ['p(95)<300']`, `http_req_failed: ['rate<0.01']`).
- Run against a local/staging copy with production-like data size — **never against production without the user's go-ahead**. Report: max throughput before the threshold breaks, the bottleneck found, and what was changed.

## 7. Before handing over
State what you measured (before → after), the indexes/queries added, any cache with its TTL + invalidation, and any facts you asked the user for.
