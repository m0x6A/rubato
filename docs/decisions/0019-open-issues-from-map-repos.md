# 0019. List open issues for the repos named in the map

- **Status:** proposed
- **Date:** 2026-11-10
- **Decided by:** agent (Claude Code, issue #48 design session); proposed for Mattias Jonsson to accept
- **Seam:** server-github
- **Issue:** m0x6A/rubato#48

## Context

The server learns about an issue only once it is pinned, launched, planned or has a session, and reads it one at a time. Issue #48 (and #47) ask that all open issues are visible, "because we don't know whether they are to be pinned or not if we don't see them". Design: `docs/plans/48-interactive-map/design.md` (D1).

## Decision

On every poll tick the server lists the open issues of each repo the map names (in `repos[]` or on a node's `repo`), pull requests excluded, at most 500 per repo, and replaces that repo's rows in a table `open_issues` owned by `github_poll`. The overview carries the ones not already tracked as a separate `backlog`, with the time of the last good listing, whether the list was cut, and the last error per repo. Backlog issues have no progress state. There is no server-side label filter.

## Alternatives considered

- **Unpinned issues inside `issues` with state Unknown**: every open issue would then cost five calls per tick in the per-issue poll.
- **Listing on demand from the browser**: needs a GitHub token per user and gives each person a different list.
- **A separate list of repos in the server config**: a second place to name repos; the map already has one.

## Consequences

Every open issue of the team's repos is visible to everyone who can log in, private repos included, and can be pinned by picking it; who can log in is ADR 0022 (people with access to the App's installation). Cost: one to five listing calls per repo per tick. A listing that fails keeps the previous rows and says so; covered by the `server-github` O-row (`rubato.backlog.listed_at` on "Backlog age", the `open issues listing failed` log line). A repo with more than 500 open issues shows the first 500 and says so.

## Supersedes

None.
