//! `POST /api/issues`: create an issue as the GitHub App, pinned where it was started
//! (ADR 0025). At most one issue per request id; see the concurrency table in
//! `docs/plans/71-flow-view-and-issues/design.md`.

use super::store::{self, Creation};
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::pins;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::issue_new::{problems, routes};
use rubato_protocol::otel::attr;
use rubato_protocol::{CreationState, NewIssueRequest, NewIssueResponse, render_issue};

const SLICE: &str = "new_issue";

/// The reason values of a refused request, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const INVALID: &str = "invalid";
    pub const UNKNOWN_TARGET: &str = "unknown_target";
    pub const UNKNOWN_REPO: &str = "unknown_repo";
    pub const CANNOT_WRITE: &str = "cannot_write";
    pub const NO_ISSUE_PERMISSION: &str = "no_issue_permission";
    pub const FAILED: &str = "failed";
}

type Refused = (StatusCode, String);
type Answer = (StatusCode, Json<NewIssueResponse>);

fn answer(c: &Creation) -> Answer {
    let (status, message) = match c.state {
        CreationState::Created => (StatusCode::OK, None),
        CreationState::Opening => (
            StatusCode::ACCEPTED,
            Some("Still opening: GitHub has not confirmed this issue yet. It will appear, pinned, once the listing shows it.".to_owned()),
        ),
        CreationState::Failed => (
            StatusCode::CONFLICT,
            Some(format!(
                "This request failed: {}. Start a new issue to try again.",
                c.error.as_deref().unwrap_or("unknown")
            )),
        ),
    };
    (
        status,
        Json(NewIssueResponse {
            state: c.state,
            issue: c.issue(),
            url: c.url.clone(),
            message,
        }),
    )
}

pub(super) fn count(app: &App, outcome: &'static str) {
    app.metrics
        .issues_created
        .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
}

pub async fn create(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
    Json(req): Json<NewIssueRequest>,
) -> Result<Answer, Refused> {
    let refuse = |reason: &str, status: StatusCode, msg: String| {
        app.reject(routes::ISSUES, reason, &user);
        (status, msg)
    };
    let missing = problems(&req);
    if !missing.is_empty() {
        return Err(refuse(
            reason::INVALID,
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("The issue needs {}.", missing.join(", ")),
        ));
    }
    let map = app
        .map_of(ws.as_str())
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".to_owned()))?;
    if let Err((_, msg)) = pins::check_target(&map.map, &map.version, &req.target) {
        return Err(refuse(
            reason::UNKNOWN_TARGET,
            StatusCode::UNPROCESSABLE_ENTITY,
            msg,
        ));
    }
    if !map.map.repos_named().contains(req.repo.as_str()) {
        return Err(refuse(
            reason::UNKNOWN_REPO,
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{} is not a repo the map names", req.repo),
        ));
    }
    if !app.github.can_write() {
        return Err(refuse(
            reason::CANNOT_WRITE,
            StatusCode::CONFLICT,
            "This Rubato cannot write to GitHub: it runs on a personal token or none, and only a GitHub App installation can open issues.".into(),
        ));
    }
    if !app.github.can_create_issues() {
        return Err(refuse(
            reason::NO_ISSUE_PERMISSION,
            StatusCode::CONFLICT,
            "The GitHub App's installation does not have Issues: write. An admin approves the new permission on the installation; Rubato notices within five minutes.".into(),
        ));
    }

    // At most once per request id: a second submit answers with what the first got.
    if !store::begin(
        &app.db,
        ws.as_str(),
        &req.request_id,
        &req.repo,
        &req.target,
        &req.title,
        &user,
        Utc::now(),
    )
    .await
    .map_err(|e| (internal(SLICE, e), String::new()))?
    {
        let c = store::get(&app.db, &req.request_id)
            .await
            .map_err(|e| (internal(SLICE, e), String::new()))?
            .ok_or((StatusCode::CONFLICT, "request id in use".to_owned()))?;
        return Ok(answer(&c));
    }

    let body = render_issue(&req, &user);
    match app
        .github
        .create_issue(&req.repo, req.title.trim(), &body)
        .await
    {
        Ok((number, url)) => {
            let issue = rubato_protocol::IssueRef::new(req.repo.clone(), number);
            store::created(&app.db, &req.request_id, number, &url)
                .await
                .map_err(|e| (internal(SLICE, e), String::new()))?;
            pins::pin_created(&app, ws.as_str(), &issue, &req.target, &user)
                .await
                .map_err(|e| (internal(SLICE, e), String::new()))?;
            count(&app, "created");
            tracing::info!(
                rubato.issue = %issue,
                rubato.user = %user,
                request_id = %req.request_id,
                target = req.target.id(),
                "issue created"
            );
            app.notify("issue");
            Ok((
                StatusCode::CREATED,
                Json(NewIssueResponse {
                    state: CreationState::Created,
                    issue: Some(issue),
                    url: Some(url),
                    message: None,
                }),
            ))
        }
        Err(e) => {
            let refused = !app.github.can_create_issues();
            tracing::warn!(
                step = "create",
                rubato.repo = %req.repo,
                request_id = %req.request_id,
                refused,
                error = %format!("{e:#}"),
                "issue creation failed"
            );
            if refused {
                // GitHub said no (403): nothing was created, so the row fails now.
                store::failed(&app.db, &req.request_id, &format!("{e:#}"))
                    .await
                    .map_err(|e| (internal(SLICE, e), String::new()))?;
                count(&app, "failed");
                app.reject(routes::ISSUES, reason::FAILED, &user);
            }
            // Otherwise it may exist: the row stays opening for the listing to find.
            let c = store::get(&app.db, &req.request_id)
                .await
                .map_err(|e| (internal(SLICE, e), String::new()))?
                .ok_or((StatusCode::INTERNAL_SERVER_ERROR, String::new()))?;
            let (status, Json(mut r)) = answer(&c);
            if !refused {
                r.message = Some(format!(
                    "GitHub did not confirm the issue ({e}). It may exist: Rubato looks for it in the next listing and pins it there."
                ));
            }
            Ok((
                if refused {
                    StatusCode::BAD_GATEWAY
                } else {
                    status
                },
                Json(r),
            ))
        }
    }
}
