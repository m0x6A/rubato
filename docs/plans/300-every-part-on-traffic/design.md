# Every part on the Traffic lens: design (#300)

Plan: [`../300.md`](../300.md). ADRs proposed: 0082 (the web's spans through the server), 0083 (last seen and the three states).

## Why each part is missing today

| Part | What it emits | Why the lens does not show it |
| --- | --- | --- |
| web | nothing | The browser exports no telemetry. The server's HTTP spans are `TraceLayer`'s internal spans (`crates/rubato-server/src/app.rs:221`), and a service graph is built only from client and server spans in one trace. |
| skill-rubato | `rubato.skill.call` client spans with `peer.service` set to the map id it reads or writes (`skills/rubato/scripts/plan.ts:200`, ADR 0061) | Only where an OTLP endpoint is set: sessions the companion launches have it (`crates/rubato-companion/src/launch/script.rs:51`), sessions started by hand usually do not. Its calls to `github` match; its calls to `plan-file` and `system-map` become virtual nodes that no part names, so they count only in `unmapped_services`. |
| plan-check | the same spans (`actions/plan-check/src/cli.ts:80`, `:102`) | The CI step has no OTLP settings. |
| plan-file, system-map | nothing (files) | Reachable only as the server end of a client span with `peer.service`; neither carries a `service`. |

Since #298 the lens fades every part with no calls in the window. That is right for an instrumented part that went silent and wrong for the rest.

## 1. The web's spans, relayed (ADR 0082)

**In the browser** (`web/src/shared/trace.ts`, no new dependency):

- `request()` in `shared/api.ts` makes a span for each call:
  - a random 16-byte trace id and 8-byte span id from `crypto.getRandomValues`;
  - sends `traceparent: 00-<trace>-<span>-01`;
  - records the method, the route, the status and the start and end in nanoseconds.
- The route is the path with the issue, pull and launch numbers and the workspace replaced by placeholders (`/api/w/{ws}/pins`, `/api/pulls/{owner}/{repo}/{number}`), so the span names stay few.
- Spans wait in a buffer. It flushes within 1 s of its first span, as one OTLP/JSON `ExportTraceServiceRequest` of at most 60 KiB:
  - resource `service.name=web`, `service.namespace=rubato`;
  - kind `SPAN_KIND_CLIENT`;
  - attributes `http.request.method`, `url.path` (the route) and `http.response.status_code`.
- The flush is a plain `fetch(…, {method: "POST", keepalive: true, credentials: "same-origin", headers: {"content-type": "application/json"}})`, not `request()`. So the post is never traced itself, a 401 never raises needs-login, and `keepalive` lets the last flush outlive `pagehide` within the browser's 64 KiB quota.
- Why 1 s: the service graph pairs a client span with its server span only if both reach the generator within its wait (Grafana Cloud Tempo: 10 s). The server exports every 5 s.
- No `server.address`: an unpaired client span would otherwise become a virtual node named after it.
- If the relay answers 404, the page stops recording and sending until reload. Any other failure drops the batch; nothing retries.

**In the server** (new slice `telemetry_relay`, no table):

- `POST /api/telemetry/traces`, behind the web login (`CurrentUser`).
- Limits, in order:
  - 404 when the server exports no traces;
  - 401 without a login;
  - 413 over 256 KiB (`DefaultBodyLimit` on the route, as the webhook route has);
  - 429 past a token bucket per login (burst 4, one more every 2 s, in memory), so two tabs flushing at once both get through;
  - 400 for a body that is not OTLP/JSON, or has no spans or more than 512.
- The batch is rewritten, not trusted. Nothing the browser wrote as free text goes on:
  - one resource, with exactly `service.name=web` and the server's own `service.namespace` (ADR 0080's default);
  - each span keeps its ids, its parent, its kind, its status code and its times;
  - its name becomes `<METHOD> <route>`, where the route must be one of the server's own route templates. A span with any other route is dropped, so span names stay bounded.
  - the status message and every other attribute are dropped;
  - a span that is not client-kind, or whose times are more than an hour from now, is dropped.
- The rewrite is forwarded to the server's OTLP traces endpoint (`{endpoint}/v1/traces`) with a 5 s timeout. The OTLP headers are read once into `Config` with a `Debug` that prints names only, as `PrometheusConfig` does, and are never logged.
- The browser gets 202 once the batch is accepted. The export result is a counter, not the browser's business.
- Every post counts once in `rubato.telemetry.relay` by `rubato.outcome`, and the spans dropped in the rewrite in `rubato.telemetry.relay.dropped`.

**Continuing the trace.** `TraceLayer::new_for_http()` gets a `make_span_with`:

- A request with a valid `traceparent` makes `rubato.http.request` with `otel.kind=server` and `http.route`. Its parent is set with `otel::TraceParent::parse` and `set_parent`, as `companion_link/ws.rs` does for envelopes.
- A request without one, or with one that does not parse, keeps today's internal span.
- Each case counts once in `rubato.http.traceparent` (`linked`, `absent`, `invalid`).

So the service graph pairs the browser's client span with the server's span and draws `web → rubato-server`. Health checks, asset loads and `curl` draw no virtual `user` node.

**Trust.** The relay is the only way in, and it runs as a logged-in team member. A forged batch can at worst add calls to `web → rubato-server` on the team's own lens. It cannot name another service, cannot carry attributes onward, and cannot reach the backend's credentials. The browser never sees the endpoint or the headers.

## 2. `web`, `plan-file` and `system-map` carry a service, and the server names its reads

Through the shared draft (ADR 0018) and the map's pull request:

