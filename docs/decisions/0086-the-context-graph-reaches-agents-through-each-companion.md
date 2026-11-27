# 0086. The context graph reaches agents through each companion: a synced copy, read-only tools, and a stdio bridge for sessions started by hand

- **Status:** accepted
- **Date:** 2026-11-26
- **Decided by:** Mattias Jonsson (m0x6A), accepted in the Claude Code session 2026-11-27 ("Approve all"); drafted by agent (Claude Code, claude-fable-5-2, issue #322); that agents read the graph through each person's companion, from a copy the server sends, is Mattias Jonsson's choice, 2026-11-26. That every such companion receives every source is his answer to plan question Q1, 2026-11-27.
- **Seam:** companion-server, companion-mcp
- **Issue:** m0x6A/rubato#322

## Context

ADR 0084 puts a context graph on the server. Issue #322 wants every agent to read it over MCP,
read-only, the ones Rubato launches and the ones people start by hand. Three accepted decisions
stand in the way as written: ADR 0071 gives the companion's MCP three tools "and no others
without a new ADR" and serves launched sessions only; ADR 0078 keeps MCP from sessions started
by hand, because they have no launch id; and ADR 0011 sends the pairing token on the websocket
upgrade only. A pairing is also never re-checked against the team, while ADR 0022 lets a web
session count only with a team record at most 7 days old. Design:
`docs/plans/322-context-graph/design.md` §6–§7.

## Decision

**A copy on each machine.** A companion that can keep a copy announces capability
`context_graph` in `Hello`; a server with the flag of ADR 0084 on announces it in `Welcome`. To
such a companion the server sends `ServerMessage::GraphSources`, the whole list of sources (at
most 64) with each one's head, content hash and counts, after every `Welcome` and whenever a
source was replaced or removed. The companion fetches each source that differs from its copy and
deletes the ones the list no longer names. A `Welcome` without the capability changes nothing in
the copy.

**The bytes go over the server URL, not the websocket.** `GET /api/graph/snapshot?source=…`
answers gzipped NDJSON: a header line, then one line per node in id order and per edge in
(`from`, `kind`, `to`, `detail`) order, with a sha256 over the lines after the header, computed
by the code that serves them. At most 64 MiB compressed and 512 MiB uncompressed. The companion
reaches it through the same gate as the websocket: TLS, except on loopback or for a pairing made
with `--insecure` (ADR 0067).

**Who may read.** The companion sends its pairing token as a bearer. The server answers, and
sends `GraphSources`, only for a known, unrevoked pairing whose owner has a fresh team record:
a web session of that login whose `team_checked_at` passes ADR 0022's rule of at most 7 days.
A stale or absent record is 403 `team_record_stale`; an unknown or revoked token is 401. The
token opens this one route and no other; every web route still refuses it. The route is 404 with
the flag off.

**The copy** is `<state dir>/graph.sqlite` (0600), written one source per transaction after the
hash matched, by the crate the server uses (`crates/rubato-graph`). A failed fetch, a mismatch
or a 403 keeps the old copy; a 403 marks it not refreshed. A 401 deletes it.

**Five read-only tools** join the companion's MCP: `graph_status`, `graph_search`, `graph_node`,
`graph_related` and `graph_context`. They answer from the copy through a read-only connection,
with or without the server, and every answer carries each source's head, generated-at and
whether a newer one has been announced. No others without a new ADR. No tool argument is
recorded on a span or a log line.

**Launched sessions** get them on `POST /mcp`, under the identity rule of ADR 0071, which
stands: a graph call without a known launch id is refused there too.

**Sessions started by hand** get them through two things:

- `POST /mcp/read` on the same loopback receiver, behind the same token check, body limit and
  loopback `Origin` check as `/mcp`. It needs no launch id and lists only tools marked read-only
  that answer from the copy. Any other tool name is an unknown tool, as on `/mcp`.
- `rubato-companion mcp`, an MCP server over stdio that a developer adds to Claude Code once.
  Rubato installs nothing. The bridge reads the receiver's port and token from
  `hook-endpoint.json` for each message and posts to `/mcp/read`, so no config file holds a
  token or a port. It passes the working directory's repository as a default for a read, which
  selects no identity. It answers `companion_down` when nothing listens.

**Room forks stay without MCP** (ADR 0071). The bridge lists no tools when `RUBATO_LAUNCH_ID` is
set, and none when `RUBATO_FORK_TURN` is set, which the companion sets for every fork turn.

**Read-only, checked.** A companion test calls every graph tool, prints how many, and fails if
the outbox gained an envelope or the copy's file changed, with a must-fail fixture tool that
writes. A second fails if `/mcp/read` lists a tool that is not read-only. A third runs the
bridge with a fork turn's environment and finds no tools.

**Dependencies.** `rubato-companion` gains `sqlx` with the `sqlite` feature, `flate2` and `sha2`
(all MIT OR Apache-2.0), pinned as its other dependencies are. The workspace already compiles
`sqlx` and `sha2` for the server.

## Alternatives considered

- **MCP on `rubato-server` with a read token per person**: one place and no copies, but a new
  credential on every machine and CI runner, and no answer when the server is unreachable.
- **A request and a result per call over the websocket**, as the fleet tools do (ADR 0072): no
  copy to keep fresh, but nothing with the link down, and a round trip through the server for
  every search.
- **Snapshots in chunks on the websocket**: no new route, but a launch or a session message
  would queue behind tens of megabytes, and every envelope is stored in `companion_events`.
- **Send SQLite files instead of NDJSON**: no import step, but the wire format would be a
  database file rather than a JSON Schema under `schema/protocol/` (ADR 0015).
- **Keep the copy in memory**: no SQLite in the companion, but hundreds of megabytes in a
  background process at ADR 0084's caps.
- **Let the pairing alone open the snapshot route**: no weekly login for the owner of a headless
  companion, but a credential that never expires would keep reading every repository's structure
  after its owner left the team, against ADR 0022.
- **Re-check the owner's access with their stored GitHub token on each fetch**: no login needed,
  but a GitHub call per companion per fetch, and a second place that decides who is on the team.
- **Lift `/mcp`'s launch rule for read-only tools**: one route, but "a call without a known
  launch id is refused" would gain an exception inside the route that carries identity.
- **A static `.mcp.json` with the loopback URL and token**: a secret in a file that projects
  commit, and stale after every companion restart on a new port, the fault ADR 0078 names.
- **`--strict-mcp-config` on fork turns**: keeps the bridge out of forks, and also removes the
  developer's own MCP servers from them, which is a change nobody asked for.
- **A read-only tool that runs SQL**: every question answerable, but the schema would become a
  contract with every agent's prompt.

## Consequences

Easier: an agent started by hand reads the same graph as a launched one, with no credential of
its own; the graph answers on a laptop with no network, and says how old it is.

Harder, and to watch:

- **Every paired companion whose owner has a fresh team record receives every source** (plan
  Q1, decided). File and symbol names of every repository the maps name reach every such machine,
  whichever repositories that person works in. A team that needs per-repository access needs a
  later ADR.
- **The owner of a headless companion logs in to Rubato at least once a week**, or its copy
  stops being refreshed and says so.
- The pairing token is now also sent on one more route. On the team's own plain-http server it
  is as exposed there as it already is on the websocket.
- A copy can be behind, and every companion could be behind at once with the server looking
  healthy. Rule `ContextGraphCopyBehind` fires when a connected companion's copy has been behind
  the announced list for three intervals, with a firing test.
- The loopback receiver carries one more route. `/mcp/read` shares `/mcp`'s body limit, and the
  flood test that keeps `/hook` answering covers it.

O-rows: `companion-server`: span `rubato.graph.snapshot.serve` and counter
`rubato.graph.snapshots` on the server, span `rubato.graph.sync` and gauge
`rubato.graph.copy_behind` on the companion, on the panel "Context graph snapshots served and
synced". `companion-mcp`: span `rubato.mcp.call` and counter `rubato.mcp.calls` with
`rubato.mcp.tool` = `graph_*` and `rubato.mcp.client` = `launched` or `manual`, on the existing
panel "MCP calls by tool and outcome".

On acceptance, each of these ADRs gets "superseded in part by 0086" after its status: 0004,
0011, 0062, 0071 and 0078.

## Supersedes

In part:

- **0004**, as amended by 0071: the loopback port also serves read-only context graph calls from
  sessions the same OS user started by hand. Outbound only, and only the paired owner starts
  sessions: both stand.
- **0011:** "The token is sent as a bearer on the websocket upgrade" reads "… on the websocket
  upgrade and on the context graph snapshot route". The rest stands.
- **0062:** the client contract's lists gain `GraphSources` in, the snapshot route, and the
  `context_graph` capability.
- **0071:** "Three tools, and no others without a new ADR" gains the five graph tools; "It
  serves only sessions Rubato launched" stands for `/mcp` and for the three tools, and no longer
  describes the whole receiver. The identity rule, the report rule, "room forks get no MCP" and
  "no tool writes an approval" stand.
- **0078:** "ADR 0071's MCP stays for sessions a companion launched" stands for `/mcp`; a
  session started by hand may read the context graph through `rubato-companion mcp`. Nothing is
  installed at user level by Rubato: the developer adds the server themselves, and that is the
  opt-in to its calls being counted.

ADR 0052 is not changed: nothing new leaves the machine. ADR 0010 is not changed: the skill
still reads the map and the ADRs from the local clone. ADR 0022 is not changed: its 7-day rule
is the one this decision applies to pairings that read the graph.
