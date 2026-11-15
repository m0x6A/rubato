# 0052. Only prompts and final replies of launched sessions leave the machine; they form the team-visible session log

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/122#issuecomment-6891477242 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #122), choices answered by Mattias Jonsson 2026-11-14 (hook payloads as the source; purge what is stored)
- **Seam:** companion-server, companion-claude-code
- **Issue:** m0x6A/rubato#122

## Context

The issue says that only prompts and assistant text leave the developer's machine, never tool
calls or tool output. Today the companion forwards every hook's raw payload, including
`tool_input` and `tool_response`, and the server stores it in `companion_events`, which nothing
reads. #122 needs each launched session's conversation visible to the team. The spike showed that
`UserPromptSubmit` carries the prompt and `Stop` carries the last assistant message, both with the
same `prompt_id`.

## Decision

**Trimming.** The companion trims every hook payload to an allowlist before it leaves the machine:
- session id, event name, cwd, transcript path and permission mode;
- notification type, tool name, source, reason, `prompt_id`, `stop_hook_active`;
- for sessions launched from Rubato only, the prompt and the last assistant message, each capped in
  `Limits`.

**The session log.** The server keeps those prompts and replies as the session log (table
`session_log_entries`, owned by slice `session_log`), one prompt and one reply per `prompt_id`. It
is readable by everyone who may log in (ADR 0022), and never posted to GitHub. A reply that names a
delivered message is linked to it. The server takes text only from companions that announce the
allowlist, and only for launched sessions.

**Storage and the purge.**
- The server stores hook events without their payload, whatever the companion sends, and new
  envelope kinds with an empty one.
- Migration 0015 removes the payload from hook events already stored, and the server runs VACUUM
  right after it, so the bytes do not stay in free pages.

`rubato-companion pair` says what leaves the machine.

## Alternatives considered

- **A transcript reader**: every prompt and every mid-turn text, but more code (offsets, a new envelope); it can fill the same table later.
- **Keep forwarding full payloads**: tool output (file contents, command output) on a shared server, against the issue.
- **Keep what is stored**: leaves data that should never have left the machine.

## Consequences

Easier: one honest rule for what leaves the machine. Replies come from hooks, never from panes.
Harder: only the final reply of each turn is shown, and a hook whose 2 s curl fails loses that
line, which is counted. The purge cannot be undone. A database copy is taken before that deploy
and destroyed within 7 days, once the deploy is checked, since it holds exactly the purged data.
O-rows: companion-claude-code (`rubato.hook.payload_trimmed`,
`rubato.session_log.dropped{rubato.reason}`), companion-server (log entries by session).

## Supersedes

None.
