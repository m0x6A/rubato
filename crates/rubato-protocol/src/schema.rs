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
        one::<crate::pins::IssuePin>("issue_pin"),
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
        one::<crate::auth::Me>("me"),
        one::<crate::map::MapResponse>("map_response"),
        one::<crate::progress::IssueSummary>("issue_summary"),
        one::<crate::progress::Finding>("finding"),
        one::<crate::pairing::PairingInfo>("pairing_info"),
        one::<crate::pairing::PairApproveRequest>("pair_approve_request"),
        one::<crate::pins::PinRequest>("pin_request"),
        one::<crate::layout::MapLayout>("map_layout"),
        one::<crate::map_draft::DraftOpRequest>("draft_op_request"),
        one::<crate::map_draft::ProposeResponse>("propose_response"),
        one::<crate::queue::QueueRequest>("queue_request"),
        one::<crate::queue::QueueOrder>("queue_order"),
        one::<crate::launch::LaunchRequest>("launch_request"),
        one::<crate::launch::LaunchResponse>("launch_response"),
        one::<crate::issue_new::NewIssueRequest>("new_issue_request"),
        one::<crate::issue_new::NewIssueResponse>("new_issue_response"),
        one::<crate::progress::Overview>("overview"),
        one::<crate::web_link::WebEvent>("web_event"),
    ]
}
