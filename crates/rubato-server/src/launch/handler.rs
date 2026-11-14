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
use chrono::Utc;
use rubato_protocol::otel::{attr, event, span};
use rubato_protocol::{
    CapHit, CapKind, IssueRef, LaunchCommand, LaunchRequest, LaunchResponse, ServerMessage,
};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use uuid::Uuid;

const SLICE: &str = "launch";

/// A launch past the attempt limit is an event, not only a refusal: log `rubato.cap.hit` before
/// the write, record the cap hit, and answer 409 whether or not the write worked.
async fn refuse_past_attempt_limit(
    db: &SqlitePool,
    issue: &IssueRef,
    attempt: u32,
    attempt_limit: u32,
) -> (StatusCode, String) {
    let hit = CapHit {
        issue: issue.clone(),
        session_id: None,
        kind: CapKind::Attempts,
        value: f64::from(attempt),
        limit: f64::from(attempt_limit),
        at: Utc::now(),
    };
    tracing::warn!(rubato.issue = %hit.issue, kind = ?hit.kind, value = hit.value, limit = hit.limit, "{}", event::CAP_HIT);
    if let Err(e) = store::store_cap_hit(db, &hit, hit.at).await {
        tracing::error!(slice = SLICE, rubato.issue = %hit.issue, error = %e, "storing cap hit");
    }
    (
        StatusCode::CONFLICT,
        format!("attempt limit {attempt_limit} reached for {issue}"),
    )
}

/// Start a session on an issue through one of the caller's own companions (ADR 0004).
pub async fn launch(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, (StatusCode, String)> {
    let map = app
        .default_map()
        .await
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
    let pins: Vec<String> = pins::list(&app.db, rubato_protocol::DEFAULT_WORKSPACE)
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
        let refusal = refuse_past_attempt_limit(&app.db, &req.issue, attempt, attempt_limit).await;
        app.notify("cap hit");
        return Err(refusal);
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
    store::store_launch(
        &app.db,
        rubato_protocol::DEFAULT_WORKSPACE,
        companion.companion_id,
        &command,
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    #[tokio::test]
    async fn a_failed_cap_hit_write_is_logged_and_still_answers_409() {
        let (events, _guard) = capture::start();
        // A database with no tables: the insert into `cap_hits` fails.
        let db = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let issue = IssueRef::new("m0x6A/rubato", 30);

        let (status, body) = refuse_past_attempt_limit(&db, &issue, 4, 3).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body, "attempt limit 3 reached for m0x6A/rubato#30");
        let events = events.lock().unwrap();
        let hit = events
            .iter()
            .position(|e| e.message == event::CAP_HIT)
            .expect("the rubato.cap.hit event");
        assert_eq!(events[hit].level, tracing::Level::WARN);
        assert_eq!(
            events[hit].fields.get("rubato.issue").map(String::as_str),
            Some("m0x6A/rubato#30")
        );
        assert_eq!(
            events[hit].fields.get("kind").map(String::as_str),
            Some("Attempts")
        );
        let failed = events
            .iter()
            .position(|e| e.message == "storing cap hit")
            .expect("the storing cap hit line");
        assert!(hit < failed, "the event is logged before the write");
        let e = &events[failed];
        assert_eq!(e.level, tracing::Level::ERROR);
        assert_eq!(e.fields.get("slice").map(String::as_str), Some("launch"));
        assert_eq!(
            e.fields.get("rubato.issue").map(String::as_str),
            Some("m0x6A/rubato#30")
        );
        assert!(e.fields.contains_key("error"));
    }
}
