# 0035. A team member's approval comment accepts the ADRs its plan pins; the session waits for it

- **Status:** accepted
- **Date:** 2026-11-12
- **Decided by:** Mattias Jonsson (m0x6A), accepted 2026-11-12 by approval comment https://github.com/m0x6A/rubato/issues/90#issuecomment-6837591374 (self), recorded by hand; drafted by agent (Claude Code, claude-opus-5-5, issue #90), asked for by Mattias Jonsson 2026-11-12 ("a comment on an issue from a teammember IS acceptance"); choices on self-approval, pinning, scope and the seam answered by him the same day
- **Seam:** skill-github, skill-plan-file
- **Issue:** m0x6A/rubato#90

## Context

AGENTS.md says an agent may draft an ADR as *proposed* and only a human moves it to *accepted*. In
practice the human's act was a separate edit after approving the plan, so a plan that proposed ADRs
needed two steps, and the planning session stopped until someone told it the plan was approved.
Mattias asked that waiting be automatic, part of the rubato skill, and said that a comment on the
issue from a team member is the acceptance. Design: `docs/plans/90-approval-wait/design.md`.

## Decision

A plan that proposes ADRs lists them in a section `## ADRs proposed`, one row per ADR file with
its git blob sha, so the plan's approval sha (ADR 0027) covers their text. An approval counts for
acceptance under the same rules the server applies: a trusted author (ADR 0027), the newest trusted
approval on the issue, naming the sha of the plan **on the plan branch as pushed** (7–40 character
prefix), with the plan fitting the map (ADR 0029), and a comment that has not been edited since it
was posted. Such an approval approves the plan **and accepts every ADR its plan pins**, when each
file's blob still equals the pinned sha. The approver may be the person whose agent drafted the
ADRs; the record says so (`self`). The skill gains `plan.ts wait`, which polls the issue's comments
with the developer's `gh` (seam `skill-github`) and returns when such an approval appears, and
`adrs.ts accept`, which re-reads the comment from GitHub, re-checks all of the above, and only then
rewrites those ADRs' status to `accepted` with the approver's login, the date, the comment's URL and
`self` when it applies. It writes only in the checkout of the map repo on the plan branch; in any
other repo it prints what the approval accepts and writes nothing. Anything that fails the checks
accepts nothing and is reported. Rejecting an ADR stays a person's edit or `Decision:` comment. This
ADR is itself accepted by the approval of plan #90, recorded by hand, since its mechanism does not
exist until that plan is built.

## Alternatives considered

- **Keep acceptance a separate human edit**: two steps for one decision, and the session cannot tell when to go on.
- **The comment names the ADRs and their blobs**: explicit, but more to type for what the approved plan already pins.
- **A second team member for seam ADRs**: stronger, but one team of a handful of people; recorded `self` instead.
- **Rubato tells the session** (issue #91): only for Rubato-launched sessions; a later addition, not a replacement.

## Consequences

Approving a plan now accepts its pinned ADRs, so the `Review:` comment names them and their blobs.
An ADR edited after the plan was approved is not accepted until the plan is updated and approved
again. An approval is still a rule, not a guarantee: an agent with a team member's `gh` could post
one (ADR 0027); `self` makes a self-acceptance visible, and the skill still forbids posting one.
Connected repos get the wait but not the ADR write. O-rows: `skill-github` (`rubato.plan.wait`
lines with `started` and `exited`; the server's Approvals panel for the same comment) and
`skill-plan-file` (`rubato.adrs.accept` line).

## Supersedes

The sentence "only a human moves it to *accepted*" in AGENTS.md "Decisions are records", in part: a
human still decides, and a trusted approval of the plan that pins the ADR is how they do it.
