# 329: the context graph's copy on each companion, the tools, and the Repos view

Plan: [../329.md](../329.md) · Issues: #329 #330 #331 #332 #333 · Parent design:
[../322-context-graph/design.md](../322-context-graph/design.md) (cited below as "parent")
· ADRs 0084 and 0086 (proposed with plan #322).

The parent decides what happens. This note says where each slice lives in code. Where the two
differ, the parent wins.

## 1. Server: announce and route (#329)

`context_graph/announce.rs` (who is told, and when) and `context_graph/snapshot.rs` (the route),
both through `rubato_graph::Store::export`. `companion_link` stays unaware of the slice: the
announce listens on `app.subscribe_welcomed()`, as `plan_status` does. Who may read is one
function, used by the announce and by the route. Parent §6.1, §6.2.

## 2. Companion: the copy (#330)

Slice `graph`: `mod.rs` (the root: the handle other slices read through, the status for the
tools), `sync.rs` (fetch, check, apply, remove, marks), `fetch.rs` (the HTTP call behind a small
trait, so tests give it a fake). The copy is `<state dir>/graph.sqlite`. The runtime gives the
slice each `GraphSources` and the server's capabilities; the slice names no other slice. Parent
§6.3.

A source the server announces at the hash the copy has, but at another head, only has its row's
head updated: the graph is the same.

## 3. Tools (#331)

`agent_mcp/graph_tools.rs`: the five tools as a table of `(name, description, schema)`, all
`read_only`, answered through the `graph` slice's root. `graph_context` is one query in
`rubato-graph` (`query.rs`): it resolves a path to its owning parts, a `map:<id>` to the parts
and seams that carry the id, and collects what the parent's §7 lists. Parent §7.

## 4. By hand (#332)

`agent_mcp/read.rs` for `/mcp/read` (the same handler with a mode that lists the read-only
table only), `agent_mcp/bridge.rs` for `rubato-companion mcp` (stdin and stdout, one HTTP call
per message, no state). Parent §7.2.

## 5. Web (#333)

`web/src/features/repos/`: a line per repository from `overview.graph`; `shared/format.ts` for
the age. Parent §8.
