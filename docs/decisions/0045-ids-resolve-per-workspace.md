# 0045. Resolve ids in every workspace whose map names the repo; a plan fits each one where all its ids resolve

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-14 by approval comment https://github.com/m0x6A/rubato/issues/89#issuecomment-6876029281 (self); drafted by agent (Claude Code, issue #89 design session)
- **Seam:** server-plan-file, skill-system-map, plan-check-plan-file
- **Issue:** m0x6A/rubato#89

## Context

With several workspaces (ADR 0044) one repo can be named by two maps, and ids are unique only
within a map (ADR 0007). A plan, pin or `Map:` line names bare ids and has no workspace. ADR
0029 counts an approval only while the plan fits *the map*. Design D3, D4.

## Decision

An issue's *workspaces* are those whose map names its repo (`repos_named`), plus those where it
is pinned, placed by a `Map:` line or launched; with one workspace configured, it is always that
one. A plan *fits workspace W* when ADR 0029's rules (ids, flows, O-rows, data models) hold
against W's map. Its approval counts when the plan fits **any** of the issue's workspaces; in a
workspace it does not fit, the issue shows "counts in <name>", not *does not fit*. Accept plan
refuses only when the plan fits none of them. Unknown-id findings and `ApprovedWithoutFit` are
raised only when the plan fits none of the issue's workspaces. Plan front matter does not change (ADR 0027). When
two maps give R different `plan_path` (ADR 0009), the first workspace in config order wins and the
server raises a `PlanPathConflict` finding. The skill offers the ids of every local map that names
the session's repo, and `plan.ts check`, `wait`, `sha` and `path` count a plan that fits any of them;
when those maps disagree on the plan path, the skill refuses rather than choose. plan-check passes
a plan that fits at least one of the maps it is given and reports which.

## Alternatives considered

- **Qualified ids (`ws:id`)**: breaks every existing plan, pin and `Map:` line.
- **A `workspace:` key in plan front matter**: supersedes 0027 for something the ids already tell.
- **One workspace per repo**: rules out the shared repo the issue is about.

## Consequences

Easier: nothing in existing plans changes. Harder: the same id meaning two things in two maps of
a shared repo; Rubato counts such ids as `rubato.workspace.id_collisions` and shows a finding, never
refuses. Covered by the server-plan-file O-row (`rubato.plans.fit_none`, log `plan fit`).

## Supersedes

0029's "fits the map" becomes "fits a workspace of its repo"; amends 0007 ("existing ids" means
existing in a map that names the repo).
