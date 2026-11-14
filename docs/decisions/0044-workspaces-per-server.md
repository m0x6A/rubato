# 0044. Hold several workspaces on one server, each one map, declared in the server config

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, issue #89 design session); a human accepts
- **Seam:** server-system-map, skill-system-map
- **Issue:** m0x6A/rubato#89

## Context

Issue #89 asks for one view that combines several repositories and shows what each provides to
the application, with *workspace* as the umbrella. Today a server loads one map (ADR 0010,
`RUBATO_MAP_*`), so a team with two applications needs two servers, and a repo shared by both
(the shared authentication in AGENTS.md) shows in neither as part of the whole. The developer-local
`workspace.yml` already uses the word for a machine's clones. Design:
`docs/plans/89-workspaces/design.md` (D1, D2, D8).

## Decision

A workspace is one map, the application it describes, and that map's view state on the server.
A server holds one or more. They are listed in a YAML file named by `RUBATO_WORKSPACES_FILE`:
`{id, name?, map_repo, map_path, map_ref, map_file?}` per entry, ids matching `[a-z0-9-]{1,32}`,
in the order the switcher shows them. The file must hold an entry with id `default`: it keeps
every existing row (migration 0013 writes `default` into them) and it is what unprefixed routes
serve (ADR 0049). When the variable is unset, the legacy `RUBATO_MAP_*` variables make one
workspace with id `default` and nothing else changes. Both set, a duplicate id, a bad id, no
`default` entry, or two entries naming the same repo (compared without case), path and ref: the
server refuses to start and says why. A relative `map_file` is relative to the workspaces file. Every workspace's map is read and validated as ADR 0010 says for
the one map. Workspaces divide the view, not access: everyone who may log in (ADR 0022) sees every
workspace. On a developer machine the local config is renamed `~/.config/rubato/local.yml`
(`RUBATO_LOCAL`), with `maps:` as a list of map clones; `workspace.yml`, `RUBATO_WORKSPACE`
and a single `map:` are still read, with one deprecation warning.

## Alternatives considered

- **An index file in a repo**: one more fetch and format, and the question which repo holds it; workspaces are rarely added.
- **Each map declares its own id**: changes `system-map.schema.json`; ids are unknown until maps load, so a cold start with GitHub down cannot route.
- **One server per application**: no shared view of a shared repo, and a second login, pairing and companion per team.
- **Keep `workspace.yml`**: the word would mean two things.

## Consequences

Easier: one server for a team's applications. Harder: every map-scoped table, route and metric
carries a workspace (ADRs 0047, 0049); developers rename one file. Watch: the server-system-map
O-row, `rubato.map.loaded_at` and `rubato.map.load_failures` per `rubato.workspace`, and the
start-up log `workspace config` naming `source=file|legacy_env`.

## Supersedes

0010 (its decision holds per workspace; the server config now names one or more maps, and the
local config lists one or more map clones).
