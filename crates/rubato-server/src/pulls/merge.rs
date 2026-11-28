//! `POST /api/pulls/merge` (map-scoped, behind `same_origin`): merge as the person, at the head
//! they saw, at most once per (repo, pull request, head) (ADR 0033, design 82 §4). Shaped like
//! `approve/handlers.rs`: the existing row is answered first, the facts are read fresh, any
//! refusal sends nothing, and GitHub's own refusal leaves nothing behind.

use super::facts::{Gathered, ViewCache, gather};
use super::store::{self, Intent};
use super::view::known_repo;
use crate::auth::{self, CurrentUser, TokenError};
use crate::kernel::github::MergeError;
use crate::kernel::http::Ws;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rubato_protocol::otel::span;
use rubato_protocol::pulls::{
    MergeAsk, MergeRequest, MergeResponse, MergeState, merge_refusal, reason, routes,
};
use serde_json::{Value, json};
use tracing::Instrument;

type Refused = (StatusCode, Json<Value>);
type Answer = (StatusCode, Json<MergeResponse>);

fn answer(i: &Intent) -> Answer {
    (
        match i.state {
            MergeState::Merged => StatusCode::OK,
            MergeState::Opening => StatusCode::ACCEPTED,
        },
        Json(MergeResponse {
            state: i.state,
            login: i.login.clone(),
            method: i.method,
            merge_sha: i.merge_sha.clone(),
        }),
    )
}

pub async fn merge(
    State(app): State<App>,
    ws: Ws,
    CurrentUser(login): CurrentUser,
    Json(req): Json<MergeRequest>,
) -> Result<Answer, Refused> {
    let t = ws.key().tenant();
    let refuse = |status: StatusCode, why: &str, extra: Value| {
        app.reject(routes::PULLS_MERGE, why, &login);
        let mut v = json!({ "reason": why });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        (status, Json(v))
    };
    let internal = |e: anyhow::Error| {
        tracing::error!(rubato.repo = %req.repo, rubato.pr = req.number, error = %format!("{e:#}"), "merge failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "reason": "internal" })),
        )
    };
    let Some(map) = app.map_of(ws.key()).await else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "no_map", json!({})));
    };
    let Some(repo) = known_repo(&app, t, &map, &req.repo)
        .await
        .map_err(internal)?
    else {
        return Err(refuse(
            StatusCode::NOT_FOUND,
            reason::UNKNOWN_REPO,
            json!({}),
        ));
    };
    // A retry, a double click or a second person on the same head gets the first press's row.
    if let Some(existing) = store::find(&app.db, t, &repo, req.number, &req.head_sha)
        .await
        .map_err(internal)?
    {
        return Ok(answer(&existing));
    }
    let token = if app.github.acts_without_token() {
        String::new()
    } else {
        match auth::access_token(&app, &login).await {
            Ok(t) => t,
            Err(TokenError::NotStored | TokenError::Rejected) => {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    reason::NEEDS_GITHUB_AUTHORIZATION,
                    json!({}),
                ));
            }
            Err(TokenError::Failed(e)) => {
                tracing::warn!(login, error = %format!("{e:#}"), "token unavailable");
                return Err(refuse(
                    StatusCode::BAD_GATEWAY,
                    reason::NEEDS_GITHUB_AUTHORIZATION,
                    json!({ "retry": true }),
                ));
            }
        }
    };
    // Fresh: the cache is bypassed and replaced (design 82 §2).
    let facts = match gather(&app, ws.key(), &map, &repo, req.number, &login, true).await {
        Ok(Gathered::Found(f)) => f,
        Ok(Gathered::NotFound) => {
            return Err(refuse(StatusCode::NOT_FOUND, "no_pull", json!({})));
        }
        Err(e) => {
            tracing::warn!(rubato.repo = %repo, rubato.pr = req.number, error = %format!("{e:#}"), "pull view failed");
            return Err(refuse(
                StatusCode::BAD_GATEWAY,
                reason::GITHUB_FAILED,
                json!({}),
            ));
        }
    };
    let ask = MergeAsk {
        head_sha: &req.head_sha,
        method: req.method,
    };
    if let Some(why) = merge_refusal(
        &facts.pull_facts(),
        facts.person.as_ref(),
        &facts.gate,
        Some(ask),
    ) {
        let gate = match &facts.gate {
            rubato_protocol::pulls::PlanGate::NotApproved { why } => why.as_str(),
            rubato_protocol::pulls::PlanGate::Approved { .. } => "approved",
            rubato_protocol::pulls::PlanGate::NoPlan => "no_plan",
        };
        tracing::info!(
            rubato.repo = %repo,
            rubato.pr = req.number,
            login,
            rubato.reason = why,
            gate,
            approved_sha = facts.approved_sha.as_deref().unwrap_or("-"),
            head_hash = facts.head_hash.as_deref().unwrap_or("-"),
            "merge refused"
        );
        return Err(refuse(StatusCode::CONFLICT, why, json!({})));
    }
    if !store::open(
        &app.db,
        t,
        &repo,
        req.number,
        &req.head_sha,
        &login,
        req.method,
    )
    .await
    .map_err(internal)?
    {
        // A concurrent request for this head inserted first; it merges.
        let existing = store::find(&app.db, t, &repo, req.number, &req.head_sha)
            .await
            .map_err(internal)?;
        return existing
            .map(|i| answer(&i))
            .ok_or_else(|| internal(anyhow::anyhow!("merge row vanished")));
    }
    let merged = async {
        app.github
            .as_person()
            .merge_as(&token, &repo, req.number, &req.head_sha, req.method)
            .await
    }
    .instrument(tracing::info_span!(
        span::PR_MERGE,
        rubato.repo = %repo,
        rubato.pr = req.number,
        rubato.outcome = tracing::field::Empty
    ))
    .await;
    let cache = app.slots.get::<ViewCache>();
    cache.evict(&repo, req.number);
    let mut intent = store::find(&app.db, t, &repo, req.number, &req.head_sha)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(anyhow::anyhow!("merge row vanished")))?;
    match merged {
        Ok(sha) => {
            let sha = (!sha.is_empty()).then_some(sha);
            store::merged(&app.db, t, &intent, sha.as_deref())
                .await
                .map_err(internal)?;
            tracing::info!(rubato.repo = %repo, rubato.pr = req.number, login, method = req.method.as_str(), rubato.outcome = "merged", "pull merged from Rubato");
            intent.state = MergeState::Merged;
            intent.merge_sha = sha;
            app.notify("merge");
            Ok(answer(&intent))
        }
        Err(MergeError::Refused { status, message }) => {
            store::forget(&app.db, t, &repo, req.number, &req.head_sha)
                .await
                .map_err(internal)?;
            tracing::warn!(rubato.repo = %repo, rubato.pr = req.number, login, status, message, rubato.outcome = "refused", "GitHub refused the merge");
            Err(refuse(
                StatusCode::BAD_GATEWAY,
                reason::GITHUB_REFUSED,
                json!({ "status": status, "message": message }),
            ))
        }
        Err(MergeError::Unknown(e)) => {
            tracing::warn!(rubato.repo = %repo, rubato.pr = req.number, login, error = %e, rubato.outcome = "opening", "merge sent, not confirmed");
            Ok(answer(&intent))
        }
    }
}
