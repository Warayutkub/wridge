---
name: payments-th
description: Taking money in Thailand — PromptPay QR, a payment gateway with webhooks, or bank-slip upload with a slip-check service; order states and expiry, no double charges, refunds, daily reconciliation, receipts. Load whenever a feature charges, sells tickets/booths/products, checks a slip ("สลิป", "โอน", "PromptPay"), or handles refunds. Security of the endpoints also needs backend-security; external-call safety reliability.
area: code
---
# Payments (Thailand)

## 0. Context — ask once, save as facts
`project_knowledge` fact **`payments`**: which way money comes in (a gateway such as Opn/Omise, 2C2P, GB Prime Pay or Stripe; PromptPay QR with slip upload; bank transfer), the slip-check service if any (e.g. SlipOK, EasySlip), the receiving account/PromptPay id, and who handles refunds. Never guess a provider's API — read its current docs and save a note.

## 1. Pick the flow
| Flow | Good for | Confirmation |
|---|---|---|
| Gateway (cards, PromptPay, mobile banking) | anything with volume, refunds, cards | the gateway's **webhook** (signed) |
| PromptPay QR + slip upload + slip-check API | small shops, events, low fees | slip check result + your own rules (§3) |
| Manual transfer + admin approves | very small volume | a person, with an audit trail |

Card numbers never touch your server: hosted page or the gateway's fields only.

## 2. Orders and states
- Server decides the amount from the order (never trust an amount from the client); store money as integer satang (or `DECIMAL`), never float.
- States: `pending` → `paid` → (`refunded` / `partially_refunded`), or `pending` → `expired` / `cancelled`. Transitions in one place, each with a timestamp and who/what caused it.
- Reservations (seat, booth, stock) hold for a few minutes with `expires_at`; a job expires unpaid ones and releases the stock; paying after expiry → refund or manual review, never a silent double sale (`scale-traffic` §4).

## 3. PromptPay QR and slips
- Generate a dynamic QR with the exact amount (EMVCo PromptPay payload; use a maintained library) per order; show the amount, the receiver name and the order number next to it.
- A slip is accepted only if, from the slip-check service (not from OCR you wrote): the receiver account matches yours, the amount equals the order's, the transfer time is after the order was created, and the slip's transaction reference has **never been used before** (unique constraint on it). Store the slip image privately (not a public URL) and the check result.
- Anything that fails a rule goes to manual review with the reason shown; tell the customer what happens next.

## 4. Gateways and webhooks
- Verify the webhook signature; respond 2xx fast and process in a job; deduplicate by event id; fetch the charge from the gateway API before marking paid if the webhook is thin.
- Idempotency key on every create-charge/refund call (retries and double clicks must not charge twice).
- Test mode keys in development and staging; live keys only in production secrets (`deploy-ops`).

## 5. Refunds, receipts, reconciliation
- Refunds through the gateway API (or recorded transfer back for slip payments), linked to the original payment, with reason and who approved.
- Receipts/tax invoices: what the business needs (VAT registered or not) — ask; number them sequentially, never edit an issued one (cancel and reissue).
- **Daily reconciliation:** compare the gateway's (or bank's) report with paid orders; list mismatches for a person. Money bugs are found here first.

## 6. Before handing over
State the flow, how a payment is confirmed, the idempotency and duplicate-slip protection, expiry and refund paths, what is logged (no full account numbers in logs — `reliability`), and tests for: double webhook, duplicate slip, wrong amount, late payment after expiry (`testing-ci`).
