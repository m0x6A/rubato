//! `POST /api/launch`: start a session on an issue through one of the caller's own companions.

use super::prompt::{PROMPT_TEMPLATE, render_prompt, slugify};
use super::store;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::{github_poll, pairing, pins, plans};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::web::{LaunchRequest, LaunchResponse};
use rubato_protocol::{LaunchCommand, ServerMessage};
use std::collections::BTreeMap;
use uuid::Uuid;

const SLICE: &str = "launch";

/// Start a session on an issue through one of the caller's own companions (ADR 0004).
pub async fn launch(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, (StatusCode, String)> {
    let map = app
        .map
        .read()
        .await
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    let mine: Vec<pairing::PairingRow> = pairing::list_pairings(&app.db, Some(&user))
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .filter(|p| p.revoked_at.is_none())
        .collect();
    let connected = app.connected_companions();
    let candidates: Vec<&pairing::PairingRow> = mine
        .iter()
        .filter(|p| connected.contains(&p.companion_id))
        .collect();
    let companion = match req.companion_id {
        Some(id) => candidates
            .iter()
            .find(|p| p.companion_id == id)
            .copied()
            .ok_or((
                StatusCode::FORBIDDEN,
                format!("companion {id} is not one of your connected companions"),
            ))?,
        None => match candidates.as_slice() {
            [one] => *one,
            [] => {
                return Err((
                    StatusCode::CONFLICT,
                    "none of your companions is connected".into(),
                ));
            }
            _ => {
                return Err((
                    StatusCode::CONFLICT,
                    "several companions connected; pass companion_id".into(),
                ));
            }
        },
    };
    let pins: Vec<String> = pins::list(&app.db)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .filter(|p| p.issue == req.issue)
        .map(|p| p.target.id().to_owned())
        .collect();
    if pins.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "pin the issue to the map first".into(),
        ));
    }
    let meta = github_poll::issue_meta(&app.db, &req.issue)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .unwrap_or_default();
    let plan = plans::all(&app.db)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .find(|p| p.issue == req.issue)
        .and_then(|p| p.plan);
    let slug = req
        .slug
        .clone()
        .unwrap_or_else(|| slugify(meta.title.as_deref().unwrap_or("")));
    let branch = format!("issue/{}-{}", req.issue.number, slug);
    let attempt = store::launch_count(&app.db, &req.issue)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        + 1;
    let spend_cap = req
        .spend_cap_usd
        .or(plan.as_ref().and_then(|p| p.spend_cap_usd))
        .unwrap_or(app.config.default_spend_cap_usd);
    let attempt_limit = req
        .attempt_limit
        .or(plan.as_ref().and_then(|p| p.attempt_limit))
        .unwrap_or(app.config.default_attempt_limit);
    if attempt > attempt_limit {
        return Err((
            StatusCode::CONFLICT,
            format!("attempt limit {attempt_limit} reached for {}", req.issue),
        ));
    }
    let plan_path = map
        .map
        .plan_path_for(&req.issue.repo, req.issue.number, &slug);
    let mut otel = BTreeMap::new();
    otel.insert(attr::REPO.to_owned(), req.issue.repo.clone());
    otel.insert(attr::ISSUE.to_owned(), req.issue.to_string());
    otel.insert(attr::BRANCH.to_owned(), branch.clone());
    otel.insert(attr::NODE.to_owned(), pins.join("+"));
    let vars: BTreeMap<&str, String> = [
        ("issue", req.issue.to_string()),
        ("title", meta.title.clone().unwrap_or_default()),
        ("worktree", "(created by the companion)".into()),
        ("branch", branch.clone()),
        ("nodes", pins.join(", ")),
        ("map_version", map.version.clone()),
        ("plan_path", plan_path),
        ("spend_cap_usd", format!("{spend_cap}")),
        ("attempt", attempt.to_string()),
        ("attempt_limit", attempt_limit.to_string()),
    ]
    .into_iter()
    .collect();
    let command = LaunchCommand {
        id: Uuid::new_v4(),
        user: user.clone(),
        issue: req.issue.clone(),
        repo_url: format!("https://github.com/{}.git", req.issue.repo),
        branch,
        base_ref: "origin/main".into(),
        node_ids: pins,
        spend_cap_usd: spend_cap,
        attempt_limit,
        attempt,
        session_id: Uuid::new_v4(),
        prompt: req
            .prompt
            .clone()
            .unwrap_or_else(|| render_prompt(PROMPT_TEMPLATE, &vars)),
        otel_resource: otel,
        map_version: Some(map.version.clone()),
    };
    store::store_launch(&app.db, companion.companion_id, &command)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    if !app.send_to_companion(
        companion.companion_id,
        ServerMessage::Launch(command.clone()),
    ) {
        return Err((
            StatusCode::CONFLICT,
            "companion disconnected while launching".into(),
        ));
    }
    tracing::info!(launch_id = %command.id, rubato.issue = %command.issue, rubato.user = %user, companion = %companion.companion_id, "{}", span::LAUNCH);
    app.notify("launch");
    Ok(Json(LaunchResponse {
        command,
        companion_id: companion.companion_id,
    }))
}
