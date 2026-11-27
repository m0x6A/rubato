//! Slice `launch`: start a session on an issue through one of the caller's own connected
//! companions (ADR 0004), and record what the companion reports back. Owns tables `launches`
//! and `cap_hits`.

mod handler;
mod prompt;
mod store;

pub use handler::{launch, start};

/// The workspace a launch was started from, in its tenant. Until `launches` carries its tenant
/// (plan 345, design §13) that is the tenant its repository is live in; `None` when no attached
/// installation covers the repository any more, and then nothing acts for the launch.
pub fn ws_key(
    app: &crate::kernel::state::App,
    t: &LaunchTarget,
) -> Option<crate::kernel::tenancy::WsKey> {
    app.github
        .tenant_of(&t.issue.repo)
        .map(|tenant| crate::kernel::tenancy::WsKey::new(tenant, t.workspace.clone()))
}
pub use prompt::{PROMPT_TEMPLATE, render_prompt, slugify};
pub use store::{
    FleetLaunch, LaunchTarget, companion_launched, issues_with_live_cap_hit, launch_map_versions,
    launch_target, launched_by_another, launched_issues, launched_issues_in, launches_by_id,
    launches_of, store_cap_hit as record_cap_hit, store_launch,
    store_launch_result as record_result, workers_of,
};
