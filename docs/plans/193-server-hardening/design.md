# 193: server hardening, design

Plan: [../193.md](../193.md) · ADR 0066 (*proposed*) · Issue #193, split out of #170.

The server part of the #170 security pass. None of these is an open hole by itself. Each one is
cheap, and together they close the paths #170 lists. All the changes are in `rubato-server`. No
wire type or schema changes, so `schema/protocol/*.json` stays byte-identical.

## Where the code is today

- Routes: `crates/rubato-server/src/app.rs:37-104`. There is no layer except `TraceLayer` and the
  workspace check on `/api/w/{ws}` (`app.rs:77-79`). There is no CORS layer, so a cross-origin JSON
  POST is stopped by the preflight. A body-less POST (`map/reload`, `draft/discard`, `restore`,
  `propose`, `withdraw`) or a `text/plain` POST needs no preflight and arrives carrying the
  `SameSite=Lax` cookie when the page is same-site.
- `/ws/web` (`web_link/ws.rs:20`) checks the session cookie only. `/ws/companion`
  (`companion_link/ws.rs:24-44`) checks the bearer token only and takes axum's default
  `WebSocketConfig` (64 MiB message, 16 MiB frame).
- Every companion envelope is stored whole in `companion_events` (`companion_link/ws.rs:173-176`,
  `store.rs:27`), and a `Sessions` snapshot of any length is stored (`ws.rs:241-251`).
- The companion's hook receiver takes a body of up to 2 MiB, axum's `DefaultBodyLimit`
  (`crates/rubato-companion/src/hooks/server.rs:71-74`). It forwards the whole payload in
  `HookEvent.payload` (`crates/rubato-protocol/src/hook.rs:79-81,102`), so an honest envelope is
  at most about 2 MiB plus a few hundred bytes.
- Issue comments: `kernel/github/api.rs:588-616` reads every page with `all_pages`. The only caller
  is `github_poll::poll_comments` (`github_poll/poll.rs:316-418`), on the slower lane (ADR 0023).
  After its pass it drops the review and approval signals whose comment it did not see
  (`poll.rs:416`).
