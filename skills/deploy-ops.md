---
name: deploy-ops
description: Getting code onto a server and keeping it running — Docker images, environments (dev/staging/production), secrets, CI/CD deploys without downtime, database changes during a deploy, rollback, backups with restore drills, domains/HTTPS, a runbook. Load when setting up hosting, writing a Dockerfile or deploy pipeline, adding env vars or secrets, preparing a launch, or when "deploy", "ขึ้นระบบ", "server ล่ม" comes up. Logging, health checks and alerts live in reliability; schema changes in docs-migrations.
area: code
---
# Deploy & operations

## 0. Context — ask once, save as facts
`project_knowledge` fact **`deploy`**: host (VPS, Docker on a VM, a PaaS such as Railway/Render/Fly, Vercel/Netlify, shared hosting), regions, number of instances, the database host, who has access, and the budget. Don't design for Kubernetes when one VPS is the plan.

## 1. Environments
- At least **local, staging, production**; staging mirrors production (same image, same config shape) with its own database and test keys (payments, LINE).
- Staging data is anonymized (`privacy-pdpa`); production data never copied to laptops.
- Every environment has its own secrets; production secrets are known to the fewest people.

## 2. Config and secrets
- Config from environment variables (validated at startup — fail fast with a clear message if one is missing); `.env.example` documents every variable (`docs-migrations`), real `.env` files are git-ignored.
- Secrets live in the host's secret store / CI secrets, never in the repo, images, front-end bundles or logs; rotate after a leak or when someone leaves.

## 3. Docker (when used)
Multi-stage build (build deps don't ship), a small pinned base image, run as a non-root user, `.dockerignore` (no `.git`, `node_modules`, `.env`), `HEALTHCHECK` or the platform's health path, one process per container, logs to stdout. Tag images by git SHA, not just `latest`.

## 4. Deploys without downtime
- CI: tests → build once → deploy the same artifact to staging → check → production (`testing-ci`, `git-team`).
- Rolling or blue-green: new instances must pass `/ready` before taking traffic; old ones finish in-flight requests (graceful shutdown in `reliability`).
- **Database changes** are expand → migrate → contract: add new columns/tables first (old code keeps working), deploy code that uses them, remove old ones in a later release. Never a migration that breaks the running version (`docs-migrations`).
- Feature flags for risky features, so turning one off doesn't need a deploy.
- Deploy when people can watch it — not right before a ticket sale opens or a weekend.

## 5. Rollback
Know the command before you need it: redeploy the previous image tag / release; migrations are backward-compatible so code can go back without touching data; write down what can't be rolled back (sent emails/LINE messages, charged payments) and how to compensate.

## 6. Backups
Automatic daily (or more) database backups kept off the server, with a retention period; file uploads in object storage with versioning or backups; **a restore actually tested** on staging — an untested backup is a guess. Note the recovery time and how much data could be lost.

## 7. Domain, HTTPS, the edge
HTTPS everywhere (managed certificates or Let's Encrypt with auto-renew); HSTS once stable; DNS with a lower TTL before moving servers; a CDN for static files (`web-performance`); firewall the server to the ports in use; SSH by key, no root password login.

## 8. Runbook
A short `docs/runbook.md`: how to deploy, roll back, restart, read logs, restore a backup, rotate a secret, and who to call — written for someone stressed at 2 a.m.

## 9. Before handing over
State the environments and where each runs, the deploy steps (and what runs automatically), how config/secrets are provided, the rollback command, backup schedule and the last restore test, and anything the user must set up in accounts only they own (hosting, DNS, payment/LINE consoles).
