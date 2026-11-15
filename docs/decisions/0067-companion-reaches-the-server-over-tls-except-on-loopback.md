# 0067. The companion reaches the server over TLS, except on loopback or by an explicit `--insecure` pairing

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/194#issuecomment-6892514823 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #194) — proposed; option 2 with persistence chosen by the maintainer in conversation (2026-11-15, #194 Q1)
- **Seam:** companion-server
- **Issue:** m0x6A/rubato#194

## Context

The companion pairs over HTTP (ADR 0011) and then holds one outbound websocket to the server
(ADR 0012). Both accept `http://` and `ws://` for any host
(`crates/rubato-companion/src/server_link/client.rs:39-49`, `pairing/pair.rs`). Off loopback
that sends the pairing token in clear text, and anyone on the path can read it or inject a
`LaunchCommand` that the companion runs for its paired user. Raised in #170, split into #194.
The team's own homelab server serves plain http on the LAN, and the team accepts that risk there.

## Decision

The companion accepts a server URL when its scheme is `https` or `wss`, or when it is `http` or
`ws` and the host is loopback: `localhost`, an address in `127.0.0.0/8`, or `::1`. An `http` or
`ws` URL on any other host is accepted only when the developer pairs with
`rubato-companion pair --insecure --server <url>`. That choice is stored in `companion.json` as
`allow_plain_http: true` (default false; a file without the field reads as false), so the daemon
starts against the same URL without a flag; pairing again without `--insecure` clears it. While
the opt-in is in use the companion logs a warning naming the URL and saying that the pairing token
travels unencrypted, once at pairing and once at every daemon start. Without the opt-in, such a
URL is refused before a request is made: `pair` fails and writes no state, with an error that
names `--insecure`, and a daemon whose `companion.json` names such a URL ends its `server_link`
task with that error, which the supervisor logs as `rubato.task.exited` and turns into a non-zero
exit. Other schemes and URLs that do not parse are refused with or without the opt-in.

## Alternatives considered

- **Refuse, with no flag**: safest, but the team's LAN server would need a certificate or a tunnel on every machine.
- **A flag on every `run`, not stored**: easy to forget, and the daemon is usually started by a service manager that would have to carry it.
- **Warn and continue for any URL**: a warning in a daemon log nobody reads leaves the token on the wire by default.
- **Refuse on the server**: the server cannot tell what path the client used, and the token has already been sent by then.

## Consequences

A companion paired earlier with a plain-http off-loopback URL stops starting and says to pair
again, with `https://` or with `--insecure`. Local runs (`docs/QUICKSTART.md`, `e2e/run.ts`, the
Gherkin suite) use `http://127.0.0.1` and keep working. WSL reaches a server on the Windows host
through localhost forwarding, which is loopback. The O-row for `companion-server` in plan
`docs/plans/194.md` covers the refusal (the `rubato.task.exited` log line and the existing
`CompanionSilent` alert) and the opt-in (the warning at every start).

## Supersedes

None.
