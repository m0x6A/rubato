# Design: the observed system on the map (#140, with #107 and #142)

Plan: [../140.md](../140.md) · Track: **full** · ADRs 0040, 0059, 0060, 0061 (proposed).

## 1. The picture

#104 built the pipeline from a `TrafficReport` to the Traffic lens, but three things are missing:

- **Production has no source** (#107), so it shows `not_configured`.
- **Matching is by name only.** A service matches a part only when its name equals the part id,
  with no namespace. A service that matches nothing is only counted.
- **Skills emit nothing** (#142), so the part that writes plans is invisible to the service graph.

This unit adds the source, matches on the identity each service emits, draws what does not match
as ghosts, and lets a person carry a discrepancy into the shared map draft (ADR 0018) or dismiss
it.

The rules from #140's Settled section hold throughout:

- the overlay shows and a person merges;
- Rubato never ingests application telemetry;
- OpenTelemetry data is fine, a specific stack is not;
- every stack-specific detail sits behind the `Source` port;
- identity is `service.namespace` + `service.name` as emitted;
- Rubato assumes proper instrumentation and has no workaround for a part that emits nothing.

Landing order: PR 1 (#142) → PR 2 (#107) → PR 3 (#140). Each PR is useful on its own:

- PR 1 makes skills visible in any team's service graph.
- PR 2 lights the Traffic lens in production with today's matching widened by namespaces.
- PR 3 adds ghosts, the identity rule and the draft loop.

## 2. Skills emit spans (#142, ADR 0061)

### 2.1 `otel-emit.ts`

One file, `skills/rubato/scripts/otel-emit.ts`, using only Node built-ins (`fetch`,
`AbortSignal.timeout`, `crypto.randomBytes`, `child_process.spawn`).

```ts
export async function span<T>(
  name: string,                     // a rubato.* span name from otel.rs
  attrs: Record<string, string | number | boolean>,
  fn: (child: ChildSpans) => Promise<T> | T,
  opts?: { kind?: "internal" | "client"; peer?: string },
): Promise<T>;
// child.call("github", () => gh(...)) records a client span with peer.service=github.
```

- **Buffer and flush.** Spans are buffered. One POST is made at `beforeExit`, or when a CLI
  wrapper's child process exits.
- **Body.** OTLP/JSON `ExportTraceServiceRequest`:
  - resource attributes: `service.name` = the skill's name (`rubato`; `plan-check` for the
    action), `service.namespace` and every other key from the resource attributes (below);
  - one scope, `rubato.skill`;
  - a span's status is `ERROR` when `fn` throws or the exit code is non-zero.
- **Endpoint**, first found wins:
  1. `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` (used as is);
  2. `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/traces`;
  3. `RUBATO_OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`;
  4. `RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/traces`.

  None found: nothing is sent and nothing is buffered.
- **Headers**: `OTEL_EXPORTER_OTLP_HEADERS`, else `RUBATO_OTEL_EXPORTER_OTLP_HEADERS`, as
  `k=v,k=v` with URL-decoded values. Never printed.
- **Resource attributes**: `OTEL_RESOURCE_ATTRIBUTES`, else `RUBATO_OTEL_RESOURCE_ATTRIBUTES`.
  - `service.namespace` comes only from here. The script never invents one (#140).
  - `OTEL_SERVICE_NAME` is ignored, because inside a session it would be Claude Code's.
- **Protocol**: `OTEL_EXPORTER_OTLP_PROTOCOL` (or its mirror) of `grpc` skips at once (A9).
  Unset, `http/json` or `http/protobuf` send JSON. Collectors accept JSON on the HTTP endpoint
  whatever the configured protobuf preference.
- **Parent**: a valid W3C `TRACEPARENT` gives the trace id and parent span id. Claude Code sets it
  for Bash when its tracing beta is on. Otherwise a new trace starts.
- **Failure**: the POST has an 800 ms timeout (A5). Any error, timeout or non-2xx response is
  swallowed with no output. The process's exit code and stdout are never touched.

CLI, for a skill that is only Markdown and shell:

```
node --experimental-strip-types otel-emit.ts span --name rubato.skill.command \
  --skill <skill> [--peer github] -- <command> [args…]
```

It spawns the command with inherited stdio, waits, sends one span with `rubato.exit_code` and
the duration, and exits with the command's code (or re-raises its signal).

### 2.2 Getting the settings into a session

Claude Code strips every `OTEL_*` variable from the processes it spawns. It does pass other
variables, and it sets `TRACEPARENT` when its own tracing is on. So the launch script
(`crates/rubato-companion/src/launch/script.rs`) gains:

```sh
export OTEL_RESOURCE_ATTRIBUTES="${OTEL_RESOURCE_ATTRIBUTES:+$OTEL_RESOURCE_ATTRIBUTES,}rubato.repo=…,rubato.issue=…"
export RUBATO_OTEL_RESOURCE_ATTRIBUTES="$OTEL_RESOURCE_ATTRIBUTES"
export RUBATO_OTEL_EXPORTER_OTLP_ENDPOINT="$OTEL_EXPORTER_OTLP_ENDPOINT"
export RUBATO_OTEL_EXPORTER_OTLP_PROTOCOL="$OTEL_EXPORTER_OTLP_PROTOCOL"
export RUBATO_OTEL_EXPORTER_OTLP_HEADERS="${OTEL_EXPORTER_OTLP_HEADERS-}"
```

- **Merge, don't overwrite.** Today the script overwrites `OTEL_RESOURCE_ATTRIBUTES`. Merging
  keeps a `service.namespace` the developer's environment sets. Rubato still sets no namespace.
- **Headers are expanded at run time** from the tmux environment, never written into the script
  file (A10).
- **Meaning.** The mirror means exactly what its `OTEL_*` twin means. It exists only because of
  Claude Code's stripping, and the script prefers the standard name.

What a skill's spans carry in a launched session: the developer's `service.namespace`,
`service.name=rubato`, `rubato.repo`, `rubato.issue`, `rubato.branch` and `rubato.node` (from
the merged attributes), and the Claude Code tool span as parent when tracing is on.

### 2.3 What the rubato skill wraps

| Script | Span | Client spans (`peer.service`) |
| --- | --- | --- |
| `map.ts` | `rubato.skill.command` (`rubato.skill.subcommand=map`; the attribute cannot share the span's name in `otel.rs`) | `system-map` (reading the map file) |
| `plan.ts init/check/sha/wait/adrs-pin` | `rubato.skill.command` | `plan-file`, `system-map`, `github` (each `gh` call in `wait`) |
| `adrs.ts`, `adrs.ts accept` | `rubato.skill.command` | `github` |
| `hooks.ts`, `install.ts` | `rubato.skill.command` | none |
| plan-check `cli.ts` | `rubato.skill.command` with `rubato.skill.name=plan-check`, `service.name=plan-check` | `plan-file`, `system-map` |

The `peer.service` values are the map part ids of those externals and contracts. The map
declares `service: {name: github}` on them, with no namespace (§4.2), so the servicegraph's
virtual nodes match.

plan-check imports `otel-emit.ts` from `skills/rubato/scripts/` with a dynamic `import()`, and
only when an endpoint is set. A test pins the relative path.

## 3. The production source (#107, ADR 0040)

### 3.1 The port stays the same

`Source::fetch() -> Result<TrafficReport, SourceError>` (`traffic/source.rs:19`) is unchanged.
`ConfiguredSource` is an enum over `DevFileSource` and `PrometheusSource`, chosen in
`kernel/state.rs` from config. Setting both is a startup error. Nothing outside
`traffic/prometheus.rs` knows PromQL, metric names, label names or credentials.

### 3.2 Queries

Two instant queries, `POST {url}/api/v1/query` with form fields `query` and `time` (now):

```
topk(2001, sum by (client, server, <cns>, <sns>) (increase(<prefix>request_total[<window>s])))
topk(2001, sum by (client, server, <cns>, <sns>) (increase(<prefix>request_failed_total[<window>s])))
```

- `<prefix>` defaults to `traces_service_graph_`.
- `<cns>` and `<sns>` default to `client_service_namespace` and `server_service_namespace`, the
  labels the connector writes for `dimensions: [service.namespace]`.

Mapping:

- A sample becomes a `TrafficEdge`:
  - `calls` is the value rounded, with NaN or a negative value as 0;
  - `errors` comes from the joined failed series, capped at `calls`;
  - an empty or missing namespace label becomes `None`.
- `observed_at` is the result's sample time. `window_secs` comes from config.
- 2 001 series are cut to 2 000 with `truncated`.
- The response body is read up to 4 MiB; anything over is `Parse`.
- Connect errors, timeouts, non-2xx and `status: "error"` are `Failed`, so the state turns
  `failing`. Bad JSON and a `resultType` other than `vector` are `Parse`, so the report is
  rejected and the previous one kept, as today.

`traces_service_graph_request_server_seconds` and `connection_info` are not read (A6).

### 3.3 Config

| Env | Default | Meaning |
| --- | --- | --- |
| `RUBATO_TRAFFIC_PROMETHEUS_URL` | unset | base URL of any Prometheus-compatible query API; unset means this source is off |
| `RUBATO_TRAFFIC_PROMETHEUS_HEADERS` | unset | `k=v,…` sent on every query (Bearer or Basic); never logged |
| `RUBATO_TRAFFIC_PROMETHEUS_TIMEOUT_SECS` | 10 | per query; clamped below the fetch interval |
| `RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX` | `traces_service_graph_` | |
| `RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS` | `client_service_namespace,server_service_namespace` | |
| `RUBATO_TRAFFIC_WINDOW_SECS` | 3600 | 60–86 400 |
| `RUBATO_TRAFFIC_FETCH_SECS` | 60 | unchanged |

### 3.4 Team-side setup (QUICKSTART)

1. Every service sets `service.namespace` and `service.name` (standard
   `OTEL_RESOURCE_ATTRIBUTES` / `OTEL_SERVICE_NAME`). That includes `rubato-server` and
   `rubato-companion`, whose `service.name` is set in code and whose namespace comes from the
   environment (`kernel/telemetry.rs` keeps the environment's resource attributes; a test pins
   it).
2. The servicegraph connector (Collector or Alloy), or Tempo's metrics-generator, adds
   `service.namespace` as a dimension.
3. The backend serves the Prometheus query API.

## 4. Identity and derivation (ADR 0059)

### 4.1 `service` on a part

```yaml
nodes:
  - id: rubato-server
    kind: context
    service: { namespace: rubato, name: rubato-server }
  - id: github
    kind: external
    service: { name: github }        # a servicegraph virtual node: emitted with no namespace
```

`ServiceId { namespace: Option<String>, name: String }`, both at most 200 bytes. A part without
`service` is never matched. `SystemMap::validate` refuses two parts with an equal `ServiceId`.
Because `MapOp::AddNode` and `UpdateNode` carry a whole `MapNode`, the draft carries `service`
with no new op. `map_draft/yaml.rs` gets `service`, `namespace` and `name` as known keys, so
proposals keep them.

### 4.2 Matching an end

For each edge end `(namespace?, name)`:

1. A part whose `service` equals it exactly, `None` matching only `None`: **part**.
2. Otherwise, if the end has a namespace: **ghost**, keyed by its `ServiceId`.
3. Otherwise: **unidentified**, only counted, in `unmapped_services` (same field and gauge, narrower meaning).

The issue's rule says no name-only matching. `{name: github}` is not a fallback: it is exactly
what the virtual node emits, and it can never match a namespaced end.

Ghosts come only from namespaced ends. Services that emit carry a namespace (#140, Settled). A
namespaceless end that no part claims is a virtual node nobody declared, or a service that emits
badly. Neither is drawn.

### 4.3 What is derived

These run on a fresh report only. Non-fresh states are unchanged from ADR 0039: nothing is
drawn.

| Edge ends | Result |
| --- | --- |
| part → part, seam joins | `SeamTraffic` as today |
| part → part, no seam | `UndeclaredEdge` and `UndeclaredTraffic` as today |
| part ↔ ghost, or ghost → ghost | `GhostEdge { from: TrafficEnd, to: TrafficEnd, calls, errors }` |
| any → unidentified | counted |

- **Ghosts.** Every ghost becomes a `GhostPart { service, calls_in, calls_out, errors }` and a
  `Finding::ObservedOnlyPart { service, calls }`. At most 50 ghosts and 200 ghost edges are kept,
  by calls (A4), with `ghosts_truncated`.
- **Observed.** A part is observed when any edge with calls has it at an end, a ghost edge
  included. This is a change: today an edge to an unmapped service is dropped before observation.
- **Quiet.** `SeamQuiet` additionally requires both parts to have `service`.
- **Self-calls** are ignored as today.
- **Dismissals.** `derive_traffic` takes the workspace's dismissals:
  - a dismissed `Ghost{service}`, `Edge{from, to}` (over `TrafficEnd`s) or `QuietSeam{seam}` is
    left out of the drawn lists and the findings;
  - every dismissal is listed in `ObservedTraffic.dismissed`, whatever the state, so the legend
    can offer Undo.

## 5. Dismissals (ADR 0060)

Table `traffic_dismissals`, migration `0014_traffic_dismissals.sql`, owned by slice `traffic`:

```sql
CREATE TABLE traffic_dismissals (
  id          INTEGER PRIMARY KEY,
  workspace   TEXT NOT NULL,
  target_key  TEXT NOT NULL,     -- canonical JSON of DiscrepancyKey
  target_json TEXT NOT NULL,
  author      TEXT NOT NULL,     -- GitHub login from the web session
  created_at  TEXT NOT NULL,
  UNIQUE (workspace, target_key)
);
```

Routes, map-scoped like the draft (`/api/…` for the default workspace, `/api/w/{ws}/…`
otherwise):

- `POST /api/traffic/dismissals` with body `{ target: DiscrepancyKey }`:
  - 201 with the row;
  - 200 with the existing row when the key is already dismissed;
  - 409 `TOO_MANY` past 500.
- `DELETE /api/traffic/dismissals/{id}`: 204, or 404.

Both need a logged-in team member, are counted on refusal with `rubato.web.rejected`, log
`traffic dismissal added` or `traffic dismissal removed`, and notify the workspace. A dismissal
whose target no longer appears stays until undone. It is harmless and listed.

## 6. The overlay and "take into draft"

### 6.1 Ghost band

After ELK (or the stored layout) places the declared parts, `graph.ts` computes their bounding
box and places ghosts in a column 200 px right of it:

- sorted by namespace, then name;
- 40 px apart;
- for the By repo variant, beside the outermost box.

Ghost positions are never sent to `PUT /api/layout`, and declared positions are computed exactly
as before. A Vitest compares them with and without ghosts.

### 6.2 Drawing

- **Ghost card.** Dashed outline with the same amber territory role as #104's undeclared edge,
  showing `namespace / name` and the call rate. It has no handles and takes no issue drops, has
  no pin menu, and is never a plan target.
- **Ghost edges.** Territory edges as in #104: dotted amber, chevrons (counted in the cap), and
  the label "observed, not on the map".
- **Quiet seams** are drawn as today.
- **Legend.** It gains "Ghost: a service seen in telemetry that is not on the map" and
  "Dismissed (n)", which lists each dismissal with author, time and Undo.

### 6.3 Actions (inspector, Traffic lens only)

| Selected | Take into draft | Dismiss |
| --- | --- | --- |
| ghost | opens edit mode with the Add part form prefilled: id `slug(name)` (namespace-prefixed when it collides), kind `context`, `service` as emitted. The person sets kind, repo and description and submits | yes |
| ghost edge or undeclared edge | Add seam prefilled: from, to, id `from-to`, owner `to`. Disabled with "take the part first" while an end is a ghost without an add-part op of that `service` in the draft | yes |
| quiet seam | Remove seam, through the existing op | yes |

- **Prefill.** `DraftPanel` takes `prefill?: { nonce, op, values }`. Its forms are keyed by
  `nonce` and use `defaultValue`, so a new prefill resets the form and nothing is posted until
  the person submits.
- **New inputs.** The Add part and Change part forms gain optional *service namespace* and
  *service name* inputs.
- **In the draft.** A ghost whose `service` is in a draft add-part op shows "in draft" instead of
  the action.
- **After merge.** Once the PR merges and the map reloads, the ghost's service matches a part. It
  stops being a ghost and its edges become seam traffic.

## 7. Telemetry

| Name | Kind | Where |
| --- | --- | --- |
| `rubato.skill.command` | span | skill scripts, plan-check |
| `rubato.skill.call` | span (client, with `peer.service`) | skill scripts, plan-check |
| `rubato.skill.name`, `rubato.skill.command`, `rubato.exit_code` | attributes | skill spans |
| `rubato.traffic.source` (`dev_file`, `prometheus`) | attribute | `rubato.traffic.fetch`, `rubato.traffic.fetches` |
| `rubato.traffic.ghost_parts` | gauge per workspace | server |
| `rubato.traffic.dismissals` | gauge per workspace | server |

Dashboard panels:

- **"Map versus territory"** gains ghosts and dismissals; its unmapped panel is relabelled "unidentified ends".
- **"Skills emitting"** is new: `rubato.skill.command` spans per `rubato.skill.name` and per
  `rubato.issue`, read from the trace backend's span metrics, not from Rubato.

## 8. What stays out

- Hook-derived spans for unwrapped skills: a later issue (#142 Q3).
- History of traffic, trends, or acting on traffic beyond the draft.
- Any receiver of telemetry in Rubato.
- Matching by id, or by name without the namespace a service emits.
