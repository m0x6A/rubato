# 0070. Retire tmux-ide; the companion runs tmux and git itself

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#147), asked by Mattias Jonsson; a human accepts. First drafted as 0050 on 2026-11-14 and approved with plan 62db41a; renumbered and refreshed against main, because main gave 0050 to rooms
- **Seam:** companion-tmux-ide, companion-server
- **Issue:** m0x6A/rubato#147

## Context

ADR 0003 made tmux-ide the runner, so we could stitch its session discovery, agent status,
`send`, `wait` and MCP instead of building them.

Since then, parts of 0003 have been superseded:
- **0051:** typed input is a tmux paste, because tmux-ide `send` collapses newlines.
- **0062:** a headless companion makes worktrees with `git worktree` and needs neither tmux nor
  tmux-ide.
- **0071:** an agent's MCP is served by its own companion.

What is left of tmux-ide in an interactive companion is five calls:
- **`worktree create`:** a wrapper over `git worktree add`.
- **`adopt`:** only for tmux-ide's status bar; failures are ignored.
- **`team --json`:** agent state, but only for panes that tmux-ide's own Claude hooks stamped.
- **`events --follow`:** a change trigger that streams only for adopted sessions.
- **`--version`:** for the pin.

Sessions are already created with `tmux new-session`. Panes already come from `tmux list-panes`
and `ps`. Every event for a launched session already comes from our own hooks (ADR 0013).
tmux-ide still has to be installed and pinned on every machine and in CI. It keeps a second set
of Claude hooks on the machine, and it adds a state source we only partly read.

Issue #147 asks whether we need it. The plan for #147 shows we don't.

## Decision

**What the companion does without tmux-ide.** It never calls tmux-ide:
- It creates a worktree with `git worktree add` at the path tmux-ide used,
  `<repo>-worktrees/<branch>`. It uses the local branch when one exists, else `origin/<branch>`
  tracked, else a new branch from the base ref with no upstream until the first push. There is
  one implementation for interactive and headless mode.
- It starts the session with `tmux new-session`, as today.
- It finds panes with `tmux list-panes` and `ps`.
- It takes agent state only from our hooks, and from the headless turn process in headless mode.
- Reconcile runs on its timer and is woken by launches, as today.

**The launch dependency check** becomes tmux at or above a pinned version. The pin is
`companion.tmux_min_version`, default 3.7. A headless companion checks no tmux (0062). The
heartbeat reports the version as `Heartbeat.tmux_version`, and the start log line as
`rubato.tmux.version`.

**What stays.** Claude Code stays the only agent in v0. tmux stays required in interactive mode.
Panes are still never scraped.

**The wire.** `StateSource::tmux_ide` and `Heartbeat.tmux_ide_version` stay readable, so stored
snapshots and an older companion still parse. A current companion never sends them.

**Manual sessions.** A session started outside Rubato reports state only when the developer opts
it in with the skill's `hooks.ts`. Without that, it shows as a pane with *unknown* state. It gets
no MCP: 0071 serves only sessions a companion launched, because the launch id is the identity.

## Alternatives considered

- **Keep tmux-ide (ADR 0003)**: one more pinned tool on every machine, for a git wrapper and for a
  status source we only trust when its own hooks stamped the pane.
- **Install our hooks at user level from the skill, so every manual session reports**: a manual
  session has no launch to bind to, and a launched session would then fire twice under
  `--settings`. It is worth its own design, not part of retiring a dependency.
- **Remove `tmux_ide` from the wire now**: rows in `session_snapshots` hold the old value and
  would stop parsing. A later cleanup can drop it with a migration.
- **Keep tmux-ide only for `team --json` on manual sessions**: the whole install and pin for state
  that is trusted only when tmux-ide's own hooks are installed.

## Consequences

**Easier:**
- One fewer install and pin.
- One source of agent state per mode: hooks interactive, the turn process headless.
- A Gherkin suite that needs only tmux and git.
- No second set of hooks to merge under `--settings`.
- One worktree implementation for both modes.

**Harder:**
- The change trigger that tmux-ide's `events --follow` gave for adopted sessions is gone. A pane
  that closes is seen at the next reconcile (`team_poll_interval_secs`, 30 s) unless a hook
  arrives first.
- A manual session without opted-in hooks shows *unknown* where tmux-ide could sometimes say
  *working*.

**Watching.** The `companion-tmux-ide` seam carries nothing after this, and the map marks it
retired. Its O-row moves to the companion's own signals: the counter `rubato.launches` by outcome
and reason, on the panel "Launches by outcome", and the span `rubato.reconcile`.

## Supersedes

In part:

- **0003:** "only through tmux-ide's CLI", "`tmux-ide events` and `team --json` are the
  reconciliation sources" and "pins a tmux-ide version and reports it as
  `rubato.tmux_ide.version`". These read: the companion runs tmux and git itself; reconciliation
  is `tmux list-panes` with `ps`; the pin is on tmux, reported as `rubato.tmux.version`. "Claude
  Code is the only agent in v0", "tmux is required" (interactive, per 0062) and "panes are never
  scraped" stand.
- **0013:** "the tmux-ide reconciliation poll cover the gap" reads "the pane reconcile covers
  liveness; state still needs a hook". The note that tmux-ide's user-level hooks keep firing under
  `--settings` no longer applies. Hook delivery itself is unchanged.

On acceptance, the status lines of 0003 and 0013 add "superseded in part by 0070".
