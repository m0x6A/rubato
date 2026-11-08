# 0013. Deliver Claude Code hooks to the companion over loopback HTTP, config injected at launch

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (drafted by agent, Claude Code v0 planning session)
- **Seam:** companion-claude-code, skill-plan-file
- **Issue:** none

## Context

Claude Code runs a shell command per hook event with JSON on stdin. The event must reach the
companion daemon fast, must not block the agent, and must not fire for sessions that were not
started through Rubato on repos where the hook config is committed.

## Decision

The companion listens on `127.0.0.1` on an ephemeral port and writes the port and a random
per-start token to its state file. The hook command is
`curl -sS -m 2 -H "Authorization: Bearer <token>" --data-binary @- http://127.0.0.1:<port>/hook`,
run `async` so it never blocks the session. The hook configuration is a template in
`skills/rubato/hooks.json` with `{port}` and `{token}` placeholders; the companion renders it
per launch and passes the file with `--settings`, so only Rubato-launched sessions emit. A
developer may opt a manual session in by rendering it into `.claude/settings.local.json`.
Events sent while the daemon is down are lost; the `companion-claude-code` O-row (hook counter
at zero while a session is *working*) and the tmux-ide reconciliation poll cover the gap.

## Alternatives considered

- **`rubato-companion hook` over a unix socket with a disk spool**: no port and no loss, but a second binary invocation per event and a spool to drain; rejected as more machinery than the lab needs now.
- **Append JSONL that the daemon tails**: no backpressure, file rotation to manage.
- **Commit hooks to the repo's `.claude/settings.json`**: fires for every developer, installed or not.

## Consequences

Easier: one daemon, no socket files, curl is on every machine. Harder: a listening loopback
port, guarded by the per-start token; lost events while the daemon is down, made visible by the
O-row. `--settings` must stay a Claude Code flag and hooks must merge across settings levels
under it so tmux-ide's own user-level hooks keep firing (verified in task 4).

## Supersedes

none
