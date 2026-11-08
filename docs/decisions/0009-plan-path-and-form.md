# 0009. Place plans at docs/plans/<issue>.md, overridable per repo, with tables parsed to JSON

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (drafted by agent, Claude Code v0 planning session)
- **Seam:** skill-plan-file, server-plan-file, plan-check-plan-file
- **Issue:** none

## Context

AGENTS.md leaves the exact plan path inside connected repos open. Three readers (skill,
server, plan check) and one writer must agree on path and shape. PantryPilot, a likely
connected repo, keeps plans at `features/<issue>-<slug>/plan.md`.

## Decision

Default path `docs/plans/<issue>.md`; a repo may override it with `repos[].plan_path` in the
map, using `{issue}` and `{slug}` placeholders. The plan is Markdown with YAML frontmatter
holding only `issue`, `spend_cap_usd` and `attempt_limit`. The sections `## Affected parts`
and `## O-rows` are tables with fixed columns; both parsers convert them to JSON validated by
`schema/plan.schema.json`. Shared fixtures in `schema/fixtures/plans/` keep the Rust and
TypeScript parsers identical.

## Alternatives considered

- **Fixed path everywhere**: forces PantryPilot to move its plans.
- **All data in frontmatter**: duplicates the human table or removes it.
- **`.rubato/plans/`**: hides plans from people browsing docs.

## Consequences

Easier: human and machine read one table. Harder: two parsers of a Markdown table; O-rows
`server-plan-file` and `plan-check-plan-file` and the shared fixtures cover it.

## Supersedes

none
