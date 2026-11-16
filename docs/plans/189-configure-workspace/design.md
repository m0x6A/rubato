# Design: workspace settings at runtime (#189)

Plan: [`docs/plans/189.md`](../189.md). ADR: [0075](../../decisions/0075-workspace-settings-live-in-the-server-database.md).

## 1. The spec

**What the change does**

- **S1. Open the view.** A team member opens **Configure workspace** from an item in the shell
  header that is always drawn, also on a server with one workspace (the switcher is hidden
  there). The view shows the workspace's name, map source, traffic source and launch defaults.
  Secrets show only as "set" or "not set".
- **S2. Save a change.** It takes effect without a restart, and no other workspace's map,
  traffic or view changes:
  - a moved `map_ref` reloads that workspace's map;
  - a changed traffic source restarts that workspace's traffic task.
- **S3. Create and delete.** A member creates a workspace: an id, a name, and a map that they
  themselves can read on GitHub, in a map whose named repos they can read too. It appears in
  every browser's switcher. They can delete any workspace but `default`. Its rows are kept as
  orphans, and its id cannot be reused while they remain.
- **S4. Refusals.** A change is refused, with a reason, when:
  - the start-up checks would refuse it: a bad or duplicate id, two workspaces on one map, or
    deleting `default`;
  - it changes `map_repo` or `map_path`;
  - it moves `map_ref` on a `map_file` workspace, or while its map draft is open;
  - the map does not load fresh at that ref;
  - the member's token cannot read the map repo, or a repo it names;
  - the URL is bad: not http(s), with userinfo, or an address in the refused ranges;
  - a secret is set without `RUBATO_TOKEN_KEY`;
  - a source is set while `RUBATO_DEV_TRAFFIC` is set;
  - the edit is stale;
  - a create reuses an id;
  - the server is open login.
- **S5. Upgrade.** A running deploy keeps working with no config change. The first start seeds
  `workspace_settings` in one transaction. It writes one row per entry of
  `RUBATO_WORKSPACES_FILE`, or `default` from `RUBATO_MAP_*`, and copies each set
  `RUBATO_TRAFFIC_*` and `RUBATO_DEFAULT_*` value into every seeded row. After that the database
  owns the settings. Every start warns once for each such variable or file entry still set, and
  says whether it differs. `map_file` stays process-side: it is read from the file or env by id
  at each start and is never stored.
- **S6. Per workspace.** Each workspace with a source has its own report, findings, gauges and
  notifications. A launch takes its defaults from its workspace, else the built-in 25 USD and 3
  attempts. The plan's front matter still wins.
- **S7. Change record.** Every change logs `workspace config changed` with the workspace, the
  user and the names of the keys that changed, and records the span
  `rubato.workspace.config_changed`. Values are never recorded.

**How we'll know it works.** The plan's checklist names each test and its S.

**What it must not break**
- N1. ADR 0044's start-up refusals: one shared `kernel::workspaces::check`.
- N2. Single-workspace deploys: the same overview and no switcher (`tests/workspaces_golden.rs`
  is unchanged). Only the header item is new.
- N3. ADR 0049 routes and `check_workspace`.
- N4. Orphan counting, now also re-recorded after a create or delete.
- N5. Traffic with one workspace, from a seeded row. The `traffic::prometheus_tests` keep a test
  of the env-to-row seed path before their source changes.
- N6. Launch caps from the plan's front matter.
- N7. Same-origin writes (ADR 0066).
- N8. The map draft and its proposal: a ref move is refused while either is open.
- N9. The start-up log `workspace config` with source `file`, `legacy_env` or `database`.

**How we'll debug it at 3 am.** O-rows are in the plan.
- A refusal is `rubato.web.rejected{rubato.route, reason}`.
- A change is the log line and the span.
- A workspace's traffic is `rubato.traffic.fetches{rubato.outcome, rubato.workspace}` and
  `rubato.traffic.report_age{rubato.workspace}`, next to `rubato.traffic.configured
  {rubato.workspace}` (1 while a source is set, 0 after it is cleared or the workspace is
  deleted).