- The PR body of a map proposal: `map_draft/propose.rs:68-83` writes `o.op.summary()` raw into a
  table cell. Part and seam ids are checked by `valid_id` when they are added
  (`map_draft/apply.rs:6-14`). Ids that are already in the map file are not checked (#170, #155), and
  `UpdateNode`, `RemoveNode` and seam ends can name those.
- OAuth callback: `auth/handlers.rs:113-156` returns `format!("token exchange failed: {e}")`,
  `user lookup failed: {e}` and the decode error text to the browser. It returns the database
  error text on 500 (`handlers.rs:119,208,228`).
- Secure cookie: `public_url.starts_with("https")` at `auth/handlers.rs:83,231,268`. Nothing at
  start-up says when that is false off loopback.

## Decisions

### D1. One same-origin rule for unsafe methods and websocket upgrades

`kernel::http::same_origin` is a `route_layer` on the whole API router (`app.rs`), so it applies to
every matched route, map-scoped ones included, and knows the route template for the metric. It
classifies with a pure function, `cross_origin(method, headers, public_url) -> bool`:

1. `GET`, `HEAD` and `OPTIONS` without `Upgrade: websocket` are not checked.
2. `Sec-Fetch-Site: same-origin` or `none`: allowed. Any other value is refused.
3. No `Sec-Fetch-Site` and no `Origin`: allowed. This is not a browser. Browsers send `Origin` on
   every non-GET request and on websocket upgrades.
4. `Origin` equal (ignoring ASCII case) to the origin of `public_url`, or to `http://<Host>` or
   `https://<Host>`: allowed. Anything else, `null` included, is refused.

This is the rule Go 1.25 ships as `CrossOriginProtection`. Rule 2 comes first because Vite's dev
proxy (`web/vite.config.ts:9-13`) keeps the browser's `Origin` of `localhost:5173` while the
request reaches the server. Fetch metadata still says `same-origin`. Node's `fetch` sends
`Sec-Fetch-Mode` but no `Sec-Fetch-Site` and no `Origin`, so the e2e helpers
(`web/e2e/map.spec.ts:16-19,139`) pass by rule 3. Playwright's `page.request` passes by rule 3 or 4.

A refusal is `403` with `{"reason":"cross_origin"}`, `app.reject(route, "cross_origin", "-")`
(`rubato.web.rejected`, info line `web write rejected`), and a `warn!("cross-origin request
refused")` that carries the route, the method, `Origin` and `Sec-Fetch-Site`. The user is `-`
because the check runs before authentication. That is on purpose: a refused request never touches
the session.

The companion websocket goes through the same check. tokio-tungstenite sends no `Origin`, so the
companion passes by rule 3 and then meets its bearer check as today.

### D2. Security headers on every response

A `map_response` layer on the whole router, the static fallback included, adds:

- `X-Content-Type-Options: nosniff`
- `Content-Security-Policy: default-src 'self'; connect-src 'self' <ws>; font-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`

`<ws>` is `ws://host[:port]` for an `http` public URL and `wss://…` for `https`. `'self'` covers
same-origin websockets in current browsers. The explicit origin covers older WebKit. What the SPA
loads was checked against `web/dist` from `pnpm build` on this branch:

| Loaded | From | Covered by |
| --- | --- | --- |
| `/assets/index-*.js`, module script, no inline script | self | `default-src 'self'` |
| `/assets/index-*.css`, no inline `<style>` | self | `default-src 'self'` |
| Space Grotesk and JetBrains Mono woff2 (ADR 0036) | self, and one subset inlined as `data:font/woff2` (Vite's 4 KB inline limit) | `font-src 'self' data:` |
| `fetch` to `/api/...`, websocket `/ws/web` (`web/src/features/live/subscribe.ts:21`) | self | `connect-src` |
| React `style={…}` props | CSSOM, not markup | not governed by `style-src` |
| ELK (`elkjs` bundled, no `workerUrl`) | in-page fake worker, no `eval` | — |
| Links to github.com (`features/map/explain.ts:99`) | navigation | not governed |

The plan's checks include a headless Chromium load of the built app against the server, and it
fails on any `securitypolicyviolation` event.

### D3. The Secure cookie and an http public URL

The rule stays: the cookie is `Secure` when `RUBATO_PUBLIC_URL` starts with `https`. At start-up,
`Config::from_env` adds one warning when the public URL is `http` and its host is not loopback
(`localhost`, `127.0.0.0/8`, `::1`): "RUBATO_PUBLIC_URL is http on a non-loopback host; the session
cookie is sent without Secure". It goes out through the existing `config.warnings` loop
(`main.rs:16-17`). It warns and does not refuse, because #184 (ADR 0064, proposed) already refuses
the open login modes off loopback. A refusal here would also stop an App deployment on a plain-http
LAN host. Q1 asks whether to refuse instead.

To keep away from #184's `is_local` in the same file (`config.rs:57-121` on its branch), the check
is a separate function at the end of `config.rs` with a one-line call next to the
`RUBATO_SESSION_SECRET` warning. Once both land, `is_local` can take it over.

### D4. Companion message size

`/ws/companion` sets `max_message_size` and `max_frame_size` to `COMPANION_MAX_MESSAGE_BYTES`,
4 MiB. That is twice the largest honest envelope (see above), so no honest companion meets the cap
and gets stuck replaying an outbox entry the server can never read. A larger message makes
tungstenite fail the read with `Space limit exceeded: Message too long`. The loop then logs
`warn!("companion socket read failed")` with the error. When the error is a capacity error it also
counts `rubato.events.dropped{reason="too_large"}`. A plain reset is logged and not counted,
because it is not a dropped event. A test pins the error text, so a tungstenite upgrade that changes
it fails the test instead of silently dropping the count.

`/ws/web` is left alone. A browser sends nothing the server reads, and #193 does not ask for it.

### D5. Sessions per snapshot

`MAX_SESSIONS_PER_SNAPSHOT` is 256. A `Sessions` envelope with more is acknowledged and not stored.
Spend is not recorded for it, and the previous snapshot stands. It is counted as
`rubato.events.dropped{reason="too_many_sessions"}`, with `warn!("sessions snapshot over the cap")`
naming the companion and the count. Dropping the snapshot rather than storing the first 256 keeps
the stored snapshot true: a truncated one would make sessions vanish from a lane while they still
run. The envelope itself is already in `companion_events`, bounded by D4. A1 records this.

### D6. Bounded issue-comment reads

`issue_comments` reads at most `MAX_COMMENT_PAGES` pages of 100, which is 5 pages and 500
comments:

- Page 1 is read. When `number_of_pages()` is 5 or fewer, the rest is read as today.
- Otherwise pages `last-4 ..= last` are read (the newest 500, since GitHub lists issue comments
  oldest first), and page 1 is thrown away. `warn!("issue comments cut")` logs the repo, the issue,
  the page count and the pages read.

`rubato.github.calls{endpoint=comments}` counts every page fetched, page 1 included, so the panel
shows the real cost. A cut read stays a complete read of the newest window: reviews and approvals
older than the window fall out of the view through `drop_signals_except`, while stored `Decision:`
records are never dropped by this pass. The newest approval, the one that counts (ADR 0027), is
always in the window.

`since=` was considered and rejected. It filters by `updated_at`, so the poll would need a cursor
per issue. It would also no longer see that a comment was deleted, and `drop_signals_except` relies
on that. That is more state than a cap.

### D7. Escaped op summaries in the proposal body

Each change and author cell becomes a code span: `` `…` `` with any backtick replaced by `'`, `|`
escaped as `\|` (GFM splits table cells before inline parsing), and CR and LF replaced by spaces.
Inside a code span GitHub renders no mention, link, image or HTML. The author column already uses
backticks. It goes through the same function. The summaries themselves (`MapOp::summary`, in the
protocol crate) do not change.

### D8. The OAuth callback answers with fixed text

Each failure in the callback answers with fixed text. The upstream or database error goes to the
log:

| Step | Status | Browser sees | Logged / counted |
| --- | --- | --- | --- |
| HTTP client build | 500 | `Login failed on the server.` | `internal("auth", e)` |
| token exchange | 502 | `GitHub did not complete the login. Try again.` | `error!("login failed", step="token_exchange", error)`; `rubato.web.rejected{reason=token_exchange_failed}` |
| `/user` lookup or decode | 502 | same | `step="user_lookup"`; `reason=user_lookup_failed` |
| user upsert, session create | 500 | `Login failed on the server.` | `internal("auth", e)` |

`oauth state mismatch`, `not_on_team` and `installation_lookup_failed` are unchanged. Their text is
already ours.

## Conflicts with open pull requests

- #184 (`kernel/config.rs`, `auth/handlers.rs`): D3 adds one call and one function at the end of
  `config.rs`. D8 edits callback lines that #184 does not touch (it edits `current_user.rs` and
  the team check). Small textual overlap is possible in `handlers.rs`.
- #180 and #185 (`companion_link/ws.rs`): D4 adds one statement at the top of `upgrade` and
  changes the `let Some(Ok(frame)) = inbound` line in `handle`. D5 adds a guard at the top of the
  `Sessions` arm, where #185 inserts `own_sessions`. Whichever lands second rebases. The hunks are
  a few lines each.
- New tests go in a new file, `tests/hardening.rs`, not in `tests/integration.rs`, which all three
  PRs extend.

## Why one pull request

The issue allows two (headers and Origin, then limits). They are kept together because every part
is small, each has its own test, and they share one ADR and one review. Splitting would not avoid
the `ws.rs` overlap with #180 and #185, because D1 does not touch `ws.rs` anyway.
