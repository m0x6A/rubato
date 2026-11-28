//! `GET /api/launches/{launch_id}/log`. Everyone who may log in reads a launch's log: the
//! conversation is part of the shared picture (ADR 0050, ADR 0052).

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::launch;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::LaunchLog;
use rubato_protocol::session_log::routes;
use serde_json::{Value, json};
use uuid::Uuid;

/// The `{launch_id}` of a tenant route. Named, because the path also carries `{tenant}` (ADR 0089).
#[derive(serde::Deserialize)]
pub struct LaunchParam {
    launch_id: Uuid,
}

impl From<Uuid> for LaunchParam {
    fn from(launch_id: Uuid) -> Self {
        Self { launch_id }
    }
}

pub async fn log(
    State(app): State<App>,
    scope: crate::kernel::tenancy::TenantScope,
    CurrentUser(user): CurrentUser,
    Path(LaunchParam { launch_id }): Path<LaunchParam>,
) -> Result<Json<LaunchLog>, (StatusCode, Json<Value>)> {
    let t = scope.tenant();
    let internal = |e: anyhow::Error| {
        tracing::error!(slice = "session_log", error = %format!("{e:#}"), "session log failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "reason": "internal" })),
        )
    };
    // A launch of another tenant is unknown here, like one that does not exist (ADR 0089).
    if launch::launch_target(&app.db, launch_id)
        .await
        .map_err(internal)?
        .is_none_or(|l| app.github.tenant_of(&l.issue.repo).as_ref() != Some(t))
    {
        app.reject(routes::LAUNCH_LOG, "unknown_launch", &user);
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "reason": "unknown_launch" })),
        ));
    }
    let entries = store::for_launch(&app.db, t, launch_id)
        .await
        .map_err(internal)?;
    Ok(Json(LaunchLog { launch_id, entries }))
}
