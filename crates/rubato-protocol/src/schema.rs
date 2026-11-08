//! JSON Schema export. The files under `schema/protocol/` are snapshots of these; a test fails
//! when they drift (seams `companion-server`, `web-server`).

use schemars::{JsonSchema, Schema, schema_for};

/// Every root type exported, by file stem.
pub fn all() -> Vec<(&'static str, Schema)> {
    fn one<T: JsonSchema>(name: &'static str) -> (&'static str, Schema) {
        (name, schema_for!(T))
    }
    vec![
        one::<crate::ids::IssueRef>("issue_ref"),
        one::<crate::ids::MapRef>("map_ref"),
        one::<crate::session::IssuePin>("issue_pin"),
        one::<crate::map::SystemMap>("system_map"),
        one::<crate::plan::Plan>("plan"),
        one::<crate::launch::LaunchCommand>("launch_command"),
        one::<crate::launch::LaunchResult>("launch_result"),
        one::<crate::launch::CapHit>("cap_hit"),
        one::<crate::hook::HookEvent>("hook_event"),
        one::<crate::session::SessionState>("session_state"),
        one::<crate::session::Heartbeat>("heartbeat"),
        one::<crate::session::StaleSession>("stale_session"),
        one::<crate::github::PullRequestState>("pull_request_state"),
        one::<crate::github::DecisionRecord>("decision_record"),
        one::<crate::progress::Progress>("progress"),
        one::<crate::limits::Limits>("limits"),
        one::<crate::message::CompanionEnvelope>("companion_envelope"),
        one::<crate::message::ServerEnvelope>("server_envelope"),
        one::<crate::pairing::PairStartRequest>("pair_start_request"),
        one::<crate::pairing::PairStartResponse>("pair_start_response"),
        one::<crate::pairing::PairPollRequest>("pair_poll_request"),
        one::<crate::pairing::PairPollResponse>("pair_poll_response"),
        one::<crate::web::Me>("me"),
        one::<crate::web::MapResponse>("map_response"),
        one::<crate::web::IssueSummary>("issue_summary"),
        one::<crate::web::Finding>("finding"),
        one::<crate::web::PairingInfo>("pairing_info"),
        one::<crate::web::PairApproveRequest>("pair_approve_request"),
        one::<crate::web::PinRequest>("pin_request"),
        one::<crate::web::LaunchRequest>("launch_request"),
        one::<crate::web::LaunchResponse>("launch_response"),
        one::<crate::web::Overview>("overview"),
        one::<crate::web::WebEvent>("web_event"),
    ]
}
