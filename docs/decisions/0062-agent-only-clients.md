# 0062. An agent-only client is a headless companion, owned by one person; the client contract does not depend on how it pairs

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #122), from Mattias Jonsson's answer of 2026-11-14 on #138 ("start by 1 but dont rule out 2": a headless companion first, a token client with the agent elsewhere kept open); a human accepts
- **Seam:** companion-server, companion-claude-code, web-server
- **Issue:** m0x6A/rubato#122, m0x6A/rubato#138

## Context

Rubato is an ecosystem of clients: people with their agents, and agents. #122 settled that every
client is paired with one GitHub login that owns it, and that an agent never approves. #138 asked
three things about a client with no person at the keyboard:
- how it receives session messages and room call-ins without tmux and the paste adapter;
- how it is paired, and how its owner sees and stops it;
- how spend caps and attempt limits apply.

The #122 spike showed that `claude -p --resume <session>` writes into the session's transcript.
That is unsafe only beside a live session on the same transcript. A client with no interactive
session has nothing to collide with, as long as only one turn per session ever runs.

## Decision

**A headless companion.** The companion config gets `mode: interactive | headless`, with
`interactive` as the default; one companion has one mode.
- In headless mode it needs neither tmux nor tmux-ide. It makes worktrees with `git worktree` and
  injects hooks as today.
- Each turn runs as a child process group that the companion holds:
  - the launch is `claude -p "<prompt>" --session-id <id> --output-format stream-json`;
  - each later message is `claude -p --resume <id> "<text>"`.
- At most one turn per session runs, also across a companion restart. The turn's process group is
  recorded, checked at start, and killed on every exit path.

**The child process is the authority, not hooks.** In headless mode the companion takes these
signals from the child it holds:
- the turn's end comes from the child's exit and its stream-json `result`, sent to the server as
  an idle snapshot that triggers dispatch;
- the reply comes from the `result`;
- liveness comes from stream events;
- spend comes from the transcript sum, checked at each turn end and periodically during a turn.

Hooks, if they fire, are cross-checked only. Turns are bounded by a configurable total time and a
no-output time.

**The same contract as any client.** A client is defined by:
- one owner;
- the companion websocket envelopes;
- the capabilities it announces;
- `SessionMessage` in, and `SessionMessageResult`, `SessionState`, `HookEvent`, `DraftBundle` and
  `RoomReply` out (ADR 0051).

A headless companion is a second `SessionInput` adapter behind that contract. Delivery stays at
most once, as in 0051, and room call-ins follow ADR 0057.

How a client binds its owner is not part of the contract. Device-code pairing is the only way
today. An owner-minted, scoped token for an agent running elsewhere (CI, a cloud session) would be
a later ADR that adds a second way to authenticate the same socket and changes nothing above.

**The owner sees it and stops it.**
- Hello carries `client: interactive | headless`, stored on the pairing. The web shows *headless*
  and the owner on the companion's lane and on its sessions.
- A new session-message kind, `stop`, is accepted only from the owner, for interactive and
  headless sessions alike. It is never queued: it answers `companion_offline` when the companion
  is not live. It is not held behind an in-flight message. It skips only the idle check, kills the
  turn's process group (or the tmux session) and ends the session.
- Revoking the pairing closes the companion's socket. On that, or on a 401 at reconnect, the
  companion stops every headless turn and exits.
- New enum values are sent only to a peer that announced the capability (`headless`,
  `session_stop`).

**Caps apply as for any launch.** A cap hit kills the process group and is a `CapHit`. The attempt
limit counts launches and draft resumes; a resume turn is not an attempt.

**It never approves.** Approval stays a web route for a logged-in team member (ADR 0054). No
client of either mode holds a web session.

## Alternatives considered

- **A token client first, with the agent running elsewhere**: kept open by the contract above, but
  it needs a new way to authenticate and a new trust story. A headless companion reuses pairing,
  caps and hooks as they are.
- **Record the rules only, build later**: leaves #122's room and chat with no agent-only client to
  test against.
- **Rely on hooks under `-p` for idle, reply and spend**: unproven, and a lost hook would stall
  dispatch and skip the cap.
- **Headless turns beside a live interactive session**: unsafe (the #122 spike).

## Consequences

Easier: an agent can run on a machine with no terminal, under its owner's caps, visible and
stoppable. There is no paste race, so `mixed` never applies. A lost hook cannot hide a turn's end.

Harder: a turn in flight cannot take input, so messages wait for its end, as in interactive mode.
A long build in one turn needs a generous total bound. The companion must reap its own process
groups, and gets a never-silent rule, `HeadlessTurnsKilled`, for kills by timeout, by quiet or at
restart.

O-rows:
- companion-server: `rubato.session_messages{rubato.outcome}`; `stop` refused by reason on
  `rubato.web.rejected`; `rubato.client` on the Hello span;
- companion-claude-code: span `rubato.session_input.deliver` with `rubato.adapter=headless`,
  `rubato.headless.turns{rubato.outcome}`, and
  `rubato.headless.killed{rubato.reason=stop|cap|revoke|timeout|quiet|companion_restarted}`.

On acceptance, each of these ADRs gets "superseded in part by 0062" after its status: 0003, 0004,
0006, 0013 and 0051.

## Supersedes

In part, for headless companions unless noted:

- **0003:** "tmux is required" and "only inside tmux, only through tmux-ide".
- **0004:** "in their own tmux" reads "through their own companion". Only the paired owner starts
  sessions, and nothing is inbound; both stand.
- **0006:** the pane cwd. Identity comes from the launch's worktree and branch, the same
  branch rule.
- **0013:** the tmux-ide reconciliation poll as the backstop for lost hooks. It is replaced by the
  child process the companion holds.
- **0051:**
  - its list of kinds gains `stop`;
  - "`plan_approved` goes first" becomes "`stop`, then `plan_approved`";
  - "typed only at idle" does not apply to `stop`, for interactive and headless alike.

The rest of each stands.
