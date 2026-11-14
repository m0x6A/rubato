# 0060. Keep dismissed traffic discrepancies per workspace on the server

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-14 by approval comment https://github.com/m0x6A/rubato/issues/140#issuecomment-6882263766 (self); drafted by Mattias Jonsson (#140 Q4, 2026-11-14); drafted by agent (Claude Code, claude-opus-5-5, issue #140) — proposed
- **Seam:** web-server
- **Issue:** m0x6A/rubato#140

## Context

Some discrepancies between the map and the territory are intended, for example a test client or
a one-off job. #140 lets a person take a discrepancy into the map draft or dismiss it. Mattias
Jonsson chose to keep dismissals as a per-workspace list on the server, over not keeping them or
keeping an ignore list in `system-map.yml`.

Design: `docs/plans/140-observed-system/design.md` §5.

## Decision

**Storage.** `rubato-server` keeps dismissals in a SQLite table `traffic_dismissals`, owned by
slice `traffic`. Each row is one `DiscrepancyKey` per workspace (a ghost's service, an edge
between two ends, or a quiet seam), at most 500 per workspace, with the author (the logged-in
GitHub login) and the time.

**Routes.** `POST /api/traffic/dismissals` dismisses (idempotent per key) and
`DELETE /api/traffic/dismissals/{id}` undoes. Both are map-scoped (ADR 0049) and both notify the
workspace.

**Effect.** `derive_traffic` hides a dismissed key from the drawn overlay and from the findings,
and always lists the dismissals so the web can show who dismissed what and offer Undo.

**What a dismissal is not.**

- It is never written to the map file.
- It is never an input to progress, pins, plans or approvals.
- It never hides an unknown state, because nothing is drawn then anyway.

## Alternatives considered

- **Dismissals not kept**: the discrepancy would come back on every reload, and intended
  discrepancies would drown out real ones.
- **An ignore list in `system-map.yml`, through the draft**: versioned and reviewed, but it needs
  a schema change, a new draft op and a PR to silence a test client.

## Consequences

- **Outside the map.** Dismissals are team state outside the versioned map. Anyone on the team
  can dismiss or undo, and the author is shown and logged (`traffic dismissal added` /
  `removed`).
- **O-row.** The `web-server` O-row of plan #140 covers it: `rubato.traffic.dismissals` per
  workspace and `rubato.web.rejected` on refused writes.

## Supersedes

None.
