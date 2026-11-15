# 134: a lead agent for one person's own fleet; headless hosts and API keys

Plan: [../134.md](../134.md) · ADRs: [0072](../../decisions/0072-a-lead-agent-directs-only-its-owners-sessions.md)
(the lead; supersedes parts of 0004, 0050, 0051, 0062 and 0071) and
[0076](../../decisions/0076-headless-companion-as-a-daemon-or-container-with-an-api-key.md)
(hosts and API keys), both proposed · Issue: m0x6A/rubato#134 · Map: flows changed on
`companion-server`, `companion-mcp`, `companion-claude-code` and `web-server`; the `claude-code`
node's description.

The issue asks for four things:
- an orchestration agent that leads a fleet of agents in place of a human;
- docs that state today's assumptions;
- Claude through the API;
- an agent that runs as a daemon or in a container.

The first goes against accepted non-goals. Mattias decided on 2026-11-15 that it gets a
superseding ADR and is built behind a flag that is **off by default**, up to a merged PR, and not
deployed. The rest needs no non-goal change.

## 1. What Rubato assumes today

| Assumption | Where it is written | What it costs |
| --- | --- | --- |
| One person directs each agent: they press Start, type to it, stop it | ADR 0004, 0050, 0051, 0062; `launch/handler.rs:53` takes a `CurrentUser` | an agent cannot start or prompt another |
| An interactive session runs in tmux, through tmux-ide | ADR 0003; `kernel/tmux.rs`, `kernel/tmux_ide.rs` | needs a terminal machine |
| `claude` is installed and logged in on the developer's machine | nowhere explicit; `CompanionConfig.claude_command` (`kernel/config.rs:68`) | billing and auth are invisible |
| A headless companion (ADR 0062) runs `claude -p` turns with no terminal | ADR 0062; `kernel/claude.rs:70`, `:82` | no documented way to host it unattended |
| The companion process is started by hand | README | it stops when the terminal closes |

The design keeps all of these as the default and adds, behind flags or as documentation, what #134
asks for. `docs/design/companion-hosts.md` (new) writes the table down for people who run a
companion.

## 2. The lead

A **lead** is a launch with `role: lead`. It is started like any launch, through the owner's own
companion, on an issue: the issue that tracks the fleet's work, for example an epic. It has
everything a launch has: branch, caps from its plan, hooks, the session log, the agent MCP of ADR
0071. In addition it has four tools on that MCP server:

| Tool | Hint | Takes | Answers |
| --- | --- | --- | --- |
| `fleet_list` | `readOnlyHint` | nothing | the owner's launches in the lead's workspace that are live (§5) or launched in the last 24 h, at most 50, newest first. Each has issue, branch, companion, client kind, session state (or *unknown*), progress, `started_by` (`person` or `lead:<id>`), queued messages, and the last reply's first line (at most 200 characters). Other people's launches are never in it. |
| `fleet_launch` | none | `issue` (`owner/name#n`), optional `companion_id`, optional `slug`, optional `request_id` | the new launch id, or a refusal with its reason |
| `fleet_message` | none | `launch_id`, `text`, optional `request_id` | the queued message id, or a refusal |
| `fleet_stop` | none | `launch_id`, optional `request_id` | the stop's message id, or a refusal |

What a lead never gets: approve, review, decide, pin, merge, create an issue, call an agent into a
room, edit the map, start another lead. None of these is a tool. The ADR 0071 tool guard (no
non-read-only tool named or described like `approv|accept|merge|handoff`) runs over the four new
tools, and its must-fail fixture still fails it.

