# 0040. Read production traffic from the OTel service graph through the Prometheus query API, behind the Source port

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** Mattias Jonsson (decisions on #107, 2026-11-14); drafted by agent (Claude Code, claude-opus-5-5, issue #140) — proposed
- **Seam:** none until a person adds the source's part and seam to the map (plan `docs/plans/140.md`, A1); the data it produces crosses `web-server` as ADR 0039 says
- **Issue:** m0x6A/rubato#107

## Context

ADR 0039 fixed what the server does with a `TrafficReport` and left where reports come from to
this ADR. Production shows `not_configured` because there is no source. On #107, Mattias Jonsson
decided three things:

- Rubato does not take in application telemetry, so there is no OTLP receiver and no push.
- Depending on OpenTelemetry data is fine; depending on a specific stack is not.
- Every stack-specific detail is abstracted behind the existing `Source` port
  (`crates/rubato-server/src/traffic/source.rs:19`).

Design: `docs/plans/140-observed-system/design.md` §3.

## Decision

The production source is an adapter, `PrometheusSource` in `crates/rubato-server/src/traffic/prometheus.rs`,
implementing `Source`.

**What it reads.** It runs two instant queries against any Prometheus-compatible query API
(`POST /api/v1/query`). They read the OTel Collector servicegraph connector's
`traces_service_graph_request_total` and `traces_service_graph_request_failed_total`, increased
over the report window and summed by `client`, `server` and the two namespace labels. These
metrics are produced the same way by the Collector, Alloy or Tempo's metrics-generator.

**What it fills.** Each sample becomes a `TrafficEdge` with the client's and the server's
`service.namespace` when present.

**Configuration.** The source is chosen by `RUBATO_TRAFFIC_PROMETHEUS_URL`. Setting it together
with `RUBATO_DEV_TRAFFIC` is a startup error.

**What stays in the adapter.** PromQL, the metric prefix, the label names, credentials
(`RUBATO_TRAFFIC_PROMETHEUS_HEADERS`, never logged) and timeouts live only in the adapter and its
config. Nothing in `rubato-protocol`, the web, or the rest of the server knows which backend
answers.

**Failures.** A connect error, timeout, non-2xx response or `status: error` makes the source
failing. A malformed or non-vector result rejects the report. Both keep ADR 0039's "unknown is
never quiet". Unconfigured, Rubato behaves exactly as before.

**Later backends.** Adding another backend is a new adapter only.

## Alternatives considered

- **Services push summaries to `POST /api/traffic`**: rejected on #107. It needs code in every
  service and makes Rubato a receiver of application data.
- **An OTLP receiver in Rubato that builds its own service graph**: rejected on #107. It ingests
  application telemetry.
- **Grafana's or Tempo's own service graph API**: ties Rubato to one stack.
- **Count only Rubato's own seams**: it shows only what Rubato sees, not the team's system.

## Consequences

- **What lights the lens.** Production lights up only where the team's services emit traces with
  `service.namespace` and the connector carries that dimension. QUICKSTART documents both as
  setup on the team's side.
- **New attack surface.** The server makes one outbound query pair per fetch interval to a URL
  from config. The response is capped at 4 MiB and at 2 000 edges.
- **O-row.** The `web-server` O-row of plan #140 covers it:
  `rubato.traffic.fetches{rubato.outcome, rubato.traffic.source}`, `rubato.traffic.report_age`
  and alert `TrafficReportStale`.
- **A source seam.** A person adds an external part and seam for the source through a map draft.

## Supersedes

None.
