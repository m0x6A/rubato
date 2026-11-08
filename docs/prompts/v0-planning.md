# v0 planning prompt

Run this in Claude Code, in plan mode, from the root of `rubato`. The output is a plan file, not code.

---

You are planning v0 of Rubato. Read `AGENTS.md` first and treat it as binding. Then read `docs/design/v0.md` (the design note; if it is not in the repo yet, say so and plan from `AGENTS.md` alone).

## Your task

Produce `docs/plans/v0.md`: the implementation plan for Rubato v0, written the same way we expect Rubato to receive plans from agents in connected repos. We eat our own dog food here, so the plan must follow `schema/plan.schema.json` if it exists; if it does not, the plan's shape below is the first draft of that schema and you should also propose the schema.

Do not write code. Do not create branches. Do not open issues.

## Shape of the plan

1. **Goal**, one paragraph, in the words of `AGENTS.md`: each developer orchestrates their own agents; Rubato keeps the whole picture and keeps teammates aligned.

2. **The map.** Propose `docs/design/rubato-map.yml`: Rubato's own system map with nodes for `rubato-server`, `rubato-companion`, `rubato-protocol`, `web`, `skills/rubato`, `actions/plan-check`, and external nodes for GitHub, tmux-ide and Claude Code. Name every seam explicitly: companion↔server (the protocol), server↔GitHub (OAuth, REST, Projects v2 GraphQL), companion↔tmux-ide (`team --json`, MCP), companion↔Claude Code (hooks), skill↔plan file. Give each seam an owner side and say what check guards it.

3. **Affected parts.** A table: node id, affected / new / quiet, what changes, which seam it touches. For v0 everything is new; still fill it in, because this table is what Rubato will later render.

4. **Data flow.** The chain from issue to merged, as in the progress model in `AGENTS.md`, with the type from `rubato-protocol` that carries each step.

5. **Work breakdown**, ordered by dependency, as tasks that each fit one branch and one PR. Expected order, which you may argue with:
   1. `rubato-protocol`: the types, serde, JSON Schema export, a snapshot test of the schema.
   2. `schema/system-map.schema.json` and `schema/plan.schema.json`; the plan check as a Node script with a test that makes it fail on purpose.
   3. `rubato-companion`: Gherkin feature files first (pairing; refusing launch for another user; worktree per issue; forwarding hook events; stale session detection; discovering sessions from `tmux-ide team --json`), then the binary that makes them pass against a private tmux server and temp git repos.
   4. `rubato-server` minimum: GitHub OAuth login, device-code pairing for companions, receive companion events, store map and pins in SQLite, derive progress, websocket fan-out.
   5. `web` minimum: render `system-map.yml`, pin an issue to a node, show sessions and progress on nodes, "start session" button that goes to the current user's companion only.
   6. `skills/rubato`: install the hook configuration, give the session the map and node ids, write the plan to the agreed path, affected-parts format.
   7. End to end: two connected repos (an auth provider and one consumer), three issues through the flow, OTLP resource attributes set, one Grafana dashboard showing issue → node → session → cost.
   For each task: inputs, outputs, the seam it touches, the check that proves it, and an estimate in hours.

6. **Assumed, not yet approved.** List every decision you had to make that `AGENTS.md` does not settle, with the option you chose and the alternatives. At minimum you will meet: the plan file path inside connected repos; how the companion authenticates to the server after pairing; polling interval versus push for `team --json`; how a session is matched to an issue when the branch name does not follow the convention; what "stale" means in seconds. Mark each one *inside a node* (assume and proceed) or *on a seam* (blocks until a human answers).

7. **Questions for the team.** Numbered, each with 2–4 options, only for the ones marked *on a seam*. Do not ask questions you could settle from the code or from `AGENTS.md`.

8. **Review map.** Where a reviewer should look first in this plan, at most three places.

9. **O-rows.** One per seam in the affected-parts table, as `AGENTS.md` requires: if it breaks at 3 am, which signal leads back to it, on which dashboard panel, and which side of the seam owns it. Name the span, metric or log line, and the `rubato.*` attributes it carries. Also list the alert rules from the "never silent" list that v0 must ship with, each with how its test fires it.

11. **Decision records.** For every decision in "Assumed, not yet approved" that is marked *on a seam*, and for every decision `AGENTS.md` lists as already made that v0 depends on, draft an ADR at `docs/decisions/NNNN-<slug>.md` using `docs/decisions/0000-template.md`. Status *proposed* for seam decisions, *accepted* for the ones already made. Do not write ADRs for inside-a-node assumptions; those live in the plan.

10. **Cost.** A rough token and time budget for executing this plan with Claude Code, per task, and the attempt limit after which a task stops and asks.

## Constraints

- Rust for `rubato-protocol`, `rubato-companion`, `rubato-server`; TypeScript for `web` and skill scripts; no Python.
- Stitch, don't build: tmux-ide for sessions, Claude Code hooks for events, GitHub for everything GitHub does. If you find yourself planning to rebuild something one of those already does, stop and say so.
- No inbound access to developer machines, anywhere in the plan.
- Progress is derived, never stored by hand.
- Keep the plan under 1,500 words excluding tables. Diagrams as Mermaid, one for the map and one for the data flow.

## When you are done

Write the plan to `docs/plans/v0.md`, the proposed map to `docs/design/rubato-map.yml`, the ADRs to `docs/decisions/`, and stop. Report in four lines: the number of tasks, the number of seam-level questions, the number of ADRs drafted, and the single biggest risk you see.
