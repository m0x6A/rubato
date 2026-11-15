# 0055. The approved plan reaches the branch in one handoff commit, and the issue gets one link comment from the approver

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), from Mattias Jonsson's decisions in #122 ("the issue keeps track of its plan", "handoff commit carries the approval") and his answers of 2026-11-14 (summary from the agent's reply; connected-repo ADRs stay proposed)
- **Seam:** server-github, skill-plan-file, companion-claude-code
- **Issue:** m0x6A/rubato#122

## Context

After approval in Rubato (ADR 0054), the plan must reach Git once, carrying its approval, and the
issue must show that it is addressed and by which plan. ADR 0030 lets the server act as a person,
but only "on a request they made in the browser", and lists the approval comment as its only write.
The link comment comes later, once the commit exists.

## Decision

**The prompt.** The approval reaches the owner's session as `plan_approved` (ADR 0051). The
companion types one prompt that carries the record as arguments: approval id, sha, approver, time,
`self` and Rubato's link.

**`plan.ts handoff`.** The agent runs it. It:
1. refuses when the plan's hash is not the approved sha, or the bundle's hash is not the approved
   bundle's;
2. writes the record into the front matter;
3. in the map repo's checkout, moves each ADR pinned in `## ADRs proposed` whose blob still matches
   to `accepted`, naming the approver, the date and the approval, as `adrs.ts accept` does for a
   comment, and refuses when a pinned ADR changed;
4. makes one commit of the plan's bundle, with trailers `Approved-by:` and `Rubato-Approval:`;
5. pushes;
6. reports its outcome to the companion's loopback endpoint, which forwards it to the server. The
   outcome reaches Rubato whatever the agent says in its reply.

A plan in a connected repo leaves its map-repo ADRs *proposed*; the handoff prints them, and Rubato
shows `AdrsNotAccepted`.

**The link comment.**
- The server reads the plan at the branch head's sha. When the record there equals Rubato's and the
  plan hashes to it, the server posts one comment on the issue as the approver, with the approver's
  user-to-server token. The comment holds a permalink to the plan at that commit, the approver and
  the time, the approved summary and a `Rubato-Approval:` line.
- It is posted at most once per approval, recorded in table `plan_links`, owned by `approve`. A new
  head after the handoff makes no second comment. An unknown outcome waits until the comments pass
  finds that line, and is never posted again on its own.
- With no stored token, the server waits and raises `LinkWaitingForApprover`, trying again each
  full lane.
- A mismatched record posts nothing (ADR 0054).
- Each re-approval and handoff makes one more comment.

This extends ADR 0030: a person's token may be used for the link comment that follows from an
approval they made in the browser.

## Alternatives considered

- **The companion commits, without the agent**: the agent stays the only writer of the plan and the code.
- **Rubato commits through the GitHub API**: it would race the agent's worktree, and the commit would not be the agent's.
- **The App posts the link comment, naming the approver**: works without the approver's token, but the issue asks that the approver post it. It stays the alternative if waiting proves a problem.
- **The record as a file outside the worktree**: reading it would trigger a permission prompt.

## Consequences

Easier: from the issue, one click to the exact approved plan; plan, decisions, ADR acceptance and
approval merge together. Harder: the approver's token is used after they leave the page. An absent
token stalls the link, which is shown.

O-rows:
- server-github: `rubato.plan.link_comment`, `link_comment_missing`, alert `HandoffNoLinkComment`;
- skill-plan-file: the `rubato.plan.handoff` line;
- server-plan-file: `handoff_overdue`, alert `ApprovalNoHandoff`.

On acceptance, ADR 0030's status becomes "accepted; write list extended by 0055".

## Supersedes

0030's "in v0 the only such write is the approval comment" and "on a request they made in the
browser", for the link comment that follows from an approval the person made in the browser. The
rest of 0030 stands.
