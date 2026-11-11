# 0016. Keep one shared map layout on the server

- **Status:** proposed
- **Date:** 2026-11-10
- **Decided by:** agent (Claude Code, issue #48 design session); proposed for Mattias Jonsson to accept
- **Seam:** web-server
- **Issue:** m0x6A/rubato#48

## Context

Issue #48 asks that parts of the map can be dragged to a new position. Positions are computed by ELK in the browser on every load today and stored nowhere. ADR 0008 and AGENTS.md describe pins as the one piece of state GitHub does not hold, so storing positions needs its own record. Design: `docs/plans/48-interactive-map/design.md` (D2).

## Decision

`rubato-server` stores one position per map part in a SQLite table `map_layout`, owned by a `layout` slice. `PUT /api/layout` writes the moved parts, `DELETE /api/layout` clears the layout and returns what it removed, and the layout travels to browsers inside the overview. The last write wins per part. The layout is presentation: it is never an input to progress, pins or plans, and a part with no stored position is placed by the automatic layout. A position may be stored for a part of the merged map or of the draft (ADR 0018); rows for other ids are ignored and deleted on the next write.

## Alternatives considered

- **Browser storage per user**: no backend change, but each person would see a different map, which is not a shared picture.
- **Positions in `system-map.yml`**: versioned and reviewable, but every drag becomes a pull request.

## Consequences

The team sees one arrangement and can move it together. The server now holds a second kind of state that GitHub does not; it is small, bounded by the number of parts, and safe to lose. Two people moving the same part at once: the later drop wins. Covered by the `web-server` O-row of plan 48 (`rubato.layout.writes`, `rubato.web.rejected`).

## Supersedes

None. It adds to 0008, which stays as written for pins.
