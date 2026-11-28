//! `GET /api/overview`: the map, every issue with its derived progress, findings, companions.

use super::build;
use super::inputs::{Maps, build_overview_inputs};
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::{map_draft, pairing};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::{MapResponse, Overview};

const SLICE: &str = "overview";

pub async fn overview(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    _user: CurrentUser,
) -> Result<Json<Overview>, StatusCode> {
    let map = app
        .map_of(ws.key())
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    // The workspace list and every loaded map, read once for this request (#171).
    let all = crate::map::list_workspaces(&app, ws.key().tenant()).await;
    let maps = Maps::read(&app).await;
    let mut inputs = build_overview_inputs(&app, ws.key(), &all, &maps)
        .await
        .map_err(|e| internal(SLICE, e))?;
    // The companions of this tenant's members only (design §6.2): a person in another tenant
    // is nobody here.
    let members = crate::tenants::member_logins(&app, ws.key().tenant())
        .await
        .map_err(|e| internal(SLICE, e))?;
    let companions: Vec<_> = pairing::list_pairings(&app.db, None)
        .await
        .map_err(|e| internal(SLICE, e))?
        .into_iter()
        .filter(|r| {
            members
                .as_ref()
                .is_none_or(|m| m.contains(&r.user.to_ascii_lowercase()))
        })
        .map(|r| pairing::to_info(&app, r))
        .collect();
    // A revoked companion's lane takes no starts, so its queue is not shown (its rows go with
    // the next queue write).
    let queue = std::mem::take(&mut inputs.queue)
        .into_iter()
        .filter(|q| {
            companions
                .iter()
                .any(|c| c.companion_id == q.companion_id && c.revoked_at.is_none())
        })
        .collect();
    let draft = map_draft::snapshot(&app.db, ws.key(), &map.map)
        .await
        .map_err(|e| internal(SLICE, e))?;
    // A part that exists only in the draft may have a position too (ADR 0016). Read before the
    // build takes the stored positions.
    let preview_positions: Vec<_> = draft
        .preview
        .iter()
        .flat_map(|preview| &preview.nodes)
        .filter_map(|n| {
            inputs
                .layout
                .positions
                .get(&n.id)
                .map(|p| (n.id.clone(), *p))
        })
        .collect();
    let built = build::build(inputs, Some(&map), &app.config.limits, Utc::now());
    let mut layout = built.layout;
    layout.positions.extend(preview_positions);
    // A repo two workspaces share may get two plan paths or an id meaning two parts.
    let here = all
        .iter()
        .find(|w| w.id == ws.as_str())
        .map(|w| w.name.as_str())
        .unwrap_or_default();
    let mut findings = built.findings;
    findings.extend(build::cross_workspace_findings(
        here,
        &maps.named(ws.key().tenant(), &all),
    ));
    Ok(Json(Overview {
        map: MapResponse {
            map: map.map.clone(),
            version: map.version.clone(),
            loaded_at: map.loaded_at,
        },
        issues: built.issues,
        findings,
        companions,
        unpinned_sessions: built.unpinned_sessions,
        backlog: built.backlog,
        layout,
        queue,
        draft,
        can_create_issues: app.github.can_create_issues_anywhere(),
        repos: built.repos,
        pulls: built.pulls,
        traffic: built.traffic,
        companion_silent_secs: app.config.limits.companion_silent_secs,
        comment_approvals_until: app.config.comment_approvals_until,
        workspace: ws.as_str().to_string(),
        // Only when there is a choice: one workspace shows no switcher, and its overview stays as
        // it was before workspaces (the golden test).
        workspaces: if all.len() > 1 { all } else { vec![] },
        // What each live launched agent last said (ADR 0071): shown with its age, never progress.
        agent_lines: crate::agent_reports::all(&app, ws.key().tenant())
            .await
            .map_err(|e| internal(SLICE, e))?,
        // A person's lead agents and their workers, only with the flag on (ADR 0072).
        orchestrator: app.config.orchestrator,
        leads: crate::fleet::leads(&app, ws.key()).await,
        // Each context graph source and how fresh it is; absent while the flag is off (ADR 0084).
        graph: crate::context_graph::overview(&app).await,
        generated_at: Utc::now(),
    }))
}
