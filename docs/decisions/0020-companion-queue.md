# 0020. Keep a queue of intended issues per companion, writable only by its owner

- **Status:** proposed
- **Date:** 2026-11-10
- **Decided by:** agent (Claude Code, issue #48 design session); proposed for Mattias Jonsson to accept
- **Seam:** web-server
- **Issue:** m0x6A/rubato#48

## Context

Issue #48 asks that issues can be matched to companions and that each companion shows what is being done with it. A session's issue comes from its branch (ADR 0006), a user may start sessions only on their own companion (ADR 0004), and Rubato does not steer anyone else's agent (ADR 0001). So a match made by hand can only be an intention on one's own companion. Design: `docs/plans/48-interactive-map/design.md` (D5).

## Decision

`rubato-server` stores, per companion, an ordered list of issues its owner intends to run there, in a table `companion_queue` owned by a `queue` slice. Everyone can see every queue; only the companion's paired user can add, reorder or remove, and the server refuses anything else with 403. An entry leaves the queue when the companion reports a launch of that issue as started, when its owner removes it, or when the companion is revoked; a refused or failed launch leaves it in place. The queue is not a progress state and is never an input to `derive_progress`; once a session exists, the session is the truth.

## Alternatives considered

- **Drop equals launch**: nothing stored, but matching and starting become one gesture with no room to line work up.
- **Assigning an issue to another person's companion**: steers someone else's agent; against 0001 and 0004.
- **Suggesting a companion automatically**: left for later; it needs signals we do not have (idle time, worktrees present).

## Consequences

A team lead can line up work per machine and others can see it, without anyone being told what to run. The same issue may be queued on two people's companions; that is shown, not prevented. Covered by the `web-server` O-row (`rubato.queue.depth`, `rubato.web.rejected{reason=not_own_companion}`).

## Supersedes

None.
