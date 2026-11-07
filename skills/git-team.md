---
name: git-team
description: Team Git workflow — branches, Conventional Commits, pull requests, reviews, conflicts, releases. Load when creating branches, committing, opening/updating PRs or resolving conflicts.
area: code
---
# Git for teams

## 0. Follow the repo first
`git log --oneline -20`, `CONTRIBUTING.md`, `.github/pull_request_template.md`, branch names on the remote: match their language and style (e.g. Thai subjects if the repo uses them). The rules below fill the gaps.

## Branches
- Never commit to `main` directly. One short-lived branch per task from the latest main: `feat/<scope>-<short>`, `fix/…`, `chore/…`, `refactor/…`, `docs/…`, `test/…`.
- Keep it current: merge or rebase main into it often (rebase only branches nobody else uses); never force-push a shared branch (`--force-with-lease` on your own only).

## Commits — Conventional Commits
- `type(scope): subject` — types `feat fix refactor perf test docs chore ci build`; imperative, ≤ 72 chars; body says **why**; `BREAKING CHANGE:` footer when it breaks callers; reference issues (`Closes #12`).
- One logical change per commit; tests and code together; formatting-only changes in their own commit.
- Never commit secrets, `.env*`, build output, `.wara/`, large binaries; check `git status`/`git diff --staged` before committing.

## Pull requests
- Small (aim < ~400 changed lines), one purpose; open as draft early if feedback helps.
- Description: what and why, how it was tested, screenshots for UI, migration/config steps, risks/rollback.
- Ready = CI green, self-reviewed diff, no debug code/TODOs without an issue. At least one reviewer approval; answer every comment; squash-merge with a Conventional title; delete the branch.
- Reviewing: correctness, security (auth checks, input), tests, naming/structure; suggest, don't rewrite.

## Conflicts
Understand both sides before choosing; keep both intents when possible; rerun tests after resolving; ask the author when unsure. Lockfiles: regenerate (`npm install`) instead of hand-merging.

## Releases
SemVer tags (`v1.4.0`) from main; changelog from commit types; hotfix = `fix/` branch → PR → patch tag.

## Outward actions
Pushing, opening/merging PRs, tagging and changing GitHub settings affect the team: confirm with the user before doing them.
