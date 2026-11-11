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
            .set_auth_uri(AuthUrl::new("https://github.com/login/oauth/authorize".into()).ok()?)
            .set_token_uri(
                TokenUrl::new("https://github.com/login/oauth/access_token".into()).ok()?,
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
        .get("https://api.github.com/user")
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
            .get("https://api.github.com/user/installations?per_page=100")
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
        let _ = store::delete_web_session(&app.db, &id).await;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(COOKIE, "", 0, app.config.public_url.starts_with("https")),
    );
    res
}

pub async fn me(super::CurrentUser(login): super::CurrentUser) -> Json<Me> {
    Json(Me {
        login,
        avatar_url: None,
    })
}
