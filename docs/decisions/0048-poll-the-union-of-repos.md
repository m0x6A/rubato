# 0048. Poll each distinct repo of all workspaces once per tick

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-14 by approval comment https://github.com/m0x6A/rubato/issues/89#issuecomment-6876029281 (self); drafted by agent (Claude Code, issue #89 design session)
- **Seam:** server-github
- **Issue:** m0x6A/rubato#89

## Context

ADRs 0019, 0032 and 0034 list issues, plan branches and pull requests for "the repos the map
names". With several maps a shared repo would be listed once per workspace. Design D6.

## Decision

Each tick the server reads the union of `repos_named` over all workspaces, plus tracked repos,
each distinct repo once, and keeps the facts in the existing repo-keyed tables. Every workspace's
overview reads them from there. The GitHub call budget scales with distinct repos, not with
workspaces. App coverage (`uncovered_repos`) is checked against the same union and must also
include every workspace's map repo.

## Alternatives considered

- **Poll per workspace**: double cost for a shared repo and two copies that can disagree.

## Consequences

Adding a workspace that only reuses repos costs no GitHub calls beyond its map file. Covered by
the server-github O-row (`rubato.github.calls` flat as workspaces are added,
`rubato.workspace.shared_repos`).

## Supersedes

The "repos the map names" wording in 0019, 0032 and 0034; their other rules stand.
