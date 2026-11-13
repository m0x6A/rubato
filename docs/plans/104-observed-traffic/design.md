# Design: observed traffic on seams, drawn on the map (#104)

Plan: [../104.md](../104.md) · Map: `rubato-protocol`, `rubato-server`, `web` over seam
`web-server` · Track: **full**, because new wire types and two findings change
`schema/protocol/overview.schema.json` and `finding.schema.json`, and a new data source is
needed whose seam the map does not have yet.

Issue #104 asks for two things: the server summarises observed calls per seam and sends the
summary in the overview; the map's conduits carry particles that mean traffic, and the
map-versus-territory findings (a declared seam with no traffic, traffic with no seam) are drawn on
them. It is follow-up 2 of #81 (design §9 there).

One premise in the issue does not hold: the server does **not** already receive production
telemetry. Claude Code, the companion and the server export OTLP to the team's backend (Grafana
Cloud today); nothing flows back into Rubato, and AGENTS.md says nothing in Rubato may depend on
that stack. So this note splits the work in two:

- **What the server does with a summary, and everything on the web side** (§2–§5, §7): fully
  specified, built against one input shape (`TrafficReport`) that any source can produce, and
  testable today with a development file.
- **Where the summary comes from** (§6): designed as options for a person to choose (plan Q1),
  because each option adds an external part and a seam to the map, and a person adds those.

## Where it sits on the map

- `rubato-protocol` (contract): new module `traffic` with `TrafficReport`, `ObservedTraffic`,
  `SeamTraffic`, `UndeclaredEdge`, `TrafficState`, the pure `derive_traffic`; two new `Finding`
  variants; `Overview.traffic`; new names in `otel.rs`.
- `rubato-server`: a new slice `traffic` (source port, the development-file source, the latest
  report in memory, telemetry); the overview build calls `derive_traffic`; gauges.
- `web`: a **Traffic** lens; chevron particles; quiet and undeclared drawn on conduits; findings
  text; inspector rows.
- Seam `web-server` carries the new data. No other seam changes in PR 1–2. The real source (PR 3)
  needs a new external part and seam, named in Q1 and added by a person through a map draft.

`system-map` is read as today; its schema does not change unless Q2 is answered (b).

## 1. What v0 shows, and what it does not

AGENTS.md: "When the two disagree (a service calls a node the map does not connect it to, or a
declared seam carries no traffic), that is a finding, shown on the map, not an error to suppress.
v0 only shows it; nothing acts on it." So:

- Traffic is shown, never acted on: no alert fires on a quiet seam, no plan is blocked, no
  progress state reads it.
- A finding is only as good as the signal behind it. The trust check comes first (AGENTS.md
  progress model): **a missing, failing or stale report renders as *unknown*, never as quiet.**
  This is the traffic version of "a session with no heartbeat renders as unknown, never as
  working".

## 2. The input: `TrafficReport`

One shape, whatever the source:

```text
TrafficReport {
  observed_at: DateTime<Utc>,      // when the source's window ended
  window_secs: u32,                // how far back the counts reach
  edges: [ { client: String,       // OTel service.name of the caller
             server: String,       // OTel service.name of the callee
             calls: u64,           // requests in the window
             errors: u64 } ]       // of which failed
}
```

It is the same shape as the OpenTelemetry Collector's `servicegraph` connector output
(`traces_service_graph_request_total` and `…_failed_total`, labelled `client` and `server`), which
is in the Collector's contrib distribution and vendor-neutral, so every Q1 option can fill it
without translation. It lives in `rubato-protocol` and gets a schema file
(`schema/protocol/traffic_report.schema.json`) because under Q1 (b) it becomes a wire type.

Limits: at most 2 000 edges per report (more are cut and `truncated` is set); service names at
most 200 bytes; `window_secs` between 60 and 86 400. A report outside the limits is rejected
whole, with a reason (§8). Service names come from outside the team (any instrumented
service can choose its own); they are shown only as text (React escapes them), never used as ids,
class names or URLs, and never become map parts.

