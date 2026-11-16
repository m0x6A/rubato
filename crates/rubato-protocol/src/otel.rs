//! The `rubato.*` OpenTelemetry names. Companion and server use only these; the dashboard and
//! alert rules reference the same strings. GenAI conventions cover anything model-related.

/// Resource and span attributes.
pub mod attr {
    pub const REPO: &str = "rubato.repo";
    pub const ISSUE: &str = "rubato.issue";
    /// A pull request's number, beside `rubato.repo` (issue #82).
    pub const PR: &str = "rubato.pr";
    pub const BRANCH: &str = "rubato.branch";
    pub const NODE: &str = "rubato.node";
    pub const SEAM: &str = "rubato.seam";
    pub const SESSION_ID: &str = "rubato.session_id";
    pub const SESSION_SOURCE: &str = "rubato.session.source";
    pub const SESSION_STATE: &str = "rubato.session.state";
    /// The derived progress state on `rubato.issues.by_state`.
    pub const ISSUE_STATE: &str = "rubato.issue.state";
    /// Whether the issues counted on `rubato.issues.by_state` are finished (#265): `true` or
    /// `false`. Every pair of state and finished is recorded, zeros included.
    pub const ISSUE_FINISHED: &str = "rubato.issue.finished";
    /// The station a blocked issue left from on `rubato.issues.blocked_from` (ADR 0043):
    /// `started`, `designing`, `designed`, `building` or `off_rail`.
    pub const ISSUE_BLOCKED_FROM: &str = "rubato.issue.blocked_from";
    /// Which hook blocked a session (ADR 0043), on `rubato.hook.forward` and
    /// `rubato.hook.forwarded`: `permission_prompt`, `idle_prompt`, `stop_failure`.
    pub const SESSION_BLOCKED_BY: &str = "rubato.session.blocked_by";
    pub const COMPANION_ID: &str = "rubato.companion_id";
    pub const USER: &str = "rubato.user";
    pub const HOOK_EVENT: &str = "rubato.hook_event";
    pub const LAUNCH_ID: &str = "rubato.launch_id";
    pub const MAP_VERSION: &str = "rubato.map.version";
    /// The workspace a route, map load, gauge or launch belongs to (ADR 0044).
    pub const WORKSPACE: &str = "rubato.workspace";
    /// The names of the settings a workspace config change touched (ADR 0075), comma-separated.
    /// Never a value.
    pub const WORKSPACE_CHANGED_KEYS: &str = "rubato.workspace.changed_keys";
    pub const PLAN_PATH: &str = "rubato.plan.path";
    pub const PLAN_UNKNOWN_IDS: &str = "rubato.plan.unknown_ids";
    pub const PLAN_O_ROWS_MISSING: &str = "rubato.plan.o_rows_missing";
    /// Retired map ids a plan names (ADR 0077): reported by the plan check, never failed.
    pub const PLAN_RETIRED_IDS: &str = "rubato.plan.retired_ids";
    pub const GITHUB_ENDPOINT: &str = "rubato.github.endpoint";
    /// The tmux version a companion found at start (`tmux -V`, ADR 0070).
    pub const TMUX_VERSION: &str = "rubato.tmux.version";
    pub const DROP_REASON: &str = "rubato.reason";
    pub const SUBSCRIBERS: &str = "rubato.subscribers";
    /// Agent panes one reconcile cycle found (`rubato.reconcile`, ADR 0070).
    pub const RECONCILE_PANES: &str = "rubato.reconcile.panes";
    pub const CAP_KIND: &str = "rubato.cap.kind";
    /// The background task a `rubato.task.exited` event names.
    pub const TASK: &str = "rubato.task";
    /// The route a refused web request named (`rubato.web.rejected`).
    pub const ROUTE: &str = "rubato.route";
    /// How something ended, or which state it is in: `opened`, `failed`, `withdrawn`, `merged`,
    /// `closed` for a map proposal; `opening`, `proposed`, `merged` for its state.
    pub const OUTCOME: &str = "rubato.outcome";
    /// A session message (ADR 0051): its id, and its kind (`chat`, …).
    pub const SESSION_MESSAGE_ID: &str = "rubato.session_message.id";
    pub const SESSION_MESSAGE_KIND: &str = "rubato.session_message.kind";
    /// A capability a companion announced (`rubato.companions.by_capability`).
    pub const CAPABILITY: &str = "rubato.capability";
    /// A session log entry's role (`user`, `assistant`) and kind (`prompt`, `reply`, `review`,
    /// `summary`), on `rubato.session_log.entries`.
    pub const LOG_ROLE: &str = "rubato.log.role";
    pub const LOG_KIND: &str = "rubato.log.kind";
    /// A draft revision number (ADR 0053).
    pub const DRAFT_REV: &str = "rubato.draft.rev";
    /// A `rubato_approvals` row (ADR 0054).
    pub const APPROVAL_ID: &str = "rubato.approval_id";
    /// A plan's room, `repo#number@branch` (ADR 0057).
    pub const ROOM: &str = "rubato.room";
    /// What a model call cost in USD (`rubato.room.fork_turn`).
    pub const COST_USD: &str = "rubato.cost_usd";
    /// The skill a span came from (ADR 0061): `rubato`, `plan-check`, or any other skill that
    /// uses `otel-emit.ts`. Also its `service.name`.
    pub const SKILL_NAME: &str = "rubato.skill.name";
    /// The skill script and subcommand a `rubato.skill.command` span ran, e.g. `plan check`.
    pub const SKILL_SUBCOMMAND: &str = "rubato.skill.subcommand";
    /// The exit code of a skill command, a command wrapped by `otel-emit.ts span`, or a
    /// room fork turn (`rubato.room.fork_turn`).
    pub const EXIT_CODE: &str = "rubato.exit_code";
    /// Which traffic source answered a fetch (ADR 0040): `dev_file` or `prometheus`.
    pub const TRAFFIC_SOURCE: &str = "rubato.traffic.source";
    /// Which shared layout a write or clear touched (ADR 0068): `flat` or `grouped`.
    pub const LAYOUT_ARRANGEMENT: &str = "rubato.layout.arrangement";
    /// The GitHub webhook event a delivery carried (ADR 0074): `issues`, `issue_comment`,
    /// `pull_request`, `check_suite`, `check_run`, `push`, `ping` or `other`.
    pub const GITHUB_EVENT: &str = "rubato.github.event";
    /// What a catch-up change was (ADR 0074): `issue`, `pull` or `plan`.
    pub const CHANGE_KIND: &str = "rubato.change.kind";
    /// A speech call (ADR 0073): `stt` or `tts`.
    pub const VOICE_OP: &str = "rubato.voice.op";
    /// The speech adapter: `openai_compatible` or `fake`.
    pub const VOICE_PROVIDER: &str = "rubato.voice.provider";
    /// Bytes sent to or read from the speech provider in one call.
    pub const VOICE_BYTES: &str = "rubato.voice.bytes";
    /// A fleet request's action (ADR 0072): `list`, `launch`, `message`, `stop`.
    pub const FLEET_ACTION: &str = "rubato.fleet.action";
    /// The launch a fleet message or stop is aimed at.
    pub const TARGET_LAUNCH_ID: &str = "rubato.target_launch_id";
    /// Whose launch that is: on a refusal, whether the line between people was tested.
    pub const TARGET_OWNER: &str = "rubato.target_owner";
    /// How the companion's Claude Code is authenticated (ADR 0076): `api_key`, `bedrock`, `vertex`,
    /// `login`. Never a value.
    pub const CLAUDE_AUTH: &str = "rubato.claude.auth";
    /// The MCP tool or method a companion's `rubato.mcp.call` served (ADR 0071): `get_assignment`,
    /// `report_status`, `plan_status`, `initialize`, `ping`, `tools/list`, `discovery` or
    /// `unknown`. Never a client-chosen string.
    pub const MCP_TOOL: &str = "rubato.mcp.tool";
}