- `TrafficReportStale` fires per workspace and names it.

## 2. Where it fits

| Part | Change |
| --- | --- |
| kernel `state` | `AppState::workspaces`: a `watch` channel holding the registry, each entry with a revision. `AppState::new` fills it from `Config` (tests stay green); the composition root replaces it from the table at start. `AppState::traffic` keyed by workspace. |
| kernel `secrets` | `seal` and `open`, moved here from `auth::tokens` (auth keeps using them). |
| kernel `workspaces` | `check` shared by seed and runtime; `report` gains source `database` and is re-run after a create or delete. |
| kernel `config` | the seed inputs become `Option`s, so "unset" and "set to the default" differ. |
| new slice `workspace_settings` | table `workspace_settings` (migration 0027); seed and drift warnings; routes; validation; the member-token checks; one async write lock; the change span and log; the registry swap. |
| slice `map` | `load::fetch_and_validate(entry)`: fresh, no fallback, no side effects. `reload_ws` applies a load only if the entry's revision is still current. `forget(ws)` on delete. |
| slice `traffic` | an always-running supervisor, `Essential`, subscribed to the registry's `watch`. It runs one task per workspace with a source, or one dev-file task feeding all. Generation per task. `Source` adapters publish their field list and parse a row's fields; `PrometheusConfig::from_fields` replaces `from_env` and is the one validator. A connector that checks every address, IP literals included; `.no_proxy()`; no redirects. |
| slice `launch` | defaults from the registry. |
| slice `map_draft` | `is_open(ws)`, for the ref-move refusal. |
| `rubato-protocol` | new module `workspace_settings`: `WorkspaceSettings`, `TrafficSourceSettings` (source name, fields as text, `secrets: SecretState`), `SourceFields` (the adapter's field list), `WorkspaceSettingsUpdate` (revision; secrets keep, set or clear), `NewWorkspace`, reasons, routes. The span name goes in `otel`. |
| `web` | new feature `settings`: the view, drawn from `SourceFields`; create; delete; the header item. |

**The slice graph.** `workspace_settings` names `map` and `map_draft` through their roots.
`traffic` and `launch` read the kernel registry, and nobody names `workspace_settings`, so the
graph stays acyclic.

## 3. Sequences

### A change to `shop`'s traffic source

```mermaid
sequenceDiagram
  participant B as Browser (member)
  participant S as workspace_settings
  participant K as kernel registry (watch)
  participant T as traffic supervisor
  participant P as shop's Prometheus
  B->>S: PUT /api/w/shop/settings {revision 4, traffic: {fields, secrets: keep}}
  S->>S: same-origin, CurrentUser, not open login
  S->>S: take the write lock; re-read row (revision 4?)
  S->>S: adapter parses fields; URL origin changed and secrets "keep" → secrets cleared
  S->>S: COMMIT row, revision 5; span rubato.workspace.config_changed
  S->>K: send registry with shop@5
  S-->>B: 200 WorkspaceSettings (secrets: not set), release lock
  K-->>T: changed
  T->>T: cancel shop's task (gen 7), await with deadline, start gen 8
  T->>P: query (connector checks the address, no proxy, no redirect)
  T->>B: Changed { traffic, workspace: shop } when its summary changes
```

### A `map_ref` move

Under the write lock:
1. Refuse if the map draft is open, or if the workspace is a `map_file` workspace.
2. Call `fetch_and_validate` at the new ref, outside any SQLite transaction. A failure answers
   `422 map_invalid` and changes nothing.
3. Check that the member's token can read every repo the new map names.
4. Commit the row at the new revision.
5. Send the registry, then `set_map` and `store_map` with that revision.
6. Notify `Changed { map, workspace }`.

A poll reload of the old ref that finishes afterwards carries the old revision and is dropped.

### Create and delete

**Create**, under the lock:
1. Check the id.
2. Refuse `id_has_rows` if any `SCOPED_TABLES` or `map_cache` row has that id.
3. Run `check` over the registry with the new entry, plus the table's
   `UNIQUE (map_repo COLLATE NOCASE, map_path, map_ref)`.
4. Run the fetch and token checks.
5. Insert, swap the registry, `set_map`.
6. Notify `Changed { workspaces }` (unscoped), and re-record `workspaces_configured`.

**Delete**, under the lock:
1. Refuse `default`.
2. Delete the row and swap the registry; the supervisor stops the task and records
   `configured=0`.
3. `map::forget`. Rows are kept.
4. Notify unscoped, and re-run `report`.

## 4. Data model

`workspace_settings` (migration 0027, owned by the slice):

| Column | Type | Note |
| --- | --- | --- |
| `workspace_id` | TEXT PK | `[a-z0-9-]{1,32}` |
| `position` | INTEGER | switcher order: file order at seed, new ones last |
| `revision` | INTEGER | starts at 1, +1 per change; the compare-and-set token |
| `name` | TEXT NULL | else the map's name |
| `map_repo`, `map_path`, `map_ref` | TEXT | `UNIQUE (map_repo COLLATE NOCASE, map_path, map_ref)`; repo and path are fixed after insert |
| `traffic_source` | TEXT NULL | adapter name (`prometheus`); null: no source |
| `traffic_fields` | TEXT NULL | JSON object of text fields, validated by the adapter |
| `traffic_secrets_sealed` | TEXT NULL | JSON object, sealed with `RUBATO_TOKEN_KEY` |
| `default_spend_cap_usd` | REAL NULL | null: built-in 25 |
| `default_attempt_limit` | INTEGER NULL | null: built-in 3 |
| `updated_by`, `updated_at` | TEXT | |

The registry entry carries the sealed text only. `WorkspaceEntry`'s `Debug` prints neither
fields nor secrets. The traffic slice opens the secrets with `kernel::secrets::open` when it
builds a task.

**Rollback.** Back up the database before the upgrade (the deploy runbook gets the line). An
older binary refuses a database with migration 27 applied (sqlx `VersionMissing`), so a rollback
means restoring that backup. Workspaces created at runtime are lost.

## 5. Concurrency and idempotency

- **One write lock.** A `tokio::sync::Mutex` in the slice is held for validate, commit, re-read
  and swap. Two writers cannot reorder their swaps, and two creates cannot both pass `check`.
- **Compare-and-set** on the integer `revision`. A stale `PUT` answers
  `409 changed_meanwhile` with the current settings.
- **Network outside transactions.** The GitHub reads happen under the lock but outside any
  SQLite transaction.
- **Revision-checked map loads.** `reload_ws` takes the entry's revision and applies its result
  only if the registry still holds that revision for that id. A delete removes the id, so an
  in-flight load cannot bring it back.
- **Traffic generations.** Each task has a generation. Its writes to `AppState::traffic[ws]`
  check it, so a report from a cancelled task never lands. The supervisor is `Essential`: if it
  dies the server stops, which is loud rather than quiet.
- **Seeding** is one transaction, and it runs only on an empty table. A crash leaves the table
  empty, and the next start seeds again.
- **Restart is idempotent.** Sending the registry twice restarts nothing, because the
  supervisor compares revisions.

## 6. Decisions

ADR 0075 holds the seam decisions. The plan's Decisions table holds the rest, including three
seam questions Mattias may overturn:
1. Repos a member may use.
2. Headers at seed without a key.
3. The seed copying the env traffic and caps into every workspace.

## 7. Delivery

Four PRs, each green on its own:

1. **Kernel registry, secrets in the kernel, table, seed and drift warnings.** No change in
   behaviour: every reader uses the registry, and `AppState::new` fills it from `Config`.
2. **Per-workspace traffic and launch defaults.** Includes the field-list adapter, the
   connector guard, the per-workspace alert and its fire test, and the revision-checked map
   loads.
3. **The settings API.** Protocol types, routes, validation, the member-token checks, the span.
4. **The web view and the header item, and e2e** on both e2e servers.
