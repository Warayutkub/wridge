# Project decisions

## D-1 · 2026-10-01 · Payments go through Stripe
Tags: payments, infra
Decision: Stripe payment intents only.
Why: Thai cards work.

## D-2 · 2026-10-02 · Webhooks are idempotent
**Tags:** Payments
Decision: store the event id.

## Hand-written rule without id
Keep prices in satang.

## D-3 · 2026-10-05 · Use PromptPay too
Tags: payments
Supersedes: D-1
Decision: Stripe + PromptPay QR.
