# Design: the companion stale threshold N in the overview (#105)

Plan: [../105.md](../105.md). Follow-up 3 of #81 (`docs/plans/81-bold-restyle/design.md` §9).

## 1. Where N lives today

"A companion that has not reported for N minutes (N from config, default 10)" is one rule in
AGENTS.md ("Things that must never be silent"). In the code it is three values that do not
know about each other:

| Where | Value | Read by |
| --- | --- | --- |
| `crates/rubato-protocol/src/limits.rs:14`, default `:28` | `Limits::companion_silent_secs = 600` | the server config |
| `crates/rubato-server/src/kernel/config.rs:148-149` | `RUBATO_COMPANION_SILENT_SECS` overrides it (documented at `docs/QUICKSTART.md:99`) | only the `Welcome` envelope (`crates/rubato-server/src/companion_link/ws.rs:224`); the companion does not read the field and nothing on the server compares against it |
| `observability/alerts/rubato.rules.yml:8` | literal `> 600`, and "over 10 minutes" in the summary at `:15` | `CompanionSilent`, tested at `observability/alerts/tests/rubato.test.yml:18,30` |
| `web/src/features/companions/Lanes.tsx:13` | `COMPANION_STALE_MS = 10 * 60_000` | `freshness()` at `:16`, the lane's bar at `:118-119`, `freshness.test.ts:9` |

So they **can drift, and today they already do in one direction**: a team that sets
`RUBATO_COMPANION_SILENT_SECS=300` changes nothing anyone sees. The alert still waits 10 minutes
and the bar still drains over 10 minutes. The issue's own criterion ("N set to 5 on the server …
shows stale in the web *and fires the alert*") cannot pass by touching the overview alone.

## 2. The change

The server's `Limits::companion_silent_secs` becomes the one N, and both readers take it from the
server:

1. **Overview** (`web-server`, additive). `Overview` gains `companion_silent_secs: u64`, filled
   from `app.config.limits` in `crates/rubato-server/src/overview/handler.rs:61`. The field
   has `#[serde(default = …)]` returning `Limits::default().companion_silent_secs`. The schema
   then does not list it as required, `protocol.ts` makes it optional, and a client reading an
   older server's overview falls back to 600 rather than failing.
2. **Web**. `Lanes` passes `(overview.companion_silent_secs ?? DEFAULT_COMPANION_SILENT_SECS) * 1000`
   to `freshness()`. `COMPANION_STALE_MS` is replaced by `DEFAULT_COMPANION_SILENT_SECS = 600`,
   which is only the fallback. `freshness()` keeps its signature.
3. **Alert** (Q2, default (a)). The server records a gauge `rubato.companion.silent_secs` (name
   added to `crates/rubato-protocol/src/otel.rs` and its list) with the configured N on every
   poll tick, next to `rubato.poll.ticks` (`crates/rubato-server/src/tasks.rs:395`). Recording
   it only once at start would rely on the exporter keeping a gauge's last value between
   collections. That holds for cumulative temporality but not for delta. The rule becomes:

   ```promql
   time() - max by (rubato_companion_id, rubato_user) (rubato_companion_heartbeat)
     > on() group_left() (max(rubato_companion_silent_secs) or vector(600))
   ```

   The `or vector(600)` keeps the alert firing at the AGENTS.md default when the threshold
   series is missing (an older server, or a dropped export). An alert that never fires would be
   silent, and silence is exactly what this alert exists to catch. The summary reads
   `has not reported for {{ $value | humanizeDuration }}` instead of a fixed "10 minutes". The
   `Companions` panel gains the threshold as a second series, `N`.
4. **Config floor**. `RUBATO_COMPANION_SILENT_SECS` below 60 is raised to 60 with a warning,
   following `clamp_poll_secs` (`crates/rubato-server/src/kernel/config.rs:254`, test at `:307`). With
   N = 0 the alert fires for every companion and every bar is empty, and at present a typo there
   is ignored without a word.

## 3. What stays as it is

- `connected` still draws a full bar (`Lanes.tsx:118`). While a websocket is open the browser's
  copy of `last_seen` only moves when the overview is fetched again, so draining it would show
  a healthy companion as fading. A companion that is connected but hung is what the alert
  catches.
- `SESSION_STALE_SECS` (`Lanes.tsx:10`) has the same drift against `Limits::session_stale_secs`.
  It is left alone here. Q1 (c) would fix both together, which is the reason to choose it.
- The companion ignores `companion_silent_secs` in `Welcome`, as before.

## 4. Sibling plans

#103, #104 and #106 add `Overview` fields on `web-server` in parallel. This change only adds one
field with a default, and removes and renames nothing. When branches meet, the conflicts are
textual, in `progress.rs` (`Overview`), `overview/handler.rs` (the struct literal),
`schema/protocol/overview.schema.json` and `web/src/protocol.ts`. Resolve them by keeping every
field, then regenerate with `UPDATE_SCHEMAS=1 cargo test -p rubato-protocol` and
`pnpm -C web gen:types`.

## 5. Caveats

- **Clock skew.** The bar compares the server's `last_seen` with the browser's clock. That was
  true before this change and is not made worse.
- **Several server instances.** `max(...)` assumes one server per team (ADR 0008). Two servers
  configured with different N would give the larger one, which errs towards later.
- **Grafana import.** The rule goes through Grafana's Prometheus conversion route
  (`observability/publish.ts`). `on() group_left()`, `or vector()` and `humanizeDuration` are
  plain PromQL and Go templates, but `publish.ts --verify-live` after the publish must list
  `rubato_companion_silent_secs` as stored.
