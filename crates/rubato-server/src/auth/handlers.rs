//! GitHub OAuth login, logout and `me` for the web view.

use super::current_user::{COOKIE, cookie_value};
use super::store;
use super::team::{self, reason};
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
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let token = match client
        .exchange_code(AuthorizationCode::new(q.code))
        .request_async(&http)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("token exchange failed: {e}"),
            )
                .into_response();
        }
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
            Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        },
        Err(e) => {
            return (StatusCode::BAD_GATEWAY, format!("user lookup failed: {e}")).into_response();
        }
    };
    // As a GitHub App: the person is on the team when their account can access the
    // installation the server acts as (ADR 0022). A failed lookup is a fault of ours or of
    // GitHub, not "no access", and answers differently.
    let mut team_checked_at = None;
    if let Some(app_cfg) = &app.config.github_app {
        let answer = http
            .get(format!(
                "{}/user/installations?per_page=100",
                app.config.github_api()
            ))
            .bearer_auth(token.access_token().secret())
            .header(header::USER_AGENT, "rubato-server")
            .header(header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .and_then(|r| r.error_for_status());
        let answer: serde_json::Value = match answer {
            Ok(r) => match r
                .text()
                .await
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
            {
                Ok(v) => v,
                Err(e) => return installation_lookup_failed(&app, &user.login, &e),
            },
            Err(e) => return installation_lookup_failed(&app, &user.login, &e.to_string()),
        };
        if !team::member_of(&answer, app_cfg.installation_id) {
            tracing::warn!(
                login = %user.login,
                installation_id = app_cfg.installation_id,
                "login refused: no access to the installation"
            );
            app.reject(routes::CALLBACK, reason::NOT_ON_TEAM, &user.login);
            return (
                StatusCode::FORBIDDEN,
                "Your GitHub account has no access to the repositories this Rubato is installed on.",
            )
                .into_response();
        }
        team_checked_at = Some(Utc::now());
    }
    if let Err(e) = store::upsert_user(
        &app.db,
        &user.login,
        Some(user.id),
        user.avatar_url.as_deref(),
    )
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
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
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let mut res = Redirect::to("/").into_response();
    let secure = app.config.public_url.starts_with("https");
    res.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(COOKIE, &app.signer.sign(&id), 60 * 60 * 24 * 30, secure),
    );
    res.headers_mut()
        .append(header::SET_COOKIE, set_cookie(STATE_COOKIE, "", 0, secure));
    res
}

/// The membership lookup itself failed: 502 with its own reason, never "no access".
fn installation_lookup_failed(app: &App, login: &str, error: &str) -> Response {
    tracing::error!(login, error, "installation lookup failed at login");
    app.reject(routes::CALLBACK, reason::INSTALLATION_LOOKUP_FAILED, login);
    (
        StatusCode::BAD_GATEWAY,
        "Could not check your access with GitHub. Try again; if it persists, the GitHub App settings need a look.",
    )
        .into_response()
}

pub async fn logout(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(v) = cookie_value(&headers, COOKIE)
        && let Some(id) = app.signer.verify(&v)
    {
        if let Ok(Some((login, _))) = store::web_session(&app.db, &id).await {
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
        set_cookie(COOKIE, "", 0, app.config.public_url.starts_with("https")),
    );
    res
}

pub async fn me(State(app): State<App>, super::CurrentUser(login): super::CurrentUser) -> Json<Me> {
    let can = super::tokens::can_act(&app, &login).await;
    Json(Me {
        login,
        avatar_url: None,
        can_act_on_github: can.is_ok(),
        cannot_act_reason: can.err().map(str::to_owned),
    })
}
