//! GitHub OAuth login, logout and `me` for the web view.

use super::current_user::cookie_value;
use super::store;
use crate::kernel::state::App;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::Utc;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use rubato_protocol::Me;
use rubato_protocol::auth::routes;
use serde::Deserialize;

const STATE_COOKIE: &str = "rubato_oauth_state";

fn set_cookie(name: &str, value: &str, max_age: i64, secure: bool) -> HeaderValue {
    let mut s = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure {
        s.push_str("; Secure");
    }
    HeaderValue::from_str(&s).expect("cookie header")
}

type OAuthClient = oauth2::basic::BasicClient<
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

fn oauth_client(app: &App) -> Option<OAuthClient> {
    let id = app.config.github_client_id.clone()?;
    let secret = app.config.github_client_secret.clone()?;
    let redirect = RedirectUrl::new(format!(
        "{}{}",
        app.config.public_url.trim_end_matches('/'),
        routes::CALLBACK
    ))
    .ok()?;
    Some(
        oauth2::basic::BasicClient::new(ClientId::new(id))
            .set_client_secret(ClientSecret::new(secret))
            .set_auth_uri(
                AuthUrl::new(format!("{}/login/oauth/authorize", app.config.github_web())).ok()?,
            )
            .set_token_uri(
                TokenUrl::new(format!(
                    "{}/login/oauth/access_token",
                    app.config.github_web()
                ))
                .ok()?,
            )
            .set_redirect_uri(redirect),
    )
}

pub async fn login(State(app): State<App>) -> Response {
    let Some(client) = oauth_client(&app) else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "GitHub OAuth is not configured (GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET)",
        )
            .into_response();
    };
    let (url, state) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("read:user".into()))
        .url();
    let mut res = Redirect::to(url.as_str()).into_response();
    res.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            STATE_COOKIE,
            state.secret(),
            600,
            app.config.public_url.starts_with("https"),
        ),
    );
    res
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct GitHubUser {
    login: String,
    id: i64,
    avatar_url: Option<String>,
}

pub async fn callback(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(client) = oauth_client(&app) else {
        return StatusCode::NOT_IMPLEMENTED.into_response();
    };
    if cookie_value(&headers, STATE_COOKIE).as_deref() != Some(q.state.as_str()) {
        return (StatusCode::BAD_REQUEST, "oauth state mismatch").into_response();
    }
    let http = match oauth2::reqwest::ClientBuilder::new()
        .redirect(oauth2::reqwest::redirect::Policy::none())
        .timeout(super::tokens::TOKEN_ENDPOINT_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => return login_failed_here(e),
    };
    let token = match client
        .exchange_code(AuthorizationCode::new(q.code))
        .request_async(&http)
        .await
    {
        Ok(t) => t,
        Err(e) => return login_failed_upstream(&app, "token_exchange", &e.to_string()),
    };
    let user: GitHubUser = match http
        .get(format!("{}/user", app.config.github_api()))
        .bearer_auth(token.access_token().secret())
        .header(header::USER_AGENT, "rubato-server")
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => match r
            .text()
            .await
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<GitHubUser>(&t).map_err(|e| e.to_string()))
        {
            Ok(u) => u,
            Err(e) => return login_failed_upstream(&app, "user_lookup", &e),
        },
        Err(e) => return login_failed_upstream(&app, "user_lookup", &e.to_string()),
    };
    // A login GitHub now gives another id belongs to someone else: the adapter revokes what
    // the old person held before anything of this login is written (Decision 15).
    match app
        .admission()
        .before_login(&app, &user.login, user.id)
        .await
    {
        Ok(false) => {}
        Ok(true) => {
            tracing::warn!(login = %user.login, "identity changed: the login has a new GitHub id; the old person's sessions, tokens and pairings are revoked");
            app.reject(
                routes::CALLBACK,
                rubato_protocol::tenant::reason::IDENTITY_CHANGED,
                &user.login,
            );
        }
        Err(e) => return login_failed_here(e),
    }
    // ADR 0088, superseding ADR 0022's gate: a session is made only for an operator or for a
    // person invited to at least one tenant. The invite alone is the gate; GitHub is not asked
    // what the account can access. Refused before anything of this login is kept.
    let invited = match app.admission().tenants_of(&app, user.id).await {
        Ok(tenants) => !tenants.is_empty(),
        Err(e) => return login_failed_here(e),
    };
    if !invited && !app.config.is_operator(Some(user.id)) {
        tracing::warn!(login = %user.login, github_id = user.id, "login refused: not invited to any tenant");
        app.reject(
            routes::CALLBACK,
            rubato_protocol::tenant::reason::NOT_INVITED,
            &user.login,
        );
        return (StatusCode::FORBIDDEN, NOT_INVITED).into_response();
    }
    // Kept on the session for the seed's "who was here lately" (design §8); no gate reads it.
    let team_checked_at = Some(Utc::now());
    if let Err(e) = store::upsert_user(
        &app.db,
        &user.login,
        Some(user.id),
        user.avatar_url.as_deref(),
    )
    .await
    {
        return login_failed_here(e);
    }
    // ADR 0030: keep the user-to-server token, so Rubato can post an approval as this person.
    let granted = super::tokens::Granted {
        access: token.access_token().secret().clone(),
        expires_in: token
            .expires_in()
            .and_then(|d| chrono::Duration::from_std(d).ok()),
        refresh: token.refresh_token().map(|r| r.secret().clone()),
        refresh_expires_in: None,
    };
    match super::tokens::keep(&app, &user.login, &granted).await {
        Ok(None) => tracing::info!(login = %user.login, "user token kept"),
        Ok(Some(why)) => tracing::info!(login = %user.login, why, "no user token kept"),
        Err(e) => {
            tracing::error!(login = %user.login, error = %format!("{e:#}"), "keeping the user token failed")
        }
    }
    let id = match store::create_web_session(&app.db, &user.login, team_checked_at).await {
        Ok(id) => id,
        Err(e) => return login_failed_here(e),
    };
    if let Err(e) = app
        .admission()
        .after_login(&app, &user.login, user.id)
        .await
    {
        tracing::error!(login = %user.login, error = %format!("{e:#}"), "noting the login on its memberships failed");
    }
    let mut res = Redirect::to("/").into_response();
    let secure = app.config.public_url.starts_with("https");
    res.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            app.config.session_cookie(),
            &app.signer.sign(&id),
            60 * 60 * 24 * 30,
            secure,
        ),
    );
    res.headers_mut()
        .append(header::SET_COOKIE, set_cookie(STATE_COOKIE, "", 0, secure));
    res
}

