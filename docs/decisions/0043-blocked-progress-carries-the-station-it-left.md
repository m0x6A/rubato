# 0043. A blocked issue's progress carries the station it left from, derived from the same signals

- **Status:** proposed
- **Date:** 2026-11-13
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #106) — proposed; a human accepts
- **Seam:** web-server, companion-server
- **Issue:** m0x6A/rubato#106

## Context

`Progress` says *Blocked* but not where on the rail the issue stood when it blocked, so the web's
stage rail puts a fixed notch beside Started (`web/src/styles.css:476`, plan 81 A8). AGENTS.md
says progress is derived, never stored. `derive_progress` decides *Blocked* from two signals that
are still present while the issue is blocked: a live cap hit and a trusted blocked session
(`crates/rubato-protocol/src/progress.rs:85`, `:93`). One case those signals cannot tell apart: a
session blocked by an `idle_prompt` notification is waiting after it stopped, while one blocked by
`permission_prompt` or `StopFailure` stopped mid-turn (`crates/rubato-protocol/src/hook.rs:35`,
`:39`); the session snapshot says only `blocked`. Design: `docs/plans/106-blocked-from/design.md`.

## Decision

`Progress` gains `blocked_from: Option<ProgressState>`, absent unless `state` is `blocked`, and
then one of `started`, `designing`, `designed`, `building`, or absent for "off the rail". It is
computed by a pure function in `rubato-protocol` from the same `ProgressInputs` as the state, on
every overview build, and is written to no table. The rule: a spend cap hit, or a blocked session
blocked mid-turn, left from the working station (`building` with a plan file, `designing`
without); a session blocked while waiting (`idle_prompt`) left from the idle station (`designed`
with a plan, `started` without); an attempts cap hit (a refused Start) left from what the other
signals say without the cap (`designed` with a plan, `started` when launched, otherwise absent). A
blocked session whose cause is not known (an older companion, a tmux-ide-sourced status) counts
as mid-turn. To tell the two kinds of blocked session apart, `SessionState` gains `blocked_by:
Option<BlockCause>` (`permission_prompt`, `idle_prompt`, `stop_failure`), which the companion sets
from the hook that blocked the session and clears with the next status-changing hook. Both fields
are optional and additive in `schema/protocol/`.

## Alternatives considered

- **Store the state before *Blocked* in a table when the issue blocks**: progress stored by hand,
  against AGENTS.md; it also goes stale when the plan file lands later.
- **A `station` field on every issue's progress**: moves the web's state-to-station map to the
  server for all nine states; a larger wire change for one state's need.
- **Let the web infer it from `IssueSummary` (plan present → Building)**: a second derivation in
  TypeScript that can drift from the Rust one, and the web cannot see the cap kind.
- **Treat every blocked session as mid-turn, no `blocked_by`**: no companion change, but a session
  waiting for plan approval after writing its plan (the common wait) shows Building, one station
  late.
- **Read the blocking hook from `companion_events` on the server**: a first reader of an
  append-only, unpruned log, and tmux-ide-sourced blocks have no hook there.

## Consequences

The rail can show the station a blocked issue left from without anything stored. The derivation
gains one input kind (`CapKind` instead of a bool) and one session field. An issue whose plan file
lands while it is blocked moves its notch from Designing to Building; that is the truer station
now, and is accepted. The O-rows are in `docs/plans/106.md`: the server gauge
`rubato.issues.blocked_from` (web-server) and `rubato.session.blocked_by` on
`rubato.hook.forwarded` (companion-server), both on the dashboard panel "Blocked, by station left
from".

## Supersedes

None.
