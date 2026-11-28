# 0090. Rubato may face the internet: TLS, required secrets and rate limits off loopback

- **Status:** proposed
- **Date:** 2026-11-27
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #345), drafting; that the demo runs on the public internet is Mattias Jonsson's choice, 2026-11-27
- **Seam:** web-server, companion-server, server-github
- **Issue:** m0x6A/rubato#345

## Context

The server has only run on a homelab LAN over plain HTTP (ADR 0067, #276). ADR 0074 rejected
making the whole server public and exposed one signed path. A demo for several tenants (ADR 0087)
puts every route on the internet: login, pairing and webhooks are reachable by anyone, and the
other routes by any invited person. Design: `docs/plans/345-tenants/design.md` §7.

## Decision

Off loopback the server refuses to start when `RUBATO_PUBLIC_URL` is not https, unless
`RUBATO_ALLOW_HTTP=1` (a LAN, as today); when it is not set, `RUBATO_SESSION_SECRET` (at least 32
bytes), `RUBATO_TOKEN_KEY`, the GitHub App and `RUBATO_OPERATORS` are also required. TLS ends at
a reverse proxy in the stack (Caddy with ACME in `deploy/compose.yaml`); client addresses come
from `X-Forwarded-For` only when the request arrives from `RUBATO_TRUSTED_PROXY`. Over https the
server sends `Strict-Transport-Security` and names its session cookie `__Host-rubato_session`
with `Secure`. Every request has a 1 MiB body limit unless its route sets another, and a 30 s
timeout unless it is a websocket or chains GitHub writes (map draft propose, approve, merge: 120
s); the server bounds concurrent requests. Metrics label a route by its template, never by the
concrete path. Anonymous paths
(`/auth/*`, `/api/pair/start`, `/api/pair/poll`) have per-address token buckets, pending pairing
codes are capped per address as well as overall, and writes and pairing approvals have per-person
buckets; a refusal is 429 with `Retry-After`. GitHub webhooks are on, with the installation check
of ADR 0087.

## Alternatives considered

- **Stay on the LAN and tunnel only webhooks** (ADR 0074): no demo for anyone outside it.
- **An identity-aware proxy in front of everything**: hides the GitHub login the demo shows, and
  a dependency per deployment.
- **Rate limiting only at the proxy**: the proxy cannot tell a person from an address, and the
  limits would differ per deployment.

## Consequences

The LAN deployment sets `RUBATO_ALLOW_HTTP=1` until #276 lands there too. The stack gains a Caddy
service and ports 80 and 443. Companions pair over TLS without `--insecure` (ADR 0067's default
path). Covered by the `web-server` O-row (`rubato.web.rejected{reason=rate_limited}`, the
`RateLimited` alert) and the start-up refusal log line `refusing to start: <reason>`.

## Supersedes

In part 0067 (plain HTTP accepted off loopback without an explicit opt-in); records that 0074's
rejected alternative "make the whole server public" is taken, with these limits.
