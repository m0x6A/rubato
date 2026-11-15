# Design: workspaces, several maps on one server (#89)

Plan: [../89.md](../89.md) · ADRs 0044–0049, all *proposed*. Choices answered by Mattias Jonsson
on 2026-11-14 in the planning session:

- many workspaces per server;
- a repo's features are derived from the map;
- one repo may be in several workspaces, with ids resolved by derivation;
- workspaces are declared in a server config file;
- `workspace.yml` is renamed to `local.yml`.

Revised after the design review: four blockers (what `default` means; issues in repos no map names; approval and fit across workspaces, and `plan.ts`; a migration with no rollback), three should-fix (unscoped readers, gaps in the web-server contract, gauges and panels) and the nits. All taken. The by-repo view moved first in the stack.

Issue #89 (voice note): "combine multiple repositories in One view where you can see what each…
repository provides to the whole Application… workspaces is what we are trying to define as the
umbrella for our solution."

## D1. What a workspace is

A **workspace** is one application on a Rubato server. It is made of three things:

- one map file, which holds the id space (ADR 0007);
- the repos that map names (`SystemMap::repos_named`, `crates/rubato-protocol/src/map.rs:265-273`), which are never declared a second time;
- the view state the team keeps for that map: layout, draft and proposal, pins, `Map:` placements and the map cache.

| Concept | How many | Role |
| --- | --- | --- |
| Team: the GitHub App installation (ADR 0022) | one per server | Decides who may log in. Sees every workspace. |
| Workspace | 1..n per server | One map and its view state. Has an id (slug) and a name. |
| Map file | one per workspace | Holds the ids. Ids are unique within a map, not across maps. |
| Repo | named by 0..n maps | Holds the code behind nodes. Its GitHub facts are read once and shared. |
| Local config (`local.yml`) | one per machine | Not a workspace. Holds the server URL, the clones of map repos and code repos, otel and companion settings. |

The companion never reads a map: its only use is a test (`crates/rubato-companion/src/kernel/config.rs:241`). It only passes `rubato.workspace` through `otel_resource`.

## D2. Declaration (ADR 0044)

`RUBATO_WORKSPACES_FILE` names a YAML file like this:

```yaml
workspaces:
  - id: default            # keeps today's rows
    map_repo: m0x6A/rubato
    map_path: docs/design/rubato-map.yml
    map_ref: main
  - id: shop
    name: Shop             # optional; falls back to the map's `name`
    map_repo: acme/shop-map
    map_path: system-map.yml
    map_ref: main
    map_file: ./dev/shop-map.yml   # optional, local runs and e2e
```

- **Unset.** The legacy `RUBATO_MAP_REPO`, `RUBATO_MAP_PATH`, `RUBATO_MAP_REF` and `RUBATO_MAP_FILE` (`kernel/config.rs:197-200`) make one workspace with id `default`.
- **Refusals at start.** The server refuses to start, naming the reason, if:
  - both the file and the legacy variables are set;
  - an id is duplicated, or does not match `[a-z0-9-]{1,32}`;
  - no entry has id `default`;
  - two entries name the same repo, path and ref (repo compared without case, as `connected_repos` does).
- **`default`.** The entry with id `default` keeps every existing row, because migration 0013 writes `default` into them. It is also what unprefixed routes serve. This is one rule in ADRs 0044 and 0049: unprefixed means id `default`, never "the first entry".
- **Order.** The order of the list is the switcher order only.
- **`map_file`.** A relative path is relative to the workspaces file.
- **PR 3 limit.** Until PR 4 (scoping) lands, the server refuses a file with more than one entry ("several workspaces need the scoped API"). An operator therefore never runs two maps while every route still serves `default`.
- **Start-up log.** `workspace config` names the ids and `source=file|legacy_env`.

## D3. Resolution (ADR 0045)

`AppState.map: RwLock<Option<LoadedMap>>` (`kernel/state.rs:64`) becomes `maps: RwLock<BTreeMap<WsId, LoadedMap>>`, plus a `Workspaces` value from the config. These helpers go in the kernel:

- `workspaces_of(repo) -> Vec<WsId>`: the workspaces whose loaded map names the repo, in config order.
- `union_repos()`: the distinct `repos_named` over all maps (ADR 0048).
- `default_map()`: used by today's call sites while PR 3 keeps the behaviour unchanged.