/// Metric instrument names.
pub mod metric {
    pub const COMPANION_HEARTBEAT: &str = "rubato.companion.heartbeat";
    /// Companion counter, by `rubato.hook_event` and `rubato.companion_id`. No session id: that
    /// is on the `rubato.hook.forward` span, so the series stay bounded (#43).
    pub const HOOK_FORWARDED: &str = "rubato.hook.forwarded";
    /// Hook events whose payload the companion cut to its allowlist before forwarding (ADR 0052).
    pub const HOOK_PAYLOAD_TRIMMED: &str = "rubato.hook.payload_trimmed";
    pub const SESSIONS_DISCOVERED: &str = "rubato.sessions.discovered";
    /// Server gauge, from each companion snapshot, by `rubato.session_state` and
    /// `rubato.companion_id`. Only the server records it (#43), so a sum counts each session once.
    pub const SESSIONS_BY_STATE: &str = "rubato.sessions.by_state";
    pub const ISSUES_BY_STATE: &str = "rubato.issues.by_state";
    /// Server gauge: blocked issues by `rubato.issue.blocked_from`, all five values recorded
    /// on every refresh, zeros included (ADR 0043). Its sum equals the blocked series of
    /// `rubato.issues.by_state`.
    pub const ISSUES_BLOCKED_FROM: &str = "rubato.issues.blocked_from";
    pub const EVENTS_DROPPED: &str = "rubato.events.dropped";
    pub const GITHUB_RATE_LIMIT_REMAINING: &str = "rubato.github.rate_limit.remaining";
    pub const GITHUB_RATE_LIMIT_LIMIT: &str = "rubato.github.rate_limit.limit";
    pub const PLAN_PARSE_FAILURES: &str = "rubato.plan.parse_failures";
    pub const PLAN_UNKNOWN_ID_COUNT: &str = "rubato.plan.unknown_id_count";
    pub const SEAM_OPEN_PLANS: &str = "rubato.seam.open_plans";
    pub const MAP_LOADED: &str = "rubato.map.loaded";
    pub const MAP_LOAD_FAILURES: &str = "rubato.map.load_failures";
    /// Unix seconds of the last successful map load; the "Map age" panel is `time()` minus this.
    pub const MAP_LOADED_AT: &str = "rubato.map.loaded_at";
    pub const WS_CLIENTS: &str = "rubato.ws.clients";
    /// One per server poll tick; `ServerPollSilent` fires when it stops.
    pub const POLL_TICKS: &str = "rubato.poll.ticks";
    /// Unix seconds of the last good listing of a repo's open issues; 0 for a repo the map no
    /// longer names. The "Backlog age" panel is `time()` minus this.
    pub const BACKLOG_LISTED_AT: &str = "rubato.backlog.listed_at";
    /// Open issues listed per repo, recorded every poll tick.
    pub const BACKLOG_ISSUES: &str = "rubato.backlog.issues";
    /// A web request refused by a rule, by route and reason.
    pub const WEB_REJECTED: &str = "rubato.web.rejected";
    /// Repos the map or a tracked issue names that the GitHub App installation does not cover.
    pub const GITHUB_REPOS_UNCOVERED: &str = "rubato.github.repos_uncovered";
    /// Accepted writes to the shared map layout: a part moved, or the layout cleared.
    pub const LAYOUT_WRITES: &str = "rubato.layout.writes";
    /// Issues queued per companion, recorded every poll tick for every pairing, zeros included.
    pub const QUEUE_DEPTH: &str = "rubato.queue.depth";
    /// Live operations in the shared map draft, recorded every poll tick.
    pub const MAP_DRAFT_OPS: &str = "rubato.map.draft.ops";
    /// Map proposals by how they ended (`rubato.outcome`).
    pub const MAP_PROPOSALS: &str = "rubato.map.proposals";
    /// Unix seconds since when the map proposal has been in each state (`rubato.outcome`),
    /// recorded every poll tick: the time for the current state, 0 for every other. No unit,
    /// like `rubato.map.loaded_at`. "Map proposal age" is `time()` minus this, where it is not 0.
    pub const MAP_PROPOSAL_STATE_SINCE: &str = "rubato.map.proposal.state_since";
    /// Issues whose newest approval names an earlier version of the plan (finding
    /// `ApprovalStale`, ADR 0027), recorded every poll tick.
    pub const PLANS_APPROVAL_STALE: &str = "rubato.plans.approval_stale";
    /// `Map:` lines read from the open-issues listing, by `rubato.outcome`: `pinned`,
    /// `already_pinned`, `unknown_id`, `kind_mismatch` once per line applied; `untrusted_author`
    /// per listing that holds a line from someone outside the team (ADR 0026).
    pub const BACKLOG_MAP_LINES: &str = "rubato.backlog.map_lines";
    /// Issues created from Rubato (ADR 0025), by `rubato.outcome`: `created` (the create call
    /// answered), `found` (a create that did not answer, seen in the listing by its
    /// `Rubato-Request:` line), `failed` (refused, or unseen for ten minutes), `ignored` (that line
    /// on an issue the App did not write, once per listing, issue #165).
    pub const ISSUES_CREATED: &str = "rubato.issues.created";
    /// Creations sent and not yet confirmed, recorded every poll tick.
    pub const ISSUES_OPENING: &str = "rubato.issues.opening";
    /// GitHub REST calls made, one per page, by `rubato.github.endpoint` (ADR 0032). The
    /// budget is checked against this, not against the rate-limit gauge alone.
    pub const GITHUB_CALLS: &str = "rubato.github.calls";
    /// Tracked issues whose plan does not fit the map (ADR 0029), recorded every poll tick.
    pub const PLANS_NOT_FITTING: &str = "rubato.plans.not_fitting";
    /// Approvals Rubato posted as a person that GitHub has not confirmed for two minutes
    /// (ADR 0030), recorded every full poll tick.
    pub const PLANS_APPROVALS_OPENING: &str = "rubato.plans.approvals_opening";
    /// Traffic report fetches (ADR 0039), by `rubato.outcome`: `ok`, `rejected`, `failed`.
    pub const TRAFFIC_FETCHES: &str = "rubato.traffic.fetches";
    /// Seconds since the last good traffic report's window ended (since the source started,
    /// before the first one); recorded each fetch, absent when no source is configured. No
    /// unit, so the series keeps this name. `TrafficReportStale` fires above 600.
    pub const TRAFFIC_REPORT_AGE: &str = "rubato.traffic.report_age";
    /// Declared seams with no calls whose two parts show traffic (finding `SeamQuiet`).
    pub const TRAFFIC_SEAMS_QUIET: &str = "rubato.traffic.seams_quiet";
    /// Edges between two parts no seam joins (finding `UndeclaredTraffic`).
    pub const TRAFFIC_UNDECLARED_EDGES: &str = "rubato.traffic.undeclared_edges";
    /// Services in the report that name no map part.
    pub const TRAFFIC_UNMAPPED_SERVICES: &str = "rubato.traffic.unmapped_services";
    /// Ghost parts on the overlay: services seen that no part names (ADR 0059), dismissed ones
    /// left out; recorded each fetch, per workspace.
    pub const TRAFFIC_GHOST_PARTS: &str = "rubato.traffic.ghost_parts";
    /// Dismissed discrepancies per workspace (ADR 0060), recorded each fetch.
    pub const TRAFFIC_DISMISSALS: &str = "rubato.traffic.dismissals";
    /// 1 while a workspace has a traffic source, 0 after it is cleared or the workspace goes
    /// (ADR 0075): the other traffic gauges keep their last value until restart.
    pub const TRAFFIC_CONFIGURED: &str = "rubato.traffic.configured";
    /// Writes of an issue's spend history (ADR 0037), by `rubato.outcome`: `stored` (a session's
    /// spend changed and the issue's point was written), `unchanged` (same or older value),
    /// `rejected` (not finite, or negative), `failed` (the write returned an error).
    pub const SPEND_POINTS: &str = "rubato.spend.points";
    /// The companion stale threshold N in seconds, from config, recorded every poll tick.
    /// `CompanionSilent` compares heartbeats against it, with 600 when it is missing (#105).
    pub const COMPANION_SILENT_SECS: &str = "rubato.companion.silent_secs";
    /// Session messages by outcome and kind as they settle (ADR 0051): `delivered`, `not_idle`,
    /// `refused`, `failed`, `interrupted`, `delivery_unknown`.
    pub const SESSION_MESSAGES: &str = "rubato.session_messages";
    /// Age of the oldest message queued for a launch whose session waits for input: a queue that
    /// should drain and does not (alert `SessionMessageStuck`, plan 122 PR 7).
    pub const SESSION_MESSAGES_OLDEST_IDLE_QUEUED: &str =
        "rubato.session_messages.oldest_idle_queued_seconds";
    /// Connected companions per announced capability, so the cutover waits for all of them.
    pub const COMPANIONS_BY_CAPABILITY: &str = "rubato.companions.by_capability";
    /// Companion: pastes that failed, by reason.
    pub const SESSION_INPUT_FAILED: &str = "rubato.session_input.failed";
    /// Headless turns ended (ADR 0062), by `rubato.outcome`: `ok`, `failed`, `no_result`,
    /// `timeout`, `killed`.
    pub const HEADLESS_TURNS: &str = "rubato.headless.turns";
    /// Headless turns the companion killed, by `rubato.reason`: `stop`, `cap`, `revoke`,
    /// `timeout`, `quiet`, `companion_restarted`. `HeadlessTurnsKilled` fires on the last three.
    pub const HEADLESS_KILLED: &str = "rubato.headless.killed";
    /// Companion: launch results, by `rubato.outcome` (`started`, `refused`, `failed`) and, for a
    /// refusal, `rubato.reason` (the refusal reason); with `rubato.companion_id` (ADR 0070).
    pub const LAUNCHES: &str = "rubato.launches";
    /// Session log entries stored, by role and kind (ADR 0052).
    pub const SESSION_LOG_ENTRIES: &str = "rubato.session_log.entries";
    /// Hooks that should have made a log entry and did not, by reason (`no_text`, `no_prompt_id`).
    pub const SESSION_LOG_DROPPED: &str = "rubato.session_log.dropped";
    /// Draft revisions stored (ADR 0053).
    pub const DRAFTS_REVISIONS: &str = "rubato.drafts.revisions";
    /// Draft bundles not stored, by reason (`too_large`, `owner_conflict`, `unknown_launch`).
    pub const DRAFTS_REFUSED: &str = "rubato.drafts.refused";
    /// Room turns by `rubato.outcome` (answered, failed, stopped) (ADR 0057).
    pub const ROOMS_AGENT_TURNS: &str = "rubato.rooms.agent_turns";
    /// Times the agent turn limit stopped a room's turns.
    pub const ROOMS_TURN_LIMIT_HITS: &str = "rubato.rooms.turn_limit_hits";
    /// Companion: handoff reports from `plan.ts handoff` forwarded to the server, by
    /// `rubato.outcome` (ADR 0055).
    pub const HANDOFF_REPORTS: &str = "rubato.handoff.reports";
    /// Approvals delivered more than 30 minutes ago, still current, with no matching commit and
    /// no `committed` handoff report (alert `ApprovalNoHandoff`, ADR 0055).
    pub const PLANS_HANDOFF_OVERDUE: &str = "rubato.plans.handoff_overdue";
    /// Handoff commits whose link comment is not posted (alert `HandoffNoLinkComment`).
    pub const PLANS_LINK_COMMENT_MISSING: &str = "rubato.plans.link_comment_missing";
    /// Plans on a branch whose approval record differs from Rubato's (alert `ApprovalRecordMismatch`).
    pub const PLANS_APPROVAL_RECORD_MISMATCH: &str = "rubato.plans.approval_record_mismatch";
    /// `Approved:` comments first read at or after the cutover, which do not count (ADR 0054,
    /// finding `CommentApprovalIgnored`).
    pub const PLANS_COMMENT_AFTER_CUTOVER: &str = "rubato.plans.comment_after_cutover";
    /// Workspaces configured (ADR 0044), recorded at start and every poll tick.
    pub const WORKSPACES_CONFIGURED: &str = "rubato.workspaces.configured";
    /// Repos named by more than one workspace's map (ADR 0048), recorded every poll tick.
    pub const WORKSPACE_SHARED_REPOS: &str = "rubato.workspace.shared_repos";
    /// Rows whose workspace is no longer configured (ADR 0047), counted at start.
    pub const WORKSPACE_ORPHAN_ROWS: &str = "rubato.workspace.orphan_rows";
    /// Ids that name parts with different repos in two workspaces sharing a repo (finding
    /// `IdCollision`, ADR 0045), recorded every poll tick.
    pub const WORKSPACE_ID_COLLISIONS: &str = "rubato.workspace.id_collisions";
    /// Tracked plans that fit none of their issue's workspaces (ADR 0045), every poll tick.
    pub const PLANS_FIT_NONE: &str = "rubato.plans.fit_none";
    /// GitHub webhook deliveries (ADR 0074), by `rubato.github.event` and `rubato.outcome`:
    /// `accepted`, `duplicate`, `ignored`, `rejected` (bad signature), `malformed`,
    /// `too_large`, `error` (not stored).
    pub const GITHUB_WEBHOOK_DELIVERIES: &str = "rubato.github.webhook.deliveries";
    /// Unix seconds of the newest accepted delivery, 0 before any; recorded every poll tick while
    /// webhooks are on, absent otherwise. No unit, like `rubato.map.loaded_at`.
    pub const GITHUB_WEBHOOK_LAST_ACCEPTED_AT: &str = "rubato.github.webhook.last_accepted_at";
    /// Age in seconds of the oldest accepted delivery not yet refreshed, 0 when none; recorded
    /// every poll tick while webhooks are on. `GitHubWebhooksStuck` fires above 300.
    pub const GITHUB_WEBHOOK_PENDING_OLDEST: &str = "rubato.github.webhook.pending_oldest_seconds";
    /// Changes a catch-up poll found that no delivery brought, by `rubato.change.kind`.
    pub const GITHUB_CATCHUP_CHANGES: &str = "rubato.github.catchup.changes";
    /// Unix seconds of the newest such change, 0 before any; recorded every poll tick while
    /// webhooks are on. `GitHubWebhooksSilent` compares it with the last accepted delivery.
    pub const GITHUB_CATCHUP_LAST_CHANGE_AT: &str = "rubato.github.catchup.last_change_at";
    /// Pull request views by outcome (`cached`, `fetched`, `failed`), issue #82, ADR 0034.
    pub const PULLS_VIEW_FETCHES: &str = "rubato.pulls.view_fetches";
    /// Merges sent and not confirmed for more than 120 s (ADR 0033), every poll tick.
    pub const PULLS_MERGES_OPENING: &str = "rubato.pulls.merges_opening";
    /// Speech calls by `rubato.voice.op`, `rubato.outcome` and `rubato.voice.provider` (ADR 0073);
    /// alert `SpeechProviderFailing`.
    pub const VOICE_SPEECH: &str = "rubato.voice.speech";
    /// Seconds one speech call took, by `rubato.voice.op`.
    pub const VOICE_DURATION: &str = "rubato.voice.duration";
    /// Voice join tokens minted or refused, by `rubato.outcome`.
    pub const VOICE_TOKENS: &str = "rubato.voice.tokens";
    /// Failed voice joins the browser reported, by `rubato.reason`.
    pub const VOICE_JOIN_FAILED: &str = "rubato.voice.join_failed";
    /// Fleet requests by `rubato.fleet.action`, `rubato.outcome` and `rubato.reason` (ADR 0072);
    /// alert `FleetCrossOwner` on `other_owner` and `identity`.
    pub const FLEET_REQUESTS: &str = "rubato.fleet.requests";
    /// Fleet notices to leads, by `rubato.outcome` (`queued`, `coalesced`).
    pub const FLEET_NOTICES: &str = "rubato.fleet.notices";
    /// Companion: MCP requests from launched sessions (ADR 0071), by `rubato.mcp.tool`,
    /// `rubato.outcome` (`ok`, `refused`, `coalesced`, `error`) and `rubato.reason`.
    pub const MCP_CALLS: &str = "rubato.mcp.calls";
    /// Companion: agent reports drained, by `rubato.outcome` (`sent`, `not_sent`) and
    /// `rubato.reason` (`server_too_old`).
    pub const MCP_REPORTS: &str = "rubato.mcp.reports";
    /// Companion: launches with no MCP request 5 minutes after they started, once each.
    pub const MCP_NOT_INITIALIZED: &str = "rubato.mcp.not_initialized";
    /// Server: `PlanStatus` pushes, by `rubato.reason` (`welcome`, `change`) and
    /// `rubato.outcome` (`sent`, `offline`).
    pub const PLAN_STATUS_SENT: &str = "rubato.plan_status.sent";
}

