# 0084. Generate a context graph per source on the server, on a timer, and keep it in its own SQLite file

- **Status:** proposed
- **Date:** 2026-11-26
- **Decided by:** agent (Claude Code, claude-fable-5-2, issue #322) — proposed; what the graph holds, which repositories it covers and the 15-minute default are Mattias Jonsson's choices, 2026-11-26
- **Seam:** server-github, web-server
- **Issue:** m0x6A/rubato#322

## Context

Issue #322 asks that many agents work from the same knowledge of the repositories. Today each
agent rebuilds its own picture in its own worktree, and what Rubato already joins (pins, plans,
pull requests on map parts) is in the web's Overview and nowhere an agent can ask. The server
reads one file at a time from GitHub (`GitHubApi::file`) and never a whole repository. Design:
`docs/plans/322-context-graph/design.md` §3–§5.

## Decision

`rubato-server` generates one context graph, in a new slice `context_graph`, behind
`RUBATO_CONTEXT_GRAPH`, off by default. With the flag on, the server requires the GitHub App
(ADR 0022) unless it is local in ADR 0064's sense, and refuses to start with
`RUBATO_OPEN_LOGIN=1`.

**Sources.** The graph is a set of sources, each regenerated whole and replaced whole in one
transaction:

- `repo/<owner>/<name>` for every repository the maps name (the union of ADR 0048) that the
  installation covers, built from that repository's tarball at its default-branch head, read as
  the GitHub App: files, the symbols and imports of ADR 0085, ADRs, feature scenarios and
  AGENTS.md slices. It is rebuilt when the head sha or the generator version changed.
- `workspace/<id>` for every workspace, built from its loaded map and its pins: parts, seams,
  flows, and which issue is pinned where.
- `team`, one, built from the facts the server keeps per repository (ADR 0047), through their
  slices' roots: repositories, pinned issues, plans and open pull requests.

The last two are written only when their content hash changed.

**Timer.** Every `RUBATO_GRAPH_REGEN_SECS`, default 900, clamped to 300–86400. A tick costs one
GitHub call per repository when nothing changed, and one tarball per change of head. The
`GitHubApi` port gains `default_head` and `tarball`; the compressed tarball is read under its
cap and decompressed as a stream, with its own deadline and no retry inside a tick. Repository
builds run one at a time across the server.

**Store.** A separate SQLite file, `rubato-graph.db` (`RUBATO_GRAPH_DATABASE_URL`), with tables
`sources`, `nodes`, `edges` and an FTS5 index, owned by the slice. It is derived data and is
never backed up or migrated: a schema version other than the code's deletes it, which costs one
tick. A failed build keeps the previous rows and records its error on the source. A replace
checks the source's generation inside its transaction, so a build that was overtaken or whose
source was removed writes nothing. No source is removed on a tick where the maps have not
loaded.

**What it holds, and does not.**

- Node kinds: `part`, `seam`, `repo`, `issue`, `plan`, `pull`, `file`, `symbol`, `package`,
  `adr`, `scenario`, `slice`. Edge kinds: `joins`, `flow`, `backed_by`, `pinned_to`, `plans`,
  `affects`, `touches`, `cites`, `for`, `defines`, `imports`, `references`, `supersedes`,
  `governs`. Kinds travel as strings and a reader keeps a kind it does not know, so a new kind
  is additive. Every node id is produced by exactly one source.
- No file contents: for code only names, paths, line ranges, one signature line and one doc
  line; for a decision document its Decision section, capped at 4 KiB.
- No progress state and no session. Progress stays derived (`derive_progress`), and the graph is
  never an input to it, to pins, to plans or to approvals.
- Repository text comes from the default branch only. Only pinned issues become nodes, with
  their title only when the author's association is OWNER, MEMBER or COLLABORATOR (the rule of
  ADR 0026); only pull requests whose head is in the repository.
- Which part owns a file, and which part or seam a plan or an ADR names, are resolved when
  asked, not stored, so a map change never forces a repository rebuild.

**Caps.** A tarball of 256 MiB compressed and 1 GiB read, 50 000 files, 250 000 nodes and
1 000 000 edges per source, in `rubato-protocol::limits`. Over a cap the source is marked
`truncated` and counted; it is not a failure.

**Dependencies.** `rubato-server` gains `tar` and `flate2` (both MIT OR Apache-2.0), pinned as
its other dependencies are, to read the tarball and to write snapshots (ADR 0086), and names
`http-body-util` (MIT), which octocrab already brings, to read the download in frames. The parser is
ADR 0085's.

**The Overview** carries each source's head, generated-at, checked-at, counts, `truncated` and
last error, in an additive field that is absent when the flag is off.

## Alternatives considered

- **Only what Rubato already has, no repository reads**: the smallest build, but no ADRs,
  scenarios or code, which is most of what an agent rebuilds each session.
- **Wrap an existing code-graph tool**: a new external part on the map, and its storage format
  becomes our seam.
- **Clone with `git` on the server**: the server has never run `git` in production; a tarball
  through the port needs no working tree and no credentials on disk.
- **Read the tree and each blob through the contents API**: thousands of calls for one
  repository where a tarball is one.
- **Keep the graph in the main database**: one file, but a replace of a hundred thousand rows
  would hold the single writer that hooks, the poll and web writes wait on (the `SQLITE_BUSY`
  fault fixed in #307).
- **Patch a source incrementally from the diff between heads**: less work per tick, and a second
  code path whose result can drift from a full build with nothing to compare it to.
- **Store file-to-part edges**: every map change would then invalidate every repository source.
- **Let the flag run on an open server**: any GitHub account could then pair and read the
  structure of every repository.

## Consequences

Easier: one picture for every agent, at a known head, with its age; a question such as "what
else touches this seam" has an answer outside the web.

Harder: the server now downloads and parses whole repositories, so it needs CPU and memory in
bursts; one build at a time, decompressed as a stream in `spawn_blocking`, keeps the burst to
one repository's compressed archive.
Symbol and file names of every map repository are in one more file on the server's disk. A
source can be stale, and that must never be silent: rule `ContextGraphStale` fires when no tick
has found a source current for three intervals, with a firing test.

O-rows: `server-github`: span `rubato.github.call` with endpoints `repo_head` and `tarball`,
span `rubato.graph.build` with `rubato.graph.source`, `rubato.outcome` and `rubato.reason`, gauge
`rubato.graph.source_age`, on the panels "Context graph source age" and "GitHub calls by
endpoint". `web-server`: the Overview's `graph` with `checked_at` per source, span
`rubato.ws.fanout` after a replace.

## Supersedes

None.
