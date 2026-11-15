# 0067. The companion reaches the server over TLS, except on loopback

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #194) — proposed; a human accepts
- **Seam:** companion-server
- **Issue:** m0x6A/rubato#194

## Context

The companion pairs over HTTP (ADR 0011) and then holds one outbound websocket to the server
(ADR 0012). Both accept `http://` and `ws://` for any host
(`crates/rubato-companion/src/server_link/client.rs:39-49`, `pairing/pair.rs`). Off loopback
that sends the pairing token in clear text, and anyone on the path can read it or inject a
`LaunchCommand` that the companion runs for its paired user. Raised in #170, split into #194.

## Decision

The companion accepts a server URL only when its scheme is `https` or `wss`, or when it is
`http` or `ws` and the host is loopback: `localhost`, an address in `127.0.0.0/8`, or `::1`.
Any other URL, including other schemes and URLs that do not parse, is refused before a request is
made: `rubato-companion pair --server <url>` fails and writes no state, and a daemon whose
`companion.json` names such a URL ends its `server_link` task with the error, which the
supervisor logs as `rubato.task.exited` and turns into a non-zero exit. There is no flag to
allow plain text elsewhere; a team without a certificate reaches the server through an SSH
tunnel or port forward to loopback, or a TLS-terminating proxy.

## Alternatives considered

- **An `--insecure` flag**: one more way to ship the token in clear text; a tunnel to loopback does the same job without it.
- **Warn and continue**: a warning in a daemon log nobody reads leaves the token on the wire.
- **Refuse on the server**: the server cannot tell what path the client used, and the token has already been sent by then.

## Consequences

A companion paired earlier with a plain-http off-loopback URL stops starting and says why; it
must pair again with `https://`. Local runs (`docs/QUICKSTART.md`, `e2e/run.ts`, the Gherkin
suite) use `http://127.0.0.1` and keep working. WSL reaches a server on the Windows host through
localhost forwarding, which is loopback. The O-row for `companion-server` in plan
`docs/plans/194.md` covers the refusal: the `rubato.task.exited` log line and the existing
`CompanionSilent` alert.

## Supersedes

None.
