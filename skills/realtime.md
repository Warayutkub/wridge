---
name: realtime
description: Live updates — choosing polling, long polling, Server-Sent Events or WebSockets (or a managed service), auth per connection and channel, heartbeats and reconnects, presence, and running it on several servers (pub/sub, sticky sessions, load balancer timeouts). Load when a feature updates without refresh — queue position, live seat map, chat, notifications, dashboards, "เรียลไทม์". Capacity planning lives in scale-traffic.
area: code
---
# Realtime

## 0. Context
`project_knowledge` fact `deploy` (serverless platforms such as Vercel functions can't hold long WebSocket connections) and whether Redis or a managed realtime service (Pusher, Ably, Supabase Realtime, Firebase) is available. Ask once if missing.

## 1. Pick the simplest that works
| Need | Use |
|---|---|
| Updates every 10 s+ is fine | Polling (with `ETag`/`If-None-Match`, back off when the tab is hidden) |
| Server → browser, near-instant | **SSE** (`EventSource`): plain HTTP, auto-reconnect, `Last-Event-ID` to resume; mind the 6-connections-per-domain limit on HTTP/1.1 (fine on HTTP/2) |
| Both directions, many messages (chat, collaborative edits, games) | **WebSocket** |
| Serverless host, or no time to run it | A managed service; keep your server as the source of truth |
Long polling is a fallback when SSE/WebSocket is blocked; don't build both unless needed.

## 2. Security per connection
Authenticate at connect (short-lived token, not a cookie the page can't refresh), re-check when the token expires; authorize every channel/room join on the server (a user can only subscribe to their own booking, their organization's dashboard); validate and rate-limit messages from clients like any API input (`backend-security`).

## 3. Connections that survive the real world
- Heartbeat (ping every ~25–30 s) — load balancers and proxies drop idle connections (often at 60 s).
- Reconnect with exponential backoff **and jitter**, so a server restart doesn't bring every client back at the same second.
- Resume, don't reset: an event id/sequence per channel; on reconnect send what was missed, or tell the client to refetch.
- Messages can arrive twice or out of order: idempotent handlers, ordering by sequence where it matters.
- Phones sleep and change networks: on visibility/online events, reconnect and refetch the current state.

## 4. More than one server
- Each server only knows its own connections: publish events through Redis pub/sub, NATS or the database's notify, and every server forwards to its clients.
- Sticky sessions are needed for Socket.IO's polling fallback (or force WebSocket transport); set load-balancer idle timeouts above the heartbeat.
- Each connection holds memory and a file descriptor: count connections per instance in capacity plans (`scale-traffic`); drain connections gracefully on deploy (tell clients to reconnect elsewhere).
- Presence ("12 people viewing"): heartbeats with a TTL in Redis, not in-process sets.

## 5. Testing
Test reconnect (kill the server mid-session), duplicate and late messages, a token expiring while connected, two servers behind a balancer. Load test connections with k6's WebSocket module when numbers matter (`scale-traffic` §6, with its limits told to the user).

## 6. Before handing over
State the transport and why, how connections authenticate and which channels exist with their rules, heartbeat/reconnect/resume behaviour, how it runs on several servers, and the expected connection count.
