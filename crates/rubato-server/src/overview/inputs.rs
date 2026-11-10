//! Read every signal the overview derives from, through each owning slice.

use super::build::OverviewInputs;
use crate::kernel::state::App;
use crate::{companion_link, github_poll, launch, pins, plans};
use anyhow::Result;

pub async fn build_overview_inputs(app: &App) -> Result<OverviewInputs> {
    Ok(OverviewInputs {
        pins: pins::list(&app.db).await?,
        sessions: companion_link::all_sessions(&app.db).await?,
        plans: plans::all(&app.db).await?,
        prs: github_poll::all_prs(&app.db).await?,
        decisions: github_poll::all_decisions(&app.db).await?,
        launched: launch::launched_issues(&app.db).await?,
        cap_hits: launch::issues_with_live_cap_hit(&app.db).await?,
        meta: github_poll::all_issue_meta(&app.db).await?,
        launch_map_versions: launch::launch_map_versions(&app.db).await?,
    })
}