```mermaid
sequenceDiagram
  participant P as person (web)
  participant S as rubato-server
  participant C as owner's companion
  participant L as lead (claude)
  participant W as worker (claude)
  P->>S: LaunchRequest {issue: epic, role: lead} (web-server)
  S->>C: LaunchCommand {role: lead} (companion-server; only to a companion that announced fleet)
  C->>L: claude --mcp-config mcp.json (with X-Rubato-Lead); fleet tools listed
  L->>C: tools/call fleet_launch {issue: #12} (companion-mcp)
  C->>S: FleetRequest {request_id, lead_launch_id, launch {#12}} (link up only, not in the outbox)
  S->>S: claim (companion, lead, request_id, action hash); authorize; launch::start(owner, …, lead_launch_id)
  S->>C: LaunchCommand {role: worker, lead_launch_id}
  S->>C: FleetResult {request_id, ok, launch_id}
  C-->>L: {launch_id}
  C->>W: claude … (an ordinary worker launch)
  L->>C: tools/call fleet_message {launch_id, text}
  C->>S: FleetRequest {…, message}
  S->>S: target.user == owner (else other_owner) and target.lead_launch_id == lead (else not_led)
  S->>S: session_messages::enqueue_via(chat "From m0x6A's lead agent (…):\n…", author owner, via_launch lead)
  S->>C: FleetResult {ok, message_id}
  S->>C: SessionMessage when W waits for input (ADR 0051)
  C->>W: the message, typed or a headless turn
  S->>C: SessionMessage to L: "Fleet: worker … on #12 is idle" (when W goes idle after working)
```

**What workers see.** The server writes a lead's message with a first line naming it:
`From <owner>'s lead agent (launch <short id>):`. The companion types the message as it types any
chat. A worker can then tell its lead from its person. A worker's prompt at launch says it was
started by a lead.

**What wakes the lead.** A lead's turn ends like any session's. So that the next turn has a cause,
the `fleet` slice watches each `Sessions` snapshot. When one of a lead's workers ends, becomes
*blocked*, or goes idle after working, it queues one chat to the lead, authored `rubato`, for
example `Fleet: worker 3f2a… on m0x6A/rubato#12 is idle (last reply: "…")`. Notices are coalesced
to at most one per lead every 60 s, listing every change since the last one. They go through the
same queue (ADR 0051), so they are typed only when the lead waits for input, and they count on
the lead's cap like any turn. With the flag off, no notice is ever queued.

**What the team sees.** The lane shows *lead* on the lead's session and *via lead* on each worker.
The chat shows *via lead agent* on each message a lead sent. The overview carries `leads:
[{launch_id, owner, workers: [launch_id]}]`, built only from launches in the current companion
snapshots, and empty while the flag is off.

**The lead's prompt.** `skills/rubato/prompt-lead.md` is embedded in the server like `prompt.md`.
It names the four tools and the boundary of §3, says that approval waits for a person, and says
that fleet notices arrive as messages.

## 3. The boundary

AGENTS.md's test, "does it start steering someone else's agent?", must still hold. The server
holds it, not the companion and not the agent. Every `FleetRequest` goes through one function,
`fleet::authorize`, in this order:

1. **Flag.** Server flag off: refused `orchestrator_off`.
2. **Claim.** Insert into `fleet_requests` with the key (`companion_id`, `lead_launch_id`,
   `request_id`), the action's hash, and state `in_progress`.
   - If the key exists with the same hash, answer the stored result, or `in_progress` while it is
     still running, and do nothing.
   - If it exists with another hash, refuse `request_id_reused`.

   The claim comes first, so a repeat can never act twice and no check below runs on a repeat.
3. **Identity.** `lead_launch_id` must be a launch of *this pairing's* companion: otherwise
   `identity`. The companion took it from the call's `X-Rubato-Launch` header, never from tool
   arguments.
4. **Role.** That launch's `role` column is `lead`: otherwise `not_a_lead`.
5. **Owner.** owner = the pairing's user, which is also that launch's `user`.
6. **Target**, for `message` and `stop`:
   - the target launch exists: otherwise `unknown_launch`;
   - its `user` is not the owner: `other_owner`;
   - it is the lead itself, or its `lead_launch_id` is not this lead: `not_led`.
7. **Text**, for `message`: the same empty and too-long checks as the web route
   (`session_messages/handlers.rs:181-196`), counting the first line that names the lead.