## 3. From a report to the overview: `derive_traffic`

A pure function in `rubato-protocol::traffic`, so the server and its tests share it:

```text
derive_traffic(map: &SystemMap, report: Option<&TrafficReport>, source: SourceStatus, now) -> (ObservedTraffic, Vec<Finding>)
```

1. **State.** `not_configured` when no source is configured; `failing` when the last fetch
   failed (with its error); `stale` when `now - observed_at > 3 × fetch interval`;
   otherwise `fresh`. Only `fresh` goes on to steps 2–5; every other state returns no seams, no
   undeclared edges and no findings, plus the state, the last good `observed_at` and the error.
2. **Service to part.** A service maps to the map part whose id equals its `service.name`
   (Q2 (a)). Services that map to no part are counted in `unmapped_services` and listed (at most
   20 names) for the inspector; they never make a finding, because the map does not have to
   name every service.
3. **Edge to seam.** An edge between two mapped parts adds its calls and errors to every seam
   that joins the two parts, as `forward` (client is the seam's `from`) or `reverse`. A self-call
   (client and server the same part) is ignored. An edge that no seam joins goes to
   `undeclared` and makes `Finding::UndeclaredTraffic { from, to, calls }`.
4. **Observed parts.** A part is *observed* when it is the client or server of any mapped edge in
   the report.
5. **Quiet.** A declared seam with zero calls in both directions whose two parts are both
   observed makes `Finding::SeamQuiet { seam, window_secs }`. A seam to a part that emits
   nothing (a contract, a file, an external part without instrumentation) is never quiet; it is
   simply not drawn as traffic.

Output on the wire:

```text
ObservedTraffic {
  state: not_configured | fresh | stale | failing,
  observed_at: Option<DateTime>,   // the last good report
  window_secs: Option<u32>,
  error: Option<String>,           // the last fetch error, when failing
  seams: [ SeamTraffic { seam, forward_calls, reverse_calls, errors } ],   // only seams with calls
  undeclared: [ UndeclaredEdge { from, to, calls, errors } ],
  observed_parts: [String],
  unmapped_services: u32,
  unmapped_sample: [String],       // at most 20
  truncated: bool,
}
```

`Overview.traffic: ObservedTraffic`, `#[serde(default)]` (default `not_configured`), so an older
server or a missing field reads as not configured. The findings join `Overview.findings` like the
others. Traffic is never an input to `derive_progress`, pins, plans, fit or approvals (ADR 0039).

## 4. Server: slice `traffic`

`crates/rubato-server/src/traffic/` (new slice, owns no table):

- `TrafficSource` port in the slice: `async fn fetch(&self) -> Result<TrafficReport, SourceError>`.
  PR 1 has one implementation, `DevFileSource`, reading `RUBATO_DEV_TRAFFIC` (a JSON
  `TrafficReport`; `observed_at` may be left out and is then the file's mtime), the same
  pattern as `RUBATO_DEV_ISSUES`. PR 3 adds the chosen real source.
- A task in `tasks.rs`, best-effort, ticking every `RUBATO_TRAFFIC_FETCH_SECS` (default 60,
  clamped 15–300). Each tick fetches, validates against the limits, swaps the latest report in a
  `tokio::sync::RwLock<Option<(TrafficReport, SourceStatus)>>` in the slice's state (held through
  `AppState`), and calls `app.notify("traffic")` (`kernel/state.rs:65`) only when the derived summary changed
  (compared as the derived `ObservedTraffic` minus `observed_at`), so a steady map does not make
  every browser refetch every minute.
- The overview build reads the latest report and calls `derive_traffic`. No SQLite table, no
  migration: a restart is *unknown* until the first fetch, which is the honest answer.
- No source configured: the task does not start, the state is `not_configured`, the lens says so.

Telemetry (names added to `crates/rubato-protocol/src/otel.rs`):

| Name | Kind | When |
| --- | --- | --- |
| `rubato.traffic.fetch` | span, `info` | each fetch; `rubato.outcome` (`ok`, `rejected`, `failed`), edges, error status |
| `rubato.traffic.fetches` | counter | each fetch, by `rubato.outcome` |
| `rubato.traffic.report_age` | gauge, seconds | at each gauge refresh: `now - observed_at` of the last good report; absent when not configured |
| `rubato.traffic.seams_quiet` | gauge | quiet seams in the last derivation |
| `rubato.traffic.undeclared_edges` | gauge | undeclared edges in the last derivation |
| `rubato.traffic.unmapped_services` | gauge | services that map to no part |
| log `traffic report rejected` | `warn` | with the reason (`too_many_edges`, `bad_window`, `name_too_long`, `parse`) |
| log `traffic source failing` | `warn` | on the change from ok to failing, with the error; `info` `traffic source recovered` on the way back |

`rubato.traffic.report_age` gets an alert `TrafficReportStale` (report older than 10 minutes
while a source is configured) with a test that fires it, and the dashboard gets a row **Map versus
territory** with three panels: report age, quiet seams and undeclared edges, fetches by outcome.
This is the trust check for the view: if the report stops, the lens turns *unknown* and the alert
says why.

## 5. Web: the Traffic lens

### 5.1 Where traffic is drawn

A fourth switchable lens, **Traffic**, beside Structure, Data flow and Control flow. In it, each
declared seam is drawn once (its structure arrow, solid conduit), and what moves on it is
observed calls only. Declared-flow particles (dots, rings) are not drawn in this lens, and
traffic chevrons are drawn in no other lens. So the two meanings never share a conduit at the
same time: *declared, this way* (dots and rings in Data and Control) and *observed, this much*
(chevrons in Traffic). The lens is offered whatever the state; when the state is not `fresh` it
says why (§5.5).

### 5.2 The particle: a chevron

Traffic must be told apart from declared flows **by shape, not colour only** (#81 rule). The
vocabulary becomes:

| Mark | Shape | Means | Lens |
| --- | --- | --- | --- |
| dot | filled circle | declared data flow | Data flow, scenario |
| ring | hollow circle | declared control flow | Control flow, scenario |
| **chevron** | a filled `›`, 9 × 7 px, pointing the way the calls go | **observed calls** | Traffic |
| marching dashes | violet `6 6`, moving | a flow a plan proposes | plan |

The chevron is a `div.traffic` with `clip-path: polygon(0 0, 55% 50%, 0 100%, 40% 100%, 95% 50%,
40% 0)`, `offset-path` = the edge path, `offset-rotate: auto` (and `auto 180deg` for calls
against the seam's direction, since the animation runs reversed), coloured with the green role
(`--ok`), so it also reads as "live" in colour. Its point carries the direction; a circle has
none, so a chevron and a dot cannot be confused even in greyscale.

### 5.3 Density means volume; speed does not

Speed stays at 64 px/s (`PARTICLE_SPEED`), the same as declared particles, so motion still only
says direction. Volume is how many chevrons travel, per direction, by calls per minute over the
report's window:

| Calls per minute | Chevrons per direction |
| --- | --- |
| 0 | none (and the seam may be quiet, §5.4) |
| above 0, below 1 | 1 |
| 1 to below 60 | 2 |
| 60 and up | 3 |

The exact rate is text: the seam chip gains `· 12/min` (and `· 3 % errors` when errors are
above zero) in the Traffic lens, and the inspector shows calls and errors each way with the window.
Errors are text only in v0; no red particle.

**The cap of 120 holds.** `particleBudget` takes a count per item instead of the fixed 2
(`{ id, near, count }`); declared flows pass 2, traffic seams pass their chevron count (up to 6
per seam, two directions). Past the cap, seams next to the selection or a lit part move first; the
rest show the still chevron. Only one lens is on at a time, so declared and traffic particles never
add up.

### 5.4 Findings drawn on conduits

- **Quiet seam** (`SeamQuiet`): the conduit is drawn hollow (core removed, rim only, `.quiet`
  class), with no chevrons, and the chip gains `· no calls in 1 h` and the ⚠ mark. Outside the
  Traffic lens the chip keeps the ⚠ mark (with a screen-reader text of its own), because a
  finding is shown on the map, not only in one lens. `graph.ts` routes `seam_quiet` to a new
  `edge.quiet` flag, not into `edge.findings`, so the chip takes the amber `quiet` style and not
  the magenta `collision` style that `findings` sets today (`SeamChip.tsx`).
- **Undeclared traffic** (`UndeclaredTraffic`): in the Traffic lens only, a *territory edge*
  between the two parts: amber (`--warn`), dotted `1 5` with round caps, **static**, with the
  chevrons of its calls on it and a label `observed, not on the map · N/min`. It is not a seam:
  it has no chip button, cannot be selected, pinned or dropped on, and has its own test id
  `territory-<from>-<to>`. It is distinct from every other line: the draft's violet `3 4` dashes,
  the proposed violet `6 6` marching dashes and the declared control `7 5` dashes are all longer
  dashes in another colour, and none of them carries chevrons.
- Both findings are listed in the Findings panel with their text (`findingText`):
  `no calls observed on <seam> in the last <window>, though <from> and <to> both show traffic`;
  `<from> calls <to> (<n> calls in the last <window>), but no seam on the map joins them`.

### 5.5 Unknown, not quiet

When `traffic.state` is not `fresh`, the Traffic lens draws every seam plain (no chevrons, no
hollow, no territory edges) and the legend reads, in order of state:

- `not_configured`: "Rubato has no traffic source. Observed traffic is not shown."
- `stale`: "Traffic unknown: the last report is from <time ago>." (the time from `observed_at`)
- `failing`: "Traffic unknown: the source failed: <error>."

and the quiet ⚠ marks are absent from every lens, because the server sends no such findings.

### 5.6 Legend in the Traffic lens

"`›` Observed calls move as chevrons, more of them for more calls. A hollow seam declares a
connection nobody used in the last <window>. A dotted amber line is traffic the map does not
declare." Plus `<n> services are not on the map` when `unmapped_services > 0`, as a `details`
listing the sample.

### 5.7 Reduced motion

All of it sits under `:root[data-motion="full"]`: the chevron's animation rule is written only
there. With reduced motion each direction with calls shows one still chevron at 30 % of the
path from its own start (70 % for reverse, as `STILL_AT` does), clear of the label in the middle;
the rate stays as text on the chip. The motion Vitest over `styles.css` (from #81) counts the new
rule and fails if it sits outside the switch; the reduced-motion e2e gains the Traffic lens.

## 6. Where the report comes from (Q1, design only)

Each option fills a `TrafficReport`. Each needs one external part and one seam on the map, which
a person adds through a map draft; this plan names none of them in its tables, because they do
not exist yet.

**(a) Query a Prometheus-compatible HTTP API.** Config `RUBATO_TRAFFIC_PROM_URL` and an optional
`RUBATO_TRAFFIC_PROM_TOKEN`. Every fetch runs one instant query,
`sum by (client, server) (increase(traces_service_graph_request_total[<window>]))`, and the same
for `…_failed_total`, against `/api/v1/query`, with a 10 s timeout. Prerequisite on the team's
side: an OTel Collector (or Tempo's metrics generator) with the `servicegraph` connector. The
Prometheus query API is served by Prometheus, Mimir, Thanos, VictoriaMetrics and Grafana Cloud,
so this depends on a protocol, not on LGTM; but it is an outbound read of production metrics
from the server, a new credential and a query language in Rubato. Map: external part
`telemetry-backend`, seam `server-telemetry` (owner `rubato-server`), data flow
telemetry-backend → rubato-server.

**(b) Accept pushed reports.** `POST /api/traffic` with a `TrafficReport` body and a bearer token
from config (`RUBATO_TRAFFIC_PUSH_TOKEN`, compared in constant time), 1 MiB limit, answered 202.
Anything that can count calls pushes: a Collector-side job, a cron script over any backend.
Rubato has no query language and no outbound call; the cost is a new inbound write route and a
pusher the team runs and watches. Map: external part `traffic-pusher`, seam
`traffic-pusher-server`, data flow traffic-pusher → rubato-server.

**(c) Count only what the server sees itself.** The server already counts its own calls: companion
envelopes received (`companion-server`), GitHub calls (`rubato.github.calls`, `server-github`), web
requests and websocket clients (`web-server`). A built-in source turns these into a report with
the server's own part ids. No new part, seam or dependency; but it only works on a map that
contains Rubato (this repo's own map), and says nothing about the team's services, which is what
the AGENTS.md section is about.

**(d) Defer the source.** Ship PR 1–2 with the development file only. In production the state is
`not_configured` and the lens says so. The source becomes its own issue, planned when the team
knows which backend it keeps.

Recommended: **(a)**, because the servicegraph connector already produces exactly the report's
shape, the query API is the most widely implemented read path for metrics, and nothing new has to
run on the team's side beyond a Collector setting. If Q1 is answered (a), (b) or (c), this plan is
edited to add PR 3 with its source, ADR 0040 and the map draft, and asks for approval again.

## 7. Neighbours and what must hold

- **Declared particles** (#81 M1, M3): dots in Data, rings in Control, pulse in scenario, the
  still marker under reduced motion. `particleMode` returns `none` for every declared arrow in the
  Traffic lens; the existing `particles.test.tsx` cases stay as they are.
- **`moving` means proposed**: territory edges and quiet seams never get `moving`; the
  proposed-stays-distinct Vitest gains the territory edge (no `moving`, no marching dash).
- **The cap of 120**: `particleBudget` with counts; the existing tests pass with count 2; a new
  test puts 30 traffic seams at 6 chevrons (180) under the cap.
- **The one motion switch**: the motion Vitest over `styles.css` counts the chevron rule.
- **Performance**: the perf e2e (`web/e2e/perf.spec.ts`) gains a Traffic-lens run on the same
  40/60 fixture with traffic on every seam (cap reached).
- **Slices**: the server gets slice `traffic` (AGENTS.md block updated); the web needs no new
  feature folder (the lens lives in `map`, the text in `findings`).
- **Additive wire**: `Overview.traffic` and the two `Finding` variants are additive; sibling plans
  #103, #105 and #106 also add Overview fields. Conflicts are textual (the `Overview` struct,
  `findingText`'s switch, `schema/protocol/overview.schema.json`); whoever merges second
  regenerates the schema and `protocol.ts`.
- **Findings on the chip**: today the chip's screen-reader text assumes every seam finding is
  `flow_off_seam` (`SeamChip.tsx`); it becomes one sentence per finding kind.

## 8. Failure

| Failure | What happens | Seen by |
| --- | --- | --- |
| The dev file or source is unreadable, times out or returns an error | state `failing`, nothing drawn as traffic, legend says why | `rubato.traffic.fetches{rubato.outcome=failed}`, log `traffic source failing`, `TrafficReportStale` after 10 min |
| A report breaks a limit | rejected whole; the previous report stays until it goes stale | `rubato.traffic.fetches{rubato.outcome=rejected}`, log `traffic report rejected` with the reason |
| The source stops updating but answers | `observed_at` stops moving; after 3 fetch intervals the state is `stale` | `rubato.traffic.report_age`, `TrafficReportStale` |
| Service names do not match part ids | no seams lit, no findings; `unmapped_services` high | gauge `rubato.traffic.unmapped_services`; the legend's count and sample |
| Many edges | at most 2 000 kept, `truncated` set, the legend says so | `truncated` in the overview |
| A burst of changes | the slice notifies only when the derived summary changes | the unit test "same summary, no notify" (PR 1) |

## 9. Concurrency

One writer (the traffic task) swaps the report under a write lock; the overview build takes a
read lock and clones. A fetch that times out leaves the previous report in place. No database
state, so no row can be left half-written; a restart forgets the report and shows *unknown* until
the next fetch.
