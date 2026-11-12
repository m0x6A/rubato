# 0027. Plan review and approval are issue comments that Rubato derives and shows

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** proposed by agent (Claude Code, claude-opus-5-5, issue #71); awaiting Mattias Jonsson
- **Seam:** server-github, server-plan-file, skill-plan-file
- **Issue:** m0x6A/rubato#71

## Context

Since #20 plans in this repo carry `track`, `review`, `approval` and `status` in front matter,
written by the feature-workflow skills. Nothing reads or checks them, values have drifted, and they
contradict ADR 0009's three-key rule. Mattias asked that review and approval are not published in the
plan files but watched in Rubato. Decisions already travel as `Decision:` comments (ADR 0014) that
the poll reads.

## Decision

The planning workflow's review step posts a `Review: <track> — <outcome>` comment on the issue. A
person approves by commenting `Approved: plan <sha>` or `Approved: plan <sha> on <branch>`.
The sha is the git blob sha of the plan file with every task checkbox normalised (`[x]` → `[ ]`),
so ticking tasks keeps an approval current and any other edit does not; `plan.ts sha` prints it.
Both comments count only when their author is not a Bot and has association OWNER, MEMBER or
COLLABORATOR; the repo is public. Rubato derives "reviewed" and "approved" per issue from these
comments and shows them on the issue, its parts and the overview. It compares an approval with the
plan on the named branch, or on the branch it reads the plan from, and marks an approval of another
version as earlier (finding `ApprovalStale`, gauge `rubato.plans.approval_stale`). Approval is a badge, not a progress state. Plan front matter holds
`issue`, `title`, `spend_cap_usd`, `attempt_limit` and optionally `map_version`. Existing plans
stay as they are; plan-check counts their legacy keys and does not fail on them.

## Alternatives considered

- **Keep the keys in front matter and teach plan-check them**: the record would still be hand-written and could lie; Mattias asked for it to be watched in Rubato.
- **GitHub PR reviews as approval**: plans are approved before code, often before a PR exists.

## Consequences

Review and approval arrive within one full poll lane (300 s, ADR 0023). An agent with a team
member's `gh` could post `Approved:`; the skill never does, and that is a rule, not a guarantee.
O-rows: server-plan-file (`ApprovalStale`), skill-plan-file (`legacy_keys` on the
`rubato.plan.check` line).

## Supersedes

0009's front matter key list. The rest of 0009 (path and form) stands.
