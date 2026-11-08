# 0003. Claude Code is the only agent in v0; tmux is required; tmux-ide is the runner

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** companion-tmux-ide, companion-claude-code
- **Issue:** none

## Context

Stitch, don't build. Session discovery, agent status from hooks, `team --json`, `send`,
`wait` and MCP already exist in tmux-ide. Claude Code already emits hooks and OTel.
Supporting a second agent or a second terminal multiplexer in v0 would mean rebuilding both.

## Decision

The companion launches only Claude Code, only inside tmux, only through tmux-ide's CLI
(`worktree create`, `send`, `events`, `team --json`, `wait`); the TypeScript SDK is not used
from Rust. Hooks are the event source; `tmux-ide events` and `team --json` are the
reconciliation sources. The companion pins a tmux-ide version and reports it as
`rubato.tmux_ide.version`. Panes are never scraped by Rubato, and a tmux-ide state that came
from its own screen-reading fallback renders *unknown*.

## Alternatives considered

- **Direct tmux control from the companion**: rebuilds what tmux-ide does.
- **Agent-agnostic adapter layer now**: speculative; no second agent is in scope.

## Consequences

Easier: one event shape, one runner. Harder: a tmux-ide or Claude Code upgrade can change a
seam silently; O-rows `companion-tmux-ide` and `companion-claude-code` watch for it.

## Supersedes

none
