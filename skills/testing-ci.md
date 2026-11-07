---
name: testing-ci
description: Writing tests and CI — unit → integration (→ a few E2E), coverage gates, GitHub Actions pipelines. Load when adding/changing logic or API endpoints, fixing a bug (regression test first), or setting up/changing CI.
area: code
---
# Testing & CI

## 0. Use what the project has
Check `package.json` scripts/devDeps, `vitest/jest/playwright` configs, `*_test.go`, `.github/workflows/`. Extend the existing setup; never add a second test runner or replace a working workflow. Nothing yet → Next.js/TS: **Vitest** (+ `@vitest/coverage-v8`); Go: `testing` + `testify`; E2E: Playwright. Version-specific config: `project_knowledge` topic `testing` (read, or research + save). CI provider: fact `ci` (default GitHub Actions; ask if the repo shows something else).

## 1. The pyramid — what to test where
| Level | Tests | Rules |
|---|---|---|
| **Unit** (most) | services / logic (`lib/services`, `internal/…`): calculations, rules, permissions, parsing, formatting | pure inputs → outputs; no DB/network; fast (ms) |
| **Integration** | API route / handler + real DB + auth | real database in Docker (CI `services:` or testcontainers), migrations applied, seeded per test; for every endpoint: success, invalid input (400), not signed in (401), **other tenant's/user's record (403/404)** |
| **E2E** (few) | critical flows only: sign in, payment, the product's core action | Playwright against a built app; not for every screen |
- A bug fix starts with a test that fails because of the bug.
- Test behaviour, not implementation details; one reason to fail per test; Arrange–Act–Assert; names say the rule (`"charges VAT on the discounted price"`).
- Deterministic: fake clock/timezone (Thai dates, Buddhist year, month ends), seeded data, no real external APIs (mock at the HTTP boundary: `msw`, `httptest.Server`), no order dependence, no sleeps.
- Edge cases that matter: empty, boundaries, rounding of money, duplicates, concurrency where it can happen (`go test -race`).

## 2. Coverage
- Gate **80% lines and branches on logic/services folders**, enforced in CI (Vitest `coverage.thresholds` with `include`, Go `-coverprofile` + a check). UI components are not gated.
- Never write assertion-free or trivial tests to reach the number; untested risky code is listed in the PR instead.

## 3. GitHub Actions pipeline
`.github/workflows/ci.yml` on `pull_request` and `push` to the main branch; `concurrency` cancels outdated runs; dependency cache (`actions/setup-node` `cache: npm`, `actions/setup-go` cache). Jobs, fast first:
1. **check** — install (`npm ci`), lint, typecheck (`tsc --noEmit`) / `go vet`, `gofmt -l`.
2. **unit** — tests with coverage + thresholds; upload the report as an artifact.
3. **integration** — `services:` (mysql/postgres/redis) with health checks, run migrations, then tests. `needs: check`.
4. **build** — `next build` / `go build ./...`.
5. **security** — `npm audit --audit-level=high` / `govulncheck ./...`.
- Secrets only from `secrets.*` / `vars.*`; test env values in the workflow are fake. Pin actions to a major version (`@v4`), least `permissions:` (`contents: read`).
- k6 load tests (see `backend-performance`) go in a separate `workflow_dispatch` workflow, not on every PR.
- Required checks + branch protection are GitHub settings: tell the user which checks to require; don't change repo settings yourself.

## 4. Before handing over
Run the tests locally (and the CI steps you changed); report: tests added per level, pass/fail, coverage of the touched folders, anything left untested and why.
