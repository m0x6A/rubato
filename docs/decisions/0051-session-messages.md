# 0051. Send messages to your own launched session, queued on the server, typed once when it waits for input

- **Status:** accepted; superseded in part by 0062 and 0072
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), choices answered by Mattias Jonsson 2026-11-14 (launched sessions only; tmux paste as the first adapter)
- **Seam:** companion-server, companion-claude-code
- **Issue:** m0x6A/rubato#122

## Context

#122 needs a way to put text into a person's own Claude Code session from Rubato: a chat message,
the approval that starts the handoff (replacing #91's comment path), and a room turn. Today the
server can only send a `LaunchCommand` to a connected companion. A message for an offline
companion is lost: there is no server-side replay (ADR 0012 promised one, and it was never built).
The issue named tmux-ide `send` as the first mechanism. The spike in
`docs/plans/122-plan-in-rubato/design.md` found that tmux-ide 2.9.18 `send` replaces newlines with
spaces for agent panes, and turns any message over 150 characters into a file in the worktree that
the agent is told to execute.

## Decision

**One type, a server-side queue.**
- There is one server-to-companion type, `SessionMessage`, with a `kind`: `chat`, `plan_approved`
  or `room_turn`. Its answer is `SessionMessageResult`: `delivered` (with `mixed`), `not_idle`,
  `refused` with a reason, or `failed`.
- Messages wait on the server in table `session_messages`, owned by slice `session_messages`.
  They are keyed by launch, so a `/clear` that changes the session id does not strand them. At
  most one message per launch is in flight; `plan_approved` goes first, then the rest in order.
- The server dispatches on enqueue, on an idle hook for that session (`Stop`, or `Notification`
  with `idle_prompt`), on the companion's Hello, and after a `not_idle`. It pushes only to a
  live companion: connected, announcing the capability, and heard from within two heartbeats. A
  message for an offline companion stays queued until its next Hello.

**At most once.**
- A pushed message with no answer is pushed again, with the same id, at the companion's next
  Hello. The companion records `typing` before it pastes and `delivered` after. A repeat it
  already delivered is answered without typing, and one it was pasting when it stopped is answered
  `interrupted`, which becomes *delivery unknown*.
- A message unanswered for 60 s while its companion stayed live becomes *delivery unknown*, and is
  never pushed again. A late answer still applies. "Send again" makes a new message.
- ADR 0012's server replay, if it is ever built, excludes these messages.
- The companion keeps its launches, hook port and token across its own restarts, so a companion
  upgrade does not cut off sessions it launched earlier.

**Who may send, and when it is typed.**
- A direct message is accepted only from the session's paired user (ADR 0004), and only for a
  session launched from Rubato. The companion refuses one whose user is not its paired user, or
  whose session it did not launch.
- It types only when the session's last hook is an idle one and is newer than its own last input,
  so a status copied from tmux-ide never counts as idle.

**How it is typed.** The companion types through a kernel port. Its first adapter pastes with tmux
directly: `load-buffer` from stdin, `paste-buffer -p` into the session's pane, then Enter. This
amends ADR 0003 for typed input, as the launch already calls `tmux new-session` directly. Nothing
in Rubato reads a pane.

**Capabilities.** Hello and Welcome each carry `capabilities`, and neither side sends a kind the
other has not announced. `PROTOCOL_VERSION` stays 1.

## Alternatives considered

- **tmux-ide `send`**: collapses newlines and rewrites long messages into a file the agent is told to execute.
- **A queue in the companion**: lost on restart, and the server could not show *waiting for the turn to end*.
- **Retry until acknowledged**: a prompt typed twice runs twice. A typed prompt cannot be safely retried.
- **A headless turn on the same session (`claude -p --resume`)**: the spike showed it writes into the live session's transcript.

## Consequences

Easier: one path for chat, approval and room turns, all visible per message. Harder: a paste is
appended to whatever a person has half-typed. That is shown as `mixed`, not prevented. Watch the
queue age. O-rows: companion-server (`rubato.session_messages{rubato.outcome}`,
`oldest_idle_queued_seconds`, alert `SessionMessageStuck`) and companion-claude-code
(`rubato.session_input.deliver`). On acceptance, ADR 0003's status becomes "accepted; typed input
superseded in part by 0051", and ADR 0012's becomes "accepted; replay superseded in part by 0051".

## Supersedes

0003's "only through tmux-ide's CLI", for typed input into a session: the companion pastes through
tmux directly. 0012's server replay, for session messages. The rest of 0003 and 0012 stands.