**An issue's workspaces** (ADR 0045) are `workspaces_of(repo)` plus every W where the issue is pinned (`pins`, `map_line_pins`), placed, or launched (`launches.workspace_id`). With one workspace configured, an issue's workspaces are always that one, whatever its repo. That is today's behaviour for tracked issues in repos no map names (`tasks.rs:150`, `connected_repos` with `in_map: false`). Without this rule, such an issue would get no fit, and `approval_state` would turn `DoesNotFit` into `Current` (`overview/build.rs:113,237-241`).

**Plans.** The poll still reads each plan once per (repo, branch). `plan_fit` runs over the full ADR 0029 rules (ids, flows, O-rows, data models) against each map among the issue's workspaces, producing `fits_in: Vec<WsId>`.

- **Showing.** W's overview shows the plan of any issue that has W among its workspaces.
- **`IssueSummary.fit`.** Keeps its type. It is W's own `PlanFit` when the plan fits W. Otherwise it is the `PlanFit` of the first workspace it fits, and `IssueSummary.fits_in` carries the names.
- **Approval.** It counts when `fits_in` is non-empty. In a workspace the plan does not fit, the badge reads "counts in <name>", not *does not fit*. `approve` under `/api/w/{ws}` refuses `does_not_fit` only when `fits_in` is empty (`approve/handlers.rs:61,79`).
- **Findings.** Unknown-id findings and `ApprovedWithoutFit` are raised only when `fits_in` is empty, and then in each of the issue's workspaces.
- **`rubato.plans.fit_none`.** Counts plans whose `fits_in` is empty.
- **The `plan fit` log line.** Logged at info level once when a plan's `fits_in` changes, not on every overview build.

**Plan path conflicts.** If two maps give the same repo different `plan_path` values, the first workspace in config order wins and the server raises the finding `PlanPathConflict { repo, workspaces }`.

**Id collisions.** If a shared repo's id names nodes with different `repo` values in two maps, the server raises the finding `IdCollision` and counts it on `rubato.workspace.id_collisions`. It never refuses.

## D4. `Map:` lines (ADR 0046)

`parse_map_line` (`crates/rubato-protocol/src/issue_body.rs:37`) today accepts exactly two words after `Map:` (line 60). It will return a `MapPlacement { target: MapRef, workspace: Option<String> }`:

- two words parse as today;
- four words of the form `<kind> <id> in <ws>`, where both ids pass `is_id`, give the workspace;
- three words, a third word other than `in`, five or more words, and `in` followed by an invalid id are rejected;
- `map_line(target, workspace)` writes the qualified form only when given a workspace.

`OpenIssue.map_line` (`progress.rs:257`) becomes a `MapPlacement`. The field is additive: an old value with only a target still deserialises. Its metric labels name the repo's workspaces, never the word taken from the issue body, so issue text cannot add cardinality.

The shared fixtures in `schema/fixtures/issues/` gain `qualified.md` and `qualified-bad.md`.

Placement in `github_poll` puts an unqualified line in every W in `workspaces_of(repo)` whose map has the id with that kind. A qualified line goes into W only, or into none with outcome `unknown_workspace`.

## D5. Storage: migration `0013_workspaces.sql` (ADR 0047)

Old migrations stay untouched, because sqlx checksums them. On a rebuilt table, `workspace_id` is `TEXT NOT NULL` with **no default**, so a store call that forgets to bind it fails instead of writing to `default`. A column added with `ALTER` needs a default, because SQLite requires one to add a `NOT NULL` column; each store test inserts through the store function and asserts the workspace it wrote. No table has a foreign key (migrations 0001–0012), so dropping and renaming under `foreign_keys(true)` (`kernel/db.rs:14`) is safe.