8. **Launch**, for `launch`:
   - fewer than `RUBATO_FLEET_MAX` live workers of this lead: otherwise `fleet_full`;
   - the issue has no launch by another user in the last 30 days and no draft owned by another
     user: otherwise `someone_elses_issue`. A lead cannot use up a teammate's attempt budget
     (`launch_count`, `launch/store.rs:131`, counts every user's launches);
   - then `launch::start(app, ws, owner, req{role: worker, lead_launch_id}, note)`
     (`launch/handler.rs:64`). Its existing checks apply unchanged: the companion must be one of
     the owner's connected companions (`handler.rs:80-112`), the issue pinned or with a fitting
     plan, the attempt limit. A refusal there becomes the result's reason.

The claimed row is then updated with the outcome and the result. A crash between claim and update
leaves `in_progress`. A repeat sees that, and a row `in_progress` for more than 60 s is answered
`unknown`, never acted on again. The lead can check with `fleet_list`.

`other_owner` and `identity` are the refusals that mean the line between people was tested. Each
is counted with its reason and the target's owner, and fires `FleetCrossOwner` at once. `not_led`
is the owner's own session and is counted only.

**Inside one OS user** (ADR 0072). The companion's loopback token is shared by every session it
launched, and launch ids are in `launches.json` (ADR 0071's §4). So the companion adds a check:
- each lead launch gets a random lead secret, written only into that launch's `mcp.json` (0600) as
  header `X-Rubato-Lead`;
- it is kept in the companion's memory and in its state dir's `leads.json` (0600), and is in no
  environment variable and not in `launches.json`;
- `tools/list` and `tools/call` offer the fleet tools only when `X-Rubato-Launch` names a lead
  launch and `X-Rubato-Lead` matches its secret, compared in constant time;
- a fleet call without the secret is refused `identity` and counted
  `rubato.mcp.calls{rubato.reason="lead_secret"}`.

This stops a worker that only knows the lead's launch id. It does not stop one that reads the
lead's `mcp.json` as the same OS user. ADR 0072 accepts that, on the record: the harm stays inside
the owner's own sessions and caps. The isolation available today is a separate companion for the
lead, as another OS user or container.

**Tests that hold it** (server `fleet/tests.rs` and companion `agent_mcp`). Each prints what it
checked. The fixture is two users, `alice` and `bob`, each with a companion; alice runs lead L and
worker A1 that L launched, plus A2 that she started by hand; bob runs worker B.
- L's `fleet_message` and `fleet_stop` to B: refused `other_owner`. `session_messages` is
  unchanged, bob's companion gets nothing, and the counter goes up with `rubato.target_owner=bob`.
- L's `fleet_message` to A2: refused `not_led`.
- L's `fleet_launch` with bob's `companion_id`: refused by `launch::start` (403), nothing sent.
- L's `fleet_launch` on an issue bob launched last week: refused `someone_elses_issue`.
- L's `fleet_list`: includes A1 and A2, never B, even when B is on the same issue.
- A `FleetRequest` on bob's websocket naming L: refused `identity`.
- A `FleetRequest` from A1 naming A1: refused `not_a_lead`.
- Companion: a call to `/mcp` with A1's launch header and L's launch id but no lead secret is
  refused `identity`. With the wrong secret, it is refused. The fleet tools are not in A1's
  `tools/list`.
- The same `request_id` twice launches once. The same id with another action is refused
  `request_id_reused`. A row left `in_progress` past 60 s answers `unknown`.
- With the server flag off, every action is refused `orchestrator_off`, and a lead `LaunchRequest`
  from the web is refused. A lead launch to a companion that did not announce `fleet` is refused
  `companion_cannot_lead`.
- Must-fail: a test-only `authorize` that skips step 6 makes the first test fail. It runs under
  `#[should_panic]`, so the check is shown to bite.

The ADR 0071 server guard, which replays every `CompanionMessage` fixture and asserts the approval
tables are unchanged (#226), gets a `FleetRequest` fixture for each action.

## 4. Wire

All additive, gated by the capability `fleet`. The companion announces it only with `orchestrator:
true` in its config. The server announces it in Welcome only with `RUBATO_ORCHESTRATOR=1`. Neither
side sends a fleet message to a peer that did not announce it.

| Model | Where | Shape |
| --- | --- | --- |
| `LaunchRole` | protocol `launch` | `worker` (default, skipped when serialised) \| `lead` |
| `LaunchRequest.role` | web → server | optional, default `worker` |
| `LaunchCommand.role`, `LaunchCommand.lead_launch_id` | server → companion | optional; a worker a lead started carries its lead's id |
| `CompanionMessage::FleetRequest` | companion → server | `{ request_id, lead_launch_id, action }`; `action` is `list` \| `launch {issue, companion_id?, slug?}` \| `message {launch_id, text}` \| `stop {launch_id}` |
| `ServerMessage::FleetResult` | server → companion | `{ request_id, lead_launch_id, outcome: ok \| refused \| in_progress \| unknown, reason?, fleet?: [FleetEntry], launch_id?, message_id? }` |
| `FleetEntry` | in `FleetResult` | the columns of §2's `fleet_list` |
| `SessionMessageView.via_launch` | server → web | optional launch id of the lead that sent it |
| `Overview.leads` | server → web | `[{ launch_id, owner, workers }]` from current snapshots only; empty while the flag is off |
| `Overview.orchestrator`, `CompanionView.can_lead` | server → web | the flag, and per companion whether it announced `fleet`, so *Start as lead* shows only where it can work |
| capability `fleet` | Hello and Welcome | |
| `mcp.json` header `X-Rubato-Lead` | companion → claude-code | the lead secret, lead launches only |

**Sent only while the link is up.** A `FleetRequest` takes the next sequence number like any
envelope, but it is not kept in the outbox, as `Hello` is not (`server_link/client.rs:223`). With
the link down, the tool answers `link_down` at once, and a reconnect never replays a fleet request
late. On the server, a `FleetRequest` whose sequence was already recorded
(`companion_link/ws.rs:268`, `Ok(false)`) is not dropped: `fleet::on_replay` answers its stored
result, as `SessionMessageResult` is re-applied today. Expiry is not needed; the companion's clock
plays no part.

`FleetRequest` is stored in `companion_events` with an empty payload, as ADR 0052 asks of new
kinds. A message's text reaches the server once, in `session_messages.body`, as a person's chat
does.

**On the companion.** The `agent_mcp` slice (ADR 0071, #221) lists the four tools only when all
of these hold:
- the config's `orchestrator` is true;
- the launch record's `role` is `lead`;
- the lead secret matches;
- the server's Welcome announced `fleet`.

The `request_id` is the agent's when given, otherwise a new UUID. A call sends one `FleetRequest`
and waits up to 15 s for the `FleetResult` with that `request_id` and lead. On timeout the tool
answers `{"status":"pending","request_id":…}`, and calling again with that `request_id` gets the
stored answer (§3, step 2). `LaunchRecord` gains `role` and `lead_launch_id` (serde default),
written from the `LaunchCommand`. A `LaunchCommand {role: lead}` that reaches a companion with
`orchestrator` off is refused `LaunchResult::Refused {reason: orchestrator_off}`. The server
should never send one (§7).

## 5. Storage

| Table | Slice | Migration | Rows |
| --- | --- | --- | --- |
| `fleet_requests` | `fleet` (new) | `0028_fleet.sql` (or the next free number) | key (`companion_id`, `lead_launch_id`, `request_id`); `action_hash`, `action`, `owner`, `target_launch_id`, `state` (`in_progress`, `done`), `outcome`, `reason`, `result` (JSON), `claimed_at`, `done_at`; rows older than 30 days are deleted on insert |
| `launches.role`, `launches.lead_launch_id` | `launch` | same | nullable columns, written by `store_launch` from the command; index `launches_by_lead (lead_launch_id)` and `launches_by_user (user, created_at)`; backfill is not needed (old rows are workers) |
| `session_messages.via_launch` | `session_messages` | same | nullable; set only through `session_messages::enqueue_via` |

**Live**, for the fleet size and `leads`: a launch whose `lead_launch_id` is the lead and whose
result is not `refused` or `failed`. Either it has no result yet (pending counts), or its session
is in its companion's current snapshot and has not ended. One indexed query gets the lead's
workers, and `app.sessions` decides which are live in memory. `fleet_list` makes one query by
`(user, created_at)` with `LIMIT 50`. It then reads queued counts with one grouped query on
`session_messages_by_launch`, and last replies with one query on the session log for those launch
ids, with no per-entry round trip.

The migration is additive. Rollback: drop `fleet_requests` and leave the nullable columns, which
nothing else reads.

Slice edges:
- `companion_link → fleet`: dispatch of the new kind, the replay path, and the `Sessions` arm for
  notices;
- `fleet → launch, session_messages, session_log, plan_drafts, pairing`: reads, plus the two write
  paths.

Neither `launch` nor `session_messages` names `fleet`. `session_messages::stop`'s body
(`handlers.rs:36`) is split into `stop_as(app, user, launch_id, via)`, which the web route and
`fleet` both call.

## 6. Hosts and Claude through an API (ADR 0076)

**Claude through an API means Claude Code with an API key.** Claude Code is the only agent (ADR
0003). It already reads `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK` and
`CLAUDE_CODE_USE_VERTEX` from its environment, and `apiKeyHelper` from its settings. The
companion passes its environment on (`kernel/claude.rs:149-171` removes only `LAUNCH_ENV`). So
nothing in the launch changes. What is new:

- `kernel/claude_auth.rs` works out the kind, never reading the values:
  - from which variables are set: `api_key` (`ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`),
    `bedrock`, `vertex`;
  - or from whether `~/.claude/settings.json` has an `apiKeyHelper` key: `api_key`;
  - otherwise `login`.

  A test sets each variable to a marker and checks that the marker appears in no output.
- `hello_capabilities` (`runtime.rs:427`) adds `claude_auth:<kind>`. The span `rubato.launch` and
  the Hello span carry `rubato.claude.auth`.
- The companion's lane shows *API key*, *Bedrock* or *Vertex* when that is the kind, and nothing
  for `login`.

The label describes the companion's own environment. An interactive session's pane inherits the
tmux server's environment, which can differ; `companion-hosts.md` says so. This is not behind the
orchestrator flag. It changes no non-goal, and it is a label.

**Daemon.** `docs/design/companion-hosts.md` gives a systemd user unit and a launchd plist for
`rubato-companion run`, with `Restart=on-failure`. The key goes in an `EnvironmentFile` with mode
0600, never on a command line.

**Container.** `Dockerfile.companion` at the repository root:
- build stage: `cargo build --release --locked -p rubato-companion`;
- runtime: `node:22-bookworm-slim` with `git` and `ca-certificates`, and
  `npm i -g @anthropic-ai/claude-code@<pinned>` (an `ARG`, default a version at least two weeks
  old);
- user `rubato` (uid 1000), one volume `/home/rubato` holding `~/.config/rubato`, the state dir,
  the clones and the worktrees;
- `ENV RUBATO_IN_CONTAINER=1`, `CMD ["rubato-companion","run"]`; the person writes `local.yml`
  (with `companion.mode: headless`) into the volume, as on any machine;
- `rubato-companion run` refuses to start with `mode: interactive` when `RUBATO_IN_CONTAINER=1`,
  so a config copied from a laptop fails loudly instead of looking for tmux.

There is no Docker-in-Docker and no Docker socket. CI does not build or publish the image, and
`.github/workflows` is untouched. The build is checked by hand and the PR lists the commands:
- `docker build -f Dockerfile.companion .`;
- `docker run --rm … rubato-companion --version`;
- a run with an interactive config, which must exit with its message;
- `pair` and `run` against a local dev server, which must show the companion's lane as headless
  with its `claude_auth`.

## 7. The flag

| Side | Setting | Default | Off means |
| --- | --- | --- | --- |
| server | `RUBATO_ORCHESTRATOR=1` | unset (off) | no `fleet` in Welcome; `Overview.orchestrator = false`, so no *Start as lead*; a lead `LaunchRequest` refused `orchestrator_off` (`rubato.web.rejected`); a `FleetRequest` refused `orchestrator_off`; no fleet notices |
| server | `RUBATO_FLEET_MAX` | 4 | live workers per lead |
| companion | `companion.orchestrator: true` in `local.yml` | false | no `fleet` in Hello. The server then refuses a lead launch to it (`companion_cannot_lead`), and the web offers *Start as lead* only on lanes whose `can_lead` is true. |

The deployed server has no `RUBATO_ORCHESTRATOR`, and the deploy compose is not changed. A test in
each side's config pins the default off. The server's golden overview (flag off) shows
`orchestrator: false`, no `leads`, and `can_lead: false`.

## 8. Observability

| Signal | Where | Attributes |
| --- | --- | --- |
| span `rubato.fleet.request` | server, per `FleetRequest` | `rubato.launch_id` (the lead), `rubato.fleet.action`, `rubato.target_launch_id`, `rubato.user` (owner), `rubato.target_owner`, `rubato.outcome`, `rubato.reason` |
| counter `rubato.fleet.requests` | server | `rubato.fleet.action`, `rubato.outcome`, `rubato.reason` |
| counter `rubato.fleet.notices` | server | `rubato.outcome` (`queued`, `coalesced`) |
| alert `FleetCrossOwner` | `observability/alerts/rubato.rules.yml` | `increase(rubato_fleet_requests_total{rubato_reason=~"other_owner\|identity"}[10m]) > 0`, severity page, seam companion-server, owner rubato-server; with a promtool test that fires it and one with `not_led` that does not |
| `rubato.mcp.calls{rubato.mcp.tool=~"fleet_.*"}` | companion (ADR 0071's counter) | plus `rubato.outcome="pending"` on a 15 s timeout, `rubato.reason="lead_secret"`, `"link_down"` |
| `rubato.claude.auth` | companion, Hello and `rubato.launch` spans | `api_key`, `bedrock`, `vertex`, `login` |
| panel "Fleet requests by action and outcome" | `rubato-overview.json`, Trust check row | `rubato_fleet_requests_total`, `rubato_fleet_notices_total` |

AGENTS.md's "never silent" list gains: *a lead agent's request refused for touching another
person's agent, or for a forged identity* (`FleetCrossOwner`).

## 9. What we'd lose, what to watch

- **Spend.** A lead can start four workers, each with its own cap, plus its own turns, and every
  notice is a turn of the lead. The fleet size is bounded, notices are coalesced, and each
  worker's cap hit is a `CapHit` as today. A cap for the whole fleet is a later step.
- **Approval stays a bottleneck by design.** Workers stop at plan approval until a person approves
  in Rubato. This is the issue's "in place of a human" with one exception, and the ADR says so.
- **Isolation within one owner.** Any of the owner's sessions on the lead's companion could act as
  the lead (§3). The fix available today is a separate companion for the lead.
- **Cross-repo ordering.** A lead may start workers on issues in several repos. Rubato shows that
  on the map (each worker lights its pins) and does not judge it. Two workers on one seam raise
  `SeamCollision`, as any two plans do.
- **A headless lead's turn bounds.** A lead that waits inside one turn for its workers hits the
  quiet bound (ADR 0062) and pages `HeadlessTurnsKilled`. The prompt tells it to end its turn and
  wait for a notice.
- **The agent MCP comes first.** The tools ride on ADR 0071's `/mcp`. This build needs:
  - #220–#225 for the protocol, `agent_mcp`, `/mcp`, `mcp.json` and `server_link`;
  - #226 for the every-fixture approval guard;
  - #227 for its migration number;
  - #228 for `PlanStatus`, which `fleet_list` does not use but which lands in the same files.

  It also needs the panel "MCP calls by tool and outcome" from #220–#221. So it lands after
  #220–#228 merge (plan, task 0).

## 10. Answers to the issue's open questions

| Question | Answer | Where |
| --- | --- | --- |
| Revisit existing issues to remove the one-human assumption? | No. A lead is a delegate of one person; every rule about people stays. New issues that add fleet tools cite ADR 0072. | Decisions 5 |
| Which container or daemon approach? | A headless companion in a plain container, or as a systemd or launchd service; no Docker-in-Docker | ADR 0076, §6 |
