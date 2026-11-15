# 0072. A person may run a lead agent that directs only the sessions it launched for them, behind an orchestrator flag that is off by default

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#134), drafted at Mattias Jonsson's request of 2026-11-15 that #134 get a superseding ADR, built behind a flag that is off by default and not deployed; a human accepts it
- **Seam:** companion-server, companion-mcp, web-server
- **Issue:** m0x6A/rubato#134

## Context

Every part of Rubato assumes one person directing their own agents: they press Start, they type
to their session, they stop it. #134 asks for an orchestration agent, an "AI team lead", that
does this for a fleet of agents in the person's place. That runs into three accepted rules:
ADR 0050 says Rubato "never orders work across repositories" and is not an orchestrator;
AGENTS.md's test for any feature is "does it start steering someone else's agent?"; and ADR 0071
gives a launched agent three MCP tools "and no others without a new ADR". Mattias decided on
2026-11-15 that #134 gets this superseding ADR and is built behind an off-by-default flag, up to a
merged PR and not deployed. Plan: `docs/plans/134.md`; design: `docs/plans/134-orchestrator/design.md`.

## Decision

**A lead is a launch.** A person may start a session with `role: lead` on an issue, through their
own companion, like any launch (ADR 0004). It has the issue's caps, branch, hooks, MCP and log.
It is shown as *lead* to the whole team.

**What a lead may do, and only this.** Its MCP server (ADR 0071) offers four more tools, and only
to a lead launch:
- `fleet_list`: the owner's own launches in the lead's workspace, with state, progress and the
  last reply's first line. This is the same data the team already sees.
- `fleet_launch { issue, companion_id?, slug? }`: a worker launch as the owner, through the
  server's ordinary launch path. It goes only to one of the owner's own connected companions. It
  is recorded with `lead_launch_id`.
- `fleet_message { launch_id, text }`: a chat message to a worker this lead launched. It goes
  through the session-message queue (ADR 0051), as the owner, marked `via_launch`.
- `fleet_stop { launch_id }`: a stop for a worker this lead launched (ADR 0062), as the owner.

Each tool call becomes one `CompanionMessage::FleetRequest` on the companion's websocket. The
server answers it with one `ServerMessage::FleetResult` and keeps the outcome in table
`fleet_requests`, keyed by request id. A replay therefore never launches twice.

**The boundary.** The server decides who the lead is and whose agents those are. It does not take
the agent's word for either.
- The lead is the launch named in `X-Rubato-Launch` (ADR 0071). It must be a launch of this
  pairing with `role: lead`; otherwise the request is refused `identity` or `not_a_lead`.
- Its owner is that launch's user. A `fleet_message` or `fleet_stop` whose target launch has
  another user, or was not launched by this lead, is refused `not_your_agent`. Nothing is queued
  or sent, and the refusal is counted and alerts (`FleetCrossOwner`).
- `fleet_launch` runs `launch::start` as the owner, so it reaches only the owner's own connected
  companions. It cannot start a lead, and stops at a configured fleet size (default 4 live
  workers per lead).
- No tool approves, reviews, decides, pins, merges, calls an agent into a room, or touches the
  map. Approval stays a web route for a person (ADR 0054). The ADR 0071 tool guard and the
  every-fixture approval test cover the new tools and the new message.

**The flag.** The server's `RUBATO_ORCHESTRATOR` and the companion's `orchestrator` config are
both off by default, and both must be on.
- Off on the server: Welcome does not announce `fleet`; a lead launch is refused
  `orchestrator_off`; a `FleetRequest` is refused `orchestrator_off` and nothing else happens.
- Off on the companion: Hello does not announce `fleet`, and no lead launch gets the four tools.
- With either side off, Rubato behaves exactly as before this ADR.

**What stays as it was.**
- Rubato itself orders nothing. It carries a person's own lead agent's requests to that person's
  own sessions, as it carries their own clicks, with the rights they have and no more.
- Each worker keeps its own caps, attempts, branch and plan.
- A worker's plan is approved by a person.
- No agent of another person is ever listed for action, messaged, stopped or launched.
- Claude Code is still the only agent (ADR 0003).
- Nothing listens on a developer machine beyond ADR 0071's loopback port.
- Rubato still runs no agent itself: the owner's companion does.

## Alternatives considered

- **A lead that directs any agent in the workspace**: steers other people's agents; fails AGENTS.md's test and ADR 0004.
- **A lead that directs every launch of its owner**, not only those it started: more reach than the issue needs. A person's hand-started sessions would change under them. It can widen later with a new ADR.
- **Rubato as the orchestrator**: a server-side scheduler that orders work. That is the non-goal itself, and nobody's agent would own the choices.
- **A lead outside Rubato, driving the web API with the owner's browser session or token**: needs a credential for an agent, which ADR 0054 and ADR 0062 keep out of agents' hands. Its actions would look like the person's own, so the team could not tell.
- **A separate channel for the lead (a CLI posting to a new loopback route)**: duplicates ADR 0071's identity and transport.
- **Let the lead approve its workers' plans**: an agent never approves (ADR 0054).

## Consequences

Easier: one person can hand a set of issues to their own lead agent and watch it start, prompt and
stop its workers on the map. Every step is visible as *via lead* to the team.

Harder: a lead can spend its owner's money quickly. Each worker has its own cap, the fleet size is
bounded, and the lead's own turns count on its issue. Cross-repo ordering by a person's agent is
now possible behind the flag; Rubato shows it and does not judge it. The boundary now rests on one
server check per request, tested both ways.

Watch:
- `rubato.fleet.requests{rubato.fleet.action, rubato.outcome, rubato.reason}`;
- alert `FleetCrossOwner`, which fires on any `not_your_agent` or `identity` refusal;
- span `rubato.fleet.request`.

O-rows: companion-server, companion-mcp, web-server.

On acceptance, these statuses change:
- ADR 0050 becomes "accepted; superseded in part by 0072";
- ADR 0071 becomes "accepted; superseded in part by 0072".

## Supersedes

In part, and only while the orchestrator flag is on:

- **0050:** "It never orders work across repositories" reads "Rubato itself never orders work
  across repositories; it carries the requests of a person's own lead agent to the sessions that
  lead launched for that person (0072)". "A feature that steers another developer's agent is
  rejected at review" stands unchanged.
- **0071:** "Three tools, and no others without a new ADR" gains `fleet_list`, `fleet_launch`,
  `fleet_message` and `fleet_stop`, for lead launches only. "It answers the agent's own questions
  and sends it no work" stays true for every launch that is not a lead. A lead sends work only to
  its own workers.

The rest of each stands.
