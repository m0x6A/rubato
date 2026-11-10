# Feature index — rubato

Features that went through the feature workflow, newest first. Full track: a `design.md` in
`docs/plans/<N>-<slug>/` and the plan `docs/plans/<N>.md`. Small track: `docs/plans/<N>.md` alone. The status column mirrors the design's
`status:` frontmatter. Plans 1–7 and v0 predate this index.

| Feature | Track | Status | Date | What it changes |
|---|---|---|---|---|
| [#31 Dashboard: show map staleness](plans/31.md) | small | implemented | 2026-11-10 | a `rubato.map.loaded_at` gauge and a "Map age" panel; read and parse failures counted |
| [#30 Companion: emit CapHit for the attempt limit](plans/30.md) | small | implemented | 2026-11-10 | an over-limit launch is a cap-hit event on the server and the companion; the issue shows Blocked unless a session is live |
| [#29 Write-only tables and the unpersisted companion seq](plans/29.md) | small | implemented | 2026-11-10 | the companion reserves sequence numbers in `seq.json`, so a restart is not mistaken for a replay; a replayed hello is counted |
| [#27 Companion launch arm: no block_in_place under the registry lock](plans/27.md) | small | implemented | 2026-11-10 | the launch runs in `spawn_blocking` with the registry lock released |
| [#26 Companion telemetry attributes match the alerts](plans/26.md) | small | implemented | 2026-11-10 | companion id and source on companion metrics, zeros for empty states, two spans; `SessionWorkingNoHooks` can fire |
| [#20 Refactor codebase to use vertical slices](plans/20-vertical-slices/design.md) · [plan](plans/20.md) | full | implemented | 2026-11-08 | server, companion, web and protocol reorganised into capability slices with a kernel and a composition root; skill and plan-check documented as already sliced |
