# 0014. Record decisions as issue comments starting with `Decision:`

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (drafted by agent, Claude Code v0 planning session)
- **Seam:** server-github
- **Issue:** none

## Context

Decisions made during an issue must show up in context on the map without a new store and
without Rubato becoming a chat. Issue comments are where the team already records them.

## Decision

A decision is an issue comment whose first line is `Decision: <one-line title>`, followed by
the headings `Context`, `Options`, `Chosen` and `Why`. The server polls comments on pinned
issues, parses comments matching this shape into `DecisionRecord`, and shows them on the
issue's node. Comments that do not match are ignored. The skill offers the template.

## Alternatives considered

- **Key: value block**: harder to read on GitHub.
- **Issue form via a `decision` label**: label plumbing per repo.
- **ADR files only**: right for seam decisions, too heavy for inside-a-node ones.

## Consequences

Easier: nothing new to store. Harder: polling cost on `server-github`; the rate-limit O-row
and `GitHubRateLimitLow` alert cover it.

## Supersedes

none
