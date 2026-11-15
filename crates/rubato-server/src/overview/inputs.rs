//! Read every signal the overview derives from, through each owning slice.

use super::build::{OtherWorkspace, OverviewInputs, WorkspaceInputs};
use crate::kernel::state::App;
use crate::{companion_link, github_poll, launch, layout, pins, plans, queue, spend_history};
use anyhow::Result;

pub async fn build_overview_inputs(app: &App, ws: &str) -> Result<OverviewInputs> {
    Ok(OverviewInputs {
        pins: pins::list(&app.db, ws).await?,
        sessions: companion_link::all_sessions(&app.db).await?,
        plans: plans::all(&app.db).await?,
        prs: github_poll::all_prs(&app.db).await?,
        decisions: github_poll::all_decisions(&app.db).await?,
        reviews: github_poll::all_reviews(&app.db).await?,
        approvals: github_poll::all_approvals(&app.db).await?,
        launched: launch::launched_issues(&app.db).await?,
        cap_hits: launch::issues_with_live_cap_hit(&app.db).await?,
        meta: github_poll::all_issue_meta(&app.db).await?,
        launch_map_versions: launch::launch_map_versions(&app.db, ws).await?,
        backlog: github_poll::backlog(&app.db, ws).await?,
        uncovered_repos: github_poll::uncovered(&app.db).await?,
        layout: layout::all(&app.db, ws).await?,
        queue: queue::all(&app.db).await?,
        repo_pulls: github_poll::repo_pulls(&app.db).await?,
        traffic: app.traffic.read().await.clone(),
        spend: spend_history::series(&app.db).await?,
        workspaces: workspace_inputs(app, ws).await?,
    })
}

/// The other workspaces, with their maps, pins and launches. Empty with one workspace.
async fn workspace_inputs(app: &App, ws: &str) -> Result<WorkspaceInputs> {
    let all = crate::map::list_workspaces(app).await;
    if all.len() < 2 {
        return Ok(WorkspaceInputs::default());
    }
    let mut out = WorkspaceInputs {
        launched_here: launch::launched_issues_in(&app.db, ws).await?,
        ..Default::default()
    };
    for w in all {
        if w.id == ws {
            out.here = Some((w.id, w.name));
            continue;
        }
        out.others.push(OtherWorkspace {
            map: app.map_of(&w.id).await.map(|m| m.map),
            pinned: pins::list(&app.db, &w.id)
                .await?
                .into_iter()
                .map(|p| p.issue)
                .collect(),
            launched: launch::launched_issues_in(&app.db, &w.id).await?,
            id: w.id,
            name: w.name,
        });
    }
    Ok(out)
}
