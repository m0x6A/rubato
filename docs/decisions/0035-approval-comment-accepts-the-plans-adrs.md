# 0035. A team member's approval comment accepts the ADRs its plan proposes; the session waits for it

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #90), asked for by Mattias Jonsson 2026-11-12 ("a comment on an issue from a teammember IS acceptance"); a human accepts
- **Seam:** skill-plan-file
- **Issue:** m0x6A/rubato#90

## Context

AGENTS.md says an agent may draft an ADR as *proposed* and only a human moves it to *accepted*. In
practice the human's act was a separate edit after approving the plan, so a plan that proposed ADRs
needed two steps, and the planning session stopped and waited for someone to tell it the plan was
approved. Mattias asked that waiting be automatic, part of the rubato skill, and said that a comment
on the issue from a team member is the acceptance.

## Decision

A trusted `Approved: plan <sha>` comment on an issue (ADR 0027: author OWNER, MEMBER or
COLLABORATOR, not a Bot) that names the current plan's sha approves the plan **and accepts every
ADR that the plan proposes**: an ADR in the map repo's `docs/decisions/` with status `proposed` whose
`Issue:` names that issue. The person decides by commenting; the agent only records it. The skill
gains `plan.ts wait`, which polls the issue's comments with `gh` and returns when such an approval
appears, reporting earlier-sha approvals and new `Review:` and `Decision:` comments as it goes, and
`adrs.ts accept`, which re-reads the comment from GitHub, refuses unless it is a trusted approval of
the current plan, and rewrites only those ADRs' status to `accepted` with the commenter's login, the
date and the comment's URL. The skill runs `wait` in the background right after asking for approval.
An approval of an earlier sha accepts nothing. Rejecting an ADR stays an edit or a `Decision:`
comment by a person.

## Alternatives considered

- **Keep acceptance a separate human edit**: two steps for one decision, and the session cannot tell when to go on.
- **A dedicated `Accepted: ADR NNNN` comment**: more precise, but one more thing to type for what the approval already says.
- **Rubato tells the session** (issue #91): only for Rubato-launched sessions; kept as a later addition, not a replacement.

## Consequences

An approver accepts the plan's ADRs by approving it, so the `Review:` comment must name them (the
review template gains a line). The ADR file keeps the audit trail: who, when, and the comment's
link. A session may now keep running while it waits, so its spend cap still applies. O-row:
`skill-plan-file` (the `rubato.plan.wait` line and `adrs.ts accept`'s line).

## Supersedes

The sentence "only a human moves it to *accepted*" in AGENTS.md "Decisions are records", in part:
a human still decides, and a trusted approval comment is how they do it.
