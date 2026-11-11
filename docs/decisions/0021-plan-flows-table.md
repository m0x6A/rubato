# 0021. Let a plan carry its flows as a parsed table

- **Status:** accepted
- **Date:** 2026-11-10
- **Decided by:** Mattias Jonsson, accepted 2026-11-11 (drafted by agent, Claude Code issue #48 design session)
- **Seam:** skill-plan-file, server-plan-file, plan-check-plan-file
- **Issue:** m0x6A/rubato#48

## Context

ADR 0009 fixes the plan's parsed sections: `## Affected parts`, `## O-rows` and `## Assumed, not yet approved`. The skill also asks for a Mermaid "Data flow" section, which no parser reads, so a plan's data flow never reaches the shared map. Issue #48 asks that session plans are brought into the picture. Design: `docs/plans/48-interactive-map/design.md` (D4).

## Decision

A plan may have a section `## Flows` with a table of fixed columns `From | To | Seam | Kind | Carries`, where `Kind` is `data` or `control`. Both parsers convert it to `flows` in the plan JSON, validated by `schema/plan.schema.json`, with a shared fixture. `From`, `To` and `Seam` must be map ids and the seam must join the two nodes; the plan check fails otherwise and reports `flows_checked`. The section is optional: plans without it parse and pass as before. Everything else in 0009 stands.

## Alternatives considered

- **Parsing the Mermaid block**: free-form, two parsers to keep identical, and edge labels do not name seams.
- **A submit-plan endpoint**: against 0005.

## Consequences

A plan's proposed flows can be drawn on the map next to the declared ones. The skill's template and `SKILL.md` gain the section; connected repos get it at their next install. A table the two parsers read differently is caught by the shared fixture; a wrong row is covered by the `plan-check-plan-file` and `server-plan-file` O-rows (`rubato.plan.check` with `flows_checked`, the flow-off-seam finding).

## Supersedes

None. It extends 0009 without changing its decision.
