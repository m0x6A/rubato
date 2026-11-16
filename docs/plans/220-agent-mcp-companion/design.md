# 220: agent MCP, protocol and companion (#220–#225)

Plan: [../220.md](../220.md) · Built under the design of #197
([design](../197-agent-mcp/design.md), ADR [0071](../../decisions/0071-agent-mcp-in-the-companion.md),
accepted) · Issues: #220 #221 #222 #223 #224 #225 · Map: `companion-mcp`, `companion-claude-code`,
`companion-server`.

ADR 0071 settled what the agent-facing MCP server is: where it runs, what it offers, identity,
and what it never does. This note settles how the protocol and the companion build it, file by
file. The server side (#226–#229) and the skill (#230) are plans of their own. Until they land,
the server never announces `agent_mcp`, so a companion with this change sends no `AgentReport`,
and `plan_status` answers `unknown`.

## 1. Protocol (#220)

A new capability module `crates/rubato-protocol/src/agent.rs`, re-exported at the root, with the
types the `companion-server` seam carries:

```rust
pub struct AgentReport {
    pub launch_id: Uuid,
    pub doing: String,          // at most AGENT_DOING_MAX_CHARS
    pub parts: Vec<String>,     // at most AGENT_PARTS_MAX, each matching MAP_ID_PATTERN (schema)
    pub at: DateTime<Utc>,
}

pub struct PlanStatus {
    pub launch_id: Uuid,
    pub as_of: DateTime<Utc>,
    pub draft: Option<DraftStatus>,        // { rev: u32, at, plan_sha, fits: bool }
    pub review: Option<ReviewRecord>,      // reused from `github`
    pub approval: Option<ApprovalState>,   // reused from `github`: the record and its version
    pub branch: Option<BranchStatus>,      // { sha, carries_approval: bool }
}
```

`approval` is `ApprovalState`, the type `IssueSummary.approval` already carries
(`progress.rs:232`, `github.rs:230`). Its `ApprovalVersion` tells `Current` from `Earlier`,
`NotNamed`, `NoPlan` and `DoesNotFit` (`github.rs:214-226`), so an approval that does not count
never reads as approved. `DraftStatus.plan_sha` lets the agent match the approval to its
revision.

- `CompanionMessage::AgentReport(AgentReport)` and `ServerMessage::PlanStatus(PlanStatus)`.
- `capability::AGENT_MCP = "agent_mcp"`: one name both ways, as ADR 0071 says. The companion's
  says "I take `PlanStatus`"; the server's says "I take `AgentReport`". Neither side sends its
  message otherwise.
- `limits`: `AGENT_DOING_MAX_CHARS = 200`, `AGENT_PARTS_MAX = 8`.
- `otel`:
  - span `rubato.mcp.call`;
  - metrics `rubato.mcp.calls`, `rubato.mcp.reports`, `rubato.mcp.not_initialized` and
    `rubato.plan_status.sent`;
  - attribute `rubato.mcp.tool`, whose values are the three tool names, the four methods, or
    `unknown`;
  - drop reason `mcp_auth`.
- Fixtures `companion_agent_report.json` and `server_plan_status.json`. `schema.rs` gains
  `agent_report` and `plan_status`. Then `UPDATE_SCHEMAS=1`, and regenerate
  `web/src/protocol.ts`.

**The server compiles against it, and drops the new kind for now.** `companion_link/ws.rs`
names every `CompanionMessage` variant twice: in `kind()` (`:248-261`) and in the exhaustive
`match env.message` (`:294-489`).
- Both gain an `AgentReport` arm.
- The arm drops the report and counts `rubato.events.dropped{rubato.reason="agent_report"}`.
- `stored_payload` (`:612-624`) stores it with an empty payload (ADR 0052).

Plan #226 replaces the drop with the real arm. The server does not announce `agent_mcp` in this
plan.

## 2. The assignment (#224)

The registry `Entry` (`sessions/registry.rs:19`) already has `launch_id`, `issue`, `branch`,
`worktree`, `plan_path`, `spend_usd` and `spend_cap_usd`. The rest of the assignment is in the
`LaunchCommand` and is lost after the launch. A new `sessions::Assignment` holds it:
`{ node_ids, map_version, planning, attempt, attempt_limit, launched_at }`.

**Known before the session starts.** `begin_launch` (`launch/apply.rs:32`) runs before the
launcher starts `claude` (`launch/arm.rs:49-73`). Its signature becomes
`begin_launch(&mut self, cmd: &LaunchCommand)`, and it records a pending assignment keyed by
`cmd.id`. `apply_launch` (`:54`) moves it onto `Entry.assignment`, and `end_launch` (`:45`)
drops a pending one whose launch failed. This closes the race `apply.rs:62` notes for hooks: an
interactive session can post `initialize` before `apply_launch` runs.

`Entry.assignment` is written to `LaunchRecord` and restored with the launch. It is a serde
default, so a file an older companion wrote loads with `None`.

`Registry::assignment_of(launch_id) -> Option<AssignmentView>` answers `get_assignment`.
- It looks at the entries first, then the pending launch.
- It returns `None` for an unknown or ended launch.
- It returns an owned copy, so the caller drops the lock before it does anything else.

## 3. The receiver (#222)

`HookServer::serve` (`hooks/server.rs:117-125`) builds its router itself. It gains
`serve_with(self, open: Router, guarded: Router)`, and `serve` becomes
`serve_with(Router::new(), Router::new())`. The hooks slice gains no edge to `agent_mcp`; the
composition root passes both routers.

`guarded` is merged under a layer that, in this order:
1. limits the body to 64 KiB (`DefaultBodyLimit::max`) and answers `413` above;
2. checks the bearer token in constant time (`authorized`, `:149`). Otherwise it answers `401`
   and counts `rubato.events.dropped{rubato.reason="mcp_auth"}`, apart from `/hook`'s `auth`;
3. passes the `X-Rubato-Launch` value on as a request extension, `LaunchHeader(Option<Uuid>)`
   (`launch_id`, `:140`).

`open` is merged with no token check. It carries only `/.well-known/*`, so Claude Code's OAuth
discovery, which may not send the configured headers, gets `404` and is counted `discovery`
rather than `mcp_auth`.

The composition root (`runtime.rs:236`) passes the `agent_mcp` routers. `/hook` and `/handoff`
are untouched.

A flood test in `hooks` sends 500 oversized and 500 well-formed requests to a stand-in guarded
route, concurrently with hooks. It asserts every hook is forwarded and every `/handoff`
answered. #221 repeats it against the real `agent_mcp` router, so `/mcp` handlers and the hook
arm share the registry lock (`runtime.rs:131`) under load.

## 4. The slice (#221)

`crates/rubato-companion/src/agent_mcp/`:

| File | Holds |
| --- | --- |
| `mod.rs` | `routers(state) -> (open, guarded)`, `AgentMcp` (the shared state), `TOOLS`, `on_plan_status` |
| `rpc.rs` | the Streamable HTTP subset of ADR 0071 and #197 design §9 (see the list below this table) |
| `tools.rs` | `get_assignment`, `report_status` and `plan_status`: argument schemas, `readOnlyHint` on the two reads, results as one `text` content item holding JSON |
| `report.rs` | the throttle and its drain (below) |
| `cache.rs` | `PlanStatus` per launch, in memory, keeping the newest `as_of`; `unknown` until the first push |
| `watch.rs` | the not-initialized watch (below) |

What `rpc.rs` does:
- it decodes one JSON-RPC message;
- requests `initialize`, `ping`, `tools/list` and `tools/call` are answered; anything else gets
  `-32601`;
- a notification gets `202` with no body;
- `GET` gets `405`;
- a present `Origin` that is not a loopback origin gets `403`;
- it sends no `Mcp-Session-Id`.

**Identity.** `401` means one thing only: a bad token (§3). Every request after that needs
`LaunchHeader(Some(id))` with `assignment_of(id)` present, pending launches included (§2). If
either is missing:
- `initialize` gets a JSON-RPC error, `-32001`, "no launch";
- `tools/call` gets a tool error;
- both count `refused`/`no_launch`.

An `issue` argument that differs from the assignment's is a tool error (`isError: true`) and
counts `refused`/`identity`.

**Locks.** Each handler copies its `AssignmentView` out of `assignment_of` and drops the registry
guard before it touches the slice's own maps. No handler holds the registry lock across an
`.await` other than taking it.

**Report.** `report_status` writes only `report.rs`'s map, never the registry. That is the
never-*working* rule of ADR 0071.
- `doing` is trimmed to one line and cut at 200 characters with `…`.
- `parts` keeps the first 8 entries for which `rubato_protocol::is_map_id` holds (`map.rs:15`).
  Whether a part exists in the map is the server's check.
- The report goes into the map with `at = now`. A call that replaces a pending report counts
  `coalesced`.

**The drain, gated from the start.** A task drains the map every 5 s.
- When `server_capabilities` contains `agent_mcp`, it sends each pending report as
  `CompanionMessage::AgentReport` and counts `rubato.mcp.reports{rubato.outcome="sent"}`. This is
  the `plan_mirror` pattern (`plan_mirror/arm.rs:97-106`).
- Otherwise it removes the pending entry and counts
  `rubato.mcp.reports{rubato.outcome="not_sent", rubato.reason="server_too_old"}`, so nothing
  leaves the machine later on a Welcome that does announce it.
- Until #225 the companion does not announce `agent_mcp` either. Until #226 no server does. So
  nothing is sent before `WHAT_LEAVES` says so.
- The tool call itself still counts `ok`.

**Not initialized.** The watch takes `launched_at` from the `Assignment` (§2). Any authenticated
`/mcp` request with a known launch marks that launch initialized, not only `initialize`. Once a
minute, a launch started more than 5 minutes ago and not marked counts
`rubato.mcp.not_initialized` once and logs `warn!`. Launches restored at start are marked, since
they initialized under the previous process and Claude Code does not initialize again.

**Protocol version.** The slice pins the versions it supports: `2025-11-25`, `2025-06-18` and
`2025-03-26`. It answers the client's version when it is one of those, and the newest otherwise.

**Telemetry.**
- A span `rubato.mcp.call` per request, with `rubato.mcp.tool` (the tool, the method, or
  `unknown`), `rubato.launch_id`, `rubato.issue`, `rubato.outcome` and `rubato.reason`.
- The counter `rubato.mcp.calls`.
- `warn!(rubato.launch_id, rubato.mcp.tool, rubato.reason, "mcp call refused")`.
- A panel "MCP calls by tool and outcome" in the Trust check row, querying
  `rubato_mcp_calls_total`, `rubato_mcp_reports_total` and `rubato_mcp_not_initialized_total`,
  and required by `observability/check.ts`.

**Tasks.** The drain and the watch are spawned through `Tasks` (`runtime.rs:231`). The drain is
essential; the watch is best-effort. Both are named in the AGENTS.md companion block, with the
slice.

**Guards.** The tool guard is a test over `TOOLS`. It fails when a tool without `readOnlyHint`
has a name or description matching `approv|accept|merge|handoff`, and it prints its count. A
must-fail case runs the same check on a list with `approve_plan` added.

## 5. Launch (#223)

`launch.sh`'s exec line (`launch/script.rs:131-142`) becomes:

```
exec <claude_command> --mcp-config <mcp.json> --allowedTools mcp__rubato-companion --session-id … --settings … --permission-mode … "$(cat prompt)"
```

`claude_command` may carry its own arguments (`--model …`). The two flags go right after it, and
the next word, `--session-id`, ends their lists.

The turn script (`launch/launcher.rs:268-273`) becomes:

```
exec <claude> --mcp-config "$(dirname "$0")/mcp.json" --allowedTools mcp__rubato-companion --settings … "$@"
```

`turn.sh` sits in the launch dir (`sessions/headless.rs:35-40`). `mcp.json` is written with
`write_private` next to `hooks.json` (`launcher.rs:200-210`):

```json
{"mcpServers":{"rubato-companion":{"type":"http","url":"http://127.0.0.1:<port>/mcp","headers":{"Authorization":"Bearer <token>","X-Rubato-Launch":"<launch id>"}}}}
```

Fork turns (`kernel/claude.rs:452-490`) are unchanged. A guard test pins that their arguments name
no `--mcp-config` or `--allowedTools`.

Which definition real Claude Code uses when a project `.mcp.json` also defines
`rubato-companion` is a recorded manual check with real `claude`, written into the #223 PR. A
stand-in cannot show it.

## 6. The link (#225)

- `hello_capabilities` (`runtime.rs:427`) adds `agent_mcp`.
- `Dispatch::run` (`runtime.rs:494`) gains `ServerMessage::PlanStatus(s) => agent_mcp::on_plan_status(s)`.
  The cache keeps the newer `as_of`, so a replayed push never overwrites a newer one. The
  `Dispatch` test fixture (`runtime.rs:781`) gains the field.
- `plan_status` answers with `link_up` from the `connected` watch (`server_link/client.rs:138,190`),
  as #197 design §2 promised.
- `pairing::WHAT_LEAVES` (`pairing/mod.rs:9`) names "for sessions started from Rubato, what the
  agent says it is doing and the map parts it names" (ADR 0071 supersedes ADR 0052 in part). A
  unit test asserts the string names it.

## 7. Order with #147's refresh

#147's plan (Decision 10) lands its companion PR 2 before #222, #223 and #225. It touches nothing
under `hooks/`, `server_link/`, `sessions/launches.rs` or the `claude` argument list. It does
change `runtime.rs` and `launcher.rs`, and its PR 1 regenerates `otel.rs`, the schemas and
`protocol.ts`, as #220 does.

So:
- #220 (protocol) and #224 (sessions) may land before #147's PR 2. Whichever lands second merges
  main and re-runs `UPDATE_SCHEMAS=1` and `gen:types`.
- #222, #221, #223 and #225 land after #147's PR 2, on top of it.

## 8. What this plan does not do

- No server arm for `AgentReport`, no `agent_reports` table, no `PlanStatus` derivation, no web
  (#226–#229).
- No SKILL.md change (#230).
- No change to hooks, their template, or the trust rule.
