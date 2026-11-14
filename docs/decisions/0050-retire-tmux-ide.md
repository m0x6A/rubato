# 0050. Retire tmux-ide; the companion runs tmux and git itself

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, m0x6A/rubato#147), asked by Mattias Jonsson; a human accepts
- **Seam:** companion-tmux-ide, companion-server
- **Issue:** m0x6A/rubato#147

## Context

ADR 0003 made tmux-ide the runner so we could stitch its session discovery, agent status,
`send`, `wait` and MCP instead of building them. The companion ended up using five calls:
`worktree create` (a wrapper over `git worktree add`), `adopt` (only for tmux-ide's status bar,
failures ignored), `team --json` and `events --follow` (agent state only for panes tmux-ide's
own Claude hooks stamped) and `--version` for the pin. It never used `send`, `wait`, MCP or the
SDK. Sessions are already created with `tmux new-session`, panes already come from
`tmux list-panes` and `ps`, and every event for a launched session already comes from our own
hooks (ADR 0013). tmux-ide still has to be installed and pinned on every machine and in CI,
keeps a second set of Claude hooks on the machine, and adds a state source we partly read.
Issue #147 asks whether we need it; the plan for #147 shows we don't.

## Decision

The companion no longer calls tmux-ide. It creates a worktree with `git worktree add -b <branch>
<repo>-worktrees/<branch> <base>` (the same path tmux-ide used), starts the session with `tmux
new-session` as today, finds panes with `tmux list-panes` and `ps`, and takes agent state only
from our hooks. Reconcile runs on the timer and is woken by launches, as today. The launch
dependency check becomes tmux at or above a pinned version (`companion.tmux_min_version`,
default 3.7), reported as `rubato.tmux.version`. Claude Code stays the only agent in v0 and tmux
stays required; panes are still never scraped. The wire keeps `StateSource::tmux_ide` and
`Heartbeat.tmux_ide_version` readable, so stored snapshots and an older companion still parse,
but a current companion never sends them. A session started outside Rubato reports state only
when the developer opts it in with the skill's `hooks.ts`; without that it shows as a pane with
*unknown* state.

## Alternatives considered

- **Keep tmux-ide (ADR 0003)**: one more pinned tool on every machine for a git wrapper and a
  status source we only trust when its own hooks stamped the pane.
- **Install our hooks at user level from the skill, so every manual session reports**: the hook
  token changes on every companion start (ADR 0013) and a launched session would then fire
  twice; worth its own design, not part of retiring a dependency.
- **Remove `tmux_ide` from the wire now**: rows in `session_snapshots` hold the old value and
  would stop parsing; a later cleanup can drop it with a migration.

## Consequences

Easier: one fewer install and pin, one source of agent state (hooks), a Gherkin suite that needs
only tmux and git, no second set of hooks to merge under `--settings`. Harder: the change
trigger tmux-ide's `events --follow` gave for adopted sessions is gone, so a pane that closes is
seen at the next reconcile (`team_poll_interval_secs`, 30 s) unless a hook arrives first; a
manual session without opted-in hooks shows *unknown* where tmux-ide could sometimes say
*working*. The `companion-tmux-ide` seam carries nothing after this; its O-row moves to the
companion's own `rubato.reconcile` span and the launch's `missing_dependency` refusals.

Agent state for agents, the one thing tmux-ide's MCP would have given us, is kept in mind for
later. If we build it, it is an MCP server on `rubato-server` reading the derived overview
(sessions per issue, progress, plans), never one on the companion, since nothing may reach a
developer's machine inbound (ADR 0004). Nothing here rules it out: the server already holds the
state it would serve.

## Supersedes

0003
