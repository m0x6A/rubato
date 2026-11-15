# 0072. A person may run a lead agent that directs only the sessions it launched for them, behind an orchestrator flag that is off by default

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#134), drafted at Mattias Jonsson's request of 2026-11-15 that #134 get a superseding ADR, built behind a flag that is off by default and not deployed; a human accepts it
- **Seam:** companion-server, companion-mcp, web-server
- **Issue:** m0x6A/rubato#134

## Context

Every part of Rubato assumes one person directing their own agents: they press Start, they type
to their session, they stop it. #134 asks for an orchestration agent, an "AI team lead", that
does this for a fleet of agents in the person's place. That runs into these accepted rules:
- ADR 0050 says Rubato is "not an orchestrator" and "never orders work across repositories";
- AGENTS.md's v0 scope says Rubato does not "orchestrate agents across repositories";
- AGENTS.md's test for any feature is "does it start steering someone else's agent?";
- ADR 0004, 0051 and 0062 let only the person start, message or stop their sessions;
- ADR 0071 gives a launched agent three MCP tools "and no others without a new ADR".

Mattias decided on 2026-11-15 that #134 gets this superseding ADR and is built behind an
off-by-default flag, up to a merged PR and not deployed. Plan: `docs/plans/134.md`; design:
`docs/plans/134-orchestrator/design.md`.

## Decision

**A lead is a launch.** A person may start a session with `role: lead` on an issue, through their
own companion, like any launch (ADR 0004). The companion must have announced `fleet`. The lead has
the issue's caps, branch, hooks, MCP and log. It is shown as *lead* to the whole team.

**What a lead may do, and only this.** Its MCP server (ADR 0071) offers four more tools, and only
to a lead launch:
- `fleet_list`: the owner's own launches in the lead's workspace, with state, progress and the
  last reply's first line. This is the same data the team already sees; other people's launches
  are left out.
- `fleet_launch { issue, companion_id?, slug? }`: a worker launch as the owner, through the
  server's ordinary launch path, only on one of the owner's own connected companions. It is
  recorded with `lead_launch_id`. It is refused for an issue another person has launched or has a
  draft on.
- `fleet_message { launch_id, text }`: a chat message to a worker this lead launched. It goes
  through the session-message queue (ADR 0051), as the owner, marked `via_launch`, with a first
  line that names the lead.
- `fleet_stop { launch_id }`: a stop for a worker this lead launched (ADR 0062), as the owner.

The server also queues a short notice to the lead when one of its workers ends, blocks, or goes
idle after working. Without that, nothing would start the lead's next turn.

Each tool call becomes one `CompanionMessage::FleetRequest`, sent only while the companion's link
is up and never kept in its outbox. The server answers it with one `ServerMessage::FleetResult`.
It claims the request in table `fleet_requests` before acting, keyed by companion, lead and
request id, with a hash of the action. A repeat gets the stored result and never acts twice; the
same id with a different action is refused.

**The boundary between people.** Whose agents those are, the server decides from the pairing and
from stored launches. It never takes the agent's word for that.
- The lead is the launch named in the call. It must be a launch of *this pairing's* companion with
  `role: lead`; otherwise the request is refused `identity` or `not_a_lead`.
- Its owner is the pairing's user.
- A `fleet_message` or `fleet_stop` aimed at another person's launch is refused `other_owner`;
  nothing is queued or sent, and it pages (`FleetCrossOwner`). One aimed at the owner's own launch
  that this lead did not start is refused `not_led`, and is counted without paging.
- `fleet_launch` runs `launch::start` as the owner, so it reaches only the owner's own connected
  companions. It cannot start a lead, and stops at a configured fleet size (default 4 live workers
  per lead).
- No tool approves, reviews, decides, pins, merges, calls an agent into a room, or touches the
  map. Approval stays a web route for a person (ADR 0054). The ADR 0071 tool guard and the
  every-fixture approval test cover the new tools and the new message.

**The boundary inside one OS user.** This one is weaker, and the ADR says so. As ADR 0071 says,
the companion's loopback token is shared by every session it launched, and launch ids are on
disk. A lead's `mcp.json` also carries a lead secret, which is in no environment variable and not
in `launches.json`, and the companion checks it before it offers or runs a fleet tool. But a
worker running as the same OS user can still read the lead's 0600 files and act as the lead. The
secret raises the bar from knowing a launch id to reading another session's files; it does not
isolate.

So within one owner's machine or container, any session of that owner could act as the owner's
lead: launch, message or stop the owner's own sessions, within the owner's caps and the fleet
size. It can never reach another person's agents, because the owner comes from the pairing. A
person who wants more isolation runs the lead on its own companion, as another OS user or in
another container.

