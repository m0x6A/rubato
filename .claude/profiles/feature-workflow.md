# Feature workflow profile — rubato

Read by the `start-change`, `change-design` and `feature-design` skills and the `design-reviewer` agent.

## Layout
- Features directory: `docs/plans/` · central index: `docs/FEATURES.md` (not under `docs/plans/`, which CI parses as plans)
- Full track: `docs/plans/<N>-<slug>/design.md`; the implementation plan is `docs/plans/<N>.md` in the
  house format required by AGENTS.md and ADR 0009 (frontmatter `issue`, `spend_cap_usd`, `attempt_limit`;
  `## Affected parts`, `## O-rows`, `## Assumed, not yet approved` tables parsed by `schema/plan.schema.json`),
  extended with the plan template's §0 spec at a glance, per-project task sections and §6 verification.
- Small track: `docs/plans/<N>.md` holds the change.md content plus the house tables.
- Per-project history file: none (git history and `docs/plans/` are the record)
- Living specs: none; AGENTS.md and `docs/design/rubato-map.yml` are the standing description
- ADRs: `docs/decisions/NNNN-<slug>.md` · template `docs/decisions/0000-template.md` · next free number = highest + 1 (`ls docs/decisions`)
- System base diagram: `.claude/profiles/system-base.mmd` (derived from `docs/design/rubato-map.yml`; regenerate when the map changes)
- System map with the ids plans may use: `docs/design/rubato-map.yml`. Affected parts and O-rows name map node and seam ids, never invented ones.

## Tracker
- Issues: GitHub (`gh issue view <N> --repo m0x6A/rubato --json number,title,body,labels,comments`)
- Labels: `enhancement`, `bug`; no area labels yet
- Board: none

## Tracks (read by start-change)
- Branches: small and full `issue/<N>-<slug>` (AGENTS.md) · trivial `trivial/<slug>`
- Trivial threshold: ≤ 100 changed lines in ≤ 2 parts, issue referenced, no behaviour change
- Small: everything neither trivial nor full
- Full-track triggers: the start-change defaults, plus:
  - any change to `crates/rubato-protocol` that alters `schema/protocol/*.json` (seam change per AGENTS.md)
  - any change to `schema/plan.schema.json` or `schema/system-map.schema.json`
  - any change to a seam listed in `docs/design/rubato-map.yml`
  - remove: "migration" as an automatic trigger when the migration is additive and inside rubato-server (SQLite is one node); non-additive stays full

## Deployables
- Source of truth for names: node ids in `docs/design/rubato-map.yml` (`rubato-protocol`, `rubato-companion`, `rubato-server`, `web`, `skill-rubato`, `plan-check`)
- Fan-out tool for shared contracts: `cargo test -p rubato-protocol` (schema snapshot) then `cd web && pnpm gen:types && git diff --exit-code -- src/protocol.ts`
- Version bump rule: none (workspace version 0.1.0 until v1)

## Caveat packs (references/caveats.md)
- core (always) · web-frontend (web/) · data (rubato-server SQLite)
- Project rows:
  | Caveat | Question | Look at |
  |---|---|---|
  | Seam O-row | Does every seam in Affected parts have an O-row naming signal, panel and owner? | `docs/design/rubato-map.yml` seams; `actions/plan-check` |
  | Telemetry names | Are all `rubato.*` attribute, metric, span and event names taken from `crates/rubato-protocol/src/otel.rs`, and do alerts and dashboards still match? | `observability/alerts/rubato.rules.yml`, `observability/dashboards/rubato-overview.json` |
  | Derived, not stored | Is any progress state written to the database instead of derived? | `crates/rubato-protocol/src/progress.rs` |
  | Own tmux only | Can a launch reach a companion not paired to the requesting user? | server launch handler, pairing table |
  | Check reports its count | Does every new check print how many things it checked and have a must-fail fixture? | `actions/plan-check`, `observability/check.ts`, Gherkin suites |
  | Log level | Is every signal named in §2.4 emitted at `info` or above (default filter `info,sqlx=warn`)? | `crates/*/src/**/telemetry.rs` |

## Rules the reviewer applies
- Architecture rules: AGENTS.md ("Conventions", "Don't"), ADRs 0001–0015 in `docs/decisions/`
- Do not flag: SQLite single-writer (one server instance per team by design, ADR 0008); no i18n (lab tool, English only, v0); no pagination on overview (one team, a handful of repos)

## Enforcement
- CI: `pnpm plan-check --map docs/design/rubato-map.yml --plans "docs/plans/*.md"` fails on unknown ids or missing O-rows; no frontmatter `status`/`review` hook yet
