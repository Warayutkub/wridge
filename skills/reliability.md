---
name: reliability
description: Server-side error handling, timeouts/retries/fallbacks for external services, idempotency, background job safety, logging, health checks and monitoring. Load when calling external APIs (payments, LINE, email, slip checks, AI), handling money/payroll/important writes, writing jobs/cron, or adding logging/monitoring. UI error states live in interaction-code.
area: code
---
# Reliability

## 0. Context
`project_knowledge` facts: `deploy`, and **`monitoring`** (error tracker / log destination, e.g. Sentry, Grafana, plain files) — ask once if missing and save. Library-specific APIs (retry helpers, logger, Sentry SDK for this framework version): read or research + save under topic `reliability`.

## 1. Errors — one consistent way
- Distinguish **expected** errors (validation, not found, forbidden, conflict, external service unavailable) from **bugs**. Expected → typed result/error class (TS) or sentinel/wrapped errors (Go `errors.Is/As`, `fmt.Errorf("…: %w", err)`); bugs → let them reach one top-level handler.
- One error shape for the API: `{ error: { code, message, requestId } }` with the right status (400/401/403/404/409/422/429/503); `message` is safe for users, details go to logs.
- Never swallow errors (`catch {}`, ignored `err`); add context when re-throwing; one error handler per layer boundary (route/middleware), not try/catch on every line.
- A failure in one part must not break the rest: per-widget/section error boundaries (Next.js `error.tsx`), partial results with a notice, Go handlers recover panics in middleware.

## 2. Calling things that can fail (APIs, DB, email, LINE, payment)
- **Timeout on every call** (`AbortSignal.timeout(ms)`, `context.WithTimeout`); no call waits forever.
- **Retry only what is safe:** idempotent operations or those with an idempotency key; only on timeouts/5xx/429 (respect `Retry-After`); exponential backoff + jitter; 2–3 attempts max. Never retry 4xx validation errors.
- **Fallback** decided per feature and shown to the user: cached/last known data, degrade the feature ("ส่ง LINE ไม่ได้ — จะลองใหม่อัตโนมัติ"), or queue for later. Repeated failures → circuit breaker (stop calling for a while) for services that go down often.
- **Idempotency** for anything that charges, sends or creates: idempotency key per request, unique constraints in the DB, webhooks deduplicated by event id. Double-clicks and retries must not double-charge or double-send.
- **Money, payroll, stock:** DB transaction around related writes; integer minor units or `DECIMAL`, never floats; audit log of who changed what.

## 3. Background jobs & cron
At-least-once: jobs are idempotent; max attempts + dead-letter/failed list someone can see; job state persisted (not in memory); a lock so cron does not run twice on two instances; long jobs report progress.

## 4. Logging
- Structured (JSON in production): level, time, `requestId` (propagate to downstream calls and back in the error response), user/tenant id, route, duration, error with stack.
- Levels: `error` = needs a person; `warn` = degraded/retried; `info` = business events (payment received, job done); `debug` off in production.
- Never log passwords, tokens, full card/ID numbers, request bodies with personal data — redact.

## 5. Health & monitoring
- `/health` (process up) and `/ready` (DB/critical deps reachable) for the deploy platform / load balancer.
- Errors go to the error tracker (fact `monitoring`) with release version; alert on error-rate spikes, failing jobs, p95 latency; graceful shutdown (finish in-flight requests, close pools) on SIGTERM.
- Backups of the DB exist and a restore has been tried — mention it if the project has none.

## 6. Before handing over
List external calls touched with their timeout/retry/fallback, idempotency for writes that charge/send/create, and what gets logged on failure. Integration tests for the failure paths (timeout, 5xx, duplicate webhook) go with `testing-ci`.
