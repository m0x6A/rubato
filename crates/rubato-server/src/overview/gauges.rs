//! Gauges from the derived picture: issues by state, blocked issues by the station they left
//! from, open plans per seam, unknown plan ids, approvals of an earlier plan version.

use super::build;
use super::inputs::build_overview_inputs;
use crate::kernel::state::{App, LoadedMap};
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;

/// Every gauge for every workspace with a loaded map, each point with `rubato.workspace`
/// (ADR 0044). An issue two workspaces hold counts once in each, so panels take `max by
/// (rubato_workspace)` and never sum across workspaces.
pub async fn refresh_gauges(app: &App) -> Result<()> {
    let maps: Vec<(String, LoadedMap)> = app
        .maps
        .read()
        .await
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let named: Vec<(String, rubato_protocol::SystemMap)> = {
        let mut v = Vec::new();
        for w in crate::map::list_workspaces(app).await {
            if let Some(m) = maps.iter().find(|(id, _)| *id == w.id) {
                v.push((w.name.clone(), m.1.map.clone()));
            }
        }
        v
    };
    for (ws, map) in &maps {
        refresh_one(app, ws, map).await?;
        let name = crate::map::list_workspaces(app)
            .await
            .into_iter()
            .find(|w| &w.id == ws)
            .map(|w| w.name)
            .unwrap_or_default();
        let collisions = build::cross_workspace_findings(&name, &named)
            .iter()
            .filter(|f| matches!(f, rubato_protocol::Finding::IdCollision { .. }))
            .count();
        app.metrics.workspace_id_collisions.record(
            collisions as u64,
            &[KeyValue::new(attr::WORKSPACE, ws.clone())],
        );
    }
    app.metrics
        .workspace_shared_repos
        .record(shared_repos(&maps) as u64, &[]);
    Ok(())
}

/// Repos named by more than one workspace's map (ADR 0048).
fn shared_repos(maps: &[(String, LoadedMap)]) -> usize {
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, m) in maps {
        for r in m.map.repos_named() {
            *seen.entry(r.to_ascii_lowercase()).or_default() += 1;
        }
    }
    seen.values().filter(|n| **n > 1).count()
}

async fn refresh_one(app: &App, ws: &str, map: &LoadedMap) -> Result<()> {
    let w = || KeyValue::new(attr::WORKSPACE, ws.to_owned());
    let built = build_overview_inputs(app, ws).await?;
    let built = build::build(&built, Some(map), &app.config.limits, Utc::now());
    let (by_state, by_seam) = build::counts(&built);
    for (state, n) in by_state {
        app.metrics.issues_by_state.record(
            n as u64,
            &[
                KeyValue::new(attr::ISSUE_STATE, format!("{state:?}").to_lowercase()),
                w(),
            ],
        );
    }
    for (at, n) in build::blocked_from_counts(&built) {
        app.metrics.issues_blocked_from.record(
            n as u64,
            &[KeyValue::new(attr::ISSUE_BLOCKED_FROM, at), w()],
        );
    }
    for (seam, n) in by_seam {
        app.metrics
            .seam_open_plans
            .record(n as u64, &[KeyValue::new(attr::SEAM, seam), w()]);
    }
    let unknown: usize = built
        .findings
        .iter()
        .map(|f| {
            if let rubato_protocol::Finding::UnknownIds { ids, .. } = f {
                ids.len()
            } else {
                0
            }
        })
        .sum();
    app.metrics
        .plan_unknown_id_count
        .record(unknown as u64, &[w()]);
    let stale = built
        .findings
        .iter()
        .filter(|f| matches!(f, rubato_protocol::Finding::ApprovalStale { .. }))
        .count();
    app.metrics
        .plans_approval_stale
        .record(stale as u64, &[w()]);
    let not_fitting = built
        .issues
        .iter()
        .filter(|i| i.fit.as_ref().is_some_and(|f| !f.fits))
        .count();
    app.metrics
        .plans_not_fitting
        .record(not_fitting as u64, &[w()]);
    // Fits none of its workspaces (ADR 0045): with one workspace, the same as not fitting.
    let fit_none = built
        .issues
        .iter()
        .filter(|i| i.fit.as_ref().is_some_and(|f| !f.fits) && i.fits_in.is_empty())
        .count();
    app.metrics.plans_fit_none.record(fit_none as u64, &[w()]);
    Ok(())
}
