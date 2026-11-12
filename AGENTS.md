# AGENTS.md — rubato

Instructions for coding agents working in this repository. Humans read it too.

## What Rubato is

**Each developer orchestrates their own agents. Rubato keeps the whole picture and keeps teammates, human and agent, aligned.**

Rubato is a planning-time tool for a team that owns several interconnected repositories (shared authentication and authorization, for example). It shows one system map above the repos, pins issues to the parts of the system they touch, shows which agent is working on which issue and where that work lands on the map, and brings each agent's plan back into the shared picture.

It is a lab to verify a set of theses about agent orchestration, not a product. If it becomes something, it will be open source.

Control stays federated; understanding is shared. The test for any feature: does it enlarge the shared picture, or does it start steering someone else's agent? Only the first belongs here.

## v0 scope

Does:

- One shared system map: contexts, contracts and the seams between them; repos are the code behind the nodes. Declared in `system-map.yml` with node ids that everything else references.
- Issues created and shown per repository (GitHub's rule), each pinned to a map node or seam. An issue is never attached only to a repo.
- Start a Claude Code session on an issue, in a worktree, in the developer's own tmux, through tmux-ide. A user can start sessions only in their own tmux.
- The agent's plan comes back as a Markdown file on the branch at a known path. Its affected-parts section paints the map.
- Progress per issue derived from existing signals (see progress model below), read three ways: by issue, by session, by map node.
- Decisions recorded as issue comments using a `Decision:` template, shown in context.
- Cross-repo visibility: two plans touching the same seam show up on that seam. Blocking is a later gate; v0 is the view.

Does not:

- Orchestrate agents across repositories or order cross-repo changes.
- Run agents itself. Rubato never executes anything on a developer's machine; the companion does, for its own user only.
- Stream or attach to a colleague's live terminal.
- Chat. Conversation stays where the team already talks.
- Audio.

## Architecture

Three parts, two of them built here:

| Part | Lives | Built here | Role |
| --- | --- | --- | --- |
| `rubato-server` | shared, one instance per team | yes (Rust) | map, issue pins, Projects join, plans, progress, decisions, GitHub OAuth, websocket fan-out, embedded frontend |
| `rubato-companion` | one per developer machine (WSL or macOS) | yes (Rust) | connects **outbound** to the server; pushes tmux-ide session state and Claude Code hook events; accepts launch commands only for its paired user; runs `tmux`, `git worktree`, `claude` |
| GitHub | — | no | code, issues, PRs, checks, plan files |

Dependencies we stitch rather than build: **tmux-ide** (session discovery, agent status from Claude Code hooks, `team --json`, `send`, `wait`, MCP, SDK; WSL is its Windows path), **Claude Code** (the only agent in v0; hooks are the event source), **GitHub** (OAuth, REST, Projects v2 GraphQL).

No inbound access to developer machines. Ever. The companion pairs with a login through a device-code flow and the server addresses launch commands to that pairing.

## Repository layout

```
Cargo.toml                 # workspace
crates/
  rubato-protocol/         # shared types: HookEvent, SessionState, LaunchCommand, Plan, MapNode. Write this first.
  rubato-companion/        # binary; features/ holds its Gherkin contract
  rubato-server/           # binary; axum, sqlx (SQLite), oauth2, octocrab; embeds web/dist
web/                       # TypeScript, React, React Flow (or Cytoscape), ELK layout
skills/rubato/             # Claude Code skill installed in every connected repo: map, node ids, plan path, plan format, hook config
schema/
  system-map.schema.json   # JSON Schema for system-map.yml
  plan.schema.json         # the plan file's affected-parts section
actions/plan-check/        # GitHub Action: fail a PR whose plan names an unknown node or lacks an O-row per seam
observability/
  alerts/                  # alert rules for the "never silent" list, each with a test that fires it
  dashboards/              # issue → node → session → cost; map with drift findings
docs/
  design/                  # design notes; the v0 design note is the source for this file
  plans/                   # plan files for work on Rubato itself (we eat our own dog food)
  decisions/               # ADRs, one file each, numbered; 0000-template.md; see "Decisions are records"
```

### Inside a deployable: vertical slices

Decided in ADR 0015, designed in `docs/plans/20-vertical-slices/design.md`, landing one PR per
deployable. Each of `rubato-server`, `rubato-companion` and `web` is organised as:

- one folder per **capability** a user or companion exercises (the slice), holding its routes,
  store queries, websocket arms, task bodies, telemetry sites and tests;
- a `kernel/` of leaf infrastructure (config, telemetry, storage connection, shared state holder,
  adapters to other people's tools) that names no slice;
- a small **composition root** (`lib.rs`, `app.rs`, `tasks.rs`; `runtime.rs`; `App.tsx`) that
  names every slice and is named by nobody but `main.rs` and the test crates.

A slice names another slice only through its root module, never the composition root; the slice
graph is acyclic; one slice owns each SQLite table. A test in each deployable checks this layout,
prints how many slices and edges it inspected, and compares the folders with that deployable's
block below, so keep the blocks in step with the code. `rubato-protocol` is regrouped by
capability as flat modules; `skills/rubato` and `actions/plan-check` are already one file per
capability.

Each deployable's block appears here with the PR that slices it:

<!-- slices: rubato-server -->
`crates/rubato-server/src/`: composition root `lib.rs`, `main.rs`, `app.rs` (router, 404 for
unknown `/api`, `/ws`, `/auth` paths), `tasks.rs` (poll loop, task supervision); `kernel/`
(config, telemetry, db connection, `AppState`, signer, `http::internal`, GitHub port); slices:

```text
auth            GitHub login, team record, CurrentUser, tokens  users, web_sessions, user_tokens
pairing         device-code pairing, the caller's pairings      pairing_codes, pairings
companion_link  companion websocket, envelopes, acks            companion_events, session_snapshots, stale_events
pins            pin an issue to a node or seam, Map: lines      pins, map_line_pins
launch          start a session through your own companion      launches, cap_hits
layout          where each part sits on the shared map          map_layout
map             load, validate, cache and serve the map         map_cache
map_draft       the shared map draft and its pull request       map_draft_ops, map_proposal
new_issue       create an issue as the App, pinned where begun  issue_creations
plans           parse and keep plan files from issue branches   plans
approve         accept a plan as yourself, posted once          plan_approvals
queue           issues queued on your own companions, in order  companion_queue
github_poll     issue reads, open issues, placement, coverage   pull_requests, decisions, issue_meta, open_issues, backlog_repos, uncovered_repos, plan_signals
overview        derived progress, findings, gauges              (none)
web_link        websocket fan-out to browsers                   (none)
```
<!-- /slices -->
<!-- slices: rubato-companion -->
`crates/rubato-companion/src/`: composition root `lib.rs`, `main.rs`, `runtime.rs` (channels,
task spawn with essential or best-effort flag, select arms, supervision); `kernel/` (config and
state files, telemetry, tmux, tmux-ide, git and `ps` adapters, `proc` for running them with a deadline);
slices:

```text
pairing      device-code pairing with a GitHub login            pair
server_link  outbound websocket, outbox, acks, Welcome limits   client task (essential)
sessions     Registry, identity from the branch, discovery      reconcile (essential), events watcher (best-effort)
hooks        loopback hook receiver, hooks template, apply_hook  hook server, dropped counter (essential)
launch       LaunchCommand: own user only, worktree, tmux, exec  launch task (essential)
spend        transcript cost, spend cap, CapHit                  after each hook
stale        heartbeat, stale sessions                           heartbeat, sweep (essential)
```
<!-- /slices -->
<!-- slices: web -->
`web/src/`: composition root `main.tsx`, `App.tsx` (shell: tabs, layout, selection, handlers);
`shared/` (`api.ts` request, `ApiError` and needs-login; `format.ts`; `dnd.ts` the dragged issue);
`protocol.ts` (generated); features under `features/`, each with an `index.ts`, checked by
`features/features.test.ts`:

```text
live        the overview, kept current over /ws/web              GET /api/overview, /ws/web, GET /api/me (probe)
auth        who is logged in, the login page                     GET /api/me, /auth/github/login
map         the system map: graph, shared layout over ELK, MapView, scenario player  PUT, DELETE /api/layout
issues      issue list, not-pinned backlog, detail, start gate   (none)
companions  pair with a code, lanes (sessions, queue), revoke     POST /api/pair/approve, DELETE /api/pairings/:id
pins        the "Pin to…" menu, pin to a node or seam, unpin     POST, DELETE /api/pins
queue       the "Queue on…" menu, queue on your own companion    POST, PUT, DELETE /api/queue
launch      start a session through your own companion           POST /api/launch
drafts      edit the map as one shared draft, propose it         POST /api/map/draft/ops, DELETE /api/map/draft/ops/:seq, POST discard, restore, propose, withdraw, GET yaml
findings    what the map and the plans disagree about            (none)
new_issue   "New issue here…": the form, pinned where begun      POST /api/issues
plans       proposed and open plans, where they fit, Accept plan  POST /api/plans/approve
repos       connected repos, sessions not tied to an issue       (none)
```
<!-- /slices -->
<!-- slices: rubato-protocol -->
`crates/rubato-protocol/src/`: flat modules, no `kernel/` folder and no slice test (design D5).
Shared kernel modules `ids`, `limits`, `message` (envelopes), `otel`, `schema`; capability
modules, each with the types and `routes` its seam uses (every type re-exported at the root):

```text
approve    ApproveRequest, ApproveResponse, reasons; PLAN_APPROVE
auth       Me; ME, LOGIN, CALLBACK, LOGOUT
pairing    device-code requests, PairingInfo, PairApproveRequest; PAIR_START, PAIR_POLL, COMPANION_WS, PAIRINGS, PAIR_APPROVE
session    SessionState, Heartbeat, StaleSession, StateSource, TmuxLocation
hook       HookEvent
pins       IssuePin, PinRequest; PINS
issue_body the Map: line (parse_map_line, map_line), PinSuggestion
issue_new  NewIssueRequest, NewIssueResponse, render_issue, the Rubato-Request: line; ISSUES
launch     LaunchCommand, LaunchResult, CapHit, LaunchRequest, LaunchResponse; LAUNCH
layout     MapLayout, Position; LAYOUT
map        SystemMap, Scenario, MapResponse; MAP, MAP_RELOAD
map_draft  MapOp, DraftOp, MapDraft, Proposal; DRAFT_OPS, DRAFT_DISCARD, DRAFT_RESTORE, DRAFT_YAML, DRAFT_PROPOSE, DRAFT_WITHDRAW
plan       Plan and its affected parts, O-rows, assumptions
github     PullRequestState, DecisionRecord, ReviewRecord, ApprovalRecord, ApprovalState
progress   derive_progress, Overview, IssueSummary, Backlog, Finding; OVERVIEW
queue      QueuedIssue, QueueRequest, QueueOrder; QUEUE
web_link   WebEvent; WEB_WS
```

The seam is the JSON Schema under `schema/protocol/`, not these Rust paths (ADR 0015).
<!-- /slices -->

## The progress model

One state per issue, derived; never stored by hand.

| State | Signal |
| --- | --- |
| Planned | issue exists and is pinned to the map |
| Started | session launched for the issue |
| Designing | session working, no plan file yet |
| Designed | plan file on the branch; affected nodes lit |
| Building | session working after the plan |
| Blocked | session blocked (hook) or a question open |
| In review | PR open, checks green |
| Done | merged |

Identity comes from the branch: pane cwd → worktree → branch → issue. No manual tags.

Every signal carries a last-seen time. A session with no heartbeat renders as *unknown*, never as *working*. Trust check first: is the signal still flowing?

## How we work

One planning workflow, the `rubato` skill (`skills/rubato/SKILL.md`, ADR 0028), here and in every connected repo.

- **Every design starts from the map.** Read `system-map.yml`, mark nodes affected, new or quiet, write the affected-parts table and the data flow across contexts. For work on Rubato itself, the map is `docs/design/rubato-map.yml`.
- **Size the track first.** Trivial, small or full, with the triggers in the skill; a full plan gets a design note in `docs/plans/<issue>-<slug>/design.md`.
- **Plan before code.** A plan file at `docs/plans/<issue>.md` using `schema/plan.schema.json`, under the spec headings (what the change does, how we'll know it works, what it must not break, how we'll debug it at 3 am). Front matter is `issue`, `title`, `map_version`, `spend_cap_usd` and `attempt_limit` only (ADR 0027). Open decisions go in a section headed **Assumed, not yet approved**; a decision that touches a seam blocks, a decision inside one node may be assumed and labelled.
- **Review and approval are issue comments.** The design review is posted as a `Review:` comment (`skills/rubato/review-template.md`, checklist in `review-checklist.md`). A person approves with a comment naming the plan's sha (`plan.ts sha`); an agent never posts it. Rubato reads both and shows them; neither is written into the plan file.
- **Small, reversible changes.** One task, one branch named `issue/<n>-<slug>`, one PR.
- **Every check reports how many things it checked and has a test that makes it fail on purpose.** This includes the plan check and every Gherkin suite.
- **Ask with a question, not with prose.** When a choice can't be settled from the code, raise it as a numbered question with options; don't bury it in a paragraph.
- **Cite `path:line`** when reporting what the code does.
- **Review map first.** A PR description leads with "look here first", at most three places.
- **Answer the 3 am question in every plan.** See Observability below. A plan without O-rows is not done.

## Observability

Rubato is a distributed system made of other people's tools. It will break quietly: a hook that stops firing, a companion that sleeps with its laptop, a plan file that names a node nobody has, a seam where two agents' work collides with error rate at zero. Treat it like production from the first commit.

### The 3 am question

Every plan, for Rubato itself or for a connected repo, carries one **O-row per seam it touches**:

> If this change breaks something on the other side of this seam at 3 am, which signal leads back to it, and who gets woken?

An O-row names the span, metric or log line, the dashboard panel it appears on, and the owner side of the seam. "We'll see it in the logs" is not an answer. The plan check verifies that every seam in the affected-parts table has an O-row.

### What each part emits

All telemetry is OpenTelemetry over OTLP, vendor-neutral. The team's stack is LGTM (Grafana, Loki, Tempo, Mimir) with Alloy as the collector; nothing in Rubato may depend on that.

| Part | Emits | Why |
| --- | --- | --- |
| Claude Code sessions | its own OTel export (already in place), launched with resource attributes `rubato.repo`, `rubato.issue`, `rubato.branch`, `rubato.node` | cost, duration and tool calls per issue and per map node come free |
| `rubato-companion` | a heartbeat metric per machine; a span per launch, per hook forwarded, per `team --json` poll; a `rubato.session.stale` event when a session goes quiet | the trust check: is the signal still flowing? |
| `rubato-server` | a span per inbound event with the session and issue as attributes; a span per GitHub call; metrics for sessions by state, issues by state, events dropped, GitHub rate-limit remaining | the derived progress model must be explainable from traces |
| plan check | a log line per plan checked: node ids found, unknown ids, O-rows present | every check reports what it checked |
| handoffs | a span that starts when a plan touches a seam and ends when the other side's PR merges, linking both traces | cross-agent handoffs are where orchestration breaks without an error |

Follow the OpenTelemetry GenAI semantic conventions for anything model-related; use the `rubato.*` namespace for ours and keep the list in `crates/rubato-protocol/src/otel.rs` so companion and server agree.

### Cost is a signal

Runaway token spend is the new memory leak. Cost per issue, per node and per seam is a first-class view, next to progress. Every launch carries a spend cap and an attempt limit from the plan; the companion stops the session and marks the issue *Blocked* when either is hit, and that is an event, not a log line.

### Things that must never be silent

- A companion that has not reported for N minutes (N from config, default 10).
- A session reporting *working* with no hook event for longer than the heartbeat interval.
- A plan whose affected nodes are not in the map.
- Two open plans touching the same seam.
- GitHub API rate limit under 10%.

Each of these has an alert rule committed under `observability/alerts/` and a test that makes it fire on purpose.

### Map versus territory

Rubato's map is declared. Production traffic is observed. When the two disagree (a service calls a node the map does not connect it to, or a declared seam carries no traffic), that is a finding, shown on the map, not an error to suppress. v0 only shows it; nothing acts on it.

## Conventions

Rust

- Edition 2024, stable toolchain, `cargo fmt` and `cargo clippy -D warnings` clean.
- `rubato-protocol` is the contract: serde types, no I/O, no tokio. Both binaries depend on it. Breaking changes to it are a seam change and block.
- Companion behaviour is specified in Gherkin (`crates/rubato-companion/features/*.feature`) and run with cucumber-rs against a private tmux server (`tmux -L rubato-test`) and temporary git repos. No mocks of tmux or git. Plain `#[test]` for small units.
- Server: axum, sqlx with SQLite (migrations in `crates/rubato-server/migrations`), oauth2 for GitHub login, octocrab for REST, `graphql_client` only for Projects v2 and only once the join is settled.
- Errors with `thiserror` in libraries, `anyhow` in binaries. `tracing` with `tracing-opentelemetry` everywhere; export over OTLP; attribute names only from `rubato-protocol::otel`.

TypeScript

- Node 20+, pnpm, strict TS, Vitest. No Python anywhere in this repo; Node is already on every machine.
- Skill scripts in `skills/rubato/scripts/*.ts`, run with `node --experimental-strip-types` or compiled; bash only for one-liners.

Files

- YAML for `system-map.yml` and `workspace.yml`; JSON Schema for anything agents write.
- Markdown plans; no HTML.

## Decisions are records

Every decision that touches a seam, a dependency, a data format or a non-goal is an ADR in `docs/decisions/`, one file each, numbered, from `0000-template.md`. Rules:

- **Status moves, text does not.** `proposed → accepted → superseded by NNNN` or `rejected`. Never edit an accepted ADR's decision; write a new one that supersedes it.
- **Who decided, human or agent.** Every ADR names its decider. An agent may draft and mark *proposed*; only a human moves it to *accepted*. This is the audit trail Chopin's decision log gives inside a plan, kept across plans.
- **Agents read them before planning.** The `rubato` skill loads accepted ADRs the same way it loads the map. A plan that contradicts an accepted ADR is a plan check failure unless it proposes a superseding ADR.
- **Inside-a-node assumptions are not ADRs.** They stay in the plan's "Assumed, not yet approved" section. The line is the seam.
- The list below is the seed; each item becomes `000N-<slug>.md` with status *accepted* in the first PR.

## Decisions already made (do not reopen without a superseding ADR)

- Planning-time tool, not an orchestrator.
- Rust for companion and server; TypeScript for frontend and skill scripts.
- Claude Code is the only agent in v0; tmux is required; tmux-ide is the runner.
- A user starts sessions only in their own tmux; no inbound access to machines.
- Plan travels as a file in the repo, not through an API.
- Issue identity from the branch name.
- Map is a versioned file with ids; plans may only name existing ids.
- Join between issue and map node: SQLite in `rubato-server` first; GitHub Projects v2 fields are the candidate once the model settles.

Open:

- Whether PantryPilot's verifier and release-notes agents still run in Actions by label; if so, they are a second, GitHub-native signal on the same issue.
- Exact plan file path inside connected repos.

## What stays human

Choosing what to build and where the boundaries are. Approving plans, especially any touching a seam. Reading the review map. Merging, deploying, changing configuration. Deciding whether a run is worth paying for.

## Don't

- Don't add a feature that steers another developer's agent.
- Don't scrape tmux panes for state; use hooks and `tmux-ide team --json`.
- Don't store progress; derive it.
- Don't invent node ids; read the map.
- Don't imply more scale or experience than the team has. This is one team, a handful of repos, a lab.
