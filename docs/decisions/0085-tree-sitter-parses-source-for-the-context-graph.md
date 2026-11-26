# 0085. Parse source for the context graph with tree-sitter, for Rust, TypeScript and JavaScript first

- **Status:** proposed
- **Date:** 2026-11-26
- **Decided by:** agent (Claude Code, claude-fable-5-2, issue #322) — proposed; that the graph holds a code graph is Mattias Jonsson's choice, 2026-11-26
- **Seam:** none (dependency)
- **Issue:** m0x6A/rubato#322

## Context

ADR 0084's repository source holds symbols, imports and references. The workspace has no parser
for source code: the plan and ADR parsers are written by hand, and AGENTS.md makes a dependency
an ADR. The repositories on the map are written in more than one language, and more will come
with #280. Design: `docs/plans/322-context-graph/design.md` §4.3.

## Decision

`rubato-server` depends on `tree-sitter`, `tree-sitter-tags` and the grammars
`tree-sitter-rust`, `tree-sitter-typescript` (TypeScript and TSX) and `tree-sitter-javascript`,
all MIT, each pinned to an exact version. They are used only inside the `context_graph` slice's
code extractor, in `spawn_blocking`.

- Definitions and references come from each grammar's tags query. Import statements are read by
  one small query per language, kept next to the extractor with a fixture file per language.
- A reference becomes an edge only when its name resolves to exactly one definition, first among
  the files the referencing file imports, then in the repository. Other references are counted
  as `unresolved` and not stored. The graph never claims compiler-grade resolution.
- A file in a language without a grammar stays a `file` node. Adding a language is a grammar
  crate, a query and a fixture, and needs no new ADR.
- A file that does not parse yields its `file` node and a count; it never fails the build.
- Extraction is a pure function from a path and bytes to nodes and edges, tested without GitHub
  and without SQLite.

## Alternatives considered

- **Regular expressions per language**: no dependency, but wrong on nested items, generics and
  comments, and a new pile of patterns for every language.
- **`syn` for Rust and the TypeScript compiler for TypeScript**: precise for two languages, but
  two toolchains, one of them Node inside the Rust server, and nothing for a third language.
- **Language servers or SCIP indexers**: real resolution, but they need each repository's build
  set up on the server, which is running other people's builds.
- **An existing code-graph tool**: see ADR 0084.

## Consequences

The server's build compiles the grammars' C sources; a C compiler is already needed for the
bundled SQLite. The binary grows by the grammars. A grammar upgrade can change which symbols are
found, so the extractor has a generator version (ADR 0084) that is bumped with it, which
rebuilds every repository source once. Nothing crosses a seam: the companion and the web never
see tree-sitter, only nodes and edges.

## Supersedes

None.
