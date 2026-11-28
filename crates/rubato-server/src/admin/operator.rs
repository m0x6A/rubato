//! The operator's routes (ADR 0087, 0088; design §3.3, §5.4): the App's installations, tenants,
//! attach and detach, purge, and naming an admin. An operator sees names, installations and
//! counts, never a tenant's data. Anyone else is answered 404.

use super::people::{Refusal, end_last_membership, failed, invite, refuse};
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use crate::tenants;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use rubato_protocol::tenant::{
    AccountType, AdminInstallation, AdminTenant, InstallationState, InviteRequest, NewTenant,
    PurgeReport, Role, reason, routes,
};
use std::collections::BTreeMap;

/// Who may use these routes: a GitHub user id in `RUBATO_OPERATORS`, or anyone in local
/// development (`RUBATO_DEV_USER`, loopback only).
async fn operator(app: &App, login: &str, route: &'static str) -> Result<(), Refusal> {
    if app.config.dev_user.is_some() {
        return Ok(());
    }
    let id = crate::auth::github_id_of(&app.db, login)
        .await
        .map_err(failed)?;
    if app.config.is_operator(id) {
        return Ok(());
    }
    app.reject(route, reason::NOT_OPERATOR, login);
    Err(refuse(reason::NOT_OPERATOR, None))
}

fn tenant_id(raw: &str) -> Result<TenantId, Refusal> {
    TenantId::parse(raw).ok_or_else(|| refuse(reason::UNKNOWN_TENANT, None))
}

async fn existing(app: &App, raw: &str) -> Result<TenantId, Refusal> {
    let t = tenant_id(raw)?;
    if tenants::tenant_exists(&app.db, t.as_str())
        .await
        .map_err(failed)?
    {
        Ok(t)
    } else {
        Err(refuse(reason::UNKNOWN_TENANT, None))
    }
}

#[derive(serde::Deserialize)]
pub struct TenantParam {
    tenant: String,
}

#[derive(serde::Deserialize)]
pub struct AttachParam {
    tenant: String,
    installation_id: u64,
}

#[derive(serde::Deserialize)]
pub struct InstallationParam {
    installation_id: u64,
}

/// `?confirm=true` runs the purge; without it the answer only counts.
#[derive(serde::Deserialize)]
pub struct PurgeQuery {
    #[serde(default)]
    confirm: bool,
}

