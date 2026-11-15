# 0078. A session started by hand reports state only through a per-repo opt-in

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#248), asked by Mattias Jonsson; a human accepts
- **Seam:** companion-claude-code
- **Issue:** m0x6A/rubato#248

## Context

ADR 0070 retired tmux-ide. Its user-level hooks used to give state to sessions a developer
started by hand. Now such a session shows as a pane with *unknown* state, unless the developer
opts it in per repo:
- the skill's `hooks.ts --out .claude/settings.local.json`;
- or `rubato-companion render-hooks`.

Issue #248 asks whether the skill should instead install our hooks at user level, so every
session reports. Three things stand in the way:
- A launched session already gets our hooks through `--settings` (ADR 0013). Claude Code merges
  user-level hooks with those, so every hook of a launched session would arrive twice.
- A user-level hook runs in every Claude Code session on the machine, in any directory.
- Its port is fixed when it is written, and a companion that restarts on a new port
  (`hook port changed`, plan 122 §D1) silences it everywhere at once.

## Decision

A session started by hand reports state only when the developer opts its repo or worktree in.
Nothing is installed at user level.

To make the opt-in safe, easy to find and easy to keep:
- **`rubato-companion render-hooks --into <dir>/.claude/settings.local.json`** writes our hooks
  into that file.
  - It replaces only our own entries and keeps the developer's other hooks and keys.
  - Each hook it writes stands down when `RUBATO_LAUNCH_ID` is set. So a worktree that a later
    launch reuses still delivers each event once, through the launch's `--settings` hooks.
- **`rubato-companion status`** lists each workspace repo and its worktrees.
  - For each, it says `opted in`, `opted in, stale port <p>`, or prints the command.
  - It says when nothing listens on the port.
  - It ends with a count.
  - It prints the header file's path and never the token.
- **The skill and QUICKSTART** say the same.

ADR 0071's MCP stays for sessions a companion launched: a manual session has no launch id.

## Alternatives considered

- **User-level hooks from the skill, with the companion dropping a launched session's
  duplicates:**
  - It needs a source header on every hook, a dedupe rule, and a hook command that reads the
    port at run time.
  - Every Claude Code session on the machine then posts to the companion, wherever it runs.
  - It is worth building when someone needs state from sessions they start by hand, not before.
- **The companion writes `.claude/settings.local.json` into each workspace repo itself:** it
  edits the developer's files without being asked.
- **Hide sessions started by hand:** the map would no longer show who is working where, which is
  the point of the shared picture.

## Consequences

**Easier:**
- One delivery per event, even when a launch reuses an opted-in worktree.
- Nothing runs outside the directories the developer chose.
- `status` shows what is opted in, what has gone stale, and whether the companion listens,
  without printing the token.

**Harder:**
- A developer who never opts in sees their own manual sessions as *unknown*.

**Watching:** the `companion-claude-code` O-row: the `hook port changed` warn line, and the
"Hooks/min vs working sessions" panel.

## Supersedes

None. It settles what ADR 0070 left to #248.
