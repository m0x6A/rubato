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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::CurrentUser;
    use crate::kernel::config::Config;
    use crate::kernel::github::{Comment, FakeGitHub};
    use crate::kernel::http::Ws;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use crate::{approve, github_poll, plan_drafts, plans};
    use axum::Json;
    use axum::extract::State;
    use chrono::Duration;
    use rubato_protocol::otel::metric;
    use rubato_protocol::{
        DraftApproveRequest, DraftBundle, DraftFile, IssueRef, approval_hash, bundle_hash,
    };
    use std::sync::Arc;

    fn plan_text(issue: u64, change: &str) -> String {
        format!(
            "---\nissue: m0x6A/rubato#{issue}\n---\n## Affected parts\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | {change} | web-server |\n## O-rows\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| web-server | x | y | z | rubato-server |\n"
        )
    }

    fn bundle(text: &str) -> DraftBundle {
        let files = vec![DraftFile {
            path: "docs/plans/122.md".into(),
            text: text.into(),
        }];
        DraftBundle {
            launch_id: uuid::Uuid::from_u128(1),
            session_id: None,
            issue: IssueRef::new("m0x6A/rubato", 122),
            branch: "issue/122-x".into(),
            bundle_hash: bundle_hash(
                "docs/plans/122.md",
                &[("docs/plans/122.md".into(), text.into())],
            ),
            files,
            missing: vec![],
            refused: None,
            at: Utc::now(),
        }
    }

    fn comment(id: u64, at: chrono::DateTime<Utc>, sha: &str) -> Comment {
        Comment {
            id,
            author: "teammate".into(),
            author_association: "MEMBER".into(),
            author_is_bot: false,
            body: format!("Approved: plan {sha}"),
            created_at: at,
            url: "u".into(),
        }
    }

    /// ADR 0054: `rubato.plans.approval_stale` reads the merged list. Issue 71's counted comment
    /// names an earlier plan (a later comment of the current one, after the cutover, does not
    /// count, so it stays stale); issue 122's Rubato approval names a revision a newer one
    /// replaced. Two, one from each source.
    #[tokio::test]
    async fn the_stale_gauge_counts_comment_and_rubato_approvals() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let cutover = Utc::now() - Duration::hours(1);
        let mut cfg = Config::for_tests(url, "m0x6A");
        cfg.comment_approvals_until = Some(cutover);
        let github = Arc::new(FakeGitHub::new());
        let mut app = AppState::new(pool, cfg, github.clone());
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let text = include_str!("../../../../docs/design/rubato-map.yml");
        let map = LoadedMap {
            map: serde_yaml_ng::from_str(text).unwrap(),
            version: "v".into(),
            loaded_at: Utc::now(),
            yaml: text.into(),
        };
        app.set_map(rubato_protocol::DEFAULT_WORKSPACE, map.clone())
            .await;

        let i71 = IssueRef::new("m0x6A/rubato", 71);
        let (v1, v2) = (plan_text(71, "one"), plan_text(71, "two"));
        let path = plans::plan_path(&map, &i71, "issue/71-plan");
        github.with(|g| {
            g.files.insert(
                ("m0x6A/rubato".into(), path, "issue/71-plan".into()),
                v2.clone(),
            );
            g.comments.insert(
                ("m0x6A/rubato".into(), 71),
                vec![
                    comment(1, cutover - Duration::hours(1), &approval_hash(&v1)),
                    comment(2, cutover + Duration::minutes(1), &approval_hash(&v2)),
                ],
            );
        });
        plans::fetch_and_store(&app, &map, &i71, "issue/71-plan")
            .await
            .unwrap();
        github_poll::poll_comments(&app, &i71).await.unwrap();

        let (r1, r2) = (plan_text(122, "one"), plan_text(122, "two"));
        plan_drafts::on_draft(&app, &bundle(&r1), "m0x6A").await;
        let pressed = approve::approve_draft(
            State(app.clone()),
            Ws(rubato_protocol::DEFAULT_WORKSPACE.into()),
            CurrentUser("teammate".into()),
            Json(DraftApproveRequest {
                repo: "m0x6A/rubato".into(),
                number: 122,
                branch: "issue/122-x".into(),
                plan_sha: approval_hash(&r1),
                rev: Some(1),
                summary: String::new(),
            }),
        )
        .await;
        assert!(pressed.is_ok(), "{:?}", pressed.err());
        plan_drafts::on_draft(&app, &bundle(&r2), "m0x6A").await;

        refresh_gauges(&app).await.unwrap();
        let stale: Vec<u64> = reader
            .points(metric::PLANS_APPROVAL_STALE)
            .iter()
            .map(|p| p.value)
            .collect();
        assert_eq!(stale, [2], "one comment approval and one Rubato approval");
    }
}
