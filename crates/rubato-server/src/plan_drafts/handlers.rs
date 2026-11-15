//! `GET /api/drafts/{draft_id}` and `GET /api/drafts/{draft_id}/revisions/{rev}`. Everyone who
//! may log in reads drafts: they are part of the shared picture (ADR 0050, ADR 0053).

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::plan_draft::routes;
use rubato_protocol::{DraftRevision, DraftView};
use serde_json::{Value, json};
use uuid::Uuid;

type Refused = (StatusCode, Json<Value>);

fn internal(e: anyhow::Error) -> Refused {
    tracing::error!(slice = "plan_drafts", error = %format!("{e:#}"), "drafts failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "reason": "internal" })),
    )
}

fn not_found(app: &App, route: &str, user: &str) -> Refused {
    app.reject(route, "unknown_draft", user);
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "reason": "unknown_draft" })),
    )
}

pub async fn draft(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(draft_id): Path<Uuid>,
) -> Result<Json<DraftView>, Refused> {
    let Some(d) = store::get(&app.db, draft_id).await.map_err(internal)? else {
        return Err(not_found(&app, routes::DRAFT, &user));
    };
    let revisions = store::revisions(&app.db, draft_id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|r| r.info)
        .collect();
    Ok(Json(DraftView {
        draft_id,
        issue: d.issue,
        branch: d.branch,
        owner: d.owner,
        imported_from: d.imported_from,
        original_author: d.original_author,
        revisions,
    }))
}

pub async fn revision(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path((draft_id, rev)): Path<(Uuid, u32)>,
) -> Result<Json<DraftRevision>, Refused> {
    let Some(r) = store::revision_of(&app.db, draft_id, rev)
        .await
        .map_err(internal)?
    else {
        return Err(not_found(&app, routes::DRAFT_REVISION, &user));
    };
    Ok(Json(DraftRevision {
        draft_id,
        info: r.info,
        files: r.files,
    }))
}
