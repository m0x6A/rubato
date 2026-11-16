# 0066. Refuse cross-origin browser writes, send a CSP, and cap what one companion message may carry

- **Status:** accepted; superseded in part by 0073
- **Date:** 2026-11-15
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/193#issuecomment-6892514528 (self); drafted by agent (Claude Code, issue #193 design session)
- **Seam:** web-server, companion-server
- **Issue:** m0x6A/rubato#193

## Context

The security pass in #170 found that the server's only CSRF defence is the session cookie's
`SameSite=Lax`, that it sends no security headers, and that the companion websocket accepts
tungstenite's 64 MiB default message and stores every envelope whole. A same-site page can
trigger the body-less POSTs (map reload, draft discard, restore, propose, withdraw), and one
companion can fill the database with a handful of messages. Plan `docs/plans/193.md`, design
`docs/plans/193-server-hardening/design.md`.

## Decision

**Same-origin writes.** Every request that is not `GET`, `HEAD` or `OPTIONS`, and every websocket
upgrade (`/ws/web`, `/ws/companion`), passes one check before its handler. It is allowed when
`Sec-Fetch-Site` is `same-origin` or `none`. When `Sec-Fetch-Site` has any other value
(`same-site`, `cross-site`) it is refused. Without `Sec-Fetch-Site`, a request with no `Origin` is
allowed (it is not from a browser: the companion, `curl`, the test suites). A request whose
`Origin` equals the origin of `RUBATO_PUBLIC_URL` or `<scheme>://<Host>` of the request is allowed,
and any other `Origin`, `null` included, is refused. A refusal answers
`403 {"reason":"cross_origin"}` and is counted as
`rubato.web.rejected{rubato.route, rubato.reason="cross_origin"}`. The companion keeps its bearer
token. The origin check is only for browsers, and the companion sends no `Origin`.

**Headers.** Every response carries `X-Content-Type-Options: nosniff` and
`Content-Security-Policy: default-src 'self'; connect-src 'self' <ws(s) origin of the public URL>;
font-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`. `data:` fonts
are there because the build inlines font subsets under 4 KB. Nothing else is allowed from
anywhere else, which matches ADR 0036 (fonts served from the app's own origin).

**Companion caps.** The server reads a companion message of at most 4 MiB (`max_message_size`
and `max_frame_size`). That is twice the companion's 2 MiB hook body limit, so an honest companion
never reaches it. A larger message ends the socket and is counted as
`rubato.events.dropped{rubato.reason="too_large"}`. A `Sessions` snapshot of more than 256 sessions
is acknowledged and not stored, so the previous snapshot stands. It is counted as
`rubato.events.dropped{rubato.reason="too_many_sessions"}`. Both numbers are server constants and
are not in `Welcome.limits`.

## Alternatives considered

- **A CSRF token per session**: a second secret, a header on every fetch and a change in every
  caller. Fetch metadata and `Origin` give the same answer for browsers that send them, which is
  every browser the team uses.
- **Refuse a write that has no `Origin`**: it would break the companion's pairing POSTs and every
  `curl`. Browsers send `Origin` on all non-GET requests and on websocket upgrades.
- **`SameSite=Strict`**: it would not stop a same-site page, and the login redirect from github.com
  would arrive without the cookie.
- **Caps in `Welcome.limits`**: lets the companion respect them before sending. That is a protocol
  schema change, left for when a companion can actually reach them (plan 193, Q3).

## Consequences

A browser page on another origin, including a sibling subdomain, can no longer write or open a
websocket. A reverse proxy that rewrites `Host` must also set `RUBATO_PUBLIC_URL` to the origin the
browser sees, or browsers without fetch metadata are refused. The `web-server` O-row covers this:
`rubato.web.rejected{reason=cross_origin}` and the `cross-origin request refused` warning, on the
"Web writes rejected" panel. A future page that loads a script, style or font from another origin,
or embeds Rubato in a frame, needs this ADR superseded. The `companion-server` O-row covers the
caps: `rubato.events.dropped{reason=too_large|too_many_sessions}` on "Events dropped".

## Supersedes

None.
