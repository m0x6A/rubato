# 0005. The plan travels as a file in the repo, not through an API

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** skill-plan-file, server-plan-file
- **Issue:** none

## Context

The agent's plan has to reach the shared picture, survive the session that wrote it, be
reviewable in a PR, and be checkable in CI. A plan posted to an API is none of those.

## Decision

The skill writes the plan as Markdown on the issue branch at a known path (0009). The server
reads it from GitHub; the plan check reads it in the Action. There is no "submit plan"
endpoint. The plan's affected-parts section is what paints the map.

## Alternatives considered

- **POST /plans from the session**: bypasses review and CI; lost if the server is down.
- **Issue comment**: not versioned with the code it describes.

## Consequences

Easier: review, history, CI gate. Harder: the server polls for it (A13) and must parse the
same shape as the TS check (O-row `server-plan-file`).

## Supersedes

none
