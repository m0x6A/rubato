# 0015. Organise each deployable into vertical slices by capability; the wire schema, not Rust paths, is the seam

- **Status:** proposed
- **Date:** 2026-11-08
- **Decided by:** agent (Claude Code, issue #20 design session) — proposed; a human accepts
- **Seam:** companion-server, web-server
- **Issue:** m0x6A/rubato#20

## Context

v0 was written in one pass with a layered layout: one `api.rs`, one `db.rs`, one `daemon.rs`
loop, one `api.ts`. Issue #20 asks for vertical slices so that everything a capability needs sits
in one place, which is what an agent working on one issue reads. AGENTS.md says a breaking change
to `rubato-protocol` is a seam change and blocks; regrouping that crate by capability changes Rust
module paths inside the workspace while leaving the JSON wire format untouched, so the rule needs
to say which of the two is the seam.

## Decision

The three deployables with behaviour (`rubato-server`, `rubato-companion`, `web`) are organised
as one folder per capability a user or companion exercises, a `kernel/` of leaf infrastructure
that names no slice, and a small composition root (router, periodic loop, runtime) that names
every slice and is named by nobody but `main.rs` and the test crates. A slice uses another slice
only through its root module and never names the composition root; the slice graph is acyclic;
a test in each of the three reports how many slices and edges it checked, compares the declared
list with that deployable's `<!-- slices -->` block in `AGENTS.md`, and fails on a cycle, a deep
import, a kernel → slice edge or a slice → root edge, with must-fail fixtures. One slice owns
each SQLite table; other slices write through its root. `rubato-protocol` is a contract crate: it
is regrouped so that each module holds one capability's types and route constants, with `ids`,
`limits`, `message`, `otel` and `schema` as its shared modules; it has no `kernel/` folder and
no slice test. The seams `companion-server` and `web-server` are the JSON Schemas in
`schema/protocol/` (and the generated `web/src/protocol.ts`), not the Rust module paths of
`rubato-protocol`; moving a type between protocol modules without changing its name, fields or
doc comment is not a seam change. `skills/rubato` and `actions/plan-check` are already one file
per capability and are left as they are; the files `install.ts` copies into connected repos stay
byte-identical.

## Alternatives considered

- **Slices mirroring the protocol crate's modules one to one**: keeps transport-shaped files such as `web.rs`; the nouns of the progress model are the better unit.
- **A `shared` slice any slice may add to**: grows into today's `db.rs`.
- **Treating Rust paths as the seam**: would make every internal move a blocking seam change although no client can observe it.
- **A lint crate or ESLint boundaries plugin**: one more dependency for a lab; a test with fixtures does the same.

## Consequences

Easier: an agent working on one capability reads one folder; the layout cannot drift unnoticed
because the slice test compares the declared list with `AGENTS.md`. Harder: a capability that
needs another's data must go through that slice's root, and the composition root carries the
loop order that used to live inside one module. O-rows covering the move are in `docs/plans/20.md`;
the design is `docs/plans/20-vertical-slices/design.md`.

## Supersedes

none
