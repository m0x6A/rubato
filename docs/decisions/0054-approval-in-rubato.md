# 0054. Approve plans in Rubato; the approval record goes into the plan's front matter, and comment approvals count only before a cutover

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), choices answered by Mattias Jonsson 2026-11-14 (cutover for comment approvals; approval only in Rubato after it)
- **Seam:** server-plan-file, skill-plan-file, plan-check-plan-file, web-server, skill-github
- **Issue:** m0x6A/rubato#122

## Context

ADR 0027 makes approval an `Approved: plan <sha>` comment, with front matter that never carries
approval keys. ADR 0035 lets that comment accept the plan's ADRs, and lets the session wait for it.
#122 moves approval into Rubato and wants CI to see it. Today approval lives in comments CI does not
read (ADR 0029).

## Decision

**Who approves, and when.**
- A person logged in to Rubato approves a plan's version: a settled draft revision, or after
  handoff the plan at the branch head. They approve as themselves, and approving one's own agent's
  plan is recorded as `self`. Agents and companions have no route to approve.
- Rubato refuses in these cases:
  - the version is not the newest;
  - the owner's session is working;
  - a bundle file is missing;
  - an ADR the plan proposes is not pinned at its blob;
  - the plan fits none of the issue's workspaces (the rule of ADR 0029 and 0045, enforced here).
- The approval is stored in table `rubato_approvals`, owned by `approve`, with the hash of the
  whole bundle (the plan, its design note, its ADRs) and the edited summary.
  `plan_approved` is sent to the owner's session first (ADR 0051).

**The record in the plan.**
- The plan's front matter may carry `approved_by`, `approved_at` and `approved_sha`: all three or
  none, flat scalars. This extends the key list of ADR 0027 and ADR 0009.
- The approval hash removes those three lines from the front matter before clearing checkboxes. A
  plan without them hashes as before, so existing approvals stay current.
- plan-check fails a plan whose keys are partial, or whose `approved_sha` is not its hash, and
  reports `approval` on its line.

**One list of approvals.**
- Rubato's approvals and comment approvals form one list. An approval record carries its `source`.
  The newest *counted* approval decides, so a comment that no longer counts cannot hide a current
  one.
- A Rubato approval counts while its plan sha is the newest revision's. After handoff, it counts
  while the plan read at the branch head carries a record equal to Rubato's and hashes to it.
- Rubato's own record is the authority. A file whose record differs raises
  `ApprovalRecordMismatch`.

**The cutover.**
- `Approved:` comments count as in ADR 0027 when posted before the server's
  `RUBATO_COMMENT_APPROVALS_UNTIL`. Later ones are not counted and raise `CommentApprovalIgnored`. *Accept plan*, which
  posts such a comment, refuses after the cutover.
- While the cutover is unset, nothing changes and Rubato approval is refused.

**The merge gate.** ADR 0033's "counted approval at the head being merged" includes a Rubato
approval whose record matches the plan read at that head.

## Alternatives considered

- **Keep counting comments**: two approval paths. GitHub would keep a part in planning, which the issue rules out.
- **Stop counting comments at once**: breaks plans approved and in flight at deploy.
- **The record only in Rubato's database**: CI could not check it, which is the gain the issue names.
- **Hash the whole file**: writing the record would change the hash it records.

## Consequences

Easier: a plan carries its approval, and CI fails one edited after approval. Harder: anyone who can
push can hand-edit the keys. Rubato's record is the authority, and the mismatch is a finding; as in
ADR 0027, this is a rule, not a guarantee.

O-rows: plan-check-plan-file (the `approval` field), server-plan-file (`approval_record_mismatch`,
`ApprovalRecordMismatch`), skill-github (`rubato.plans.comment_after_cutover`), web-server
(`rubato.web.rejected{rubato.reason}`).

On acceptance, statuses become:

| ADR | Status |
| --- | --- |
| 0027 | superseded by 0054 |
| 0035 | superseded by 0054 |
| 0029 | accepted; enforced by Rubato's approval (0054) |
| 0033 | accepted; approval source extended by 0054 |
| 0028 | accepted; review and approval moved into Rubato by 0054 and 0056 |

## Supersedes

- 0027 and 0035, whole.
- 0029's counting of `Approved:` comments: the fit rule stands and is enforced by Rubato's approval.
- 0033's approval condition, which now also takes a Rubato approval matching the plan at the head;
  the rest of 0033 stands.
- 0028's "Review and approval go to issue comments"; the rest of 0028 stands.
- 0009's front matter key list, as 0027 had changed it.
