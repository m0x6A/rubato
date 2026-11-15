//! Read every signal the overview derives from, through each owning slice.

use super::build::{OtherWorkspace, OverviewInputs, WorkspaceInputs};
use crate::kernel::state::{App, LoadedMap};
use crate::{
    companion_link, github_poll, launch, layout, pins, plans, queue, spend_history, traffic,
};
use anyhow::Result;
use rubato_protocol::{SystemMap, WorkspaceInfo};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Every loaded map by workspace id, read under one guard: pointer copies, not map copies
/// (#171). What one request or one tick reads from, so its workspaces agree with each other.
pub struct Maps(BTreeMap<String, Arc<LoadedMap>>);

impl Maps {
    pub async fn read(app: &App) -> Self {
        Self(app.maps.read().await.clone())
    }

    pub fn get(&self, ws: &str) -> Option<&Arc<LoadedMap>> {
        self.0.get(ws)
    }

    /// Every loaded map by workspace id, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Arc<LoadedMap>)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Each loaded map by its workspace's name, in the order of `all` (config order).
    pub fn named<'a>(&'a self, all: &'a [WorkspaceInfo]) -> Vec<(&'a str, &'a SystemMap)> {
        all.iter()
            .filter_map(|w| self.get(&w.id).map(|m| (w.name.as_str(), &m.map)))
            .collect()
    }
}

/// `all` is the workspace list and `maps` the loaded maps, each read once by the caller.
pub async fn build_overview_inputs(
    app: &App,
    ws: &str,
    all: &[WorkspaceInfo],
    maps: &Maps,
) -> Result<OverviewInputs> {
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
        dismissals: traffic::dismissals(&app.db, ws).await?,
        spend: spend_history::series(&app.db).await?,
        workspaces: workspace_inputs(app, ws, all, maps).await?,
    })
}

/// The other workspaces, with their maps, pins and launches. Empty with one workspace.
async fn workspace_inputs(
    app: &App,
    ws: &str,
    all: &[WorkspaceInfo],
    maps: &Maps,
) -> Result<WorkspaceInputs> {
    if all.len() < 2 {
        return Ok(WorkspaceInputs::default());
    }
    let mut out = WorkspaceInputs {
        launched_here: launch::launched_issues_in(&app.db, ws).await?,
        ..Default::default()
    };
    for w in all {
        if w.id == ws {
            out.here = Some((w.id.clone(), w.name.clone()));
            continue;
        }
        out.others.push(OtherWorkspace {
            map: maps.get(&w.id).cloned(),
            pinned: pins::list(&app.db, &w.id)
                .await?
                .into_iter()
                .map(|p| p.issue)
                .collect(),
            launched: launch::launched_issues_in(&app.db, &w.id).await?,
            id: w.id.clone(),
            name: w.name.clone(),
        });
    }
    Ok(out)
}
