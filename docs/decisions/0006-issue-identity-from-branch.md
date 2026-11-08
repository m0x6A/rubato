# 0006. Issue identity comes from the branch name

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** companion-tmux-ide
- **Issue:** none

## Context

Progress is derived, never stored by hand. The chain pane cwd → worktree → branch → issue
exists already in git and tmux; a manual tag would be a second source of truth that drifts.

## Decision

Branches for issue work are named `issue/<n>-<slug>`. The companion resolves a pane's cwd to
a worktree, the worktree to its branch, and the branch to issue `<n>` in the repo named by
`origin`. Sessions whose branch does not match are shown as unpinned on that machine (A9).
No manual tags.

## Alternatives considered

- **Tag sessions by hand in the UI**: stored state that drifts.
- **Parse the first prompt for an issue number**: fragile; not reproducible from git.

## Consequences

Easier: identity is reproducible from the filesystem. Harder: the naming convention must be
followed; the companion creates branches so it usually is.

## Supersedes

none
