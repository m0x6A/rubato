# 0010. Keep the team's map in one designated repo; the skill reads the local clone, the server reads it via GitHub

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (drafted by agent, Claude Code v0 planning session)
- **Seam:** skill-system-map, server-system-map
- **Issue:** none

## Context

The map is one file for several repos. The skill needs it during planning on a developer
machine; the server needs it to resolve pins and plans. Fetching it from the server would
make planning depend on the server being up.

## Decision

The server config names one repo and path for `system-map.yml`. The developer's
`workspace.yml` lists the local clone of that repo; the skill reads the file from there and
records its git sha as `rubato.map.version`. The server fetches the same path through
`server-github` and validates it on load. A version mismatch between a launch and the server
is a finding on the map, not an error.

## Alternatives considered

- **Server as the source, skill fetches `/api/map`**: planning stops when the server is down.
- **A copy in every repo**: n copies drift; the check would mostly report drift.

## Consequences

Easier: planning works offline. Harder: developers pull the map repo; the `rubato.map.version`
attribute shows when they have not.

## Supersedes

none