| Table | Change |
| --- | --- |
| `map_layout` | Rebuild. PK `(workspace_id, node_id)`. |
| `map_cache` | Rebuild. PK `workspace_id`; drop `CHECK (id = 1)`. |
| `map_proposal` | Rebuild. PK `workspace_id`; drop `CHECK (id = 1)`. |
| `pins`, `map_line_pins` | Rebuild. `workspace_id` is the first column of the PK. |
| `map_draft_ops` | Add column, plus index `(workspace_id, seq)`. `seq` stays global and monotonic. |
| `issue_creations`, `launches` | Add column. |
| `issue_suggestions` | New table `(workspace_id, repo, number, suggestion)`. It is seeded from `open_issues.suggestion`, which is then no longer written. `open_issues.map_line` stays, because it is a GitHub fact. |

A rebuild means create `X_new`, copy with `INSERT … SELECT 'default', …`, drop `X`, then rename `X_new` to `X`, all inside the migration's transaction.

**Orphaned rows.** Rows whose workspace is no longer configured are kept. They are logged at start and counted as `rubato.workspace.orphan_rows`.

**Ownership.** Each table stays owned by the slice that owns it today (AGENTS.md slice block). No new slice owns storage. `issue_suggestions` is owned by `github_poll`. It is written in the same transaction as `open_issues` (`replace_open_issues`) and cleared in `forget_repo` (`github_poll/store.rs:112-160`).

**Rollback.** 0013 is not additive. Once it is applied, the previous binary refuses to start, because `sqlx::migrate!` does not know version 13 (`kernel/db.rs:20-23`). The PR 3 deploy record therefore starts with a copy of the database file taken while the server is stopped. Rolling back means stopping the server, restoring that copy and redeploying the previous sha. Nothing written after the deploy survives a rollback, and the deploy record says so.

**Seeding the migration test at 0012.** A `Migrator` built from `./migrations` keeps only versions ≤ 12 and runs those. The test seeds rows, runs the full migrator, and counts rows per table.

**Every reader of the one map and every query on a scoped table, with its rule:**

| Site | Today | Rule |
| --- | --- | --- |
| `map/load.rs`, `app.rs:186` (dev load) | one `LoadedMap` | one per workspace, loaded independently; a failing one keeps its last good map |
| `map/handlers.rs`, `overview/handler.rs`, `overview/build.rs:389` | `app.map` | the `Ws` extractor's map |
| `pins/handlers.rs:23`, `layout/handlers.rs:34`, `new_issue/handlers.rs:84` | `app.map` | the `Ws` map; writes bind `workspace_id` |
| `map_draft/handlers.rs:31`, `propose.rs:56`, `poll.rs:59` | one draft | the `Ws` map; every query `AND workspace_id = ?`, including `remove_op` (`store.rs:193`), discard and restore (`:212-237`) and `clear_merged` (`:305`); the proposal branch is `map/draft-<ws>-%Y%m%d-%H%M%S` (`propose.rs:235`) |
| `approve/handlers.rs:61` | one map | the maps of the issue's workspaces (D3) |
| `launch/handler.rs:56,96-108,143` | pins, one map | from a workspace view: W's pins and map. From the queue (team-scoped): pins from the issue's workspaces, with `rubato.workspace` set to the first one in config order where it is pinned |
| `launch/store.rs:58-73` `launch_map_versions` | all launches | `WHERE workspace_id = ?`; `MapVersionMismatch` (`overview/build.rs:215-224`) judged per workspace |
| `github_poll/backlog.rs:79,90` `suggest::place` | one map | listing over the union (D6); placement and suggestion once per W in `workspaces_of(repo)` |
| `github_poll/pulls.rs:36`, `tasks.rs:99,143-150` | `repos_named` | the union |
| `pins/map_lines.rs` | one map | per W (D4) |
| `companion_link/ws.rs:242` | `repos_named` | the union |
| `traffic/fetch.rs:133-148` | one map, gauges with `&[]` | derived per workspace with `rubato.workspace` on its gauges; a service is *unmapped* only when it is in no map |
| `overview/gauges.rs:12-62` `refresh_gauges` | one map, no attributes | per workspace with `rubato.workspace` on every gauge; a shared issue counts once in each of its workspaces, so dashboard panels use `max by (rubato_workspace)`, never `sum` across workspaces |

## D6. Polling (ADR 0048)

`tasks.rs` today loads the one map and lists `map.repos_named()` (`tasks.rs:99,120,143-148`). After the change:

- it loads each workspace's map on its own schedule, and one failing workspace does not stop the others;
- it lists `union_repos()` plus tracked repos, each distinct repo once;
- `coverage.rs` checks the union plus every map repo.

