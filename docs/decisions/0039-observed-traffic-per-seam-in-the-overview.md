# 0039. Carry observed traffic per seam in the overview; judge the map only from a fresh report

- **Status:** proposed
- **Date:** 2026-11-13
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #104) — proposed; a human accepts
- **Seam:** web-server
- **Issue:** m0x6A/rubato#104

## Context

AGENTS.md ("Map versus territory") says that when production traffic and the declared map
disagree, that is a finding shown on the map, and that v0 only shows it. #81 drew particles on
declared flows that mean "declared, this way", not traffic, and left observed traffic as a
follow-up (design §9). Issue #104 asks the server to summarise observed calls per seam and the
web to draw them. The server receives no production telemetry today, and AGENTS.md forbids
depending on the team's LGTM stack, so where the calls come from is a separate decision (plan
`docs/plans/104.md`, question Q1). This ADR fixes what the server does with a summary once it
has one, and what crosses `web-server`, whatever the source turns out to be. Design:
`docs/plans/104-observed-traffic/design.md` §2–§4.

## Decision

Every source of observed traffic produces one shape, `TrafficReport` in `rubato-protocol`:
`observed_at`, `window_secs`, and edges `{client, server, calls, errors}` where `client` and
`server` are OpenTelemetry `service.name` values. The server keeps only the latest report, in
memory; nothing about traffic is written to SQLite. On every overview build a pure function in
`rubato-protocol`, `derive_traffic`, turns the report and the map into `Overview.traffic`
(`ObservedTraffic`: the source's state, the report's time and window, calls and errors per
declared seam in each direction, edges between two map parts that no seam joins, and the number
of services that map to no part) and two findings: `Finding::SeamQuiet { seam, window_secs }`, a
declared seam with no calls in the window whose two parts both show traffic elsewhere in the same
report, and `Finding::UndeclaredTraffic { from, to, calls }`, calls between two parts that no seam
joins. A report older than three fetch intervals, a failing source and an unconfigured source all
make the state `unknown`: then no seam is quiet, nothing is undeclared and nothing is drawn as
traffic. Silence in the data is never read as silence on the seam. Both fields are additive
(`#[serde(default)]`); an overview without them reads as "not configured". Traffic is never an
input to progress, pins, plans or approvals.

## Alternatives considered

- **Store each report in SQLite**: history would allow trends, but nothing in v0 reads history,
  and a stored report outlives the freshness it needs to be trusted. A later ADR can add it.
- **Mark a seam quiet whenever it has no calls**: a seam whose parts emit nothing at all (a
  contract, a file, an uninstrumented service) would be "quiet" for ever. Requiring traffic at
  both parts keeps the finding to seams that could have carried calls.
- **Send the raw report to the web and derive there**: the findings would not exist on the
  server, so gauges, alerts and the Findings panel would disagree with the map.

## Consequences

The web can be built and tested against any source, including a development file, before the
real source is chosen. The O-row for `web-server` covers it: the gauge
`rubato.traffic.report_age`, the counters of quiet seams and undeclared edges, and the generated
`protocol.ts` diffed in CI. A wrong mapping from service to part shows up as unmapped services,
not as false findings. Whatever source Q1 picks gets its own ADR (0040) and its own seam on the
map, added by a person through a map draft.

## Supersedes

None.
