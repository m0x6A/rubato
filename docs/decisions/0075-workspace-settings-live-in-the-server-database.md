# 0075. Keep workspace settings in the server's database, set at runtime; the environment seeds them once

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (issue m0x6A/rubato#189), proposed; a human accepts
- **Seam:** web-server, server-system-map, server-github
- **Issue:** m0x6A/rubato#189

## Context

What describes a workspace is set in the server's environment or in `RUBATO_WORKSPACES_FILE` (ADR
0044): its map source, its traffic source (`RUBATO_TRAFFIC_*`, one for the whole server, ADR 0040)
and the launch defaults (`RUBATO_DEFAULT_*`, also server-wide). Adding a workspace or changing one
of its settings takes a redeploy and a restart. Two applications on one server almost certainly
read different Prometheus tenants, so a server-wide traffic source does not fit workspaces. Issue
#189 draws the line between the application (the server process) and its workspaces, and asks
for workspace settings to be set at runtime. Plan `docs/plans/189.md`, design
`docs/plans/189-configure-workspace/design.md`.

## Decision

**The divider.** Every setting is either an application setting or a workspace setting, never
both, and a workspace setting is never inherited from an application default.
- *Workspace settings* are name, map source (`map_repo`, `map_path`, `map_ref`), the traffic
  source (Prometheus URL, headers, namespace labels, metric prefix, timeout, window, fetch
  interval) and the launch defaults (spend cap, attempt limit). They live in table
  `workspace_settings` in the server's SQLite database, one row per workspace, owned by the
  `workspace_settings` slice.
- *Application settings* stay in the environment, cannot change at runtime and have no API.
  They are the bind and public URL, the database, the state dir, the web dist, the session
  secret, the token key, the GitHub OAuth client and App, the GitHub URLs, `RUBATO_OTEL_*`, and
  the health thresholds: `RUBATO_GITHUB_POLL_SECS`, `RUBATO_SESSION_STALE_SECS`,
  `RUBATO_COMPANION_SILENT_SECS`.
- `map_file`, `RUBATO_DEV_TRAFFIC`, `RUBATO_DEV_ISSUES`, `RUBATO_DEV_USER` and
  `RUBATO_LOCAL_REPOS` are paths or dev switches and are never settable at runtime.

**Source of truth: the file seeds, the database owns** (#189 question 1, option a). On a start
with an empty `workspace_settings`, the server writes one row per entry of
`RUBATO_WORKSPACES_FILE`, or `default` from `RUBATO_MAP_*`. `default` also gets
`RUBATO_TRAFFIC_*` and `RUBATO_DEFAULT_*` when they are set. From then on the database is
authoritative. At start the server warns once for each of those variables, or each file entry,
that is still set and differs from the database. The ADR 0044 checks (a bad id, a duplicate id,
two workspaces on one map, no `default`) run at seed and on every runtime change, which is
refused with the same reason. `default` cannot be deleted. A deleted workspace's rows are kept
and counted as orphans (`SCOPED_TABLES`).

**Runtime changes.** Any logged-in team member may create, edit or delete a workspace (#189
question 2, option a; ADR 0022's gate; writes are same-origin, ADR 0066). A change takes effect
without a restart: that workspace's map is reloaded and its traffic task restarted, and no other
workspace is touched.

**Fixed after create.** `map_repo` and `map_path` are fixed once a workspace exists, because
pins, layout, drafts and dismissals are keyed to that map's ids. `map_ref` may move (#189
question 3, option b). A new `map_repo` must be a repository of the GitHub App installation, and
its map must load and validate before the change is stored. Without the App, a workspace with a
new `map_repo` cannot be created at runtime.

**Traffic source, one per workspace.** This amends ADR 0040's "Configuration" paragraph; the rest
of 0040 stands. The traffic source becomes a workspace setting, and each workspace with a URL
has its own fetch task and report. A workspace that sets nothing has no traffic source.
`RUBATO_DEV_TRAFFIC` stays a process dev switch and still feeds every workspace; storing a
Prometheus URL while it is set is refused, as starting with both is today.

**The URL is an outbound request** (#189 question 4, option b). It must be http or https. The
host is resolved when the server connects, and a loopback, link-local, unspecified or multicast
address is refused unless the server itself is local (ADR 0064's `is_local`). Redirects are not
followed. Private LAN ranges are allowed, because the team's Prometheus is on one.

**Secrets are write-only.** Traffic headers are stored sealed with `RUBATO_TOKEN_KEY`, the same
seal as user tokens, and are never returned. The API says "set" or "not set". Without the key,
storing a header is refused and the reason is given.

**Every change is recorded** with who made it, when and which keys changed, never the values.
The record is the info log line `workspace config changed` and the span
`rubato.workspace.config_changed` with `rubato.workspace`, `rubato.user` and the changed key
names.

## Alternatives considered

- **Workspace-declared-in-file is read-only in the UI (#189 1b)**: two classes of workspace, and
  the env traffic source would still be server-wide for the file's workspaces.
- **No file at all (1c)**: a running deploy would lose its workspaces on upgrade unless someone
  recreated them by hand.
- **Keep everything in the workspaces file, with per-workspace keys and a reload signal**: Git
  stays the truth, but a workspace still cannot be created from the view. It is the fallback if
  Git must own the config.
- **A settings file next to the map, changed by pull request (like ADR 0018)**: reviewed and
  versioned, but secrets cannot live there, and it adds one more file and fetch per workspace.
- **An admin list (2b) or map-repo permission (2c)**: a role model this lab does not have yet.
  Any member is the most reversible choice, and an admin list can be added in front of the same
  routes later.
- **Map source changeable after create (3a)**: every pin and draft of the old map would become
  an orphan in one click.
- **A host allowlist in the environment (4a, 4c)**: a new application setting that every
  workspace change would depend on. The scheme and address check covers the metadata and
  loopback cases. An allowlist can be added later without changing this ADR.

## Consequences

- Adding a workspace, or pointing one at its own Prometheus tenant, no longer needs a deploy.
- The deployed configuration can drift from Git. The start-up warnings make that visible, and
  the change log line says who changed what.
- `Config::workspace_entries` gives way to a registry in the server's state, which the slice
  writes. Every reader takes the registry's current list.
- The traffic report, its gauges and `rubato.traffic.fetches` are per workspace.
- O-rows:
  - `web-server`: refusals are counted as `rubato.web.rejected{reason}` on the settings routes;
    changes are the span and log line above.
  - `server-system-map`: a reload after a change is the existing `rubato.map.load` span with
    `rubato.workspace`.
  - `server-github`: the installation check is the existing `installation_repositories` call.

## Supersedes

The "declared in the server config" part of 0044: the workspaces file now seeds the list once
instead of holding it. ADR 0044's checks, ids and `default` rule stand. This ADR also amends
0040's "Configuration" paragraph: the traffic source is chosen per workspace, not by
`RUBATO_TRAFFIC_PROMETHEUS_URL`.
