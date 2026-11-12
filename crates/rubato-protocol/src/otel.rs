//! The `rubato.*` OpenTelemetry names. Companion and server use only these; the dashboard and
//! alert rules reference the same strings. GenAI conventions cover anything model-related.

/// Resource and span attributes.
pub mod attr {
    pub const REPO: &str = "rubato.repo";
    pub const ISSUE: &str = "rubato.issue";
    pub const BRANCH: &str = "rubato.branch";
    pub const NODE: &str = "rubato.node";
    pub const SEAM: &str = "rubato.seam";
    pub const SESSION_ID: &str = "rubato.session_id";
    pub const SESSION_SOURCE: &str = "rubato.session.source";
    pub const SESSION_STATE: &str = "rubato.session.state";
    /// The derived progress state on `rubato.issues.by_state`.
    pub const ISSUE_STATE: &str = "rubato.issue.state";
    pub const COMPANION_ID: &str = "rubato.companion_id";
    pub const USER: &str = "rubato.user";
    pub const HOOK_EVENT: &str = "rubato.hook_event";
    pub const LAUNCH_ID: &str = "rubato.launch_id";
    pub const MAP_VERSION: &str = "rubato.map.version";
    pub const PLAN_PATH: &str = "rubato.plan.path";
    pub const PLAN_UNKNOWN_IDS: &str = "rubato.plan.unknown_ids";
    pub const PLAN_O_ROWS_MISSING: &str = "rubato.plan.o_rows_missing";
    pub const GITHUB_ENDPOINT: &str = "rubato.github.endpoint";
    pub const TMUX_IDE_VERSION: &str = "rubato.tmux_ide.version";
    pub const DROP_REASON: &str = "rubato.reason";
    pub const SUBSCRIBERS: &str = "rubato.subscribers";
    pub const CAP_KIND: &str = "rubato.cap.kind";
    /// The background task a `rubato.task.exited` event names.
    pub const TASK: &str = "rubato.task";
    /// The route a refused web request named (`rubato.web.rejected`).
    pub const ROUTE: &str = "rubato.route";
    /// How something ended, or which state it is in: `opened`, `failed`, `withdrawn`, `merged`,
    /// `closed` for a map proposal; `opening`, `proposed`, `merged` for its state.
    pub const OUTCOME: &str = "rubato.outcome";
}

/// Metric instrument names.
pub mod metric {
    pub const COMPANION_HEARTBEAT: &str = "rubato.companion.heartbeat";
    /// Companion counter, by `rubato.hook_event` and `rubato.companion_id`. No session id: that
    /// is on the `rubato.hook.forward` span, so the series stay bounded (#43).
    pub const HOOK_FORWARDED: &str = "rubato.hook.forwarded";
    pub const SESSIONS_DISCOVERED: &str = "rubato.sessions.discovered";
    /// Server gauge, from each companion snapshot, by `rubato.session_state` and
    /// `rubato.companion_id`. Only the server records it (#43), so a sum counts each session once.
    pub const SESSIONS_BY_STATE: &str = "rubato.sessions.by_state";
    pub const ISSUES_BY_STATE: &str = "rubato.issues.by_state";
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
    /// `Rubato-Request:` line), `failed` (refused, or unseen for ten minutes).
    pub const ISSUES_CREATED: &str = "rubato.issues.created";
    /// Creations sent and not yet confirmed, recorded every poll tick.
    pub const ISSUES_OPENING: &str = "rubato.issues.opening";
}

/// Span names.
pub mod span {
    pub const LAUNCH: &str = "rubato.launch";
    pub const HOOK_FORWARD: &str = "rubato.hook.forward";
    pub const TEAM_POLL: &str = "rubato.team_poll";
    pub const INBOUND_EVENT: &str = "rubato.inbound_event";
    pub const GITHUB_CALL: &str = "rubato.github.call";
    pub const WS_FANOUT: &str = "rubato.ws.fanout";
    pub const HANDOFF: &str = "rubato.handoff";
}

/// Event (log record) names.
pub mod event {
    pub const SESSION_STALE: &str = "rubato.session.stale";
    pub const CAP_HIT: &str = "rubato.cap.hit";
    pub const PLAN_CHECK: &str = "rubato.plan.check";
    /// An essential background task returned while shutdown was not requested.
    pub const TASK_EXITED: &str = "rubato.task.exited";
}

/// Every name, for the test that checks the namespace and for documentation.
pub fn all_names() -> Vec<&'static str> {
    vec![
        attr::REPO,
        attr::ISSUE,
        attr::BRANCH,
        attr::NODE,
        attr::SEAM,
        attr::SESSION_ID,
        attr::SESSION_SOURCE,
        attr::SESSION_STATE,
        attr::ISSUE_STATE,
        attr::COMPANION_ID,
        attr::USER,
        attr::HOOK_EVENT,
        attr::LAUNCH_ID,
        attr::MAP_VERSION,
        attr::PLAN_PATH,
        attr::PLAN_UNKNOWN_IDS,
        attr::PLAN_O_ROWS_MISSING,
        attr::GITHUB_ENDPOINT,
        attr::TMUX_IDE_VERSION,
        attr::DROP_REASON,
        attr::SUBSCRIBERS,
        attr::CAP_KIND,
        attr::TASK,
        attr::ROUTE,
        attr::OUTCOME,
        metric::COMPANION_HEARTBEAT,
        metric::HOOK_FORWARDED,
        metric::SESSIONS_DISCOVERED,
        metric::SESSIONS_BY_STATE,
        metric::ISSUES_BY_STATE,
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
        span::LAUNCH,
        span::HOOK_FORWARD,
        span::TEAM_POLL,
        span::INBOUND_EVENT,
        span::GITHUB_CALL,
        span::WS_FANOUT,
        span::HANDOFF,
        event::SESSION_STALE,
        event::CAP_HIT,
        event::PLAN_CHECK,
        event::TASK_EXITED,
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
}