- `web: service: { namespace: rubato, name: web }`. Without it, the relayed spans would resolve to a ghost `rubato/web` beside a hatched `web` part.
- `plan-file: service: { name: plan-file }`
- `system-map: service: { name: system-map }`

The two files have no namespace, by ADR 0059. They then match the virtual nodes that the skill's and plan-check's `peer.service` calls make, so `skill-plan-file`, `skill-system-map` and `plan-check-plan-file` carry calls.

The server reads both files through GitHub. Without more, `server-plan-file` and `server-system-map` would be flagged `SeamQuiet` falsely: both ends carry `service` and are observed elsewhere. So when the server reads a plan file at a branch (the plan read, Import) or the map, it opens a client span with `peer.service=plan-file` or `peer.service=system-map` around the read, inside the `rubato.github.call` span. The read is what the seam declares; GitHub is where the bytes come from, and both show.

## 3. plan-check in CI

The `plan-check` step in `.github/workflows/ci.yml` gets:

```yaml
env:
  OTEL_EXPORTER_OTLP_ENDPOINT: ${{ secrets.RUBATO_OTLP_ENDPOINT }}
  OTEL_EXPORTER_OTLP_HEADERS: ${{ secrets.RUBATO_OTLP_HEADERS }}
  OTEL_RESOURCE_ATTRIBUTES: service.namespace=rubato
```

`otel-emit.ts` does nothing when the endpoint is empty, so a fork's pull request (which gets no secrets) runs as today. The repository is public: the secrets are never echoed, and `otel-emit.ts` never prints the headers (ADR 0061).

## 4. Last seen, and three states (ADR 0083)

**The source.** A lookback task beside the window's fetch, in the traffic supervisor, with its own timeout. It never runs inside `source.fetch()`, whose time budget is the fetch interval.

- It runs on the first fetch after the source starts (or its settings change), then every tenth fetch.
- It asks the request-total query over 6 h, 1 d and 7 d (only lookbacks longer than the window), with the same namespace matcher.
- Its result is every edge any lookback saw, in the window or not, with the shortest lookback as `within_secs`, and the longest lookback asked as `lookback_secs`. The window still wins in `derive_traffic`.
- Each report carries the latest result, as `TrafficReport.seen` and `TrafficReport.lookback_secs`.
- Before the first result, `lookback_secs` is absent: nothing is known, and nothing is claimed.

A failing or timed-out lookback:

- keeps the previous result, or "unknown" when there is none;
- counts in `rubato.traffic.lookback`;
- logs `traffic lookback` with `rubato.outcome`;
- never makes the report failing.

The window's report alone decides fresh, stale and failing.

**The derivation** (`derive_traffic` in `rubato-protocol`, pure):

- `ObservedTraffic.last_seen`: for each part and each declared seam with no calls in the window, the smallest `within_secs` of an edge that maps to it.
- `ObservedTraffic.lookback_secs`: as the report's. With it set, a quiet instrumented part not in `last_seen` was not seen that far back. Without it, nothing is said.
- `ObservedTraffic.not_instrumented`: part ids without `service`, and seam ids with an end without `service`.
- Only on a fresh report. Unknown stays unknown.

**The lens** (on top of #298):

| State | Part | Seam |
| --- | --- | --- |
| observed | glow by error share, calls in and out (as #298) | live pipe (as #298) |
| quiet, seen in a lookback | full opacity, plain border, a chip "last seen within 6 h", "within a day" or "within a week" | plain line, chip "last seen …" |
| quiet, no lookback yet | full opacity, plain border, no chip | plain line |
| quiet, not seen in 7 d (a lookback ran) | faded as #298's `unobserved`, chip "not seen in 7 d" | faded |
| not instrumented | neutral, hatched (`--hatch` tokens), chip "not instrumented" | dashed neutral, no fade |

The headline strip gains "not instrumented N" next to "quiet seams".

## Cost

- Three more instant queries every tenth fetch. With the 60 s default that is 18 queries an hour more, each a 7 d `increase` at most. `fetch_secs` ranges from 15 to 300, so "every tenth" means every 2.5 to 50 minutes.
- The relay: about one post a second per busy browser, fewer when idle, each at most 60 KiB, one outbound HTTP call each.
- The overview grows by one entry per quiet part and seam, capped with the edges.

## Order of work

One pull request, stacked on #299, one commit per step so it can be read in order:

1. protocol (types, `derive_traffic`, routes, `otel` names);
2. server: the lookback task;
3. server: HTTP server spans, its named reads, the relay;
4. web: the tracer;
5. web: the three states;
6. scenarios and e2e;
7. map, CI and dashboards.

The map's `service` fields can also be set through Rubato's draft; the map file is in this repository, so the pull request may carry them.

## Risks

- **Volume of web spans.** One per API call. An idle page fetches the overview on each change; a busy team could send a few hundred spans a minute. The token bucket and the 512-span cap bound it. If it is too many, sampling in `trace.ts` is the knob, as a follow-up.
- **Pairing.** If the generator's wait is shorter than the gap between the browser's span and the server's, the pipe shows a fraction of the calls with no error anywhere. The e2e `callPairs` check proves the trace links; the "HTTP trace context" panel shows `linked` against `absent`. QUICKSTART names the wait.
- **`increase` and rare edges.** `increase` ignores a series' first sample. If the generator expires idle series, a single call from a rare edge may never show in a lookback. Verify on the live stack before trusting "last seen" for the skill and plan-check.
- **Route templating drift.** A new route with an id in its path makes many span names. `trace.test.ts` checks the placeholder list against `protocol.ts`'s route constants.
- **The lookback lies about short windows.** With a window of 6 h or more, the 6 h lookback is skipped and the buckets start at 1 d. The chip says the bucket, never a guessed time.
