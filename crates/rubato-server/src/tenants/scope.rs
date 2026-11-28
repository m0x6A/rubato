//! Where a [`TenantScope`] comes from (design §4.2): the membership row, read for this request
//! or this intake. Nothing else in the server grants one.

use super::store;
use crate::auth::CurrentUser;
use crate::kernel::state::App;
use crate::kernel::tenancy::{DEFAULT_TENANT, TenantId, TenantScope, unknown_tenant};
use anyhow::Result;
use axum::extract::{FromRequestParts, RawPathParams, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rubato_protocol::Role;
use rubato_protocol::tenant::reason;

/// The scope of `login` in `tenant`, or `None` when the person is not a member (or the tenant
/// does not exist: the same answer). Read on every call, so a removal counts from the next one.
///
/// Local development (`RUBATO_DEV_USER`, loopback only) has no GitHub ids: there every dev login
/// is an admin of tenant `default` (design §12).
pub async fn scope_for(app: &App, tenant: &TenantId, login: &str) -> Result<Option<TenantScope>> {
    let github_id = crate::auth::github_id_of(&app.db, login).await?;
    if app.config.dev_user.is_some() && tenant.as_str() == DEFAULT_TENANT {
        return Ok(Some(TenantScope::granted(
            tenant.clone(),
            github_id.unwrap_or(0),
            login.to_owned(),
            Role::Admin,
        )));
    }
    let Some(github_id) = github_id else {
        return Ok(None);
    };
    Ok(store::role_of(&app.db, tenant.as_str(), github_id)
        .await?
        .map(|role| TenantScope::granted(tenant.clone(), github_id, login.to_owned(), role)))
}

/// The logins of a tenant's members, lower case, for what a tenant's views show of people
/// (their companions, design §6.2). `None` means every login: local development, where every dev
/// login is an admin of tenant `default`.
pub async fn member_logins(
    app: &App,
    tenant: &TenantId,
) -> Result<Option<std::collections::HashSet<String>>> {
    if app.config.dev_user.is_some() && tenant.as_str() == DEFAULT_TENANT {
        return Ok(None);
    }
    Ok(Some(
        store::members(&app.db, tenant.as_str())
            .await?
            .into_iter()
            .map(|m| m.login.to_ascii_lowercase())
            .collect(),
    ))
}

/// Layer on every `/api/t/{tenant}/…` route (ADR 0089): the logged-in person's membership of
/// `{tenant}` becomes the request's [`TenantScope`]. No session is 401; an unknown tenant, or
/// one the person is not in, is 404 `unknown_tenant`, counted on `rubato.web.rejected`.
pub async fn check_tenant(
    State(app): State<App>,
    params: RawPathParams,
    req: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = req.into_parts();
    let user = match CurrentUser::from_request_parts(&mut parts, &app).await {
        Ok(u) => u,
        Err(status) => return status.into_response(),
    };
    // The route template, not the path: the id in the URL is not a label.
    let route = parts
        .extensions
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    let tenant = params
        .iter()
        .find(|(k, _)| *k == "tenant")
        .and_then(|(_, v)| TenantId::parse(v));
    let scope = match &tenant {
        Some(t) => match scope_for(&app, t, &user.0).await {
            Ok(s) => s,
            Err(e) => return crate::kernel::http::internal("tenants", e).into_response(),
        },
        None => None,
    };
    let Some(scope) = scope else {
        app.reject(&route, reason::UNKNOWN_TENANT, &user.0);
        tracing::warn!(rubato.route = %route, rubato.user = %user.0, "unknown tenant, or not a member");
        return unknown_tenant();
    };
    tracing::Span::current().record(rubato_protocol::otel::attr::TENANT, scope.tenant().as_str());
    parts.extensions.insert(scope);
    next.run(Request::from_parts(parts, body)).await
}