/// Span names.
pub mod span {
    pub const LAUNCH: &str = "rubato.launch";
    pub const HOOK_FORWARD: &str = "rubato.hook.forward";
    /// Companion: one reconcile cycle (`tmux list-panes`, identity, snapshot), with
    /// `rubato.companion_id` and an error status when `tmux list-panes` fails (ADR 0070).
    pub const RECONCILE: &str = "rubato.reconcile";
    pub const INBOUND_EVENT: &str = "rubato.inbound_event";
    pub const GITHUB_CALL: &str = "rubato.github.call";
    pub const WS_FANOUT: &str = "rubato.ws.fanout";
    pub const HANDOFF: &str = "rubato.handoff";
    /// An approval posted from Rubato as the person (ADR 0030).
    pub const PLAN_APPROVE: &str = "rubato.plan.approve";
    /// A merge as the person, wrapping its GitHub call (ADR 0033).
    pub const PR_MERGE: &str = "rubato.pr.merge";
    /// One fetch of the traffic report (ADR 0039), with `rubato.outcome`.
    pub const TRAFFIC_FETCH: &str = "rubato.traffic.fetch";
    /// Server: one dispatch of a launch's queue (ADR 0051).
    pub const SESSION_MESSAGE_DISPATCH: &str = "rubato.session_message.dispatch";
    /// Companion: one session message handled, with its outcome.
    pub const SESSION_INPUT_DELIVER: &str = "rubato.session_input.deliver";
    /// Server: one draft bundle received.
    pub const PLAN_DRAFT: &str = "rubato.plan.draft";
    /// Server: posting one link comment as the approver (ADR 0055).
    pub const PLAN_LINK_COMMENT: &str = "rubato.plan.link_comment";
    /// Companion: one read-only fork turn for a room (exit, cost).
    pub const ROOM_FORK_TURN: &str = "rubato.room.fork_turn";
    /// One skill command, from `otel-emit.ts` (ADR 0061), with `rubato.skill.name`,
    /// `rubato.skill.subcommand` and `rubato.exit_code`.
    pub const SKILL_COMMAND: &str = "rubato.skill.command";
    /// A call from a skill to another part, a client span with `peer.service` naming the map id
    /// of the other side (`github`, `system-map`, `plan-file`).
    pub const SKILL_CALL: &str = "rubato.skill.call";
    /// Server: one voice join token minted (ADR 0073).
    pub const VOICE_TOKEN: &str = "rubato.voice.token";
    /// Server: one transcription call to the speech provider.
    pub const VOICE_STT: &str = "rubato.voice.stt";
    /// Server: one synthesis call to the speech provider.
    pub const VOICE_TTS: &str = "rubato.voice.tts";
    /// Server: one fleet request of a lead (ADR 0072).
    pub const FLEET_REQUEST: &str = "rubato.fleet.request";
    /// Companion: one MCP request on `/mcp` (ADR 0071), with `rubato.mcp.tool`,
    /// `rubato.launch_id`, `rubato.issue`, `rubato.outcome` and `rubato.reason`.
    pub const MCP_CALL: &str = "rubato.mcp.call";
    /// Server: one stored change to a workspace's settings (ADR 0075), with `rubato.workspace`,
    /// `rubato.user` and `rubato.workspace.changed_keys`. Key names only, never a value.
    pub const WORKSPACE_CONFIG_CHANGED: &str = "rubato.workspace.config_changed";
}

