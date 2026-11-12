# Feature index — rubato

**Frozen as of #71 (ADR 0028).** New work is not added here. Rubato shows each issue's plan,
its `Review:` comment and its approval, derived from the plan file and the issue's comments
(ADR 0027), so a hand-kept status column would only drift. The rows below stay as a record.

Features that went through the feature workflow, newest first. Full track: a `design.md` in
`docs/plans/<N>-<slug>/` and the plan `docs/plans/<N>.md`. Small track: `docs/plans/<N>.md` alone. The status column mirrors the design's
`status:` frontmatter. Plans 1–7 and v0 predate this index.

| Feature | Track | Status | Date | What it changes |
|---|---|---|---|---|
| [#49 Fewer GitHub calls per tracked issue](plans/49-poll-cost/design.md) · [plan](plans/49.md) | full | implemented | 2026-11-12 | branches and open pull requests listed once per repo per tick, title and state from the open listing, plans re-read on a new head, comments and closed issues on a 300 s full lane; ADR 0023 accepted |
| [#57 Publish the dashboard and alert rules to Grafana on merge](plans/57.md) | small | implemented | 2026-11-11 | a Node publisher and a workflow send `observability/` to the Grafana stack on merge, read it back, and list queried metrics the stack has never stored |
| [#48 Make the system map interactive and link it to open issues](plans/48-interactive-map/design.md) · [plan](plans/48.md) | full | implemented | 2026-11-11 | a map that explains the system: flows, movable parts, all open issues pinned by drag, companion lanes with a queue, a plan lens, map edits proposed as a pull request; GitHub App access |
| [#31 Dashboard: show map staleness](plans/31.md) | small | implemented | 2026-11-10 | a `rubato.map.loaded_at` gauge and a "Map age" panel; read and parse failures counted |
| [#30 Companion: emit CapHit for the attempt limit](plans/30.md) | small | implemented | 2026-11-10 | an over-limit launch is a cap-hit event on the server and the companion; the issue shows Blocked unless a session is live |
| [#29 Write-only tables and the unpersisted companion seq](plans/29.md) | small | implemented | 2026-11-10 | the companion reserves sequence numbers in `seq.json`, so a restart is not mistaken for a replay; a replayed hello is counted |
| [#27 Companion launch arm: no block_in_place under the registry lock](plans/27.md) | small | implemented | 2026-11-10 | the launch runs in `spawn_blocking` with the registry lock released |
| [#26 Companion telemetry attributes match the alerts](plans/26.md) | small | implemented | 2026-11-10 | companion id and source on companion metrics, zeros for empty states, two spans; `SessionWorkingNoHooks` can fire |
| [#20 Refactor codebase to use vertical slices](plans/20-vertical-slices/design.md) · [plan](plans/20.md) | full | implemented | 2026-11-08 | server, companion, web and protocol reorganised into capability slices with a kernel and a composition root; skill and plan-check documented as already sliced |
