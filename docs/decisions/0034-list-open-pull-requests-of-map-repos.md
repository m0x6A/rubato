# 0034. List every open pull request of the map's repos; read a pull request's detail on demand

- **Status:** accepted
- **Date:** 2026-11-12
- **Decided by:** Mattias Jonsson (m0x6A), accepted 2026-11-12 by approval comment https://github.com/m0x6A/rubato/issues/82#issuecomment-6837330107 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #82); budget choice (Q3) and fork rule (Q4) answered by Mattias Jonsson 2026-11-12
- **Seam:** server-github
- **Issue:** m0x6A/rubato#82

## Context

Issue #82 asks to see the open pull requests of every connected repo with their checks, review
state and mergeability, and a pull request's view with its description, checks, reviews and changed
files. Today the server lists open pull requests only for repos with a tracked issue branch and keeps
only the one on that branch (ADR 0023); nothing reads reviews, files or mergeability. Reading all of
that for every open pull request on every tick would cost about three calls per pull request per
minute. Design: `docs/plans/82-pulls-in-rubato/design.md`.

## Decision

On every tick the server lists one page (100) of open pull requests for every repo the map names,
whether or not it has a tracked issue, and replaces that repo's rows in a table `repo_pulls` owned
by `github_poll`, with the time of the last good listing, whether it was cut and the last error. The
per-issue pass uses the same listing, so a repo is listed once per tick. Pull requests whose head is
in a fork are left out, as today. Checks of an open pull request not tied to a tracked issue are read
on the full lane (every 300 s) when its head sha changed since the last read or its stored checks
are not finished (pending, or none yet); a finished result at an unchanged head is not read again.
A tracked issue's pull request keeps its checks where ADR 0023 reads them, and the listing takes them
from there rather than storing a copy. A pull request's detail (body, check runs, reviews, changed
files, mergeable, the plan file at its head when its branch names an issue, and, with the person's
token, their permission on the repo and its allowed merge methods) is read only when someone opens
its view, for any pull request of a repo the map names and the installation covers, cached in memory
for 30 s per pull request, with one fetch in flight per pull request; a merge's fresh read replaces
the cached entry and any merge outcome evicts it. Budget: per tick, +1 call per map repo with no
tracked issue; per open pull request outside tracked issues, at most 1 checks call per 300 s; per
opened view, 4 to 6 calls as the installation (pull, reviews, one page of files, check runs at the
head; check runs at the base head only when the head has none; the plan file only when the branch
names an issue) and 1 as the person (the repo); per merge, the same fresh reads again, 1 merge call
as the person and at most 1 reconcile read. On acceptance, ADR 0023's status becomes "accepted; pull
request listing extended by 0034".

## Alternatives considered

- **Checks, reviews and mergeable for every pull request every tick**: about 3 calls per pull request per minute; the budget would scale with open pull requests rather than with people looking (Q3).
- **One GraphQL query per repo**: everything in one call, but AGENTS.md and ADR 0008 keep GraphQL for Projects v2 only; it would need its own superseding ADR.
- **List fork pull requests too**: the repo is public; a fork's pull request comes from outside the team (Q4).

## Consequences

The Repos view shows every open pull request within a tick, its checks within 300 s, and the rest
when opened. Mergeability can read "still checking" on a first view, since GitHub computes it lazily.
A repo with more than 100 open pull requests shows the first 100 and says it was cut. O-row:
`server-github` (`rubato.github.calls{rubato.github.endpoint}` on the "GitHub calls by endpoint"
panel, `rubato.pulls.view_fetches{rubato.outcome}`).

## Supersedes

The pull request listing scope in 0023 only ("per repo that has a tracked issue needing it, … the
open pull requests"). The rest of 0023 stands.
