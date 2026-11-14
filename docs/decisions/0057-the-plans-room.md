# 0057. A plan has a room for the team; an agent joins only when its owner calls it in, and answers only when addressed

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #122), from Mattias Jonsson's decisions in #122 (who calls an agent in; which instance comes in) and his answers of 2026-11-14 (no uninvited agent posts; Rubato's seam notices); a human accepts
- **Seam:** web-server, companion-server, companion-claude-code
- **Issue:** m0x6A/rubato#122

## Context

#122 wants the team to talk about a plan, and sometimes to bring an agent in, "like a lead meeting
that calls a developer in for input". Control must stay federated: only an agent's owner may lend it
to a room. The spike showed that a headless forked turn (`--resume --fork-session`) leaves the live
session untouched, and that its first turn costs about the whole context uncached.

## Decision

**The room.** Each plan (repo, issue, branch) has a room. People post in it and everyone who may log
in reads it; this is held in tables `room_messages`, `room_agents` and `room_requests`, owned by
slice `rooms`. Agents are not in a room by default.

**Calling an agent in.**
- An owner calls their own agent in. A teammate asks, and the agent joins only when the owner
  accepts; until then the request shows as pending.
- The room always shows which agents are in, who called them in, and since when.

**Turns.**
- An agent answers only when a person addresses it by its handle. Its turn carries the room since
  its last turn, or since it joined. Nothing before that reaches it.
- While the plan is not on the branch, the turn goes to the working session as a `room_turn`
  session message (ADR 0051), and the reply comes from its log.
- While building, the turn goes to a read-only fork that the owner's companion runs:
  `claude -p --resume … --fork-session --permission-mode plan`, without Rubato's hooks, then
  resumed as that fork. Its cost counts on the issue and against the owner's spend cap.
- What a fork concludes reaches the working session only as a direct message from the owner.
- A turn frames the room's messages as quoted discussion, each with its author, not as
  instructions. While planning, it is typed into a session that runs under its launch's permission
  mode. Only people who may log in post in a room, and the owner lent the agent by accepting the
  call-in.

**Leaving and limits.**
- The owner, or whoever called the agent in, dismisses it, or it leaves after 30 minutes without
  being addressed.
- Turns triggered by agents stop after 3 in a row until a person posts.

**Notices.** Agents never post in a room they were not called into. Rubato itself posts a system
notice when another open plan or settled draft touches the same seam.

## Alternatives considered

- **Agents read every room**: they would see what nobody lent them, and spend on every line.
- **A turn on every message**: a chatty room would spend the owner's cap.
- **The working session answers while building**: it is busy, and a room should not change its code.
- **Agents post notices uninvited**: needs a new channel out of the agent, and reads like steering.

## Consequences

Easier: the team can question a plan's agent in context, and the owner decides when. Harder: fork
turns cost money, and the first one costs most, so the call-in shows an estimate. Several agents in
one room need the turn limit. O-rows:

- web-server: rejected requests by reason;
- companion-claude-code: `rubato.room.fork_turn`;
- companion-server: `rubato.rooms.agent_turns{rubato.outcome}`, `turn_limit_hits`.

## Supersedes

None.
