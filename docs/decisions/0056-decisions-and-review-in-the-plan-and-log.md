# 0056. During planning, decisions are a section of the plan and the review is the agent's reply in the session log

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #122), choices answered by Mattias Jonsson 2026-11-14 (decisions in the plan; review from the reply); a human accepts
- **Seam:** skill-plan-file, server-plan-file, plan-check-plan-file
- **Issue:** m0x6A/rubato#122

## Context

ADR 0014 records decisions as `Decision:` comments on the issue, and the rubato skill posts its
design review as a `Review:` comment (ADR 0027, ADR 0028). #122 takes GitHub out of planning: the
review and decisions are part of the plan's conversation in Rubato. The durable record is the plan
and the decision log, committed at handoff.

## Decision

**Decisions.**
- A plan may carry `## Decisions`, a table with the columns `# | Decision | Chosen | Why | Decided
  by`. Both parsers read it into `Plan.decisions`, over a shared fixture; "None." gives an empty
  list.
- The agent records each decision made while planning there, as it is made, so the approval covers
  it and the handoff commits it. Rubato shows new rows as entries in the plan's thread.
- A decision that touches a seam is still an ADR (AGENTS.md).

**The review and the summary.**
- The design review is the agent's reply whose first line is `Review: <track> — <outcome>`, in the
  template's form. It is read from the session log (ADR 0052) and tied to the draft revision it
  reviewed.
- A reply whose first line is `Summary:` is the agent's draft of the discussion summary (ADR 0055).
- Neither is posted to GitHub.

**Comments still read.** `Decision:` and `Review:` comments that people write on GitHub are still
read and shown, as ADR 0014 says.

## Alternatives considered

- **Decisions as `Decision:` replies, written into the plan at handoff**: the approved text would then differ from the committed text.
- **Review and decisions as files in the bundle**: more files to mirror and approve, for text that belongs in the conversation.
- **Keep posting them to the issue**: the steps #122 removes.

## Consequences

Easier: decisions travel with the plan into Git, and the review sits next to the revision it
reviewed. Harder: a review exists only in Rubato, and in the plan's pull request only through its
effects. O-rows: server-plan-file (decisions parsed with the plan), skill-plan-file (plan-check
over the section). On acceptance, ADR 0014's status becomes "accepted; during planning decisions
are a plan section (0056)".

## Supersedes

- 0014, for decisions made while a plan is drafted in Rubato: they are a plan section. `Decision:`
  comments people write still count as 0014 says.
- 0028's review step "posts a `Review:` comment", for plans drafted in Rubato.

The rest of 0014 and 0028 stands.
