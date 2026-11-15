# 0071. Agents report on purpose through one MCP server in their own companion

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#197), under delegation by Mattias Jonsson, who
  asked for best-effort answers to the issue's six questions; a human accepts
- **Seam:** companion-mcp, companion-claude-code, companion-server, web-server
- **Issue:** m0x6A/rubato#197

## Context

An agent tells Rubato about itself only through side channels: hooks for its lifecycle (ADR 0013,
ADR 0063), the branch for where it is (ADR 0006), the worktree for its plan draft (ADR 0053), and a
typed prompt for its approval (ADR 0055). It cannot say in one line what it is working on, and it
cannot ask Rubato "which map parts is my issue pinned to?" or "is my plan approved?". Issue #197
asks for one agent-facing interface over MCP. The design note
`docs/plans/197-agent-mcp/design.md` lists every channel and what replaces or keeps it. The
answers to the issue's six questions are agent choices made under Mattias's delegation, recorded
in the plan's Decisions table.

## Decision

**Where.** Each companion serves MCP to the sessions it launched, on its existing loopback hook
receiver, at `POST /mcp` (Streamable HTTP, JSON-RPC 2.0, `application/json` replies, no stream, no
session id). At launch the companion writes `mcp.json` (0600) next to `hooks.json`, naming a server
`rubato-companion` at `http://127.0.0.1:<port>/mcp` with the receiver's token in `Authorization`
and the launch id in `X-Rubato-Launch`. It starts `claude` with `--mcp-config <mcp.json>
--allowedTools mcp__rubato-companion` ahead of every other argument, interactive and headless
alike. The token is on no command line. The developer's own MCP servers still load. Room forks
(ADR 0057) get no MCP.

**What it offers.** Three tools, and no others without a new ADR:
- `get_assignment`, read-only, answered from the companion's launch record;
- `report_status { doing, parts }`, a signal: one line (at most 200 characters) and at most 8 map
  ids, forwarded to the server as `CompanionMessage::AgentReport`, at most one per launch per 5 s,
  the newest winning;
- `plan_status`, read-only, answered from the last `ServerMessage::PlanStatus` the server pushed
  for that launch, with its `as_of`, or `unknown` before the first push. The server derives it from
  the counted approvals whatever their source (comment or Rubato), the newest review, the draft and
  the branch, and pushes it after each `Welcome` and when it changes.

Both messages are gated by a capability `agent_mcp` announced in `Hello` and `Welcome`.

**Identity.** The launch id the companion wrote is the identity; the branch and launch rules of
ADR 0006 and ADR 0062 stay the source of truth. A call without a known launch id is refused; a call
whose optional `issue` is not the launch's issue is refused and counted. The server accepts an
`AgentReport` only for a launch this pairing started and drops map ids its map does not have.
Launches of one OS user are not isolated from each other: the receiver's token is per companion, as
in ADR 0063, whose threat is another OS user.

**What it is not.**
- Hooks stay as they are and stay the evidence. An agent report touches nothing in the companion's
  session registry (not `status`, `source`, `last_seen` or `last_hook_at`), no `SessionState` on
  the server, and `derive_progress` never reads it. It is shown with its own last-seen time.
- Plans keep travelling as files (ADR 0053 holds): there is no tool that submits a draft.
- No tool writes an approval, a review, a decision or a pin. A server test replays every companion
  message fixture and fails if any writes an approval table; a companion test fails when a tool
  that is not read-only has an approve-like name or description.
- It serves only sessions Rubato launched; manual sessions are #147's business.
- It answers the agent's own questions and sends it no work.

**What leaves the machine.** The `doing` line and `parts` of a launched session's report. The
server keeps the newest per launch in `agent_reports`, stores `AgentReport` in `companion_events`
with an empty payload, deletes a launch's row once that launch is out of its companion's snapshot,
and never posts the text to GitHub.

## Alternatives considered

- **MCP on `rubato-server`** (#147's design note §8): needs a token per session bound to a launch
  on the server, the identity problem #160 and #162 showed is easy to get wrong, and does not work
  while the companion's link is down.
- **Both, the companion proxying to the server:** two places to keep in step for the same three
  questions, which the companion can answer.
- **stdio:** Claude Code would spawn a `rubato-companion mcp` child that still has to reach the
  running companion with a token of its own: a second hop and a second process.
- **Route hooks through MCP too, or replace them:** an agent that forgets a tool call would go
  silent, and the trust rule ("never *working* without hook evidence") would rest on the agent's
  good behaviour.
- **Let an MCP call set the issue:** what the agent says would become identity, the thing #162
  removed from companion messages.
- **A per-launch token for `/mcp`:** isolates nothing while `/hook` takes the shared token; it
  follows if hooks ever get one.
- **`submit_plan_draft`, `wait_for_approval`, map and ADR resources:** a second draft path racing
  the mirror; a request held open across turns where the approval already arrives as a prompt;
  resources the skill already reads from the local clone. The last is the first candidate for a
  later ADR, for clients with no clone (#134).
- **The `rmcp` crate:** a new dependency for four requests and a notification; the follow-up may
  adopt it with its own ADR if the subset grows.

## Consequences

- An agent can read its assignment and plan status without `gh` or the map clone, and the team
  sees one line of what each launched agent says it is doing, with its age.
- The loopback receiver now carries three routes (`/hook`, `/handoff`, `/mcp`); a fault in `/mcp`
  must not stop hooks, so `/mcp` has a body limit and a flood test.
- `companion-server` gains two additive messages and a capability; old companions and servers
  ignore them.
- On acceptance, the status lines of 0004, 0052 and 0062 add "superseded in part by 0071".
- Watched by the `companion-mcp` O-row: span `rubato.mcp.call` and counters `rubato.mcp.calls`
  (by `rubato.mcp.tool`, `rubato.outcome`, `rubato.reason`) and `rubato.mcp.not_initialized` on the
  panel "MCP calls by tool and outcome"; and by the `companion-server` O-row: span
  `rubato.inbound_event` for kind `agent_report`, `rubato.events.dropped{rubato.reason="agent_report"}`
  and `rubato.plan_status.sent`. `SessionWorkingNoHooks` reads no MCP series and keeps firing as
  today.

## Supersedes

In part:

- **0004:** "Nothing in Rubato listens on a developer machine except a loopback HTTP port for hook
  delivery, guarded by a per-start token" reads "… except one loopback HTTP port, guarded by a
  per-start token, for hook delivery, `plan.ts handoff`'s report (0055) and the MCP calls of
  sessions that companion launched". Outbound only, and only the paired owner starts sessions:
  both stand.
- **0052:** what leaves the machine gains the capped `doing` line and the `parts` of a launched
  session's agent report, kept as the newest per launch in `agent_reports`; `rubato-companion pair`
  says so. New envelope kinds are still stored with an empty payload.
- **0062:** the client contract's lists gain `PlanStatus` in and `AgentReport` out, and the
  `agent_mcp` capability.

The rest of each stands. ADR 0003 (with its tmux-ide MCP), 0013, 0053 and 0063 are not changed.
