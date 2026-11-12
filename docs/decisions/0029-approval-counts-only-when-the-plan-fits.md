# 0029. An approval counts only while the plan fits the map

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #80), from Mattias Jonsson's answer in the planning session; a human accepts
- **Seam:** server-plan-file, web-server
- **Issue:** m0x6A/rubato#80

## Context

Issue #80 asks that every proposed plan show where it fits in the system architecture before it can
be accepted. Approval is a person's `Approved: plan <sha>` comment (ADR 0027), and Rubato cannot stop
anyone posting one. The checks that say whether a plan fits exist today only as global findings
(`UnknownIds`, `FlowOffSeam`, `PlanParseError`) and in CI; nothing joins them to the approval.

## Decision

Rubato derives a fit per plan. A plan **fits** when it parses, every node and seam id it names is in
the map, every flow runs on a declared seam that joins its two parts, every seam in Affected parts
has an O-row, and every data model (ADR 0031) sits on a part the plan names and crosses only seams
the plan touches. An `Approved:` comment that names the current plan counts only while the plan
fits; otherwise the approval is shown as "approved, doesn't fit, not counted" (approval version
`DoesNotFit`) and raises the finding `ApprovedWithoutFit`. When the plan is fixed so that it fits
and its sha is unchanged (only possible if the fix was in the map), the approval counts again; any
edit to the plan changes its sha and needs a new approval, as in ADR 0027. Rubato's own *Accept
plan* (ADR 0030) refuses to post for a plan that does not fit. Fit and approval are badges, not
progress states.

## Alternatives considered

- **Show the fit next to the approval, count the approval anyway**: the issue says a plan cannot be accepted until its fit is shown; a warning nobody has to read does not meet that.
- **Also fail the plan-check Action while the plan does not fit**: plan-check already fails on unknown ids and missing O-rows; the extra rule is the approval, which lives in comments CI does not read.

## Consequences

A plan approved before a map change that removes one of its ids stops counting until the plan is
updated. Approvals of older plans that do not meet the O-row rule stop counting; the plans in this
repo pass plan-check, so none do today. O-row: server-plan-file (`rubato.plan.fit{outcome}`,
`ApprovedWithoutFit`).

## Supersedes

0027 on when an approval counts. The rest of 0027 (the comment forms, the sha, trusted authors)
stands.
