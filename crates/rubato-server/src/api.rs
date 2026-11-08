//! REST handlers on the `web-server` seam. Types come from `rubato_protocol::web`.

use crate::auth::CurrentUser;
use crate::db;
use crate::poller;
use crate::progress::{self, OverviewInputs};
use crate::state::App;
use anyhow::Result;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::web::{LaunchRequest, LaunchResponse, MapResponse, Me, Overview, PinRequest};
use rubato_protocol::{IssuePin, LaunchCommand, ServerMessage, otel::attr};
use std::collections::BTreeMap;
use uuid::Uuid;

/// The prompt the launched session receives. Shipped with the skill; a copy is embedded.
pub const PROMPT_TEMPLATE: &str = include_str!("../../../skills/rubato/prompt.md");

fn internal(e: impl std::fmt::Display) -> StatusCode {
    tracing::error!(error = %e, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
}

pub async fn me(CurrentUser(login): CurrentUser) -> Json<Me> {
    Json(Me {
        login,
        avatar_url: None,
    })
}

pub async fn health() -> &'static str {
    "ok"
}

pub async fn build_overview_inputs(app: &App) -> Result<OverviewInputs> {
    Ok(OverviewInputs {
        pins: db::list_pins(&app.db).await?,
        sessions: db::all_sessions(&app.db).await?,
        plans: db::all_plans(&app.db).await?,
        prs: db::all_prs(&app.db).await?,
        decisions: db::all_decisions(&app.db).await?,
        launched: db::launched_issues(&app.db).await?,
        cap_hits: db::issues_with_live_cap_hit(&app.db).await?,
        meta: db::all_issue_meta(&app.db).await?,
        launch_map_versions: db::launch_map_versions(&app.db).await?,
    })
}

pub async fn overview(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Json<Overview>, StatusCode> {
    let map = app
        .map
        .read()
        .await
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let inputs = build_overview_inputs(&app).await.map_err(internal)?;
    let built = progress::build(&inputs, Some(&map), &app.config.limits, Utc::now());
    let companions = db::list_pairings(&app.db, None)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|r| crate::pairing::to_info(&app, r))
        .collect();
    Ok(Json(Overview {
        map: MapResponse {
            map: map.map,
            version: map.version,
            loaded_at: map.loaded_at,
        },
        issues: built.issues,
        findings: built.findings,
        companions,
        unpinned_sessions: built.unpinned_sessions,
        generated_at: Utc::now(),
    }))
}

pub async fn map(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Json<MapResponse>, StatusCode> {
    let m = app
        .map
        .read()
        .await
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(MapResponse {
        map: m.map,
        version: m.version,
        loaded_at: m.loaded_at,
    }))
}

pub async fn map_reload(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Json<MapResponse>, StatusCode> {
    let m = poller::reload_map(&app).await.map_err(|e| {
        tracing::error!(error = %e, "map reload");
        StatusCode::BAD_GATEWAY
    })?;
    app.notify("map");
    Ok(Json(MapResponse {
        map: m.map,
        version: m.version,
        loaded_at: m.loaded_at,
    }))
}

pub async fn add_pin(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let map = app
        .map
        .read()
        .await
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    if !map.map.ids().contains(req.target.id()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{} is not an id in map {}", req.target.id(), map.version),
        ));
    }
    db::add_pin(
        &app.db,
        &IssuePin {
            issue: req.issue,
            target: req.target,
            pinned_by: user,
            pinned_at: Utc::now(),
        },
    )
    .await
    .map_err(|e| (internal(e), String::new()))?;
    app.notify("pin");
    Ok(StatusCode::CREATED)
}

pub async fn remove_pin(
    State(app): State<App>,
    _user: CurrentUser,
    Json(req): Json<PinRequest>,
) -> Result<StatusCode, StatusCode> {
    let removed = db::remove_pin(&app.db, &req.issue, &req.target)
        .await
        .map_err(internal)?;
    app.notify("pin");
    Ok(if removed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    })
}

/// Slug from a title: kebab-case, at most four words.
pub fn slugify(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| {
            !matches!(
                w.as_str(),
                "a" | "an" | "the" | "and" | "of" | "to" | "for" | "task"
            )
        })
        .take(4)
        .collect();
    if words.is_empty() {
        "work".into()
    } else {
        words.join("-")
    }
}

pub fn render_prompt(template: &str, vars: &BTreeMap<&str, String>) -> String {
    let mut out = template.to_owned();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

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
    let mine: Vec<db::PairingRow> = db::list_pairings(&app.db, Some(&user))
        .await
        .map_err(|e| (internal(e), String::new()))?
        .into_iter()
        .filter(|p| p.revoked_at.is_none())
        .collect();
    let connected = app.connected_companions();
    let candidates: Vec<&db::PairingRow> = mine
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
    let pins: Vec<String> = db::list_pins(&app.db)
        .await
        .map_err(|e| (internal(e), String::new()))?
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
    let meta = db::all_issue_meta(&app.db)
        .await
        .map_err(|e| (internal(e), String::new()))?
        .remove(&req.issue)
        .unwrap_or_default();
    let plan = db::all_plans(&app.db)
        .await
        .map_err(|e| (internal(e), String::new()))?
        .into_iter()
        .find(|p| p.issue == req.issue)
        .and_then(|p| p.plan);
    let slug = req
        .slug
        .clone()
        .unwrap_or_else(|| slugify(meta.title.as_deref().unwrap_or("")));
    let branch = format!("issue/{}-{}", req.issue.number, slug);
    let attempt = db::launch_count(&app.db, &req.issue)
        .await
        .map_err(|e| (internal(e), String::new()))?
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
    db::store_launch(&app.db, companion.companion_id, &command)
        .await
        .map_err(|e| (internal(e), String::new()))?;
    if !app.send_to_companion(
        companion.companion_id,
        ServerMessage::Launch(command.clone()),
    ) {
        return Err((
            StatusCode::CONFLICT,
            "companion disconnected while launching".into(),
        ));
    }
    tracing::info!(launch_id = %command.id, rubato.issue = %command.issue, rubato.user = %user, companion = %companion.companion_id, "{}", rubato_protocol::otel::span::LAUNCH);
    app.notify("launch");
    Ok(Json(LaunchResponse {
        command,
        companion_id: companion.companion_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slugify("Task 1: rubato-protocol — types, serde, JSON Schema export"),
            "1-rubato-protocol-types"
        );
        assert_eq!(slugify("Add the map view"), "add-map-view");
        assert_eq!(slugify(""), "work");
    }

    #[test]
    fn prompt_renders_placeholders() {
        let mut v = BTreeMap::new();
        v.insert("issue", "o/r#1".to_string());
        assert_eq!(
            render_prompt("on {issue} and {issue}", &v),
            "on o/r#1 and o/r#1"
        );
        assert!(PROMPT_TEMPLATE.contains("{plan_path}"));
    }
}
