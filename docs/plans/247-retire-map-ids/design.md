# Design: retire a map id (#247)

Plan: [../247.md](../247.md) · ADR: [0077](../../decisions/0077-a-map-id-is-retired-not-removed.md),
proposed.

## 1. The problem

ADR 0007: a plan may name only ids the map declares. The plan check runs every plan in
`docs/plans/` against today's map (`.github/workflows/ci.yml`, step "Plan check on this repo's
own plans"). Ten merged plans name `tmux-ide` or `companion-tmux-ide`: 4, 7, 20, 26, 27, 40, 41,
147, 155 and v0. So #147 (ADR 0070) kept both ids and wrote "retired" in their descriptions.

Every reader of the map's ids, and what it should do with a retired one:

| Reader | Where | Today | After |
| --- | --- | --- | --- |
| plan check | `actions/plan-check/src/check.ts:145` (`mapIds`) | knows it, so history passes | unchanged, and the id is reported |
| server plan fit | `rubato-protocol` `plan_fit` (`plan.rs:404`) | knows it, so a plan naming one fits | unchanged |
| skill | `skills/rubato/scripts/map.ts:103` | offers it to every session | lists it under "Retired (do not use)" |
| web map | `web/src/features/map/graph.ts:77-78` (`buildGraph`) | draws it | not drawn |
| Plans tab minimap | `web/src/features/plans/MiniMap.tsx:12-72` | draws it | not drawn |
| issue-detail system strip | `web/src/features/plans/WhereItFits.tsx:48-56` | draws it | not drawn |
| pin menu | `web/src/features/pins/PinMenu.tsx:102-109` | offers it | not offered |
| new-issue form | `web/src/features/new_issue/NewIssueForm.tsx:24-140`, default at `App.tsx:448` | offers it | not offered; the default is the first live node |
| map draft form | `web/src/features/drafts/DraftPanel.tsx:149-185,464-475` | offers it as a seam end; rebuilds a node or seam from the form | not offered as a seam end; an edit keeps `retired` |
| repo panel | `web/src/features/repos/provides.ts:21-23` | lists it under what a repo provides | not listed |
| draft proposal | `crates/rubato-server/src/map_draft/yaml.rs:25-28` (`NODE`, `SEAM` keys) | `retired` would be an unknown key, and a proposal refused | a known key, round-tripped |
| pin suggestions | `crates/rubato-server/src/github_poll/suggest.rs:132-185` (ADR 0026) | scores every id | skips retired ids |
| traffic match | `crates/rubato-protocol/src/traffic.rs:492-520` | matches a node by `service` | a retired node may not have a `service` (§3) |
| id lookups | `territory.ts:115` (taken ids), `explain.ts:81-90` | raw | raw on purpose: a retired id is still taken, and a pin on one still explains |

## 2. Options (Q1)

1. **`retired: "<ADR>"` on a node or seam (recommended).**
   - The id stays declared.
   - Readers that draw, offer or suggest ids skip it, and the plan check reports it.
   - No map history is needed, so it works the same in every repo.
2. **Each plan checked against the map at its own `map_version`.**
   - The plan check loads `git show <map_version>:<map path>` per plan, with `fetch-depth: 0` in
     CI.
   - A connected repo's map lives in another repo, so its history is not in the checkout: there
     the check falls back to today's map and history fails again.
   - Plans written before `map_version` existed have none.
3. **Delete the ids and skip the plans of closed issues:** the plan check is offline (no
   GitHub), and closing an issue would change what passes.
