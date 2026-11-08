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
}

/// Metric instrument names.
pub mod metric {
    pub const COMPANION_HEARTBEAT: &str = "rubato.companion.heartbeat";
    pub const HOOK_FORWARDED: &str = "rubato.hook.forwarded";
    pub const SESSIONS_DISCOVERED: &str = "rubato.sessions.discovered";
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
    pub const WS_CLIENTS: &str = "rubato.ws.clients";
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
        metric::WS_CLIENTS,
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
