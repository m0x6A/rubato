# 0023. Poll GitHub per repo, and comments and closed issues on a slower lane

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** proposed by agent (Claude Code, claude-opus-5-5, issue #49); awaiting Mattias Jonsson
- **Seam:** server-github
- **Issue:** m0x6A/rubato#49

## Context

Each tracked issue cost about six GitHub calls per poll tick (issue, every branch of the repo,
plan file, pull request, checks, comments), so about a dozen tracked issues used the 5000-per-hour
budget at the default 60 s tick, and closed pinned issues stayed tracked. Design:
`docs/plans/49-poll-cost/design.md` (D1).

## Decision

On every tick the server lists, per repo that has a tracked issue needing it, the `issue/` branches
with their head commits and the open pull requests, once, and shares them across that repo's
issues. Title and state come from the open-issues listing (ADR 0019) when the issue is in it. The
plan file is read again only when the branch head or the plan path changes. `checks` is read every
tick for every open pull request. Comments, the look-up of a pull request on a branch that has
never had one, and issues in repos the map does not list are read on a full tick every 300 s (and
on an issue's first tick after start). A closed issue with no open pull request and no live
session, after one last successful full pass, makes no calls of its own until it is open again.
A stored open pull request whose branch is gone is looked up by its own branch.

## Alternatives considered

- **Conditional requests (ETag, 304 not counted)**: needs hand-written requests and an ETag cache
  for each endpoint, since octocrab's typed calls hide the headers. Kept as a later step for
  `checks`.
- **Fewer ticks**: a longer default interval delays every state for everyone.
- **Comments on their own timer**: a second cadence for one call kind.

## Consequences

About 1.2 calls per active issue per tick plus 2 per repo (from about 6 per issue). New `Decision:`
comments show up within 300 s instead of 60 s. A comment added after an issue closed is not read
until it reopens or the server restarts. A reopened issue in a repo the map does not list is seen
within 300 s.

## Supersedes

None.
