# 0012. Carry companion ↔ server traffic over one outbound websocket with JSON envelopes

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (drafted by agent, Claude Code v0 planning session)
- **Seam:** companion-server
- **Issue:** none

## Context

The companion pushes heartbeats, session states and hook events and receives launch
commands. Both directions, low volume, must work from behind NAT with no inbound access.

## Decision

The companion opens one websocket to the server (tokio-tungstenite; axum on the server) and
keeps it open. Messages are `CompanionMessage` and `ServerMessage` enums from
`rubato-protocol`, serialised as JSON, each with a monotonic sequence number. On reconnect
the companion sends its last acknowledged sequence; the server replays anything newer.

## Alternatives considered

- **HTTP POST for events plus SSE for commands**: two connections, two auth paths.
- **gRPC (tonic)**: binary protocol hard to inspect and to generate TS types from.

## Consequences

Easier: one connection to watch (`rubato.companion.heartbeat`). Harder: resume logic must be
tested; the fixture replay in both suites covers it.

## Supersedes

none