Call-count test: two workspaces sharing `acme/auth` cost the same listing calls as one workspace naming both repos.

## D7. API and web (ADRs 0047, 0049)

**Server routes.** `app.rs` nests the map-scoped router twice:

- at `/api/w/{ws}`, with an extractor `Ws` that 404s `unknown_workspace` and records `rubato.workspace` on the span;
- at `/api`, with `Ws = default`.

Team routes are mounted once: me, auth, pair, pairings, queue, `/ws/*` and health. `GET /api/workspaces` is new and belongs to the `map` slice. The `Ws` extractor lives in `kernel/http`, because every map-scoped slice uses it. The route list test (`app.rs:134-162`) covers both forms.

**Overview per workspace.** Every section is filtered to W in one of three ways:

- **issues:** those that have W among their workspaces (D3);
- **backlog, `repos`, `pulls`:** the repos of W's map plus the repos of W's tracked issues;
- **team-wide, unfiltered:** companions, queue, unpinned sessions, and the findings that are not about the map (`UnattributedSession`, rate limit).

`RepoNotCovered` covers W's repos and W's map repo. Traffic is derived against W's map.

Two additive wire fields:

- `IssueSummary.fits_in` lists the names of the workspaces the plan fits;
- `IssueSummary.pinned_in` lists the other workspaces where the issue is pinned. This is what "pinned in <name>" shows.

**Web events.** `WebEvent::Changed { workspace: Option<String> }`. The field is set only for `layout`, `draft` and `map` changes. Pins, issues, approvals and polls leave it absent, because they change other workspaces' views too ("pinned in", backlog, approval). The web refetches when the field is absent or equal to its own workspace.

**Single-workspace URLs.** The web's first `GET /api/overview` is unprefixed and learns `overview.workspace`. Every later map-scoped call uses `/api/w/{ws}/…`, including on a single-workspace server, so one code path is used. The switcher renders only when there is more than one workspace.

**Unknown workspace.** When a stored or linked `?w=` is answered with `unknown_workspace`, the web clears it, drops `?w=`, falls back to the unprefixed overview, and says "workspace <id> no longer exists".

**Web.**

- `shared/api.ts` gains `scoped(path)`, which prefixes `/api/w/{ws}` using the workspace from `?w=`, then local storage, then the overview's own. Only the map-scoped feature APIs call it (`live`, `map`, `pins`, `drafts`, `new_issue`, `plans`, `launch`). Team APIs (`queue`, `companions`, `auth`, and `/ws/web` in `live/subscribe.ts:40`) keep plain paths, so they never 404.
- A new feature `workspaces` holds the switcher, rendered in the shell only when `overview.workspaces.length > 1`, so a single-workspace team sees no change.
- `features/repos` gains `repoFeatures(map, repo)`:
  - **parts**: nodes with `repo == R`, and their descriptions;
  - **owns**: seams whose `owner` is one of those parts;
  - **uses**: seams that touch the parts and are owned elsewhere;
  - **scenarios**: those with a step from or to one of the parts.
- Each `ConnectedRepos` unit expands to show these. This answers "what each repository provides to the whole application".
- `features/map` gains a **By repo** lens. ELK compound nodes, one per repo, hold that repo's parts; external parts sit outside (`hierarchyHandling: INCLUDE_CHILDREN`, `graph.ts:210-232`). Dragging is off in this lens, because the stored layout is flat. ELK returns child positions relative to their parent, so the lens keeps them that way: each repo box becomes a React Flow group node, and each child gets `parentId` set to its repo box and ELK's relative position. Saved positions are not used in this lens.

## D8. Local config and the skill (ADRs 0044, 0045)

**Local config.** `skills/rubato/scripts/lib/workspace.ts` is renamed `local.ts`. It reads `RUBATO_LOCAL`, then `~/.config/rubato/local.yml`, then the legacy `RUBATO_WORKSPACE` and `workspace.yml` with one warning. A single `map:` is read as `maps: [map]`. The companion's `kernel/config.rs:121-127` follows the same order.

**`map.ts`.** It reads every map in `maps:` and keeps those whose `repos_named` includes the cwd's `origin`:

