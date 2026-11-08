# 0007. The map is a versioned file with ids; plans may only name existing ids

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** skill-system-map, server-system-map
- **Issue:** none

## Context

Pins, plans, sessions and findings all need to agree on what a node is. If ids are invented
per plan, nothing joins.

## Decision

`system-map.yml` declares nodes (context, contract, external) and seams, each with a stable
id, validated by `schema/system-map.schema.json`. Pins, plan affected-parts and O-rows may
only reference those ids. The plan check fails a PR that names an unknown id. The map's git
sha travels as `rubato.map.version`.

## Alternatives considered

- **Free-text node names**: no join, no check.
- **Map inferred from repos**: the territory, not the map; drift findings need both.

## Consequences

Easier: one id space for everything. Harder: adding a node is a PR to the map repo before
the plan can name it; that is intended.

## Supersedes

none
