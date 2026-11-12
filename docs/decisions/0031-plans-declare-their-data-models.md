# 0031. A plan declares its data models as a parsed table

- **Status:** accepted
- **Date:** 2026-11-12
- **Decided by:** Mattias Jonsson, accepted 2026-11-12 (drafted by agent, Claude Code, claude-opus-5-5, issue #80)
- **Seam:** skill-plan-file, server-plan-file, plan-check-plan-file
- **Issue:** m0x6A/rubato#80

## Context

Mattias asked that a plan in Rubato shows where it fits, which parts it affects, and whether it
introduces new data models. Plans name parts, seams and flows in parsed tables (ADR 0009, 0021);
data models are written in prose if at all, so Rubato cannot show them and a reviewer has to dig.

## Decision

A plan may carry a `## Data models` table with the columns Model, Kind (`table`, `type`, `schema`,
`event`, `file`), Part (a map node id), Change (`new`, `changed`, `removed`), Crosses (a seam id,
optional) and Note. It is parsed to `Plan.data_models` with the same rules in Rust and TypeScript,
over shared fixtures. A plan without the section, or with "None.", parses to an empty list.
plan-check fails a plan whose model names an unknown part or seam, a part the plan does not list in
Affected parts, or a seam not touched there, and reports `data_models` on its `rubato.plan.check`
line. `plan.ts init` writes the section; the skill and the review checklist name it.

## Alternatives considered

- **Free text under Affected parts**: Rubato could not show it per part or seam, and nothing checks it.
- **A column in Affected parts**: one cell per part gets crowded and cannot say which seam a model crosses.

## Consequences

Plans get one more table to fill; the fit rule (ADR 0029) uses it. Older plans keep passing.
O-rows: skill-plan-file and plan-check-plan-file (`data_models` on the check line, shared fixtures).

## Supersedes

None. Extends 0009 and 0021.
