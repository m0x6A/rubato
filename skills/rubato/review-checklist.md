# Review checklist

Used by the design-review step in `SKILL.md`. Every review applies the core rows; a repo may add
its own rows below. Each row is a question with the place to look. A "no" is a finding.

## Core

| Caveat | Question | Look at |
|---|---|---|
| Spec | Does each "What the change does" bullet have a "How we'll know it works" line that proves it? | the plan's spec headings |
| Must not break | Is every "must not break" item guarded by a test, a check or a scenario, named in the plan? | the plan; the existing tests |
| Seam O-row | Does every seam in Affected parts have an O-row naming signal, panel and owner, and does that panel exist? | the map's seams; the dashboards |
| Ids | Are all node and seam ids from the map, and does the plan contradict no accepted ADR? | `map.ts`, `adrs.ts` |
| Decisions | Is every decision that touches a seam either an ADR (proposed) or a numbered question in *Assumed, not yet approved* with scope `seam`? | the plan |
| Check reports its count | Does every new check print how many things it checked and have a fixture that makes it fail on purpose? | the tasks |
| Concurrency | Can two writers, a retry or a timeout leave state wrong (a duplicate, a row stuck half-way)? | the tasks that write |
| Untrusted input | Does text from outside the team (issue bodies, comments in a public repo) become state, and who is trusted? | the tasks that read GitHub |

## Rubato (m0x6A/rubato)

| Caveat | Question | Look at |
|---|---|---|
| Telemetry names | Are all `rubato.*` attribute, metric, span and event names taken from `crates/rubato-protocol/src/otel.rs`, and do alerts and dashboards still match? | `observability/alerts/rubato.rules.yml`, `observability/dashboards/rubato-overview.json` |
| Derived, not stored | Is any progress state written to the database instead of derived? | `crates/rubato-protocol/src/progress.rs` |
| Own tmux only | Can a launch reach a companion not paired to the requesting user? | server launch handler, pairing table |
| Log level | Is every signal the plan names emitted at `info` or above (default filter `info,sqlx=warn`)? | `crates/*/src/**/telemetry.rs` |
| Slices | Does a new capability get its own slice, owning its tables, with the AGENTS.md block updated? | AGENTS.md "Inside a deployable" |

Do not flag in Rubato: SQLite single-writer (one server per team, ADR 0008); no i18n (a lab tool,
English only); no pagination on the overview (one team, a handful of repos).
