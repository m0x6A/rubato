# 0008. Join issues to map nodes in SQLite first; Projects v2 fields are the later candidate

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** server-github
- **Issue:** none

## Context

The pin (issue → node or seam) is Rubato's one piece of state that GitHub does not hold.
Projects v2 custom fields could hold it, but the model is not settled and the GraphQL
surface is costly to get right.

## Decision

`rubato-server` stores pins in SQLite (`sqlx`, migrations in `crates/rubato-server/migrations`).
`graphql_client` and Projects v2 are not used in v0. When the pin model settles, a
superseding ADR may move pins to a Projects v2 field.

## Alternatives considered

- **Projects v2 field now**: GraphQL surface before the model is known.
- **Issue labels `node:<id>`**: pollutes labels; one label per node per repo.

## Consequences

Easier: fast start, simple tests. Harder: pins are not visible in GitHub; the web view is
the only place. Watch `server-github` rate limit as polling grows.

## Supersedes

none
