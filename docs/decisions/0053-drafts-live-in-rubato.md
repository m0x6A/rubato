# 0053. A plan lives in Rubato as a draft until it is approved; the companion mirrors it from the worktree at settle points

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), from Mattias Jonsson's decisions in #122 ("drafts live in Rubato", "custody moves one way") and his answers of 2026-11-14
- **Seam:** companion-server, server-plan-file, web-server
- **Issue:** m0x6A/rubato#122

## Context

ADR 0005 has the plan travel as a file on the issue branch, read by the server from GitHub, and
rejects a submit endpoint because it "bypasses review and CI" and is "lost if the server is down".
During planning that costs a commit, a push and a poll per revision, and buys nothing that planning
needs. The issue keeps Git as the home of approved plans, and moves custody one way: Rubato holds a
draft until approval, then Git owns the plan.

## Decision

**What is mirrored.**
- The agent keeps writing the plan bundle in its worktree: the plan, the design note it links and
  the ADRs it lists. The bundle is `bundle_paths` in `rubato-protocol`, computed from the plan
  itself.
- For a session launched from Rubato, the companion sends the bundle at its settle points (`Stop`,
  `idle_prompt`, `SessionEnd`), and only when its hash changed. Nothing is mirrored mid-turn, and
  nothing is committed.

**What the server stores.** It keeps one draft per (repo, issue, branch), owned by the person whose
launch first mirrored to it (table `plan_drafts`). It keeps each settled revision with its files
and plan hash (`plan_draft_revisions`), both owned by slice `plan_drafts`. A bundle from another
person's session on that branch is not stored.

**Parked drafts.** When the session ends, the draft is parked. A new launch by its owner uses the
draft's branch. When the companion can restore, it writes the newest revision into the worktree
only if that launch created the worktree.

**Derived, never stored.**
- The draft states (live, parked, approved and waiting for commit, on branch and linked, approval
  stale) are derived.
- So is progress:
  - a settled draft that parses, or a plan on the branch, makes an idle session's issue *Designed*;
  - a working session is *Building* only with the plan on the branch, and is otherwise
    *Designing*.

  ADR 0043's `blocked_from` uses the same split.
- `SeamCollision` also counts settled drafts.

**Planning mode.**
- A launch says how its session plans. `git` is today's flow: commit, push and comments. `rubato`
  is the flow in this ADR. The server sends `rubato` only after the cutover of ADR 0054, and only
  to a companion and skill that can carry it.
- Drafts are mirrored and shown in both modes.

**After approval.** The approved plan reaches the branch in one commit (ADR 0055). From then on the
server reads it from the branch, as ADR 0005 says, and nothing syncs back.

## Alternatives considered

- **Commit drafts on a branch, as today**: the steps #122 removes.
- **The session posts drafts to an endpoint**: the agent would need credentials. The companion already has an outbound, authenticated link.
- **Mirror every save, debounced**: more traffic and half-written revisions; a settle point is when the agent has finished a version.
- **Write drafts back into a live worktree**: two writers on one file; the worktree is the truth while a session lives.

## Consequences

Easier: revising a plan is talking to the agent; revisions and diffs come free. Harder: until
approval the draft exists only in the worktree and Rubato's database, which the issue accepts: "a
database backup is enough for now". It is not lost when the server is down, because the files stay
in the worktree and the companion sends them again at the next settle point. O-rows:
companion-server (`rubato.drafts.refused`), server-plan-file (`draft revision stored`). On
acceptance, ADR 0005's status becomes "accepted; planning phase superseded by 0053".

## Supersedes

0005's "there is no submit plan endpoint", for the planning phase only: drafts reach the server
through the companion. The rest of 0005 stands; the approved plan still travels as a file.
