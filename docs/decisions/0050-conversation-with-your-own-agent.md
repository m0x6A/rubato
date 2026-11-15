# 0050. Build a planning-time tool, not an orchestrator, that carries the conversation with your own agent

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), asked for by Mattias Jonsson 2026-11-14 ("only the Chat non-goal goes")
- **Seam:** none (non-goal)
- **Issue:** m0x6A/rubato#122

## Context

ADR 0001 says Rubato "does not chat", because chat would duplicate where the team already talks.
Issue #122 found that the main pain of planning is not being able to talk to the planning agent
from the plan in Rubato. That conversation is part of the shared picture, not a side channel. The
thesis does not change: each developer controls their own agent, and understanding is shared.
Plan: `docs/plans/122.md`.

## Decision

Rubato shows the map, pins, sessions, plans, progress and decisions. It never orders work across
repositories, never runs an agent itself, and never attaches to or reads a colleague's terminal. A
feature that steers another developer's agent is rejected at review. What changes from 0001 is
conversation:

- A person may send messages from Rubato to a session they launched themselves (ADR 0051).
- The team sees each launched session's prompts and replies (ADR 0052).
- The team talks about a plan in its room (ADR 0057).
- An agent takes part in a room only while its owner has called it in. Calling it in is the
  owner's explicit, visible choice to lend it to the room, and it lasts until it is dismissed.

Conversation the team already has elsewhere stays there; Rubato carries only conversation about a
plan and with its agents.

## Alternatives considered

- **Keep "does not chat"**: leaves the main pain of #122 in place.
- **General team chat in Rubato**: duplicates where the team already talks, which was 0001's reason.

## Consequences

Easier: a plan is shaped by talking to its agent, in view of the team. Harder: a person's prompts
and an agent's replies are now team-visible data on the server (ADR 0052). Watch for room features
that let someone other than the owner direct an agent. O-rows: companion-server (session messages
by outcome), web-server (`rubato.web.rejected{not_your_session}`).

## Supersedes

0001.
