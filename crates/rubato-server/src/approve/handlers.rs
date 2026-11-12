//! `POST /api/plans/approve`: post `Approved: plan <sha> on <branch>` as the person (ADR 0030).
//! Refused, with nothing posted, when the plan changed, does not fit (ADR 0029) or Rubato
//! holds no token for the person. Once per (person, issue, sha): a double click, a retry or a
//! second tab finds the first request's row (design 80 §4).

use super::store::{self, Intent};
use crate::auth::{self, CurrentUser, TokenError};
use crate::kernel::github::PostError;
use crate::kernel::state::App;
use crate::plans;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rubato_protocol::approve::{reason, routes};
use rubato_protocol::otel::span;
use rubato_protocol::{ApproveRequest, ApproveResponse, ApproveState, plan_fit};
use serde_json::{Value, json};
use tracing::Instrument;

type Refused = (StatusCode, Json<Value>);
type Answer = (StatusCode, Json<ApproveResponse>);

pub fn body(sha: &str, branch: &str) -> String {
    format!("Approved: plan {sha} on {branch}\n\nPosted from Rubato.")
}

fn answer(i: &Intent) -> Answer {
    (
        match i.state {
            ApproveState::Posted => StatusCode::OK,
            ApproveState::Opening => StatusCode::ACCEPTED,
        },
        Json(ApproveResponse {
            state: i.state,
            body: body(&i.sha, &i.branch),
            comment_url: i.comment_url.clone(),
        }),
    )
}

pub async fn approve(
    State(app): State<App>,
    CurrentUser(login): CurrentUser,
    Json(req): Json<ApproveRequest>,
) -> Result<Answer, Refused> {
    let refuse = |status: StatusCode, why: &str, extra: Value| {
        app.reject(routes::PLAN_APPROVE, why, &login);
        let mut v = json!({ "reason": why });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        (status, Json(v))
    };
    let internal = |e: anyhow::Error| {
        tracing::error!(error = %format!("{e:#}"), "approve failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "reason": "internal" })),
        )
    };
    let Some(map) = app.map.read().await.clone() else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "no_map", json!({})));
    };
    let issue = req.issue.clone();
    let Some(row) = plans::all(&app.db)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|p| p.issue == issue)
    else {
        return Err(refuse(StatusCode::CONFLICT, reason::NO_PLAN, json!({})));
    };
    if row.approval_sha.as_deref() != Some(req.sha.as_str()) {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::PLAN_CHANGED,
            json!({ "current_sha": row.approval_sha }),
        ));
    }
    let fit = plan_fit(row.plan.as_ref(), row.parse_error.as_deref(), &map.map);
    if let Some(f) = fit.filter(|f| !f.fits) {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::PLAN_DOES_NOT_FIT,
            json!({ "reasons": f.reasons }),
        ));
    }
    if let Some(existing) = store::find(&app.db, &login, &issue, &req.sha)
        .await
        .map_err(internal)?
    {
        return Ok(answer(&existing));
    }
    let token = match auth::access_token(&app, &login).await {
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
    };
    if !store::open(&app.db, &login, &issue, &req.sha, &row.branch)
        .await
        .map_err(internal)?
    {
        // A concurrent request for the same intent inserted first; it posts.
        let existing = store::find(&app.db, &login, &issue, &req.sha)
            .await
            .map_err(internal)?;
        return existing
            .map(|i| answer(&i))
            .ok_or_else(|| internal(anyhow::anyhow!("approval row vanished")));
    }
    let text = body(&req.sha, &row.branch);
    let posted = async {
        app.github
            .comment_as(&token, &issue.repo, issue.number, &text)
            .await
    }
    .instrument(tracing::info_span!(
        span::PLAN_APPROVE,
        rubato.issue = %issue,
        rubato.outcome = tracing::field::Empty
    ))
    .await;
    let mut intent = store::find(&app.db, &login, &issue, &req.sha)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(anyhow::anyhow!("approval row vanished")))?;
    match posted {
        Ok(c) => {
            store::posted(&app.db, &intent, c.id, &c.url)
                .await
                .map_err(internal)?;
            tracing::info!(rubato.issue = %issue, login, rubato.outcome = "posted", "plan approved from Rubato");
            intent.state = ApproveState::Posted;
            intent.comment_url = Some(c.url);
            app.notify("approve");
            Ok(answer(&intent))
        }
        Err(PostError::Refused { status }) => {
            store::forget(&app.db, &login, &issue, &req.sha)
                .await
                .map_err(internal)?;
            tracing::warn!(rubato.issue = %issue, login, status, rubato.outcome = "refused", "GitHub refused the approval");
            Err(refuse(
                StatusCode::BAD_GATEWAY,
                reason::GITHUB_REFUSED,
                json!({ "status": status }),
            ))
        }
        Err(PostError::Unknown(e)) => {
            tracing::warn!(rubato.issue = %issue, login, error = %e, rubato.outcome = "opening", "approval sent, not confirmed");
            Ok(answer(&intent))
        }
    }
}