4. **Leave it (status quo since #147):** the ids stay drawn, offered and suggested, only
   described as retired.

The issue's own boxes assume the ids leave the map (options 2 or 3). Option 1 keeps them
declared, so the plan says which boxes it replaces (plan, "Against the issue").

## 3. The field and the map rules

**The field.** `MapNode.retired: Option<String>` and `Seam.retired: Option<String>` in
`crates/rubato-protocol/src/map.rs`:
- the ADR number that retired the id, four digits (schemars pattern `^[0-9]{4}$`);
- serde default, skipped when absent, so a map without it reads as before;
- helpers `MapNode::is_retired`, `Seam::is_retired` and `SystemMap::live_ids()`, so the server's
  readers share one rule;
- schemars generates `schema/system-map.schema.json` and the protocol schemas that embed the map
  (`system_map`, `map_response`, `overview`, `draft_op_request`);
- `web/src/protocol.ts` is regenerated.

**Three new map rules**, in `SystemMap::validate` (`map.rs:358`) and in its TS mirror
`structuralProblems` (`actions/plan-check/src/map.ts:99-158`):
1. a retired seam declares no `flows`. A scenario step over it is then already refused, since
   every step must be exactly one declared flow (`map.rs:435-437`);
2. a live seam does not end at a retired node. This keeps a drawn edge from pointing at an
   undrawn node: ELK throws on that (`graph.ts:226,291`), and the map tab would go blank;
3. a retired node has no `service`, so observed traffic is never matched to it.

**Parity.** Both validators run one shared fixture set,
`schema/fixtures/maps/retired-*.yml`: one valid map and one map per rule broken. A test on
each side asserts the same verdict per file, so the two cannot drift.

## 4. Readers

**One helper per language:**
- the server uses `SystemMap::live_ids()` and the `is_retired` helpers;
- the web uses `liveMap(map)` in `web/src/shared/map.ts`, which drops retired nodes and seams.

Every reader in §1 that draws, offers or suggests goes through one of them. The raw lookups
(`territory.ts`, `explain.ts`) stay raw, and say so in a comment.

**Plan check:**
- `mapIds` is unchanged.
- `retiredIds(map)` reports, per plan, the retired ids the plan names, as
  `rubato.plan.retired_ids` in the `rubato.plan.check` line, with the constant
  `PLAN_RETIRED_IDS` in `otel.rs` (`otel-names.test.ts` checks every `rubato.*` literal).
- The summary line ends "… N plan(s) name retired ids, not failed".

**Skill:** `map.ts` leaves retired ids out of the Nodes and Seams lists, and adds "Retired (do
not use)" with each id and its ADR. Its header line gains counts:
`12 nodes, 13 seams, 2 retired` (live counts, then retired).

**Map draft:**
- `yaml.rs` knows `retired` on nodes and seams, so a proposal round-trips it (the
  `round_trip_keeps_the_committed_maps` test reads `rubato-map.yml`).
- `DraftPanel` keeps the selected part's `retired` when it builds the `UpdateNode` or
  `UpdateSeam` op, and never offers a retired node as a seam end.

**Pins:**
- An issue pinned only to a retired id is not drawn on the map. It stays in the Issues list, and
  the issue detail names the pin with "(retired)".
- `suggest.rs` never suggests a retired id.

## 5. The map

`docs/design/rubato-map.yml` gains `retired: "0070"` on `tmux-ide` and on `companion-tmux-ide`.
Their descriptions keep saying why. The plan-check test that exempts `companion-tmux-ide` by name
(`actions/plan-check/test/plan-check.test.ts:122`, #250) changes to exempt any seam with
`retired`.

## 6. Tests

- **Protocol:**
  - a map with `retired` round-trips;
  - a value that is not four digits is refused by the schema;
  - each of the three rules refuses its fixture;
  - `plan_fit` counts a plan naming a retired id as fitting (a case beside
    `a_plan_that_names_the_map_fits`).
- **Plan check:**
  - the parity test over `schema/fixtures/maps/retired-*.yml`;
  - a fixture plan naming a retired id passes, with `retired_ids` set and counted;
  - the dogfood test passes and counts 10.
- **Skill:** `map.ts` on the real map has no retired id under Nodes or Seams, has the Retired
  section, and has the counts.
- **Server:**
  - the `yaml.rs` round trip with `retired`;
  - `suggest.rs` with an issue body that names `tmux-ide` suggests nothing retired;
  - the workspaces golden is re-recorded, failing first.
- **Web:**
  - `graph.test.ts`, `MiniMap`, `WhereItFits`, `PinMenu`, `NewIssueForm` (including the default)
    and `provides` tests: no retired id drawn or offered;
  - `DraftPanel`: an edit of a retired node posts an op that keeps `retired`;
  - the issue detail shows a retired pin with "(retired)".
- **Web e2e:**
  - `@map.every-part` and `@map.by-repo` (`map.spec.ts:31-52`) iterate over live parts only, and
    assert `node-tmux-ide` and the `companion-tmux-ide` edge have count 0;
  - the map draft proposal test still opens its PR.

## 7. What it does not do

It deletes no id and edits no merged plan. It does not fail a new plan that names a retired id;
that stays a reported line, with the skill not offering the id as the first guard.
