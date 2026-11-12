# 0032. Discover plans by branch prefix in every map repo

- **Status:** accepted
- **Date:** 2026-11-12
- **Decided by:** Mattias Jonsson, accepted 2026-11-12 (drafted by agent, Claude Code, claude-opus-5-5, issue #80)
- **Seam:** server-github
- **Issue:** m0x6A/rubato#80

## Context

Issue #80 needs every plan in a connected repo visible, including plans on branches of issues
nobody pinned or launched. ADR 0023 lists `issue/` branches only for repos with a tracked issue,
using the branch listing (every branch, filtered by the client), and tracks only pinned, launched,
session or plan issues. Design: `docs/plans/80-plans-in-view/design.md` §1.

## Decision

On every tick the server lists, for every repo the map names, the refs under `heads/issue/` with
`git/matching-refs` (filtered by the server), and uses that listing for tracked issues too. For an
open issue from the ADR 0019 listing that is not tracked and has a matching ref, it reads the plan
file at that head once per head; it tracks the issue only when a plan file was found. Untracked
issues cost no pull-request, comment or checks call. When several refs match an issue, the plan is
read from the branch the newest `Approved:` names, else the branch of the issue's open pull request
into the default branch, else the oldest matching ref that carries the plan, else the first by
name.

## Alternatives considered

- **Track every open issue with a branch**: each costs pull-request and comment reads every full tick, plan or not.
- **Keep listing branches only for repos with tracked issues**: a plan in a repo nobody has touched yet never shows.

## Consequences

+1 call per tick per map repo with no tracked issue; pages count `issue/` refs only. One contents
read per head change of an untracked issue's branch. The fake counts pages so the budget test can
fail. O-row: server-github (`rubato.github.calls{rubato.github.endpoint}`).

## Supersedes

0023's "per repo that has a tracked issue needing it" and the branch listing it implies. The rest
of 0023 stands.
