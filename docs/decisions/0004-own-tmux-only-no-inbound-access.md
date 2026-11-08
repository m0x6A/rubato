# 0004. A user starts sessions only in their own tmux; no inbound access to developer machines

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** companion-server
- **Issue:** none

## Context

The companion runs `tmux`, `git worktree` and `claude` on a developer's machine. Any path by
which another person could trigger that is both a security hole and a violation of
federated control.

## Decision

The companion connects outbound only. It pairs with exactly one GitHub login through a
device-code flow. The server addresses a `LaunchCommand` only to the pairing of the user who
pressed the button; the companion refuses any command whose user is not its paired user and
reports the refusal as `LaunchResult::Refused`. Nothing in Rubato listens on a developer
machine except a loopback HTTP port for hook delivery, guarded by a per-start token (0013).

## Alternatives considered

- **Server-initiated SSH or reverse tunnel**: inbound access; ruled out.
- **Team-wide launch permission with audit**: steers other people's agents; ruled out by 0001.

## Consequences

Easier: the trust story fits in one sentence. Harder: a teammate cannot restart a colleague's
stuck session; they ask. Watched by the Gherkin scenario "refusing launch for another user"
and `rubato.events.dropped{reason=auth}`.

## Supersedes

none
