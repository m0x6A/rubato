# 0075. Keep workspace settings in the server's database, set at runtime; the environment seeds them once

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (issue m0x6A/rubato#189), proposed; a human accepts
- **Seam:** web-server, server-system-map, server-github
- **Issue:** m0x6A/rubato#189

## Context

What describes a workspace is set in the server's environment or in `RUBATO_WORKSPACES_FILE` (ADR
0044). That covers its map source, its traffic source (`RUBATO_TRAFFIC_*`, one for the whole
server, ADR 0040) and the launch defaults (`RUBATO_DEFAULT_*`, also server-wide). Adding a
workspace or changing one of its settings takes a redeploy and a restart. Two applications on one
server almost certainly read different Prometheus tenants. Issue #189 draws the line between the
application (the server process) and its workspaces, and asks for workspace settings to be set at
runtime. Plan `docs/plans/189.md`, design `docs/plans/189-configure-workspace/design.md`.

## Decision

**The divider.** Every setting is an application setting or a workspace setting, never both, and
a workspace setting is never inherited from an application default.

*Workspace settings* are stored in table `workspace_settings`, one row per workspace, owned by
the `workspace_settings` slice:
- name;
- map source: `map_repo`, `map_path`, `map_ref`;
- traffic source: which adapter, its fields and its secrets;
- launch defaults: spend cap, attempt limit.

*Application settings* stay in the environment, cannot change at runtime and have no API:
- the bind address and public URL;
- the database, the state dir and the web dist;
- the session secret and the token key;
- the GitHub OAuth client and App, and the GitHub URLs;
- `RUBATO_OTEL_*`;
- the health thresholds (`RUBATO_GITHUB_POLL_SECS`, `RUBATO_SESSION_STALE_SECS`,
  `RUBATO_COMPANION_SILENT_SECS`).

*Never stored, never settable at runtime:* `map_file`, `RUBATO_DEV_TRAFFIC`,
`RUBATO_DEV_ISSUES`, `RUBATO_DEV_USER` and `RUBATO_LOCAL_REPOS`. They are paths on the server's
disk or dev switches. A workspace's `map_file` is read from the workspaces file or the
environment by id at every start, as today.

