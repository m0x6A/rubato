# 0068. Keep the By repo arrangement as a second shared layout next to the flat one

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** agent (delegated by Mattias, 2026-11-15), accepted with plan #144 after its design review passed; drafted by agent (Claude Code, issue #144 design session)
- **Seam:** web-server
- **Issue:** m0x6A/rubato#144

## Context

The **By repo** grouping (#89) boxes each repo's parts together. Its positions come only from
ELK and nothing in it can be moved, so when ELK gets it wrong nobody can fix it. ADR 0016 keeps
one shared position per part for the flat map; a grouped position is a different thing (relative
to a repo's box), and the issue forbids the grouping to overwrite the flat positions. Design:
`docs/plans/144-by-repo-layout/design.md` (D1).

## Decision

`rubato-server` keeps a second arrangement next to the flat layout, for the By repo grouping, in
a table `map_layout_grouped` owned by the `layout` slice: one position per part (relative to its
repo's box, or to the canvas for a part with no repo) and one position per repo box (its top-left
on the canvas, keyed by `owner/name`). `MapLayout` gains two optional maps, `grouped` and
`boxes`; `PUT /api/layout` stores whatever of the three maps it carries, all or none, with the
same rules as ADR 0016 (last write wins, ids of the merged map or the draft, positions on the
canvas; a box key must be a repo some part names). `DELETE /api/layout` still clears the flat
layout only; `DELETE /api/layout/grouped` clears the grouping only, each answering with what it
removed. The overview serves both. The grouping is presentation, like the flat layout: never an
input to progress, pins or plans, and never read for the flat map nor written by it.

## Alternatives considered

- **Derive the grouping from the flat layout** (keep each part's relative order, lay out inside the box): no new state, but a drag inside a box could only persist by writing the flat positions, which the issue forbids.
- **Keep the grouping in the browser for the current view only**: no shared picture, and the arrangement is lost on reload.
- **Store grouped positions as absolute canvas positions**: moving a box would have to rewrite every part in it.

## Consequences

The team can arrange both views and sees one arrangement of each. The `layout` slice owns a
second small table, bounded by parts plus repos and safe to lose. A part whose repo changes keeps
its relative position in its new box. `rubato.layout.writes` gains the attribute
`rubato.layout.arrangement`; covered by the `web-server` O-row of plan 144.

## Supersedes

None. It adds to 0016, which stays as written for the flat map.
