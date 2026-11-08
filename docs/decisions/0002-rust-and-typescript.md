# 0002. Use Rust for companion and server, TypeScript for frontend and skill scripts

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (recorded in AGENTS.md)
- **Seam:** none (dependency)
- **Issue:** none

## Context

Two binaries run unattended on developer machines and a shared host; one frontend and a set
of skill scripts run where Node already is. The team wants one static binary per machine and
no Python anywhere in the repo.

## Decision

`rubato-protocol`, `rubato-companion` and `rubato-server` are Rust, edition 2024, stable
toolchain, `cargo fmt` and `cargo clippy -D warnings` clean. `web` and
`skills/rubato/scripts` are TypeScript, Node 20+, pnpm, strict, Vitest. No Python. Types
cross the language line only as JSON Schema exported from `rubato-protocol`.

## Alternatives considered

- **Go for the binaries**: fine, but the team's current stack and the OTel SDK choice favour Rust.
- **TypeScript everywhere**: a Node daemon on every developer machine is heavier to ship and supervise.
- **Python for skill scripts**: another runtime on every machine; Node is already there.

## Consequences

Easier: one static companion binary; shared types. Harder: two toolchains in CI; the
generated `web/src/protocol.ts` must be diffed (O-row `web-server`).

## Supersedes

none