**The flag.** The server's `RUBATO_ORCHESTRATOR` and the companion's `orchestrator` config are
both off by default, and both must be on.
- Off on the server: Welcome does not announce `fleet`. A lead launch is refused
  `orchestrator_off`, and so is a `FleetRequest`; nothing else happens.
- Off on the companion: Hello does not announce `fleet`, the web offers *Start as lead* on no lane
  of that companion, and the server refuses a lead launch to it (`companion_cannot_lead`).
- With either side off, Rubato behaves exactly as before this ADR.

**What stays as it was.**
- Rubato itself decides and orders nothing. It carries a person's own lead agent's requests to
  that person's own sessions, as it carries their own clicks, with the rights they have and no
  more.
- Each worker keeps its own caps, attempts, branch and plan.
- A worker's plan is approved by a person.
- No agent of another person is ever acted on: not messaged, stopped or launched.
- Claude Code is still the only agent (ADR 0003).
- Nothing listens on a developer machine beyond ADR 0071's loopback port.
- Rubato still runs no agent itself: the owner's companion does.

## Alternatives considered

- **A lead that directs any agent in the workspace**: steers other people's agents; fails AGENTS.md's test and ADR 0004.
- **A lead that directs every launch of its owner**, not only those it started: more reach than the issue needs. A person's hand-started sessions would change under them. It can widen later with a new ADR.
- **A per-launch OS user or a sandbox for each worker, for real isolation within one owner**: a lot to build for harm that stays inside one person's own sessions and caps. A separate companion for the lead gives the same isolation today.
- **Rubato as the orchestrator**: a server-side scheduler that orders work. That is the non-goal itself, and nobody's agent would own the choices.
- **A lead outside Rubato, driving the web API with the owner's browser session or token**: needs a credential for an agent, which ADR 0054 and ADR 0062 keep out of agents' hands. Its actions would look like the person's own, so the team could not tell.
- **A separate channel for the lead (a CLI posting to a new loopback route)**: duplicates ADR 0071's identity and transport.
- **Let the lead approve its workers' plans**: an agent never approves (ADR 0054).

## Consequences

Easier: one person can hand a set of issues to their own lead agent and watch it start, prompt and
stop its workers on the map. Every step is visible as *via lead* to the team.

Harder:
- A lead can spend its owner's money quickly. Each worker has its own cap, the fleet size is
  bounded, and the lead's own turns count on its issue.
- Cross-repo ordering by a person's agent is now possible behind the flag. Rubato shows it and does
  not judge it.
- Inside one OS user, the lead is not isolated from its owner's other sessions (see above).
- The boundary between people rests on one server check per request, tested both ways.

Watch:
- `rubato.fleet.requests{rubato.fleet.action, rubato.outcome, rubato.reason}`;
- alert `FleetCrossOwner`, which fires on any `other_owner` or `identity` refusal;
- span `rubato.fleet.request`, with the target's owner.

O-rows: companion-server, companion-mcp, companion-claude-code, web-server.

On acceptance, these statuses change:
- ADR 0004 becomes "accepted; superseded in part by 0062, 0071 and 0072";
- ADR 0050 becomes "accepted; superseded in part by 0072";
- ADR 0051 becomes "accepted; superseded in part by 0062 and 0072";
- ADR 0062 becomes "accepted; superseded in part by 0071 and 0072";
- ADR 0071 becomes "accepted; superseded in part by 0072".

In AGENTS.md, two lines change:
- "Orchestrate agents across repositories or order cross-repo changes" gains "except that a
  person's own lead agent may direct the sessions it launched for them, behind the orchestrator
  flag (ADR 0072)";
- "Planning-time tool, not an orchestrator" gains "(a person's own lead agent behind a flag, ADR
  0072)".

## Supersedes

In part, and only while the orchestrator flag is on:

- **0050:** "not an orchestrator" and "It never orders work across repositories" read "Rubato
  itself never orders work across repositories; it carries the requests of a person's own lead
  agent to the sessions that lead launched for that person (0072)". "A feature that steers
  another developer's agent is rejected at review" stands unchanged.
- **0004:** "only to the pairing of the user who pressed the button" reads "… who pressed the
  button, or whose own lead agent asked for it (0072)". Outbound only stands, and so does the
  companion's refusal of another user's command.
- **0051:** "A direct message is accepted only from the session's paired user" reads "… from the
  session's paired user, or from that user's lead agent for a worker it launched (0072)".
- **0062:** "`stop` is accepted only from the owner" reads "… from the owner, or the owner's lead
  agent for a worker it launched (0072)".
- **0071:** "Three tools, and no others without a new ADR" gains `fleet_list`, `fleet_launch`,
  `fleet_message` and `fleet_stop`, for lead launches only. "It answers the agent's own questions
  and sends it no work" stays true for every launch that is not a lead. A lead sends work only to
  its own workers, and is sent notices about them.

The rest of each stands.
