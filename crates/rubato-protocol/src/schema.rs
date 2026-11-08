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
    ]
}
