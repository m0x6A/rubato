# 0028. One planning workflow in this repo: the rubato skill

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** proposed by agent (Claude Code, claude-opus-5-5, issue #71); awaiting Mattias Jonsson
- **Seam:** skill-plan-file
- **Issue:** m0x6A/rubato#71

## Context

Two workflows plan work here: the rubato skill (map, affected parts, O-rows, flows) and the
feature-workflow plugin through `.claude/profiles/feature-workflow.md` (tracks, spec headings,
design review). Both answer "plan this", both write `docs/plans/<N>.md`, and the plans since #20 mix
both sets of sections. Connected repos only get the rubato skill.

## Decision

The rubato skill is the one planning workflow in this repo. It takes over track sizing (trivial,
small, full, with this repo's triggers), the design-review step and the spec headings (What the
change does, How we'll know it works, What it must not break, How we'll debug it at 3 am), which
match the in-app issue form. Review and approval go to issue comments (ADR 0027).
`.claude/profiles/feature-workflow.md` is removed; its caveat rows move to
`skills/rubato/review-checklist.md`. The plugin itself is not changed. `docs/FEATURES.md` is frozen;
Rubato shows plan, review and approval per issue.

## Alternatives considered

- **Tie the rubato skill to the plugin**: connected repos without the plugin would get half a workflow.
- **Retire the plugin's ideas entirely**: loses track sizing and the design review that caught real blockers in #40–#49.

## Consequences

One trigger, one file layout, one set of headings from issue to plan. The skill grows. O-row:
skill-plan-file (skill tests on the template; `legacy_keys` on the `rubato.plan.check` line).

## Supersedes

None.
