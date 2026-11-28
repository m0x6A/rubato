//! Read every signal the overview derives from, through each owning slice.

use super::build::{OtherWorkspace, OverviewInputs, WorkspaceInputs};
use crate::kernel::state::{App, LoadedMap};
use crate::kernel::tenancy::{TenantId, WsKey};
use crate::{
    approve, companion_link, github_poll, launch, layout, pins, plan_drafts, plans, queue,
    spend_history, traffic,
};
use anyhow::Result;
use rubato_protocol::{SystemMap, WorkspaceInfo};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Every loaded map by workspace id, read under one guard: pointer copies, not map copies
/// (#171). What one request or one tick reads from, so its workspaces agree with each other.
pub struct Maps(BTreeMap<WsKey, Arc<LoadedMap>>);

impl Maps {
    pub async fn read(app: &App) -> Self {
        Self(app.maps.read().await.clone())
    }

    pub fn get(&self, ws: &WsKey) -> Option<&Arc<LoadedMap>> {
        self.0.get(ws)
    }

    /// Every loaded map by tenant and workspace, in that order.
    pub fn iter(&self) -> impl Iterator<Item = (&WsKey, &Arc<LoadedMap>)> {
        self.0.iter()
    }

    /// Each loaded map of `tenant` by its workspace's name, in the order of `all` (that tenant's
    /// workspaces, in switcher order).
    pub fn named<'a>(
        &'a self,
        tenant: &TenantId,
        all: &'a [WorkspaceInfo],
    ) -> Vec<(&'a str, &'a SystemMap)> {
        all.iter()
            .filter_map(|w| {
                self.get(&WsKey::new(tenant.clone(), w.id.clone()))
                    .map(|m| (w.name.as_str(), &m.map))
            })
            .collect()
    }
}

/// `all` is the workspace list of `ws`'s tenant and `maps` the loaded maps, each read once by
/// the caller.
pub async fn build_overview_inputs(
    app: &App,
    ws: &WsKey,
    all: &[WorkspaceInfo],
    maps: &Maps,
) -> Result<OverviewInputs> {
    let t = ws.tenant();
    // Comments under the cutover and approvals given in Rubato, as one list (ADR 0054).
    let approvals = approve::approvals(app, t).await?;
    // The workspace's own repositories only (design §11.3, the backlog gap): the open issues of
    // a repository another workspace's map names are that workspace's backlog, not this one's.
    let backlog = {
        let (mut issues, mut repos) = github_poll::backlog(&app.db, t, ws).await?;
        let named: std::collections::HashSet<String> = maps
            .get(ws)
            .map(|m| {
                m.map
                    .repos_named()
                    .iter()
                    .map(|r| r.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        issues.retain(|i| named.contains(&i.issue.repo.to_ascii_lowercase()));
        repos.retain(|r| named.contains(&r.repo.to_ascii_lowercase()));
        (issues, repos)
    };
    Ok(OverviewInputs {
        pins: pins::list(&app.db, ws).await?,
        // Only the sessions in this tenant's repositories (design §6.2).
        sessions: companion_link::all_sessions(&app.db)
            .await?
            .into_iter()
            .filter(|s| app.session_in(t, s))
            .collect(),
        plans: plans::all(&app.db, t).await?,
        drafts: plan_drafts::latest_drafts(app, t).await?,
        prs: github_poll::all_prs(&app.db, t).await?,
        decisions: github_poll::all_decisions(&app.db, t).await?,
        reviews: github_poll::all_reviews(&app.db, t).await?,
        approvals: approvals.counted,
        ignored_approvals: approvals.ignored,
        approvals_in_rubato: app.config.comment_approvals_until.is_some(),
        approval_findings: approve::link_findings(app, t).await?,
        launched: launch::launched_issues(&app.db, t).await?,
        cap_hits: launch::issues_with_live_cap_hit(&app.db, t).await?,
        meta: github_poll::all_issue_meta(&app.db, t).await?,
        launch_map_versions: launch::launch_map_versions(&app.db, t, ws).await?,
        backlog,
        uncovered_repos: github_poll::uncovered(&app.db, t).await?,
        layout: layout::all(&app.db, ws).await?,
        queue: queue::all(&app.db, t).await?,
        repo_pulls: github_poll::repo_pulls(&app.db, t).await?,
        traffic: app.traffic_of(ws).await,
        dismissals: traffic::dismissals(&app.db, ws).await?,
        spend: spend_history::series(&app.db, t).await?,
        workspaces: workspace_inputs(app, ws, all, maps).await?,
    })
}

/// The other workspaces, with their maps, pins and launches. Empty with one workspace.
async fn workspace_inputs(
    app: &App,
    ws: &WsKey,
    all: &[WorkspaceInfo],
    maps: &Maps,
) -> Result<WorkspaceInputs> {
    if all.len() < 2 {
        return Ok(WorkspaceInputs::default());
    }
    let mut out = WorkspaceInputs {
        launched_here: launch::launched_issues_in(&app.db, ws.tenant(), ws).await?,
        ..Default::default()
    };
    for w in all {
        if w.id == ws.ws() {
            out.here = Some((w.id.clone(), w.name.clone()));
            continue;
        }
        // Another workspace of the same tenant: never one of another tenant (design §5.5).
        let other = WsKey::new(ws.tenant.clone(), w.id.clone());
        out.others.push(OtherWorkspace {
            map: maps.get(&other).cloned(),
            pinned: pins::list(&app.db, &other)
                .await?
                .into_iter()
                .map(|p| p.issue)
                .collect(),
            launched: launch::launched_issues_in(&app.db, other.tenant(), &other).await?,
            id: w.id.clone(),
            name: w.name.clone(),
        });
    }
    Ok(out)
}
