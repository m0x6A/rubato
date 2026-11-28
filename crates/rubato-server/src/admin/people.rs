//! People in tenants (ADR 0088, design §4.5, §4.6): who a login is, an invite, a role, and a
//! removal with everything that must end with it. Shared by the members routes (a tenant's
//! admins) and the operator routes.

use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use crate::tenants::{self, Refused, Removed};
use anyhow::Result;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rubato_protocol::tenant::{Role, TenancyRefusal, reason};

pub(super) const SLICE: &str = "admin";

/// A refused or failed request of this slice: the response to send, boxed so a handler's
/// `Result` stays small.
pub struct Refusal(Box<Response>);

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        *self.0
    }
}

/// The answer for a refusal: its reason as JSON, under the status the reason calls for.
pub(super) fn refuse(why: &str, detail: Option<String>) -> Refusal {
    let status = match why {
        reason::NOT_A_MEMBER
        | reason::UNKNOWN_TENANT
        | reason::UNKNOWN_INSTALLATION
        | reason::NOT_OPERATOR => StatusCode::NOT_FOUND,
        reason::BAD_ID | reason::UNKNOWN_LOGIN => StatusCode::UNPROCESSABLE_ENTITY,
        reason::LOOKUP_FAILED => StatusCode::BAD_GATEWAY,
        _ => StatusCode::CONFLICT,
    };
    Refusal(Box::new(
        (
            status,
            Json(TenancyRefusal {
                reason: why.to_owned(),
                detail,
            }),
        )
            .into_response(),
    ))
}

pub(super) fn failed(e: impl std::fmt::Display) -> Refusal {
    Refusal(Box::new(internal(SLICE, e).into_response()))
}

/// `rubato.tenant.member_changed`: one line per change to who is in a tenant.
pub(super) fn member_changed(
    tenant: &TenantId,
    action: &str,
    login: &str,
    github_id: i64,
    by: &str,
) {
    tracing::info!(
        rubato.tenant = %tenant,
        action,
        login,
        github_id,
        by,
        "rubato.tenant.member_changed"
    );
}

/// Who a login is on GitHub: its user id and the login as GitHub spells it (ADR 0088: an invite
/// binds to the id). Asked through an installation of `tenant` when it has one, else through
/// any the server has, else without a credential. When GitHub cannot be asked at all, a person
/// who has logged in here before is known from that login; anyone else is refused, never
/// guessed.
pub(super) async fn who_is(
    app: &App,
    tenant: Option<&TenantId>,
    login: &str,
) -> std::result::Result<(i64, String), Refusal> {
    let login = login.trim().trim_start_matches('@');
    let live = app.github.live();
    let mut clients: Vec<_> = live
        .iter()
        .filter(|l| Some(&l.tenant) == tenant)
        .chain(live.iter().filter(|l| Some(&l.tenant) != tenant))
        .map(|l| l.client.clone())
        .collect();
    clients.push(app.github.as_person());
    let mut last = None;
    for client in clients {
        match client.user(login).await {
            Ok(Some(found)) => return Ok(found),
            Ok(None) => return Err(refuse(reason::UNKNOWN_LOGIN, None)),
            Err(e) => last = Some(format!("{e:#}")),
        }
    }
    match crate::auth::github_id_of(&app.db, login).await {
        Ok(Some(id)) => Ok((id, login.to_owned())),
        Ok(None) => {
            tracing::warn!(login, error = ?last, "invite: GitHub could not be asked who the login is");
            Err(refuse(reason::LOOKUP_FAILED, None))
        }
        Err(e) => Err(failed(e)),
    }
}

/// Invite `login` to `tenant` with `role`. An existing member is refused (`already_member`),
/// unless `promote`, where the operator names an admin and a member becomes one.
pub(super) async fn invite(
    app: &App,
    tenant: &TenantId,
    login: &str,
    role: Role,
    by: &str,
    promote: bool,
) -> std::result::Result<(i64, String), Refusal> {
    let (github_id, login) = who_is(app, Some(tenant), login).await?;
    let now = app.clock.now();
    let mut tx = app.db.begin().await.map_err(failed)?;
    let added = tenants::add_member(&mut tx, tenant.as_str(), github_id, &login, role, by, now)
        .await
        .map_err(failed)?;
    let action = match added {
        Ok(()) => "invited",
        Err(Refused(reason::ALREADY_MEMBER)) if promote => {
            tenants::set_role(&mut tx, tenant.as_str(), github_id, role)
                .await
                .map_err(failed)?
                .map_err(|r| refuse(r.0, None))?;
            "role_changed"
        }
        Err(r) => return Err(refuse(r.0, None)),
    };
    tx.commit().await.map_err(failed)?;
    member_changed(tenant, action, &login, github_id, by);
    app.notify_in(tenant, MEMBERS_CHANGED);
    Ok((github_id, login))
}

/// The reason of the event a tenant's browsers get when its members change. A socket reads its
/// own membership again when it sees one (design §6.1).
pub const MEMBERS_CHANGED: &str = rubato_protocol::web_link::changed::MEMBERS;

/// What ends when a person is in no tenant any more (design §4.6): their web sessions, user
/// tokens and pairings, and what was queued on those companions. Inside the caller's
/// transaction.
pub(super) async fn end_last_membership(
    conn: &mut sqlx::SqliteConnection,
    login: &str,
) -> Result<(u64, usize)> {
    let sessions = crate::auth::revoke_person(conn, login).await?;
    let companions = crate::pairing::revoke_all_of(conn, login).await?;
    crate::queue::drop_companions(conn, &companions).await?;
    Ok((sessions, companions.len()))
}

/// Remove a person from a tenant (design §4.6), in one transaction: the membership; their
/// agents in the tenant's rooms and their pending call-in requests; what is queued for the
/// tenant on their companions; and, when it was their last membership, their web sessions,
/// user tokens and pairings. After the commit the tenant's browsers are told, so the person's
/// own socket for it closes, and nothing more is taken from or pushed to their launches there
/// (`launch::acting_target`).
pub async fn remove_person(
    app: &App,
    tenant: &TenantId,
    github_id: i64,
    by: &str,
) -> Result<tenants::Outcome<Removed>> {
    let now = app.clock.now();
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await?;
    let login = tenants::member_login(&mut tx, tenant.as_str(), github_id).await?;
    let removed = match tenants::remove_member(&mut tx, tenant.as_str(), github_id).await? {
        Ok(r) => r,
        Err(r) => return Ok(Err(r)),
    };
    let login = login.unwrap_or_default();
    let (agents, requests) = crate::rooms::leave_person(&mut tx, tenant, &login, now).await?;
    let companions = crate::pairing::companions_of(&mut tx, &login).await?;
    let queued = crate::queue::drop_companions_in(&mut tx, tenant, &companions).await?;
    let (sessions, pairings) = if removed.last_membership {
        end_last_membership(&mut tx, &login).await?
    } else {
        (0, 0)
    };
    tx.commit().await?;
    tracing::info!(
        rubato.tenant = %tenant,
        action = "removed",
        login = %login,
        github_id,
        by,
        agents,
        requests,
        queued,
        last_membership = removed.last_membership,
        sessions,
        pairings,
        "rubato.tenant.member_changed"
    );
    app.notify_in(tenant, MEMBERS_CHANGED);
    Ok(Ok(removed))
}