- one match: today's output, byte for byte;
- several: one section per map, headed with its workspace id;
- none: with one map configured, it uses that map; otherwise it lists the maps and exits 3.

It prints `rubato.map.select repo=… maps_matched=… outcome=none|one|several`.

**`adrs.ts`.** It reads the decisions of each matched map. `accept` still runs only in a map repo's checkout.

**`plan.ts init`.** It writes `map_version` only when exactly one map matched, and logs `rubato.plan.init map_version=written|omitted_several_maps`.

**`plan.ts check | wait | sha | path`.** Today these read one `mapFile(ws)` (`skills/rubato/scripts/plan.ts:137,169,210,255`). After the change:

- They use the matched maps, and a plan *fits* when it fits any of them. That is the server's rule (D3).
- `wait` exits `does_not_fit` only when the plan fits none.
- The `rubato.plan.check` and `rubato.plan.wait` lines gain `maps_checked` and `fits`, only when more than one map matched.
- **Plan path.** When the matched maps give the repo different `plan_path` values, `init` and `path` refuse with exit 3 and name the maps instead of choosing. The server raises `PlanPathConflict` for the same case.

**Companion.** An older companion binary only reads `workspace.yml` and requires `map:` (`rubato-companion/src/kernel/config.rs:15,121-127`). QUICKSTART therefore says to rename the file only after upgrading the companion. The new companion reads both names, and treats `map:`/`maps:` as optional, because it never uses them.

**plan-check.**

- `actions/plan-check/src/cli.ts:42-44` accepts repeated `--map` and a `--repo` (default `GITHUB_REPOSITORY`).
- The composite `actions/plan-check/action.yml` gains a `maps` input (newline-separated). The existing `map` input keeps working.
- A plan passes when it fits at least one map that names the repo.
- The `rubato.plan.check` line gains `maps_checked` and `fits` only when more than one map is given, so CI's single `--map` output is unchanged.

## D9. Delivery

Each PR is stacked on the one before it, and each starts with its failing test (plan Tasks).

The by-repo view goes first. It is the part of #89 that needs no server change, and it is worth having on a single map.

| PR | Branch | Behaviour change |
| --- | --- | --- |
| 0 | `issue/89-workspaces` | none: plan, design, ADRs, map flows |
| 1 | `issue/89-repo-view` | web only: what each repo provides and the By repo lens, on today's one map |
| 2 | `issue/89-protocol` | none: additive types, schema, otel names |
| 3 | `issue/89-server-store` | none, proven by the golden test; a workspaces file with more than one entry is refused |
| 4 | `issue/89-server-scope` | several workspaces served; the one-entry limit is lifted |
| 5 | `issue/89-web` | switcher, scoped API, event filter, "pinned in" and "counts in" |
| 6 | `issue/89-skill` | `local.yml`, several maps in the skill, `plan.ts` and plan-check |

## D10. Risks

- **A write landing in the wrong workspace.** Unprefixed always means id `default` (D2). The web uses the prefixed form once it knows its workspace (D7).
- **Golden drift in PR 3.** The snapshot is recorded on PR 2's branch, which already carries `Overview.workspace` and `workspaces`, with `generated_at` and `loaded_at` masked. It covers `/api/overview`, which carries layout and draft, then pins and the draft through their own routes. Layout has no GET (`app.rs:60-63`). The snapshot is seeded with:
  - a pinned issue in a repo no map names, with a plan that does not fit and an approval (it must stay `DoesNotFit`);
  - a launch whose map version differs from the map;
  - a draft with ops and a merged proposal.
- **Gauges and dashboards.** Every gauge from `refresh_gauges` and traffic carries `rubato.workspace` (D5 table), and the new names go in `otel.rs` in PR 2. Changes in PR 4:
  - Map age: `max by (rubato_workspace)`;
  - "Cost per issue": `by (rubato_workspace, rubato_issue)`;
  - a "Findings by workspace" panel next to the Plans row;
  - `PlanUnknownNodes` and `SeamCollision` gain `by (rubato_workspace)`.

  `observability/check.ts` keeps requiring the `rubato_map_loaded_at` query, each rule test still fires, and the `gridPos` overlap check from the #103–#106 merge round runs after the panel is added.
