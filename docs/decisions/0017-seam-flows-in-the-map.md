# 0017. Declare data flow and control flow as flows on a seam

- **Status:** proposed
- **Date:** 2026-11-10
- **Decided by:** agent (Claude Code, issue #48 design session); proposed for Mattias Jonsson to accept
- **Seam:** server-system-map, skill-system-map
- **Issue:** m0x6A/rubato#48

## Context

The map's seams have a direction (`from → to`) that means "depends on", and a free-text `carries`. Issue #48 asks the map to show data flow and control flow. Over most seams data runs both ways (the companion sends events, the server sends launch commands), so one arrow cannot say it. Design: `docs/plans/48-interactive-map/design.md` (D3).

## Decision

A seam in `system-map.yml` may carry `flows`: a list of `{ kind: data | control, from, to, carries }`. `from` and `to` must be the seam's two nodes, in either order; validation fails the map otherwise. A seam may also carry a `description`, as nodes already do. Both fields are optional and the map `version` stays 1: a map without flows is valid and readers that do not know the field ignore it. A flow has no id; pins, plans and O-rows keep naming the seam.

## Alternatives considered

- **One `kind` per seam**: cannot show two directions or both kinds on one seam.
- **Flows as top-level edges with their own ids**: new ids for pins and plans to name, and a second place where two nodes are connected.
- **Deriving flows from plans only**: the standing picture would be empty until a plan is open.

## Consequences

The map can be read as structure, as data flow or as control flow. `schema/system-map.schema.json`, the plan check's map type and the skill's `map.ts` gain the field; connected repos pick it up at their next skill install and keep working until then. A map with a flow off its seam fails to load and the previous map stays in use, covered by the `server-system-map` O-row ("Map age", `rubato.map.load_failures`).

## Supersedes

None. It extends 0007 without changing it.
