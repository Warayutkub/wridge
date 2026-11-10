---
name: scale-traffic
description: Handling many users at once — capacity estimates, the order to scale in, database limits (pooling, replicas, hot rows), CDN, autoscaling, and rush moments (registration or booking opening at 10:00 — waiting room, no overselling, bots). Plus load testing with k6 as traffic scenarios, its limits told to the user, results as a report. Load when the user mentions many users, traffic, "ล่ม", "คนเยอะ", a launch/opening time, scaling, or wants a load test. Query-level speed stays in backend-performance; timeouts/retries in reliability.
area: code
---
# Scale & traffic

## 0. Context first (ask once per project, save as facts)
`project_knowledge` facts **`deploy`** (where it runs, how many instances, managed DB or not) and **`traffic`**: expected users, the peak moment (e.g. "ticket sale opens 10:00, ~3,000 people waiting"), and what must never break (no double booking, no overselling). Missing → ask before designing, then save. Without numbers, say what you assume.

## 1. Capacity in numbers, before any change
- Peak arrival rate: people × actions each ÷ seconds of the rush. 3,000 people × 5 requests in the first 60 s ≈ 250 req/s, with bursts higher.
- Concurrency in flight ≈ rate × response time (Little's law): 250 req/s × 0.4 s ≈ 100 requests at once → that many DB connections/workers would be busy.
- Compare with what one instance does (measure it, §6) → instances needed, plus ~2× headroom for the rush. Write the numbers in your reply.

## 2. Scale in this order — stop at the first step that meets the numbers
1. **Fix the hot paths** (`backend-performance`): indexes, no N+1, paginate, cache. Usually 5–20× for free.
2. **Static and cacheable at the edge:** assets, images, public pages behind a CDN with long cache headers; the origin only serves what changes.
3. **Bigger machine** (vertical) — simplest, fine for most small products.
4. **Queue the slow parts:** emails, LINE messages, PDFs, slip checks → background jobs so requests stay short.
5. **More instances** (horizontal): only when the app is stateless (sessions, uploads, locks in DB/Redis/object storage) behind a load balancer with health checks.
6. **Database:** connection pooler, then read replicas, then partitioning (§3). Sharding is a last resort — say so before proposing it.

## 3. The database is usually the ceiling
- **Pooling:** instances × pool size must stay under the DB's max connections; serverless or many instances → a pooler (PgBouncer, RDS Proxy, the host's pooler). PgBouncer in transaction mode breaks session features (some prepared statements, `SET`, advisory locks) — check the driver's setting.
- **Read replicas** for heavy reads (lists, reports); they lag — read your own writes from the primary (right after booking, show the booking from the primary).
- **Hot rows:** one row many people update at once (seats left, stock, a counter) serializes everything. Use a single atomic statement (`UPDATE seats SET left = left - 1 WHERE id = $1 AND left > 0` and check rows affected), or reservations rows with an expiry instead of one counter; avoid long transactions holding the lock.
- **Partition** very large, time-based tables (logs, attendance by month) so queries and cleanup touch one part.

## 4. Rush moments (opening at a set time)
- **Know the time → prepare:** scale up and warm caches before it; autoscaling reacts in minutes, a rush happens in seconds.
- **Waiting room:** at peak, give each visitor a place in line (signed token, position, estimated wait) and let N per second into the real flow; the queue page is static/cached and cheap. Better than everyone hitting the DB and failing together.
- **No overselling:** decrement atomically (§3), hold a reservation for a few minutes (`expires_at`), release unpaid ones by a job, unique constraints so one person can't hold two seats if that's the rule. Payment confirmation by webhook, idempotent (`payments-th`).
- **Bots and abuse:** rate limit per IP *and* per account, a challenge (e.g. Turnstile/reCAPTCHA) at the queue entry, one active session per account during the sale, block obviously scripted bursts. Tell the user what fairness rules exist.
- **Fail soft:** when overloaded, answer quickly with a friendly "busy, you keep your place" (429/503 + retry) instead of timing out; keep payment and booking working even if non-essential pages are degraded.

## 5. Autoscaling, limits and watching
- Scale on a signal that leads: CPU, queue length, request latency; min instances ≥ 2 for anything that must stay up; cap max so a bug or attack can't run up the bill.
- Before an event set a target (SLO), e.g. 99% of booking requests < 1 s, error rate < 1%, and watch it live (dashboards/alerts from `reliability`). Agree who watches and what to do if it breaks (a short runbook).
- Real-time updates at scale (queue position, live seat map): `realtime`.

## 6. Load testing with k6 — traffic as scenarios
Scripts in `tests/load/<flow>.js`; run with `k6 run --summary-export=tests/load/out/<flow>.json tests/load/<flow>.js`. Not installed → tell the user `winget install k6` (Windows) and wait.

| Scenario | k6 executor | Use for |
|---|---|---|
| Ramp | `ramping-vus` | normal growth through the day |
| Spike | `ramping-arrival-rate` with a jump to the peak in seconds | opening at 10:00 |
| Stress | `ramping-arrival-rate` rising until thresholds fail | finding the ceiling |
| Soak | `constant-vus` for hours | leaks, connections not released |
| Steady rate | `constant-arrival-rate` | N requests per second regardless of slowness |

Make it look like people: a mix of flows (e.g. 70% browse, 20% log in, 10% book) as separate scenarios or weighted branches; different accounts from a CSV via `SharedArray`; `sleep()` think time between steps; `tags: { name: 'book' }` per request so results split per endpoint; checks on responses (status, no oversell). Thresholds as pass/fail: `http_req_duration{name:book}: ['p(95)<1000']`, `http_req_failed: ['rate<0.01']`.

**Tell the user before every run (always, in your reply):**
1. One computer can simulate roughly a few thousand virtual users or a few thousand simple requests per second; more needs several machines or a cloud runner.
2. All traffic comes from one IP: rate limits, a CDN or bot protection may block or skew it — allow-list the test machine or test against the origin.
3. It simulates requests, not real browsers: page rendering and JavaScript speed are not measured — that is `web-performance`.
4. Run against your own machine or staging with production-sized data. Load-testing production while people use it can take it down; never test a site that isn't the user's.
Then ask for the go-ahead to run, and stay within the agreed number of users and duration.

## 7. Report the result
Write it as a report (`report` tool; load `reports-tasks` for the blocks): stats (max req/s before thresholds broke, p95, error rate, users simulated), a line chart of latency (p95) against users or time with a mark where it started to fail, a table per endpoint (p95, errors), a callout with the bottleneck and the fix, and a closing callout with the four limits above — so nobody reads "passed at 2,000 users" as a promise about real people.

## 8. Before handing over
State the capacity numbers and assumptions, what was changed for scale (and why that step of §2), what protects the rush (waiting room, atomic stock, rate limits), the test scenarios with results and their limits, and what to watch on the day.