/// What the browser sees when nobody has invited the account (ADR 0088).
pub const NOT_INVITED: &str =
    "You have not been invited to a tenant on this Rubato. Ask one of its admins for an invite.";

/// What the browser sees when GitHub did not complete the login (design 193 D8). The upstream
/// text is logged, never echoed.
pub const LOGIN_FAILED_UPSTREAM: &str = "GitHub did not complete the login. Try again.";
/// What the browser sees when the login failed on this server.
pub const LOGIN_FAILED_HERE: &str = "Login failed on the server.";

fn login_failed_upstream(app: &App, step: &'static str, error: &str) -> Response {
    tracing::error!(step, error, "login failed");
    let reason = match step {
        "token_exchange" => "token_exchange_failed",
        _ => "user_lookup_failed",
    };
    app.reject(routes::CALLBACK, reason, "-");
    (StatusCode::BAD_GATEWAY, LOGIN_FAILED_UPSTREAM).into_response()
}

fn login_failed_here(e: impl std::fmt::Display) -> Response {
    (crate::kernel::http::internal("auth", e), LOGIN_FAILED_HERE).into_response()
}

pub async fn logout(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(v) = cookie_value(&headers, app.config.session_cookie())
        && let Some(id) = app.signer.verify(&v)
    {
        if let Ok(Some(store::WebSession { login, .. })) = store::web_session(&app.db, &id).await {
            let _ = store::delete_web_session(&app.db, &id).await;
            if let Err(e) = super::tokens::after_logout(&app.db, &login).await {
                tracing::error!(login, error = %format!("{e:#}"), "dropping the user token at logout failed");
            }
        } else {
            let _ = store::delete_web_session(&app.db, &id).await;
        }
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(
            app.config.session_cookie(),
            "",
            0,
            app.config.public_url.starts_with("https"),
        ),
    );
    res
}

pub async fn me(State(app): State<App>, super::CurrentUser(login): super::CurrentUser) -> Json<Me> {
    let can = super::tokens::can_act(&app, &login).await;
    let github_id = store::github_id_of(&app.db, &login)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %format!("{e:#}"), "reading the GitHub id for Me failed");
            None
        });
    let tenants = match github_id {
        Some(id) => app
            .admission()
            .tenants_of(&app, id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %format!("{e:#}"), "reading tenants for Me failed");
                Vec::new()
            }),
        None => Vec::new(),
    };
    // Local development (`RUBATO_DEV_USER`, loopback only): every dev login is an operator and
    // an admin of tenant `default`, as `tenants::scope_for` grants it (design §12).
    let dev = app.config.dev_user.is_some();
    let mut tenants = tenants;
    if dev
        && !tenants
            .iter()
            .any(|t| t.id == crate::kernel::tenancy::DEFAULT_TENANT)
    {
        tenants.insert(
            0,
            rubato_protocol::TenantInfo {
                id: crate::kernel::tenancy::DEFAULT_TENANT.to_owned(),
                name: "Default".to_owned(),
                role: rubato_protocol::Role::Admin,
            },
        );
    }
    Json(Me {
        login,
        avatar_url: None,
        can_act_on_github: can.is_ok(),
        cannot_act_reason: can.err().map(str::to_owned),
        voice: app.config.voice_availability(),
        tenants,
        operator: dev || app.config.is_operator(github_id),
    })
}
