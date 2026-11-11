//! The draft's write routes: add an operation, remove one, discard, restore.

use super::store::{self, Appended, Changed};
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::pins;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::map_draft::{DRAFT_MAX_OPS, DraftOpRequest, MapOp, routes};
use rubato_protocol::{MapRef, SystemMap};

pub(super) const SLICE: &str = "map_draft";

/// The reason values of a refused draft write, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const DRAFT_FROZEN: &str = "draft_frozen";
    pub const PINNED: &str = "pinned";
    pub const DRAFT_FULL: &str = "draft_full";
    pub const INVALID_OP: &str = "invalid_op";
    pub const NOTHING_TO_RESTORE: &str = "nothing_to_restore";
}

pub(super) type Refusal = (StatusCode, String);

pub(super) const FROZEN: &str =
    "The draft is being proposed and takes no changes. Withdraw the proposal to edit it again.";

pub(super) async fn loaded_map(app: &App) -> Result<SystemMap, Refusal> {
    app.map
        .read()
        .await
        .clone()
        .map(|m| m.map)
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))
}

pub(super) fn fault(e: anyhow::Error) -> Refusal {
    (internal(SLICE, e), String::new())
}

pub async fn add_op(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<DraftOpRequest>,
) -> Result<StatusCode, Refusal> {
    let map = loaded_map(&app).await?;
    // Removing what issues are pinned to would leave those pins pointing at nothing.
    let removed = match &req.op {
        MapOp::RemoveNode { id } => Some(MapRef::node(id.clone())),
        MapOp::RemoveSeam { id } => Some(MapRef::seam(id.clone())),
        _ => None,
    };
    if let Some(target) = removed {
        let pinned: Vec<String> = pins::list(&app.db)
            .await
            .map_err(fault)?
            .into_iter()
            .filter(|p| p.target == target)
            .map(|p| p.issue.to_string())
            .collect();
        if !pinned.is_empty() {
            app.reject(routes::DRAFT_OPS, reason::PINNED, &user);
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "{} is pinned to {}: unpin or move those first",
                    pinned.join(", "),
                    target.id()
                ),
            ));
        }
    }
    let outcome = store::append(&app.db, &map, &req, &user, Utc::now(), DRAFT_MAX_OPS)
        .await
        .map_err(fault)?;
    let refuse = |why: &'static str, status: StatusCode, text: String| {
        app.reject(routes::DRAFT_OPS, why, &user);
        Err((status, text))
    };
    match outcome {
        Appended::Yes(_) => {
            app.notify("draft");
            Ok(StatusCode::CREATED)
        }
        Appended::Duplicate => Ok(StatusCode::CREATED),
        Appended::Frozen => refuse(reason::DRAFT_FROZEN, StatusCode::CONFLICT, FROZEN.into()),
        Appended::Full => refuse(
            reason::DRAFT_FULL,
            StatusCode::CONFLICT,
            format!(
                "The draft already holds {DRAFT_MAX_OPS} changes; propose it or remove some first."
            ),
        ),
        Appended::Invalid(why) => refuse(reason::INVALID_OP, StatusCode::UNPROCESSABLE_ENTITY, why),
    }
}

pub async fn remove_op(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(seq): Path<u64>,
) -> Result<StatusCode, Refusal> {
    match store::remove_op(&app.db, seq).await.map_err(fault)? {
        Changed::Yes(_) => {
            app.notify("draft");
            Ok(StatusCode::NO_CONTENT)
        }
        Changed::Nothing => Err((
            StatusCode::NOT_FOUND,
            format!("the draft has no change {seq}"),
        )),
        Changed::Frozen => {
            app.reject(routes::DRAFT_OPS, reason::DRAFT_FROZEN, &user);
            Err((StatusCode::CONFLICT, FROZEN.into()))
        }
    }
}

pub async fn discard(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<StatusCode, Refusal> {
    match store::discard(&app.db, Utc::now()).await.map_err(fault)? {
        Changed::Yes(n) => {
            // It empties the draft for the whole team, so it is said with who did it.
            tracing::info!(rubato.user = %user, operations = n, "map draft discarded");
            app.notify("draft");
            Ok(StatusCode::OK)
        }
        Changed::Nothing => Ok(StatusCode::OK),
        Changed::Frozen => {
            app.reject(routes::DRAFT_DISCARD, reason::DRAFT_FROZEN, &user);
            Err((StatusCode::CONFLICT, FROZEN.into()))
        }
    }
}

pub async fn restore(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<StatusCode, Refusal> {
    match store::restore(&app.db).await.map_err(fault)? {
        Changed::Yes(n) => {
            tracing::info!(rubato.user = %user, operations = n, "map draft restored");
            app.notify("draft");
            Ok(StatusCode::OK)
        }
        Changed::Nothing | Changed::Frozen => {
            app.reject(routes::DRAFT_RESTORE, reason::NOTHING_TO_RESTORE, &user);
            Err((
                StatusCode::CONFLICT,
                "There is no discarded draft to bring back: a new change was made since, or nothing was discarded.".into(),
            ))
        }
    }
}
