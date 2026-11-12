# 0024. Declare scenarios in the map as ordered steps over declared flows

- **Status:** accepted
- **Date:** 2026-11-12
- **Decided by:** Mattias Jonsson, accepted 2026-11-12 (drafted by agent, Claude Code, claude-opus-5-5, issue #71)
- **Seam:** server-system-map, skill-system-map
- **Issue:** m0x6A/rubato#71

## Context

#71 asks for a view "with moving parts, seeing data flow from this part to another", showing control
flows as well as data flows. The map already declares flows per seam (ADR 0017), but a flow is a
static fact: nothing says in what order they happen when someone uses the system. Without an order
there is nothing to play. Design: `docs/plans/71-flow-view-and-issues/design.md` §2.

## Decision

A map may carry an optional `scenarios` list. Each scenario is `{id, name, description?, steps[]}`
and each step is `{seam, from, to, kind, carries?, says?}`. A step must match exactly one flow
declared on that seam: same `kind`, `from` and `to`, and same `carries` when the step gives it. A
seam can declare two flows with the same kind and direction (companion-server has two control flows
from the server), so a step that matches more than one fails as ambiguous until it names `carries`.
A step that matches none, an ambiguous step, a step on an unknown seam and a scenario with no steps
fail the map load, as an off-seam flow does. Scenario ids share one namespace with node and
seam ids, so they are unique across all three, but they are not map refs: issues cannot be pinned
to them and plans cannot name them. Map `version` stays 1. Steps need not be contiguous.

## Alternatives considered

- **Animate every declared flow continuously**: no order and no story; it does not say what happens when.
- **Give flows ids and let steps name them**: rejected for ADR 0017's reason; `(seam, kind, from, to)`, with `carries` when that is ambiguous, identifies the flow.
- **Live traffic from telemetry**: map versus territory is still out of scope; connected repos would need traces per seam.

## Consequences

The web can play a scenario step by step. A draft that removes a seam or flow a scenario uses is
refused, naming the scenario. Two validators (Rust and plan-check's TypeScript) must agree; shared
fixtures under `schema/fixtures/maps/scenarios*.yml` keep them in step. O-rows: server-system-map
(`rubato.map.load_failures`, `map invalid` with `ScenarioStepNoFlow`), skill-system-map (the
`rubato.plan.check` line).

## Supersedes

None. Extends 0017.
