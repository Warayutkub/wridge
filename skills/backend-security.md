---
name: backend-security
description: Whenever writing or reviewing server-side code — API routes, Server Actions, Go handlers (Fiber/Gin), auth/login/sessions, permissions, DB queries, webhooks, file uploads, third-party/OAuth calls, secrets. Baseline rules + version-specific notes researched once per project and reused.
area: code
---
# Backend security

## 1. Version-specific notes first (once per project, then reuse)
1. `project_knowledge {projectDir, action: "list", topic: "security"}` → the stack with versions, saved notes, and `missing`.
2. For each tool the task touches (framework, auth/JWT, DB driver/ORM, validation): a fresh note exists → `get` it and follow it. Missing or `stale` → research it now:
   - Search the official docs/security pages for that tool **at that major version** (e.g. "Next.js 16 security server actions", "Fiber v2 csrf middleware", "mysql2 prepared statements"), plus its recent security advisories (GitHub advisories / CVEs).
   - Budget: 1–2 searches and **at most 3 fetched pages** per tool — the security-release/advisory page and the one guide section you need, not whole manuals (a long docs page costs ~10k tokens). Fetch more only if a rule is still unclear, and say so in the note.
   - Save with `action: "save"`: ≤ 40 lines of **concrete** rules — the exact APIs, middleware, config flags and pitfalls of that version, each with a short example; no generic advice that is already below. Include the source URLs.
3. Never re-search a tool that has a fresh note. Notes live in `.wara/` (gitignored, created by the tool).

## 2. Baseline — always
**Authentication**
- Passwords: bcrypt (cost ≥ 12) or argon2id; never store/log plain text; compare in constant time.
- Sessions: `HttpOnly; Secure; SameSite=Lax` cookies; short-lived tokens, rotate on login/privilege change, revoke on logout. JWT: fixed algorithm, verify signature + `exp` + `aud`/`iss`; no secrets in the payload.
- Login/forgot/OTP: rate limit per IP and account; the same message whether the account exists or not; reset tokens single-use and expiring.

**Authorization — the most common hole**
- Check on the server on **every** request/action: who is calling, and may they touch **this** record. Deny by default.
- Scope every query by owner/tenant (`WHERE id = ? AND school_id = ?`) — never trust an id from the client alone (IDOR). `code_graph table <name>` shows each query's filters, so unscoped ones stand out; for Supabase it also shows RLS and policies.
- Middleware / route guards are not enough on their own; check again where the data is read or written. Server Actions and API routes are public endpoints.
- Roles in one place (a permission map / service), not scattered `if (role === …)`.

**Input & queries**
- Validate at the boundary with a schema (zod / valibot; Go: `go-playground/validator` or explicit checks): types, lengths, enums, ranges. Reject unknown fields for writes.
- SQL only with placeholders (`?` / `$1`, prepared statements, ORM params) — never string concatenation / template strings / `fmt.Sprintf`. Dynamic `ORDER BY`/column names come from a whitelist.
- Mass assignment: copy allowed fields explicitly; never spread the request body into an update.

**Output**
- React/JSX escapes by default; `dangerouslySetInnerHTML` only with sanitized HTML (DOMPurify). Go: `html/template`, never `text/template` for HTML.
- Errors to clients: a plain message + request id; no stack traces, SQL or internal paths. Details go to server logs (without passwords, tokens, personal data).
- Headers: CSP, `X-Content-Type-Options: nosniff`, `Referrer-Policy`, `frame-ancestors`/`X-Frame-Options`, HSTS in production.

**Rate limiting & abuse**
- Every public endpoint has a limit per user (or API key) and per IP; respond `429` with `Retry-After`.
- Tighter, separate limits where a call costs money or resources: email/LINE/SMS sends, slip/OCR checks, PDF/export, AI calls, search; plus daily caps per account where it fits.
- Counter store follows the deployment (`project_knowledge` fact `deploy`; ask the user if missing): one instance → in-memory is fine; several instances / serverless → a shared store (Redis/Upstash, or the DB) or the platform's limiter, otherwise every instance counts separately.
- Behind a proxy/CDN, take the client IP only from the trusted proxy header.

**Webhooks & third parties**
- Verify the signature over the **raw** body (HMAC, constant-time compare; e.g. LINE `x-line-signature`), reject old timestamps, process idempotently (store event ids).
- OAuth: `state` + PKCE, exact redirect URIs, the smallest scopes that work; store refresh tokens encrypted.
- Outgoing calls: timeouts, no user-controlled URLs without an allowlist (SSRF).

**Secrets & data**
- Secrets only in env/secret store; never in client bundles (`NEXT_PUBLIC_*` is public), git, logs or error messages. `.env*` in `.gitignore`; rotate anything leaked.
- Encrypt sensitive fields at rest when a DB leak would hurt (ID numbers, bank data, tokens); TLS everywhere.

**Files**
- Limit size; check type by content (magic bytes), not extension; random stored names; serve from outside the app root or with `Content-Disposition`.

**Dependencies**
- `npm audit` / `govulncheck ./...` when adding packages or before release; update packages with known advisories.

## 3. Before handing over
List what the change exposes (endpoints/actions) and, for each: who may call it, where that is checked, how input is validated. Note anything left open and why.
