# 0037. Keep each issue's spend history on the server and carry it, downsampled, in the overview

- **Status:** proposed
- **Date:** 2026-11-13
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #103) — proposed; a human accepts
- **Seam:** web-server
- **Issue:** m0x6A/rubato#103

## Context

Issue #103 asks for each issue's spend over time as a sparkline beside the spend gauge (#81,
design §9 follow-up 1). Today the server keeps only the latest `Sessions` snapshot per companion
(`session_snapshots`, one row per companion, replaced on every snapshot) and the overview sums
the live sessions' `spend_usd` (`IssueSummary.spend_usd`). Nothing remembers an earlier value,
and a session the companion has swept leaves the sum. The web has no way to draw a line.
Design: `docs/plans/103-spend-sparkline/design.md`.

## Decision

`rubato-server` records spend as it arrives on the companion socket, in a new slice
`spend_history` that owns two tables: `spend_sessions` (the latest spend per session id, with its
issue) and `spend_points` (per issue and 15-minute bucket, the issue's total over every session
it has had, upserted). A replayed or older snapshot never lowers a stored value. The overview
carries, per issue, `IssueSummary.spend_history: Vec<SpendPoint { at, usd }>`, at most 24 points
over the issue's whole recorded life, oldest first, omitted when empty. The field is additive:
`spend_usd` keeps its meaning (the sum over live sessions), and a reader that ignores the new
field sees today's overview. History is an observation shown next to progress; it is never an
input to `derive_progress`.

## Alternatives considered

- **A new endpoint read when the issue panel opens**: one more route, cache and failure mode for
  about 24 numbers per issue; the overview is already refetched on every `Changed`.
- **Read cost from the team's telemetry backend** (Claude Code's own OTel cost metric): Rubato
  must not depend on the LGTM stack (AGENTS.md, Observability).
- **Store every snapshot's spend per session, unbucketed**: a snapshot arrives on every hook, so
  rows would grow with every tool call for no visible gain.
- **Keep history in the companion**: it lives on one laptop, sleeps with it, and a person's
  issue may run on several companions.

## Consequences

The overview grows by at most 24 points per tracked issue. Spend now survives a session leaving
the companion's snapshot, so the line's last point can be higher than the gauge after a session
ends; the web labels the line with its own last value. Rows grow with active 15-minute buckets
per issue; no retention in v0. A failed write is logged as a companion bookkeeping write and
never fails the envelope. O-rows: `web-server` and `companion-server` in plan 103, both on
the metric `rubato.spend.points` and the dashboard panel "Spend history writes".

## Supersedes

None.
