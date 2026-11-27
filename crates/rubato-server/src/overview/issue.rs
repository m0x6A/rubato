//! One issue's summary as the overview derives it, for a slice that needs the derived approval
//! without building a whole payload (the merge gate, plan 82 Decision 1).

use super::build;
use super::inputs::{Maps, build_overview_inputs};
use crate::kernel::state::App;
use crate::kernel::tenancy::WsKey;
use anyhow::{Context, Result};
use chrono::Utc;
use rubato_protocol::{IssueRef, IssueSummary};

/// The summary of `issue` in workspace `ws`, built from the same inputs as `GET /api/overview`;
/// `None` when the overview holds no such issue. An error when the workspace's map is not loaded.
pub async fn issue_summary(
    app: &App,
    ws: &WsKey,
    issue: &IssueRef,
) -> Result<Option<IssueSummary>> {
    let map = app
        .map_of(ws)
        .await
        .with_context(|| format!("no map loaded for workspace {ws}"))?;
    let all = crate::map::list_workspaces(app, ws.tenant()).await;
    let maps = Maps::read(app).await;
    let inputs = build_overview_inputs(app, ws, &all, &maps).await?;
    let built = build::build(inputs, Some(&map), &app.config.limits, Utc::now());
    Ok(built.issues.into_iter().find(|i| &i.issue == issue))
}
