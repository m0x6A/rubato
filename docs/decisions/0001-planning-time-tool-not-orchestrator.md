# 0001. Build a planning-time tool, not an orchestrator

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** none (non-goal)
- **Issue:** none

## Context

Each developer orchestrates their own agents. Tools that steer another developer's agent
centralise control and break the thing we want to study: federated control with a shared
picture. AGENTS.md states the test for any feature: does it enlarge the shared picture, or
does it start steering someone else's agent?

## Decision

Rubato shows the map, pins, sessions, plans, progress and decisions. It never orders work
across repositories, never runs an agent itself, never attaches to a colleague's terminal,
and does not chat. A feature that steers another developer's agent is rejected at review.

## Alternatives considered

- **Central orchestrator dispatching tasks to agents**: moves control to the tool; the theses are about keeping it with the developer.
- **Chat-based coordination inside Rubato**: duplicates where the team already talks.

## Consequences

Easier: trust, adoption, a small server. Harder: cross-repo ordering stays manual; v0 only
shows two plans on one seam (see SeamCollision alert). Watch for features that creep toward
steering.

## Supersedes

none
