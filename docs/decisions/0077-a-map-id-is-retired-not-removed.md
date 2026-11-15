# 0077. A map id is retired, not removed: drawn and offered nowhere, and history still checks

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#247), asked by Mattias Jonsson; a human accepts
- **Seam:** skill-system-map, server-system-map, web-server, plan-check-plan-file
- **Issue:** m0x6A/rubato#247

## Context

ADR 0070 retired tmux-ide. The map kept the `tmux-ide` node and the `companion-tmux-ide` seam,
with descriptions that say they are retired, for one reason:
- nine merged plans name the seam;
- ADR 0007 says a plan may name only ids the map declares;
- the plan check runs every plan in `docs/plans/` against today's map.

So today a retired id is still drawn on the shared map and still offered to every session by
`map.ts`, as if it were live. The next part the team retires has the same problem. Issue #247
asks how to retire an id for good.

## Decision

**The field.** A node or a seam may carry `retired: "<ADR number>"`, the ADR that retired it.
The field is optional and additive in `schema/system-map.schema.json` and in `rubato-protocol`'s
`MapNode` and `Seam`.

**What a retired id still is.** It stays declared, so ADR 0007 holds: pins, plans and O-rows that
named it still resolve.

**What every reader does with it:**
- **The plan check** counts it as known and does not fail a plan for naming it. It reports it in
  `rubato.plan.retired_ids` and counts it in its summary line.
- **The skill** (`map.ts`) lists retired ids under a heading of their own, "Retired (do not
  use)", not among the ids a plan may name.
- **The web** draws no retired node and no retired seam. It offers neither anywhere it offers
  ids. An existing pin on a retired id stays, and the issue detail marks it.
- **The server:**
  - suggests no retired id for an issue (ADR 0026);
  - round-trips the field in a map draft proposal (ADR 0018);
  - counts a plan that names a retired id as fitting, as today (ADR 0029).

**The map stays consistent.** Map validation, in the server and in the plan check alike, refuses:
- a retired seam with flows;
- a live seam that ends at a retired node;
- a retired node with a `service`.

The tmux-ide node and the companion-tmux-ide seam are the first ids marked `retired: "0070"`.

## Alternatives considered

- **Check each plan against the map at its own `map_version`:** needs the map repo's history at
  check time. CI and connected repos check out one commit, and a connected repo's map lives in
  another repo. History would then check only where history happens to be present.
- **Skip the plans of closed issues:** the plan check runs offline, with no GitHub access, and
  closing an issue would change what passes.
- **Delete the ids and edit the merged plans:** rewrites approved records.
- **Keep the ids as they are, only described as retired (today):** every session is still
  offered them, and the map still draws a part nobody runs.

## Consequences

**Easier:**
- Retiring a part is one map edit, `retired: "<ADR>"`.
- History keeps checking.
- The drawn map and the offered ids show only what is live.

**Harder:**
- A new plan can still name a retired id without failing. Only the skill not offering it, and the
  plan check's `rubato.plan.retired_ids` line, stand in the way.

**Watching:** the plan check's `retired_ids` count, and the O-rows on `skill-system-map` and
`web-server`.

## Supersedes

None. 0007 holds: a retired id is still an id the map declares.
