---
name: line-integration
description: Building on LINE — LINE Login, LIFF apps inside LINE, the Messaging API (webhooks, reply vs push, quotas), Flex Messages, rich menus, linking LINE users to your accounts. Load whenever a feature logs in with LINE, opens inside LINE, sends LINE messages or notifications, or uses a LINE Official Account ("LINE OA", "LIFF", "แจ้งเตือนทางไลน์"). External-call safety lives in reliability; secrets in deploy-ops.
area: code
---
# LINE integration

## 0. Context — ask once, save as facts
`project_knowledge` fact **`line`**: the provider, which channels exist (LINE Login, Messaging API), the LIFF id(s), whether one Official Account serves several products, and the OA's plan (it sets the monthly message quota). LINE's APIs and quotas change — read the current docs (developers.line.biz) and save a note under topic `line`.

## 1. One provider, matching user ids
A LINE `userId` is per **provider**: the Login channel and the Messaging API channel must sit under the same provider, or the ids won't match and you can't message the person who logged in. Decide this before anything ships; moving channels later is painful.

## 2. LINE Login and LIFF
- LINE Login is OAuth 2.0 / OpenID Connect: `state` against CSRF, `nonce`, verify the ID token on the server (signature, `aud` = channel id, `exp`, nonce) before trusting who it is.
- LIFF: `liff.init({ liffId })` first; get `liff.getIDToken()` and send it to your server to verify — never trust `liff.getProfile()` or a userId sent from the browser on its own.
- Ask to add the OA as a friend during login (`bot_prompt`), because push messages only reach friends.
- LIFF runs in LINE's in-app browser and in external browsers: test both; `liff.isInClient()` for features that only work inside LINE (sending messages to the chat, closing the window).
- Link LINE to your own account table (`line_user_id` unique); unlinking must be possible.

## 3. Messaging API
- **Webhook:** verify `x-line-signature` (HMAC-SHA256 of the raw body with the channel secret) before parsing; answer 200 quickly and handle events in a job; events can repeat — deduplicate by `webhookEventId`.
- **Reply vs push:** a reply token works once and only for a short time after the event — use it when answering a user's message; push (or multicast/broadcast) costs quota on most plans. Count messages per month and warn before a feature could exceed the plan.
- Notifications that matter (payment received, booth approved) also show in the product itself or by email — not everyone has added the OA or keeps notifications on.
- Retries on 429/5xx with backoff, timeouts, and a queue for bulk sends (`reliability`).

## 4. Messages that look good
- Flex Messages for cards (event ticket, booking summary): design them like a small screen, keep text readable on phones, test in the Flex Message Simulator; always set `altText` (shown in notifications).
- Rich menu: one image at the sizes LINE allows (e.g. 2500×1686 or 2500×843), big tap areas, clear labels; per-user menus (linked vs not linked) via rich menu aliases/links.
- Thai copy short and polite; include what happens next and a link back to the page.

## 5. Secrets and environments
Channel secret and access tokens only in server secrets (`deploy-ops`), never in the LIFF/front-end bundle; separate channels (or at least LIFF apps) for staging and production so tests never message real users.

## 6. Before handing over
State the provider/channel setup and how user ids match, how ID tokens are verified, which events the webhook handles (with signature check and dedupe), which messages are push vs reply and the expected monthly count against the quota, and what the user still has to set in the LINE Developers console (callback URLs, LIFF endpoint, webhook URL) — they own that account.
