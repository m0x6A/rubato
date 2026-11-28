//! A tenant's members, managed by its admins (ADR 0088, design §4.5, §4.6). Tenant routes: the
//! membership layer has run, so the caller is a member; the writes take an admin.

use super::people::{
    Refusal, failed, invite as invite_person, member_changed, refuse, remove_person,
};
use crate::kernel::state::App;
use crate::kernel::tenancy::{AdminScope, TenantScope};
use crate::tenants;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::tenant::{InviteRequest, MemberView, RoleChange, reason};

/// The `{github_id}` of a member route. Named, because the path also carries `{tenant}`.
#[derive(serde::Deserialize)]
pub struct MemberParam {
    github_id: i64,
}

/// `GET /api/t/{tenant}/members`: every member sees who is in the tenant.
pub async fn list(
    State(app): State<App>,
    scope: TenantScope,
) -> Result<Json<Vec<MemberView>>, Refusal> {
    tenants::members(&app.db, scope.tenant().as_str())
        .await
        .map(Json)
        .map_err(failed)
}

/// `POST /api/t/{tenant}/members`: an admin invites a GitHub login. The invite binds to the
/// account's id as GitHub gives it now, and the answer shows the account that was invited.
pub async fn invite(
    State(app): State<App>,
    AdminScope(scope): AdminScope,
    Json(req): Json<InviteRequest>,
) -> Result<(StatusCode, Json<MemberView>), Refusal> {
    let (github_id, _) = invite_person(
        &app,
        scope.tenant(),
        &req.login,
        req.role,
        scope.login(),
        false,
    )
    .await?;
    let member = tenants::members(&app.db, scope.tenant().as_str())
        .await
        .map_err(failed)?
        .into_iter()
        .find(|m| m.github_id as i64 == github_id)
        .ok_or_else(|| failed("the member just invited is not listed"))?;
    Ok((StatusCode::CREATED, Json(member)))
}

/// `PUT /api/t/{tenant}/members/{github_id}`: an admin changes a role. The last admin stays one.
pub async fn set_role(
    State(app): State<App>,
    AdminScope(scope): AdminScope,
    Path(p): Path<MemberParam>,
    Json(req): Json<RoleChange>,
) -> Result<StatusCode, Refusal> {
    let tenant = scope.tenant();
    let mut tx = app.db.begin().await.map_err(failed)?;
    let login = tenants::member_login(&mut tx, tenant.as_str(), p.github_id)
        .await
        .map_err(failed)?;
    tenants::set_role(&mut tx, tenant.as_str(), p.github_id, req.role)
        .await
        .map_err(failed)?
        .map_err(|r| refuse(r.0, None))?;
    tx.commit().await.map_err(failed)?;
    member_changed(
        tenant,
        "role_changed",
        login.as_deref().unwrap_or_default(),
        p.github_id,
        scope.login(),
    );
    app.notify_in(tenant, super::people::MEMBERS_CHANGED);
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/t/{tenant}/members/{github_id}`: an admin removes a member, themselves
/// included; never the last admin.
pub async fn remove(
    State(app): State<App>,
    AdminScope(scope): AdminScope,
    Path(p): Path<MemberParam>,
) -> Result<StatusCode, Refusal> {
    match remove_person(&app, scope.tenant(), p.github_id, scope.login())
        .await
        .map_err(failed)?
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(r) => {
            if r.0 == reason::LAST_ADMIN {
                app.reject(rubato_protocol::tenant::routes::MEMBER, r.0, scope.login());
            }
            Err(refuse(r.0, None))
        }
    }
}