/// Event (log record) names.
pub mod event {
    pub const SESSION_STALE: &str = "rubato.session.stale";
    pub const CAP_HIT: &str = "rubato.cap.hit";
    pub const PLAN_CHECK: &str = "rubato.plan.check";
    /// The app's user scenarios against the e2e tests that cite them (`pnpm scenarios`, ADR 0069).
    pub const SCENARIOS_CHECK: &str = "rubato.scenarios.check";
    /// A planning session waiting for a person's approval (skill `plan.ts wait`, ADR 0035).
    pub const PLAN_WAIT: &str = "rubato.plan.wait";
    /// A plan's pinned ADRs recorded as accepted by an approval (skill `adrs.ts accept`).
    pub const ADRS_ACCEPT: &str = "rubato.adrs.accept";
    /// A plan handed off to Git after approval in Rubato (skill `plan.ts handoff`, ADR 0055).
    pub const PLAN_HANDOFF: &str = "rubato.plan.handoff";
    /// What `plan.ts handoff` did with its line: posted to the companion's `/handoff`, or why not
    /// (stderr, for the agent to relay).
    pub const PLAN_HANDOFF_REPORT: &str = "rubato.plan.handoff.report";
    /// An essential background task returned while shutdown was not requested.
    pub const TASK_EXITED: &str = "rubato.task.exited";
    /// A hook's transcript not read for spend (ADR 0063), with `rubato.reason`; counted as
    /// `rubato.events.dropped` with `rubato.reason = transcript`.
    pub const TRANSCRIPT_REFUSED: &str = "rubato.transcript.refused";
    /// Which local maps the skill matched for the session repo (`map.ts`, ADR 0044).
    pub const MAP_SELECT: &str = "rubato.map.select";
    /// A plan skeleton written by `plan.ts init`.
    pub const PLAN_INIT: &str = "rubato.plan.init";
}