/// `GET /api/admin/installations`: every installation of the App as GitHub lists them, each
/// attached (to which tenant), suspended, detached or never attached; and the ones this server
/// still has a row for that GitHub no longer lists (uninstalled: detached, to purge).
pub async fn installations(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<AdminInstallation>>, Refusal> {
    operator(&app, &user, routes::ADMIN_INSTALLATIONS).await?;
    let listed = app
        .github
        .as_person()
        .app_installations()
        .await
        .map_err(|e| {
            tracing::warn!(error = %format!("{e:#}"), "the App's installations could not be listed");
            refuse(reason::LOOKUP_FAILED, Some(format!("{e:#}")))
        })?;
    let rows = tenants::installation_rows(&app.db).await.map_err(failed)?;
    let mut out: BTreeMap<u64, AdminInstallation> = BTreeMap::new();
    for i in &listed {
        let row = rows.iter().find(|r| r.installation_id == i.id);
        let state = match row {
            None => InstallationState::Unattached,
            Some(r) if r.detached => InstallationState::Detached,
            Some(_) if i.suspended => InstallationState::Suspended,
            Some(_) => InstallationState::Attached,
        };
        out.insert(
            i.id,
            AdminInstallation {
                installation_id: i.id,
                account_login: i.account_login.clone(),
                account_type: AccountType::parse(&i.account_type).unwrap_or(AccountType::User),
                state,
                tenant: row.map(|r| r.tenant.clone()),
            },
        );
    }
    // The ones GitHub no longer lists: uninstalled, their rows still here.
    for r in &rows {
        if out.contains_key(&r.installation_id) {
            continue;
        }
        out.insert(
            r.installation_id,
            AdminInstallation {
                installation_id: r.installation_id,
                account_login: r.account_login.clone().unwrap_or_default(),
                account_type: r
                    .account_type
                    .as_deref()
                    .and_then(AccountType::parse)
                    .unwrap_or(AccountType::User),
                state: if r.detached {
                    InstallationState::Detached
                } else if r.suspended {
                    InstallationState::Suspended
                } else {
                    InstallationState::Attached
                },
                tenant: Some(r.tenant.clone()),
            },
        );
    }
    Ok(Json(out.into_values().collect()))
}

/// `GET /api/admin/tenants`.
pub async fn tenants_list(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<AdminTenant>>, Refusal> {
    operator(&app, &user, routes::ADMIN_TENANTS).await?;
    let mut out = Vec::new();
    for t in tenants::tenant_rows(&app.db).await.map_err(failed)? {
        let workspaces = TenantId::parse(&t.id)
            .map(|id| app.workspaces_of(&id).len())
            .unwrap_or(0) as u32;
        out.push(AdminTenant {
            installations: tenants::installations_of(&app.db, &t.id)
                .await
                .map_err(failed)?,
            id: t.id,
            name: t.name,
            members: t.members,
            admins: t.admins,
            workspaces,
        });
    }
    Ok(Json(out))
}

/// Attach `installation` to `tenant` inside `tx`, with the account GitHub names for it.
async fn attach_in(
    app: &App,
    tx: &mut sqlx::SqliteConnection,
    tenant: &TenantId,
    installation: u64,
    by: &str,
    listed: &[crate::kernel::github::AppInstallation],
) -> Result<(), Refusal> {
    let Some(known) = listed.iter().find(|i| i.id == installation) else {
        return Err(refuse(reason::UNKNOWN_INSTALLATION, None));
    };
    match tenants::attach_installation(tx, tenant.as_str(), installation, by, app.clock.now())
        .await
        .map_err(failed)?
    {
        Ok(()) => {}
        Err(r) => {
            // Which tenant holds it, or holds its repositories' rows: the one to detach from
            // or to purge (design §3.3).
            let holder = tenants::installation_rows(&app.db)
                .await
                .ok()
                .and_then(|rows| {
                    rows.into_iter()
                        .find(|x| x.installation_id == installation)
                        .map(|x| x.tenant)
                });
            tracing::warn!(rubato.github.installation = installation, rubato.tenant = %tenant, reason = r.0, holder = ?holder, "attach_refused");
            return Err(refuse(r.0, holder));
        }
    }
    tenants::set_account(tx, installation, &known.account_login, &known.account_type)
        .await
        .map_err(failed)?;
    Ok(())
}

async fn listed(app: &App) -> Result<Vec<crate::kernel::github::AppInstallation>, Refusal> {
    app.github
        .as_person()
        .app_installations()
        .await
        .map_err(|e| refuse(reason::LOOKUP_FAILED, Some(format!("{e:#}"))))
}

/// After the tenancy rows changed: the registry routes by them, and each live installation's
/// repositories are listed.
async fn reroute(app: &App) -> Result<(), Refusal> {
    tenants::load_routes(app).await.map_err(failed)?;
    super::refresh_installations(app).await;
    Ok(())
}

/// `POST /api/admin/tenants`: a tenant, its installations and its first admin, or nothing.
pub async fn create_tenant(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<NewTenant>,
) -> Result<(StatusCode, Json<AdminTenant>), Refusal> {
    operator(&app, &user, routes::ADMIN_TENANTS).await?;
    let Some(tenant) = TenantId::parse(&req.id) else {
        return Err(refuse(reason::BAD_ID, None));
    };
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(refuse(reason::BAD_ID, Some("name".into())));
    }
    // GitHub is asked before anything is written: who the first admin is, and what is installed.
    let (admin_id, admin_login) = super::people::who_is(&app, None, &req.first_admin).await?;
    let known = if req.installations.is_empty() {
        Vec::new()
    } else {
        listed(&app).await?
    };
    let now = app.clock.now();
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await.map_err(failed)?;
    tenants::create_tenant(&mut tx, tenant.as_str(), name, &user, now)
        .await
        .map_err(failed)?
        .map_err(|r| refuse(r.0, None))?;
    for i in &req.installations {
        attach_in(&app, &mut tx, &tenant, *i, &user, &known).await?;
    }
    tenants::add_member(
        &mut tx,
        tenant.as_str(),
        admin_id,
        &admin_login,
        Role::Admin,
        &user,
        now,
    )
    .await
    .map_err(failed)?
    .map_err(|r| refuse(r.0, None))?;
    tx.commit().await.map_err(failed)?;
    tracing::info!(rubato.tenant = %tenant, by = %user, installations = ?req.installations, first_admin = %admin_login, "tenant created");
    super::people::member_changed(&tenant, "invited", &admin_login, admin_id, &user);
    reroute(&app).await?;
    Ok((
        StatusCode::CREATED,
        Json(AdminTenant {
            id: tenant.to_string(),
            name: name.to_owned(),
            installations: req.installations,
            members: 1,
            admins: 1,
            workspaces: 0,
        }),
    ))
}

/// `DELETE /api/admin/tenants/{tenant}`: only an empty one, with no workspace and no attached
/// installation. Its members go with it.
pub async fn delete_tenant(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(p): Path<TenantParam>,
) -> Result<StatusCode, Refusal> {
    operator(&app, &user, routes::ADMIN_TENANT).await?;
    let tenant = existing(&app, &p.tenant).await?;
    if let Some(w) = app.workspaces_of(&tenant).first() {
        return Err(refuse(
            reason::TENANT_NOT_EMPTY,
            Some(format!("workspace {}", w.id)),
        ));
    }
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await.map_err(failed)?;
    let members = tenants::delete_tenant(&mut tx, tenant.as_str())
        .await
        .map_err(failed)?
        .map_err(|r| refuse(r.0, Some("an installation is attached".into())))?;
    for (github_id, login) in &members {
        if !tenants::in_any_tenant(&mut tx, *github_id)
            .await
            .map_err(failed)?
        {
            end_last_membership(&mut tx, login).await.map_err(failed)?;
        }
    }
    tx.commit().await.map_err(failed)?;
    tracing::warn!(rubato.tenant = %tenant, by = %user, members = members.len(), "tenant deleted");
    app.notify_in(&tenant, super::people::MEMBERS_CHANGED);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/tenants/{tenant}/installations/{installation_id}`.
pub async fn attach(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(p): Path<AttachParam>,
) -> Result<StatusCode, Refusal> {
    operator(&app, &user, routes::ADMIN_ATTACH).await?;
    let tenant = existing(&app, &p.tenant).await?;
    let known = listed(&app).await?;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await.map_err(failed)?;
    attach_in(&app, &mut tx, &tenant, p.installation_id, &user, &known).await?;
    tx.commit().await.map_err(failed)?;
    tracing::info!(rubato.github.installation = p.installation_id, rubato.tenant = %tenant, by = %user, "installation attached");
    reroute(&app).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/admin/tenants/{tenant}/installations/{installation_id}`: detach (design §3.3).
/// Refused while a workspace of the tenant is on one of its repositories; the rows of its
/// repositories are kept, hidden, until purged.
pub async fn detach(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(p): Path<AttachParam>,
) -> Result<StatusCode, Refusal> {
    operator(&app, &user, routes::ADMIN_ATTACH).await?;
    let tenant = existing(&app, &p.tenant).await?;
    let now = app.clock.now();
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await.map_err(failed)?;
    match tenants::attached(&mut tx, p.installation_id)
        .await
        .map_err(failed)?
    {
        Some((t, _)) if t == tenant.as_str() => {}
        _ => return Err(refuse(reason::UNKNOWN_INSTALLATION, None)),
    }
    let repos = tenants::live_repos_of(&mut tx, p.installation_id)
        .await
        .map_err(failed)?;
    let on = |r: &str| repos.iter().any(|x| x.eq_ignore_ascii_case(r));
    for w in app.workspaces_of(&tenant) {
        let named = match app.map_of(&w.key()).await {
            Some(m) => m.map.repos_named().iter().any(|r| on(r)),
            None => false,
        };
        if on(&w.map_repo) || named {
            return Err(refuse(
                reason::INSTALLATION_IN_USE,
                Some(format!("workspace {}", w.id)),
            ));
        }
    }
    let gone = tenants::detach_installation(&mut tx, p.installation_id, now)
        .await
        .map_err(failed)?;
    super::lifecycle::leave(&mut tx, &tenant, &gone, now)
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    tracing::warn!(rubato.github.installation = p.installation_id, rubato.tenant = %tenant, by = %user, repos = gone.len(), "installation detached: its rows kept until purged");
    tenants::load_routes(&app).await.map_err(failed)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/installations/{installation_id}/purge`: what the purge of a detached
/// installation would delete, per table; with `?confirm=true`, deleted, the tombstones after
/// them, in one transaction.
pub async fn purge(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(p): Path<InstallationParam>,
    Query(q): Query<PurgeQuery>,
) -> Result<Json<PurgeReport>, Refusal> {
    operator(&app, &user, routes::ADMIN_PURGE).await?;
    let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await.map_err(failed)?;
    let (tenant, repos) = match tenants::detached(&mut tx, p.installation_id)
        .await
        .map_err(failed)?
    {
        None => return Err(refuse(reason::UNKNOWN_INSTALLATION, None)),
        Some(Err(r)) => return Err(refuse(r.0, None)),
        Some(Ok(found)) => found,
    };
    let rows = crate::kernel::purge::rows_of_repos(&mut tx, &tenant, &repos, q.confirm)
        .await
        .map_err(failed)?;
    if q.confirm {
        tenants::drop_tombstones(&mut tx, p.installation_id)
            .await
            .map_err(failed)?;
    }
    tx.commit().await.map_err(failed)?;
    if q.confirm {
        tracing::warn!(rubato.github.installation = p.installation_id, rubato.tenant = %tenant, by = %user, repos = repos.len(), rows = rows.values().sum::<u64>(), tables = ?rows, "installation purged");
    }
    Ok(Json(PurgeReport {
        installation_id: p.installation_id,
        tenant,
        repos,
        rows,
        purged: q.confirm,
    }))
}

/// `POST /api/admin/tenants/{tenant}/admins`: the operator names an admin (ADR 0088), so a
/// tenant whose admins are gone is never stuck. A member becomes an admin; anyone else is
/// invited as one.
pub async fn name_admin(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
    Path(p): Path<TenantParam>,
    Json(req): Json<InviteRequest>,
) -> Result<StatusCode, Refusal> {
    operator(&app, &user, routes::ADMIN_TENANT_ADMINS).await?;
    let tenant = existing(&app, &p.tenant).await?;
    invite(&app, &tenant, &req.login, Role::Admin, &user, true).await?;
    Ok(StatusCode::NO_CONTENT)
}
