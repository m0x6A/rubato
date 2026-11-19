# 0082. The web sends its own client spans through a relay on the server, and the server's HTTP spans continue the browser's trace

- **Status:** proposed
- **Date:** 2026-11-19
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #300) — proposed; the choice of a relay is Mattias Jonsson's, 2026-11-19
- **Seam:** web-server
- **Issue:** m0x6A/rubato#300

## Context

The Traffic lens on Rubato's own map cannot show `web-server`. The browser exports nothing, and the server's HTTP spans are internal. A service graph draws an edge only from a client span and a server span in one trace. Marking the server's spans as server spans alone would draw a virtual `user` client, and that also counts health checks and `curl`. Plan: `docs/plans/300.md`, design §1.

## Decision

The web records a client span for each API request it makes and sends `traceparent` on that request. Within 1 s of its first span it posts them as OTLP/JSON to `POST /api/telemetry/traces` on its own server, with a plain untraced `fetch` and `keepalive`, at most 60 KiB. The 1 s keeps both ends of a call inside the service-graph generator's pairing wait.

The relay, a new slice `telemetry_relay`:

- runs only for a logged-in web session;
- refuses a post over 256 KiB or 512 spans, or past a token bucket per login;
- answers 404 when the server exports no traces;
- rewrites the batch to one resource with `service.name=web` and the server's `service.namespace`;
- names each span `<METHOD> <route>` from a route template the server itself serves, and drops a span with any other route, any span not of client kind, and any span more than an hour from now;
- keeps only the ids, the parent, the kind, the times, the status code and the attributes `http.request.method`, `url.path` (the template) and `http.response.status_code`. Every other attribute, the status message and the browser's span name are dropped;
- forwards the result to the server's own OTLP traces endpoint. The OTLP headers are read once into the server's configuration, are printed by name only, and are never logged.

The server turns an HTTP request that carries a valid `traceparent` into a server-kind span `rubato.http.request` whose parent is the browser's span, and counts linked, absent and invalid headers. A request without one keeps today's internal span. Nothing else about a request changes, and the header is never an identity or an authorisation.

## Alternatives considered

- **Server spans only**: a virtual `user` node instead of `web`, mixed with health checks and scripts.
- **The OpenTelemetry web SDK**: a dependency of several packages for what is a few span fields and one POST.
- **The browser exporting to the backend directly**: the backend's credentials in the browser and a new origin in the CSP.

## Consequences

- `web-server` carries a new flow (spans and a header), and the server makes one more outbound call per batch, to the same backend it already exports to.
- The relay is a write route open to every logged-in person. Its limits and the rewrite bound what a person can inject: calls on `web → rubato-server`, under route names the server already has, and no free text.
- The `web` part carries `service: { namespace: rubato, name: web }`. Without it the relayed spans would be a ghost.
- The pairing depends on the generator's wait. QUICKSTART names it.
- O-rows: counters `rubato.telemetry.relay` and `rubato.http.traceparent` by `rubato.outcome`, panels "Web spans relayed" and "HTTP trace context".
- A server without an OTLP endpoint changes nothing for the web: one 404, then silence.

## Supersedes

None.
