# 0081. A companion envelope may carry a W3C `traceparent`, so companion and server share a trace

- **Status:** accepted
- **Date:** 2026-11-18
- **Decided by:** Mattias Jonsson (m0x6A), accepted in the Claude Code session 2026-11-24 ("Yes"), on his answer of 2026-11-18 ("Optional field in the envelope"); drafted by agent (session on m0x6A/rubato#287)
- **Seam:** companion-server
- **Issue:** m0x6A/rubato#287

## Context

Rubato's Traffic lens reads the service graph (ADR 0040). A service graph draws an edge between
two services that emit telemetry only when a client span and a server span are in one trace.
The companion and the server each make spans for an envelope (`rubato.hook.forward` and others
on the companion, `rubato.inbound_event` on the server), but in separate traces and with no span
kind, so `companion-server` can never show on Rubato's own map. Calls to a part that emits
nothing, such as GitHub, need only a client span with `peer.service` (ADR 0061 does this for
skills); two parts that both emit need the trace to cross the seam.

## Decision

`CompanionEnvelope` gains an optional `traceparent`: the W3C Trace Context header value
(`00-<trace id>-<span id>-<flags>`) of the companion's client span for that envelope.

- **Companion.** Each envelope it builds is wrapped in a span `rubato.envelope.send` of kind
  client, a child of whatever span caused the message (a hook forward, a reconcile), and the
  envelope carries that span's `traceparent`. The value is stored with the envelope in the
  outbox, so a replay after a reconnect sends the same one.
- **Server.** `rubato.inbound_event` becomes a span of kind server. When the envelope carries a
  valid `traceparent`, the span is its child; when it carries none or one that does not parse,
  the span starts its own trace as today. A replayed envelope makes no second span tree beyond
  the one it already makes today.
- **Additive and optional.** The field is left out when absent. A server without this change
  ignores the unknown field; a companion without it sends none. Neither side refuses the other.
- **Nothing else rides on it.** No `tracestate`, no baggage. The server never trusts the value
  for anything but parenting a span: it is not an identity and not an authorisation.
- **Server to companion** (`ServerEnvelope`) is not changed here; the calls the map declares on
  this seam are drawn from the companion's side, and a second direction can be added the same way.

Not a seam change, decided with it for the same goal: the server's `rubato.github.call` spans
become kind client with `peer.service=github`, as the skills' calls already are.

## Alternatives considered

- **`peer.service` on the companion's span only**: the service graph then draws a virtual,
  namespaceless `rubato-server`, which by ADR 0059 does not match the real part; the seam stays dark.
- **A header on the websocket upgrade**: one trace for the whole connection, hours long, and no
  link from one hook to the server's handling of it.
- **Trace ids derived from the envelope's `seq`**: no wire change, but both sides must then
  invent the same ids and neither SDK's sampler or id generator may be used as it is.

## Consequences

- `schema/protocol/companion_envelope.schema.json` changes, with the generated TypeScript types.
- One more span per envelope on the companion. Heartbeats are envelopes too: at one a minute per
  companion this is small, and the plan measures it.
- A trace now spans two machines; a slow or failing server handling shows under the hook that
  caused it.
- O-row: `companion-server`, envelopes handled with no parent although the companion sent one:
  metric `rubato.inbound.traceparent` by outcome (`linked`, `absent`, `invalid`) on the
  "Companions" panel of rubato-overview; owner side `rubato-server`.

## Supersedes

None.
