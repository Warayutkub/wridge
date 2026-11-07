---
name: docs-migrations
description: Database schema changes through migrations (incl. multi-tenant DBs) and project documentation — README/setup, API docs, architecture notes, env vars, changelog. Load when changing the DB schema, adding/changing endpoints or env vars, starting a project, or when setup/docs are missing or wrong.
area: code
---
# Migrations & documentation

## Migrations
**0. Use the project's mechanism.** Look for `migrations/`, `prisma/`, `drizzle/`, `db/migrate`, `goose`/`golang-migrate` files, a migrate script or route. Follow it exactly. None yet → propose one to the user before adding it (TS: the ORM's migrator or `node-pg-migrate`/Kysely migrations; raw SQL: numbered `.sql` files + a small runner; Go: `golang-migrate` or `goose`).
- **New projects: the schema lives in one place** — `supabase/migrations/`, `schema.prisma` (+ its migrations) or one `migrations/` folder; no tables created from dashboards, setup scripts or API routes. `code_graph data` reads it, and flags tables made elsewhere. Record the choice with `project_memory decide`.
- **Before adding a table or column:** `code_graph find <name>` (and synonyms: phone/tel/mobile) — reuse an existing one if it holds the same thing; before changing one: `code_graph impact <table>.<column>`.
- **Every schema change is a new migration file** — never edit an applied one, never change the DB by hand (Supabase dashboard included). Timestamped/numbered, one purpose (`20261006_add_attendance_early_out.sql`), committed with the code that needs it.
- Up and down (or a written rollback plan when down is unsafe). Migrations run in CI against a fresh DB (`testing-ci` integration job) and on deploy before the new code serves traffic.
- **Zero-downtime order** (old and new code run side by side during deploy): expand → migrate data → switch code → contract. Add nullable columns/new tables first; backfill in batches; make `NOT NULL`/drop/rename only in a later release. Renames = add new + copy + switch + drop old.
- Big tables: add indexes online where the DB supports it, batch updates (e.g. 1,000 rows per transaction), avoid long locks; estimate time on production-sized data.
- **Multi-tenant (DB or schema per tenant):** one migration set applied to every tenant by a runner that records the version per tenant, continues past one failure, reports which tenants failed, and can be re-run safely (idempotent). New tenants get the latest schema from the same migrations.
- Seeds: dev/test seed data separate from migrations; never real personal data.

## Documentation — short, true, next to the code
| Doc | Must contain |
|---|---|
| `README.md` | what it is, stack, **setup from zero** (prereqs + versions, `cp .env.example .env`, DB up, migrate, seed, run, test), common scripts, folder map, deploy link |
| `.env.example` | every env var with a fake value + one-line comment; update whenever code reads a new var |
| API docs | per endpoint: method + path, auth/role, input, output, errors — OpenAPI (`openapi.yaml`, or generated from zod / swaggo for Go) for public/partner APIs; a markdown table is fine for internal ones |
| `docs/architecture.md` | the big picture: layers, tenants, auth flow, external services, background jobs — diagram + a few paragraphs; decisions that surprise people (ADR-style: decision, why, date) |
| `CHANGELOG.md` | user-visible changes per release (from Conventional Commits) |
- Update docs **in the same PR** as the change that makes them wrong. Prefer one accurate page over many stale ones; delete docs that no longer apply.
- Comments in code explain *why*, not *what*. Project-wide context for AI sessions goes in `code_graph_note` (dir notes) and `AGENTS.md`/`CLAUDE.md` if the repo uses them.
- Docs follow the team's language (Thai or English) consistently.

## Before handing over
Migration: file name, what it changes, how it was run (fresh DB + existing data), rollback. Docs: which files were updated for this change.