**Source of truth: the file seeds, the database owns** (#189 question 1, option a). On a start
with an empty table, the server writes, in one transaction, one row per entry of
`RUBATO_WORKSPACES_FILE`, or `default` from `RUBATO_MAP_*`. The `RUBATO_TRAFFIC_*` and
`RUBATO_DEFAULT_*` values that are set are copied into every row written by that seed. This is a
one-time copy, not inheritance: today they apply to every workspace, so a running deploy keeps
working. Headers are sealed at seed. A start that would seed headers without `RUBATO_TOKEN_KEY`
refuses and names the variable. After the seed the database is authoritative. At every start the
server warns once for each of those variables, or each file entry, that is still set, and says
whether it differs from the database.

**Validation.** The ADR 0044 checks (a bad id, a duplicate id, two workspaces on one map, no
`default`) run at seed and on every runtime change. A change that fails them is refused with the
same reason. `default` cannot be deleted. A deleted workspace's rows are kept and counted as
orphans (`SCOPED_TABLES`). Its id cannot be created again while rows of it remain, because its
kept pins and drafts would attach to another map.

**Runtime changes.** Any logged-in team member may create, edit or delete a workspace (#189
question 2, option a; ADR 0022's gate; writes are same-origin, ADR 0066). The exception is a
server open to any GitHub account (OAuth without the App, off loopback, ADR 0064), which refuses
every settings write. A change takes effect without a restart. Only that workspace's map is
reloaded and only its traffic task is restarted.

**Map source.** `map_repo` and `map_path` are fixed once a workspace exists, because pins,
layout, drafts and dismissals are keyed to that map's ids. `map_ref` may move (#189 question 3,
option b), but not while the workspace's map draft has operations or an open proposal. A new
map source or a moved ref must pass three checks before it is stored:
- the map must load and validate at that ref, read fresh with no cached fallback;
- the requesting member's own GitHub token (ADR 0030) must be able to read the map repo;
- the same token must read every repo the new map names.

So Rubato never polls, for everyone, repos the member could not see themselves (the class of
#163).

**Traffic source, one per workspace.** This amends ADR 0040's "Configuration" paragraph, and the
rest of 0040 stands. The source is a workspace setting. A workspace that sets none has no
traffic source. The wire contract stays backend-neutral: a source name and its fields as text,
with its secrets shown only as set or not set. The web draws the form from a field list that
the adapter publishes. Only the adapter validates and interprets the fields, so PromQL, label
names and credentials still live only in the adapter. `RUBATO_DEV_TRAFFIC` stays a process dev
switch and feeds every workspace. Storing a source while it is set is refused, and a start with
it set and a stored source warns that the stored sources are ignored.

**The URL is an outbound request** (#189 question 4, option b).
- It must be http or https, with no userinfo.
- No proxy from the environment is used, and redirects are not followed.
- On every connect, IP literals included, the server refuses these addresses: loopback,
  link-local, unspecified, multicast, IPv4-mapped forms of those, IPv6 unique-local
  (`fc00::/7`) and shared (`100.64.0.0/10`). That covers the cloud metadata addresses. A server
  that is itself local (ADR 0064's `is_local`) is exempt.
- The RFC 1918 ranges are allowed, because the team's Prometheus is on one.

**Secrets are write-only.** A source's secrets (Prometheus headers) are sealed with
`RUBATO_TOKEN_KEY`, the same seal as user tokens, moved into the kernel. They are never
returned: the API says "set" or "not set". Without the key, storing one is refused with the
reason. A change of the source URL's origin (scheme, host or port) clears the secrets unless the
same request sets them, so a URL cannot be repointed to collect them. A secret that no longer
opens makes the source failing, with reason `secrets_unsealable`.

**Every change is recorded** with who made it, when and which keys changed, never the values.
The record is the info log line `workspace config changed` and the span
`rubato.workspace.config_changed` with `rubato.workspace`, `rubato.user` and the changed key
names.

**Routes.** This extends ADR 0049's list. Map-scoped `GET` and `PUT /api/w/{ws}/settings` and
`DELETE /api/w/{ws}`; team-wide `POST /api/workspaces` and `GET /api/traffic/sources` (the field
lists). ADR 0044's start-up log `workspace config` gets the source `database`.

## Alternatives considered

- **Workspaces declared in the file are read-only in the UI (#189 1b)**: two classes of
  workspace, and the env traffic source would stay server-wide for the file's workspaces.
- **No file at all (1c)**: a running deploy would lose its workspaces on upgrade.
- **Keep everything in the workspaces file, with per-workspace keys and a reload signal**: Git
  stays the truth, but a workspace still cannot be created from the view. It is the fallback if
  Git must own the config.
- **A settings file next to the map, changed by pull request (like ADR 0018)**: reviewed and
  versioned, but secrets cannot live there.
- **An admin list (2b) or map-repo permission (2c)**: a role model this lab does not have. An
  admin gate can sit in front of the same routes later.
- **Seed only `default` (the issue's "Decided")**: another workspace of a running deploy would
  silently lose its traffic lens and get the built-in spend cap.
- **The installation's reach as the repo check**: the issue rules it out. Any repo the App can
  read would be polled and shown to everyone.
- **Prometheus fields in the protocol**: it would break ADR 0040's rule that only the adapter
  knows the backend.
- **A host allowlist in the environment (4a, 4c)**: a new application setting that every change
  would depend on. It can be added later.

## Consequences

- Adding a workspace, or pointing one at its own Prometheus tenant, needs no deploy.
- The deployed configuration can drift from Git. The start-up warnings and the change log line
  make that visible.
- `Config::workspace_entries` gives way to a registry in the server's state, written only by the
  slice under one lock, with a revision per workspace. Every reader takes the registry's current
  list. A map load that finishes for a revision that is no longer current is dropped.
- The traffic report, its gauges, `rubato.traffic.fetches` and the `TrafficReportStale` alert are
  per workspace.
- Rollback to a binary from before migration 0027 means restoring the database backup taken
  before the upgrade. Workspaces created at runtime are then lost.
- O-rows:
  - `web-server`: refusals are counted on `rubato.web.rejected{reason}`; changes are the span and
    log line above.
  - `server-system-map`: `rubato.map.loaded` and `rubato.map.load_failures` with
    `rubato.workspace`.
  - `server-github`: the member-token checks and the map read are `rubato.github.call` spans.

## Supersedes

The "declared in the server config" part of 0044: the workspaces file seeds the list once
instead of holding it, and keeps `map_file`. ADR 0044's checks, ids and `default` rule stand.
This ADR also amends 0040's "Configuration" paragraph, and extends 0049's route list.