/// Standard OpenTelemetry semantic-convention keys that Rubato code may write next to its own
/// names. Outside the `rubato.*` namespace, so not in [`all_names`].
pub mod semconv {
    pub const SERVICE_NAME: &str = "service.name";
    pub const SERVICE_NAMESPACE: &str = "service.namespace";
    pub const PEER_SERVICE: &str = "peer.service";
}

/// Every name, for the test that checks the namespace and for documentation.
pub fn all_names() -> Vec<&'static str> {
    vec![
        attr::REPO,
        attr::ISSUE,
        attr::PR,
        attr::BRANCH,
        attr::NODE,
        attr::SEAM,
        attr::SESSION_ID,
        attr::SESSION_SOURCE,
        attr::SESSION_STATE,
        attr::ISSUE_STATE,
        attr::ISSUE_FINISHED,
        attr::ISSUE_BLOCKED_FROM,
        attr::SESSION_BLOCKED_BY,
        attr::COMPANION_ID,
        attr::USER,
        attr::HOOK_EVENT,
        attr::LAUNCH_ID,
        attr::MAP_VERSION,
        attr::PLAN_PATH,
        attr::PLAN_UNKNOWN_IDS,
        attr::PLAN_O_ROWS_MISSING,
        attr::PLAN_RETIRED_IDS,
        attr::GITHUB_ENDPOINT,
        attr::TMUX_VERSION,
        attr::RECONCILE_PANES,
        attr::DROP_REASON,
        attr::SUBSCRIBERS,
        attr::CAP_KIND,
        attr::TASK,
        attr::ROUTE,
        attr::OUTCOME,
        attr::WORKSPACE,
        attr::WORKSPACE_CHANGED_KEYS,
        attr::SESSION_MESSAGE_ID,
        attr::SESSION_MESSAGE_KIND,
        attr::CAPABILITY,
        attr::LOG_ROLE,
        attr::LOG_KIND,
        attr::DRAFT_REV,
        attr::APPROVAL_ID,
        attr::ROOM,
        attr::EXIT_CODE,
        attr::COST_USD,
        attr::SKILL_NAME,
        attr::SKILL_SUBCOMMAND,
        attr::TRAFFIC_SOURCE,
        attr::LAYOUT_ARRANGEMENT,
        attr::GITHUB_EVENT,
        attr::CHANGE_KIND,
        attr::VOICE_OP,
        attr::VOICE_PROVIDER,
        attr::VOICE_BYTES,
        attr::FLEET_ACTION,
        attr::TARGET_LAUNCH_ID,
        attr::TARGET_OWNER,
        attr::CLAUDE_AUTH,
        attr::MCP_TOOL,
        metric::COMPANION_HEARTBEAT,
        metric::HOOK_FORWARDED,
        metric::HOOK_PAYLOAD_TRIMMED,
        metric::SESSIONS_DISCOVERED,
        metric::SESSIONS_BY_STATE,
        metric::ISSUES_BY_STATE,
        metric::ISSUES_BLOCKED_FROM,
        metric::EVENTS_DROPPED,
        metric::GITHUB_RATE_LIMIT_REMAINING,
        metric::GITHUB_RATE_LIMIT_LIMIT,
        metric::PLAN_PARSE_FAILURES,
        metric::PLAN_UNKNOWN_ID_COUNT,
        metric::SEAM_OPEN_PLANS,
        metric::MAP_LOADED,
        metric::MAP_LOAD_FAILURES,
        metric::MAP_LOADED_AT,
        metric::WS_CLIENTS,
        metric::POLL_TICKS,
        metric::BACKLOG_LISTED_AT,
        metric::BACKLOG_ISSUES,
        metric::WEB_REJECTED,
        metric::GITHUB_REPOS_UNCOVERED,
        metric::LAYOUT_WRITES,
        metric::QUEUE_DEPTH,
        metric::MAP_DRAFT_OPS,
        metric::MAP_PROPOSALS,
        metric::MAP_PROPOSAL_STATE_SINCE,
        metric::PLANS_APPROVAL_STALE,
        metric::BACKLOG_MAP_LINES,
        metric::ISSUES_CREATED,
        metric::ISSUES_OPENING,
        metric::GITHUB_CALLS,
        metric::PLANS_NOT_FITTING,
        metric::PLANS_APPROVALS_OPENING,
        metric::TRAFFIC_FETCHES,
        metric::TRAFFIC_REPORT_AGE,
        metric::TRAFFIC_SEAMS_QUIET,
        metric::TRAFFIC_UNDECLARED_EDGES,
        metric::TRAFFIC_UNMAPPED_SERVICES,
        metric::TRAFFIC_GHOST_PARTS,
        metric::TRAFFIC_DISMISSALS,
        metric::TRAFFIC_CONFIGURED,
        metric::COMPANION_SILENT_SECS,
        metric::SPEND_POINTS,
        metric::WORKSPACES_CONFIGURED,
        metric::WORKSPACE_SHARED_REPOS,
        metric::WORKSPACE_ORPHAN_ROWS,
        metric::WORKSPACE_ID_COLLISIONS,
        metric::PLANS_FIT_NONE,
        metric::GITHUB_WEBHOOK_DELIVERIES,
        metric::GITHUB_WEBHOOK_LAST_ACCEPTED_AT,
        metric::GITHUB_WEBHOOK_PENDING_OLDEST,
        metric::GITHUB_CATCHUP_CHANGES,
        metric::GITHUB_CATCHUP_LAST_CHANGE_AT,
        metric::PULLS_VIEW_FETCHES,
        metric::PULLS_MERGES_OPENING,
        metric::MCP_CALLS,
        metric::MCP_REPORTS,
        metric::MCP_NOT_INITIALIZED,
        metric::PLAN_STATUS_SENT,
        metric::SESSION_MESSAGES,
        metric::SESSION_MESSAGES_OLDEST_IDLE_QUEUED,
        metric::COMPANIONS_BY_CAPABILITY,
        metric::SESSION_INPUT_FAILED,
        metric::HEADLESS_TURNS,
        metric::HEADLESS_KILLED,
        metric::LAUNCHES,
        metric::SESSION_LOG_ENTRIES,
        metric::SESSION_LOG_DROPPED,
        metric::DRAFTS_REVISIONS,
        metric::DRAFTS_REFUSED,
        metric::ROOMS_AGENT_TURNS,
        metric::ROOMS_TURN_LIMIT_HITS,
        metric::HANDOFF_REPORTS,
        metric::PLANS_HANDOFF_OVERDUE,
        metric::PLANS_LINK_COMMENT_MISSING,
        metric::PLANS_APPROVAL_RECORD_MISMATCH,
        metric::PLANS_COMMENT_AFTER_CUTOVER,
        metric::VOICE_SPEECH,
        metric::VOICE_DURATION,
        metric::VOICE_TOKENS,
        metric::VOICE_JOIN_FAILED,
        metric::FLEET_REQUESTS,
        metric::FLEET_NOTICES,
        span::LAUNCH,
        span::HOOK_FORWARD,
        span::RECONCILE,
        span::INBOUND_EVENT,
        span::GITHUB_CALL,
        span::WS_FANOUT,
        span::HANDOFF,
        span::PLAN_APPROVE,
        span::PR_MERGE,
        span::TRAFFIC_FETCH,
        span::SESSION_MESSAGE_DISPATCH,
        span::SESSION_INPUT_DELIVER,
        span::PLAN_DRAFT,
        span::PLAN_LINK_COMMENT,
        span::ROOM_FORK_TURN,
        span::SKILL_COMMAND,
        span::SKILL_CALL,
        span::VOICE_TOKEN,
        span::VOICE_STT,
        span::VOICE_TTS,
        span::FLEET_REQUEST,
        span::MCP_CALL,
        span::WORKSPACE_CONFIG_CHANGED,
        event::SESSION_STALE,
        event::CAP_HIT,
        event::PLAN_CHECK,
        event::SCENARIOS_CHECK,
        event::PLAN_WAIT,
        event::ADRS_ACCEPT,
        event::PLAN_HANDOFF,
        event::PLAN_HANDOFF_REPORT,
        event::TASK_EXITED,
        event::TRANSCRIPT_REFUSED,
        event::MAP_SELECT,
        event::PLAN_INIT,
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_name_is_in_the_rubato_namespace_and_unique() {
        let names = super::all_names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate otel name");
        for n in &names {
            assert!(
                n.starts_with("rubato."),
                "{n} is outside the rubato.* namespace"
            );
            assert!(
                n.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                "{n} has bad chars"
            );
        }
        println!("checked {} otel names", names.len());
    }

    /// Every `rubato.*` string declared in this file is listed in `all_names()`, so the namespace
    /// test above sees it. Parsed from the source: a constant added without listing it fails here.
    #[test]
    fn every_declared_name_is_listed() {
        let src = include_str!("otel.rs");
        let listed: std::collections::BTreeSet<&str> = super::all_names().into_iter().collect();
        let mut declared = 0;
        let mut missing = Vec::new();
        for line in src.lines() {
            let line = line.trim();
            if !(line.starts_with("pub const ") && line.contains(": &str = \"rubato.")) {
                continue;
            }
            let value = line.split('"').nth(1).expect("a quoted name");
            declared += 1;
            if !listed.contains(value) {
                missing.push(value);
            }
        }
        assert!(
            declared > 50,
            "parsed only {declared} names; the pattern is wrong"
        );
        assert!(
            missing.is_empty(),
            "declared in otel.rs but not in all_names(): {missing:?}"
        );
        println!("checked {declared} declared otel names against all_names()");
    }
}
