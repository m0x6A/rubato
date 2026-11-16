# 226: agent MCP, server and web (#226–#229)

Plan: [../226.md](../226.md) · Built under the design of #197
([design](../197-agent-mcp/design.md), ADR [0071](../../decisions/0071-agent-mcp-in-the-companion.md),
accepted) and after plan [#220](../220.md) (the protocol types and the companion) · Issues: #226
#227 #228 #229 · Map: `companion-server`, `web-server`.

Plan #220 gives the wire `AgentReport` and `PlanStatus` and teaches the companion to send and
take them once a server announces `agent_mcp`. This plan makes the server that server:
- it takes the reports;
- it keeps the newest per launch and shows it;
- it pushes each launched session its plan status.

It announces `agent_mcp` only in its last server PR, once all three server parts are in.

## 1. Taking a report (#226, `companion_link`)

The drop arm that #220 left in `on_envelope` (`companion_link/ws.rs`) becomes the real one. For an
`AgentReport` for launch `L` from pairing `P`:

1. **A launch this pairing started** (ADR 0071). The launch must pass two checks.
   - `launch::launch_target(L)` (`launch/store.rs:39`), which is the server's own record of where
     a launch was sent, must have `companion_id == P`. The `Draft` and `HandoffReport` arms
     already use this record (`ws.rs:461`, `approve/handoff.rs:29-31`).
   - `L` must also be in `P`'s newest snapshot (`app.latest_sessions()`), so a launch that ended
     cannot come back.

   Otherwise the report is dropped, counted `rubato.events.dropped{rubato.reason="agent_report"}`,
   and logged with `warn!(rubato.companion_id, rubato.launch_id, reason, "agent report refused")`,
   where `reason` is `not_this_companion` or `not_live`. A `launch_id` in a snapshot is something
   the companion claims; the `launches` row is not.
2. **Known parts only.** An id in `parts` is kept when the launch's workspace map names it as a
   node or a seam. The workspace is `LaunchTarget`'s `launches.workspace_id`, a column since
   migration 0013. Each dropped id counts
   `rubato.events.dropped{rubato.reason="agent_report_part"}`.
3. **Bounded text.** The server does not trust the companion to bound the text:
   - `doing` over `AGENT_DOING_MAX_CHARS` characters is cut;
   - a newline becomes a space;
   - `parts` over `AGENT_PARTS_MAX` is truncated.
4. **The server's clock.** The stored time is the server's `now`, not the report's `at`. Rows
   change in arrival order, so a companion clock that runs ahead cannot freeze a line. Replays
   never get here (`ws.rs:269-277`).
5. `agent_reports::record(app, P, L, doing, parts, now)` runs through `observe(…)`. Then
   `app.notify("agent report")`.

**Telemetry.** The `rubato.inbound_event` span (`ws.rs:263`) declares `rubato.launch_id`,
`rubato.issue` and `rubato.outcome` as `tracing::field::Empty` and records them in this arm.
`tracing` silently drops a `record()` on a field the span did not declare. A `capture` test
asserts the three fields.

**Retention.** In the `Sessions` arm, after the snapshot is stored,
`agent_reports::prune(app, P, &launch_ids)` runs through `observe(…)`. It deletes `P`'s rows whose
launch is not in the snapshot. So a launch that ended, or one a restarted companion no longer
has, loses its line at the next snapshot.

`stored_payload` keeps storing `AgentReport` empty (#220).

## 2. Keeping it (#227, new slice `agent_reports`)

`crates/rubato-server/src/agent_reports/` owns table `agent_reports`. The migration is
`0027_agent_reports.sql`, or the next free number when it lands:

```sql
CREATE TABLE agent_reports (
    launch_id    TEXT PRIMARY KEY,
    companion_id TEXT NOT NULL,
    doing        TEXT NOT NULL,
    parts        TEXT NOT NULL,      -- JSON array of map ids
    reported_at  TEXT NOT NULL       -- the server's clock
);
CREATE INDEX agent_reports_companion ON agent_reports (companion_id);
```

- `record(app, companion_id, launch_id, …)` runs
  `INSERT … ON CONFLICT(launch_id) DO UPDATE SET … WHERE agent_reports.companion_id = excluded.companion_id`.
  A row is only ever changed by the companion that owns its launch, even if a later bug weakened
  §1.1.
- `prune(app, companion_id, &[Uuid])` deletes that companion's rows not in the list.
- `prune_absent(app, &live)` runs from `tasks.rs` housekeeping. It deletes every row whose
  `(companion_id, launch_id)` is in no companion's newest snapshot, which covers a revoked or
  retired companion that never sends another snapshot.
- `all(app) -> Vec<AgentLine>` returns only rows whose launch is in `app.latest_sessions()`. The
  overview size is then bounded by the live sessions, whatever is left in the table.

**Rollback.** The table is additive and read only by this slice. sqlx migrations only run
forward, so rolling back is a new migration that drops the table; the overview field then comes
back empty.

## 3. Showing it (#227 overview, #229 web)

A new protocol type for `web-server`:
`AgentLine { companion_id, launch_id, doing, parts, reported_at }`.

`Overview` gains `agent_lines: Vec<AgentLine>`. It is a serde default and skipped when empty, so
the schema change is additive. `overview` names `agent_reports` only to fill this field.
`derive_progress` (`rubato-protocol/src/progress.rs`) and `build`'s progress code take nothing
from it. The progress golden `overview_build.json` does not change. `overview.json` gains the
field when its fixture has a report.

The web joins `agent_lines` to sessions on **`(companion_id, launch_id)`**, never on
`launch_id` alone:
- **Lane** (`features/companions/Lanes.tsx:149`): the session row gets a second line. It holds
  the agent's `doing` in quotes, then "· 3 min ago" from `reported_at`, then the parts as plain
  ids. It is never a chip and never a colour.
- **Issue detail** (`features/issues/IssuePanel.tsx`): each launched session of the issue shows
  the same line under it.
- A line more than `session_stale_secs` older than the session's own `last_seen` is shown muted,
  with its age. It is never hidden.
- The text is rendered as text (React escapes it). Nothing in it becomes a link.

## 4. Plan status for each launched session (#228, new slice `plan_status`)

ADR 0071 and #197's design put the derivation "in `approve`". `approve` cannot read
`IssueSummary`: `overview` names `approve` (`overview/inputs.rs:5-6,48`). So a new slice
`plan_status` names `overview` and `launch`. It must not be named by `companion_link`:
`overview` already names `companion_link` (`overview/inputs.rs:51`), and
`companion_link → plan_status → overview → companion_link` is a cycle the slice test refuses
(`slices.rs:13`). Only the composition root names `plan_status`. It does not compute approvals
itself: it reads the `ApprovalState` the overview computes from `approve::approvals()`, the
counted approvals whatever their source, which keeps ADR 0071's intent.

**The draft's fit.** `DraftSummary` today carries only `parse_error`. `IssueSummary.fits_in` is
about the plan on the branch, and only when there are two workspaces or more
(`overview/build.rs:437`, `overview/inputs.rs:84`). So `DraftSummary` gains an additive
`fits: Option<bool>`. `build` computes it with `rubato_protocol::plan_fit` (`plan.rs:404`), on the
draft's own plan (`LatestDraft.plan`, `plan_drafts/mod.rs:44`), against the workspace's map. It is
`None` when the plan does not parse.

`plan_status::derive(summary, launch_id, now) -> PlanStatus` is pure:

| Field | From |
| --- | --- |
| `draft` | `summary.draft` when its `branch` is the session's branch: `{ rev, at, plan_sha, fits: fits == Some(true) }` |
| `review` | `summary.review` |
| `approval` | `summary.approval` (its `version` says whether it counts for the plan as it is) |
| `branch` | `summary.plan` with `summary.plan_sha`: `{ sha, carries_approval: plan.approval.is_some() }` |

**One summary per launch.** An issue held by two workspaces is in both builds. The summary used is
the one from the launch's workspace (`launches.workspace_id`). A launch whose issue is in no built
overview gets nothing, counted `rubato.plan_status.sent{rubato.outcome="no_summary"}`.

**The tick.** `overview::refresh_gauges` (`gauges.rs:16`) already builds the overview per workspace
each poll tick. It now:
- builds every workspace;
- logs `warn!(rubato.workspace, error, "overview not built")` for one that fails, and goes on;
- returns the built issues of those that succeeded, with the first error.

The composition root (`tasks.rs:635`) passes them to `plan_status::on_tick`, and still returns
the error, so housekeeping logs it as today. `on_tick` does this:
- It iterates `app.connected_companions()` filtered by `app.announced(id, AGENT_MCP)`. Capabilities
  are kept after a disconnect (`kernel/state.rs:80-83`), so a sleeping laptop is skipped rather
  than counted `offline` every tick.
- For each live session with a `launch_id` on such a companion, it derives a status and keeps it
  in its cache of last derived statuses.
- It compares that with the last one sent on the companion's current socket, keyed by
  `(companion_id, conn)`, the per-socket id (`kernel/state.rs:20-27`), and ignoring `as_of`. It
  sends when they differ: reason `welcome` for a socket it has not sent on, `change` otherwise.
- It forgets sockets no longer in `app.companions`. A reconnect is a new socket, so it always gets
  a fresh push.

**Right after a Welcome.** `companion_link` calls `app.companion_welcomed(companion_id)` after it
answers a `Hello`. That is a new kernel broadcast, so `companion_link` names no slice for it. A
task in `tasks.rs` subscribes and calls `plan_status::on_welcome(app, companion_id)`. That sends
the cached statuses of the companion's live launches on its new socket, reason `welcome`, without
building an overview. A companion that flaps costs one cache read per reconnect. The next tick
corrects anything the cache missed.

**Sending.** Statuses go out through `app.send_to_companion` (`kernel/state.rs:302`).
`rubato.plan_status.sent` counts each by reason and outcome: `sent`, `offline` (send returned
`false`) and `no_summary`.

**Only the owner's own launches.** A status is sent only to the companion that `launches` says the
launch went to. `PlanStatus` steers nothing: it answers a question the agent asks about its own
plan.

## 5. Announcing the capability (#228)

Welcome's capability list (`ws.rs:335-340`) gains `agent_mcp` in the last server PR (#228). By
then `companion_link` takes reports (#226), they are kept and shown (#227), and statuses are pushed
(#228). Before that, a companion with #225 sees no `agent_mcp` and sends nothing.
`session_messages/dispatch.rs:265` adds `AGENT_MCP` to the capabilities `rubato.companions.by_capability`
counts. On-call can then tell "the companion never announced it" from "the server never pushed".

## 6. The approval guard (#226)

ADR 0071's structural guard is a test: every companion message replayed through `on_envelope`
grants no approval. It only means something if each message can reach its handler, so:

- **Fixtures.** The protocol crate's fixtures gain `companion_launch_started.json` and
  `companion_handoff_report.json`. These are the two messages that reach `approve`
  (`ws.rs:416`, `:477`).
- **Seeding.** The test seeds a database with:
  - a pairing;
  - a launch for it;
  - a draft;
  - a waiting Rubato approval.

  It rewrites each fixture's companion and launch ids to the seeded ones, so no message stops at
  an identity gate.
- **Assertions.**
  - No `rubato_approvals` or `plan_approvals` row appears.
  - No approval-granting column changes: `approved_by`, `plan_sha`, `bundle_hash`, the approval's
    own times.
  - The handoff columns a valid `HandoffReport` writes (`approve/records.rs:93`) are excluded by
    name.
  - It prints the fixtures replayed, and how many reached a handler without being refused.
- **Must-fail.** The replay takes the dispatch as a parameter. The must-fail case passes a wrapper
  that also inserts a `rubato_approvals` row on `Draft`, and the test must fail.

## 7. What this plan does not do

- No companion change (plan #220).
- No SKILL.md change (#230).
- No new alert. The drops land on "Events dropped". A new panel, "Plan status pushes", in the
  Trust check row plots `rubato_plan_status_sent_total`, and `observability/check.ts` requires it.
