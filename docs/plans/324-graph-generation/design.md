# 324: the context graph's protocol and the server's generation

Plan: [../324.md](../324.md) · Issues: #324 #325 #326 #327 #328 · Parent design:
[../322-context-graph/design.md](../322-context-graph/design.md) (cited below as "parent")
· ADRs 0084, 0085, 0086 (proposed with plan #322).

The parent decides what the graph is and why. This note only says how the first five slices are
laid out in code, where the parent leaves that open. Where the two differ, the parent wins and
this note is wrong.

## 1. Protocol (#324)

A new flat module `crates/rubato-protocol/src/graph.rs`, re-exported at the root, with no I/O.

- `GraphNode { id, kind, name, repo?, path?, line?, end_line?, summary?, detail? }` and
  `GraphEdge { from, kind, to, detail? }`. `kind` is a `String`; the known kinds are constants in
  `graph::kind::node` and `graph::kind::edge` (parent §3.2, §3.3). `detail` is a JSON object.
- `GraphSourceInfo { source, kind, head_sha?, map_version?, content_hash, generated_at, nodes,
  edges, truncated }`, and `SourceKind` (`repo`, `workspace`, `team`).
- `GraphSources { sources }` with `check()` refusing more than `limits::GRAPH_SOURCES_MAX`;
  `ServerMessage::GraphSources(GraphSources)`.
- `GraphSnapshotHeader { format, source, kind, head_sha?, map_version?, generator, content_hash,
  generated_at, nodes, edges, truncated }` and `GraphSnapshotLine` (`node` or `edge`), the lines
  of the snapshot body (parent §5, §6.2).
- `GraphOverview { regen_secs, sources }` and `GraphSourceStatus` (a `GraphSourceInfo` whose
  hash and generated-at may be absent before the first build, plus `checked_at` and `error`);
  `Overview.graph: Option<GraphOverview>`, skipped when absent.
- `graph::source_key` helpers: `repo(owner_name)`, `workspace(id)`, `TEAM`, and `parse`.
- `graph::routes::GRAPH_SNAPSHOT = "/api/graph/snapshot"`, `graph::reason::*` for the route's
  refusals.
- `capability::CONTEXT_GRAPH`, the caps of parent §4.5 in `limits`, the names of parent §10 in
  `otel` and in `all_names()`.

The companion's `server_link` gets a match arm that ignores `GraphSources` (it never announces
the capability before #330, so a correct server never sends it).

## 2. The crate (#325)

`crates/rubato-graph`, a library with `sqlx` (sqlite), `flate2`, `sha2`, `serde_json` and
`rubato-protocol`. Modules: `schema` (DDL and `user_version`), `store` (`open`, `open_read_only`,
`replace_source`, `remove_source`, `sources`), `snapshot` (`write` returning the hash, `read`
checking it), `query` (search, node, related, context). It holds no slice and names neither
binary.

## 3. The server slice (#325)

`crates/rubato-server/src/context_graph/`: `mod.rs` (root: status for the Overview), `config`
reads in `kernel/config.rs`, `store.rs` (the pool on the graph file). The pool is opened by the
slice, not by `kernel/db`, since the kernel names no slice and the file is the slice's own.

## 4. The repo source and the timer (#326)

`context_graph/{supervisor,build,extract/mod,extract/docs}.rs`. `extract` is pure: an iterator
of `(path, bytes)` in, nodes and edges out. `build` owns the tarball stream, the caps and the
replace. The task is `Kind::Essential`, like `traffic`: a loop that never ends on an error.

## 5. Workspace and team sources (#327)

`context_graph/{workspace,team}.rs`, each a pure function from what the slice read through the
other slices' roots to nodes and edges, so the untrusted-text rules are tested without a server.

## 6. Code (#328)

`context_graph/extract/code.rs` and one query file per language under
`context_graph/extract/queries/`, with a fixture source file and its expected nodes per
language.
