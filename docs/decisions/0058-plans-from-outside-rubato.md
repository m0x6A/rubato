# 0058. A plan written outside Rubato is shown but not approvable; Import makes it a draft the importer owns

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), from Mattias Jonsson's decisions in #122 ("seen, not approvable"; Import is agent work) and his answer of 2026-11-14 on #139 (the importer owns)
- **Seam:** server-plan-file, web-server, server-github
- **Issue:** m0x6A/rubato#122

## Context

Rubato is an ecosystem of clients, and a plan can be written outside it, for example by someone
running `claude` on a branch by hand. Discovery (ADR 0032) keeps finding such plans on `issue/`
branches. #122 approves plans only in Rubato, and #139 asked who owns an imported plan.

## Decision

**Outside Rubato.** A plan on an `issue/` branch is *outside Rubato* when its branch has no draft, no
Rubato approval and no counted pre-cutover comment approval. It shows with its fit and an *Import*
action, and cannot be approved as it stands.

**Import.**
- If the plan parses and fits a workspace, the server makes a draft on that branch from the files
  at the head. If not, Import launches a session on the importer's own paired companion, with a
  prompt to rewrite the plan into Rubato's form.
- **Owner.** The importer owns the draft. The plan's original author (the last commit to the plan
  file, read in one GitHub call) and the branch are recorded and shown.
- **The original session.** Nothing is sent to the original author's session.
- **Launching.** A launch is allowed for an issue that has a plan fitting the workspace, even
  without a pin.
- **Where it lives.** Import is its own server slice, with no table, naming the drafts, launch and
  plans slices, so neither of those names the other back.

From then on it is an ordinary draft: room, revisions, approval and handoff. The handoff commit
replaces the file on the branch. Plans already approved by comment before the cutover stay as they
are.

## Alternatives considered

- **The original author must consent**: like a call-in request; a person who hand-wrote a plan may be unreachable.
- **Defer Import to #139**: leaves hand-written plans with no path to approval.
- **Parse foreign plans into drafts without an agent**: rewriting them into Rubato's form takes judgement.

## Consequences

Easier: every plan has one path to approval, and its author is kept. Harder: the importer takes over
another person's plan on its branch; the recorded author and the room make that visible. O-rows:
server-plan-file (outside Rubato derived on each overview build), server-github (the commits call
on `rubato.github.calls`), web-server (Import refusals by reason). #139 is settled by this ADR.

## Supersedes

None.
