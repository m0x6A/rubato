//! GitHub OAuth login for the web view and the signed session cookie.

use crate::db;
use crate::state::App;
use anyhow::Result;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header, request::Parts};
use axum::response::{IntoResponse, Redirect, Response};
use hmac::{Hmac, Mac};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use rubato_protocol::web::routes;
use serde::Deserialize;
use sha2::Sha256;

pub const COOKIE: &str = "rubato_session";
const STATE_COOKIE: &str = "rubato_oauth_state";

/// HMAC-SHA256 signer for cookie values `id.sig`.
#[derive(Clone)]
pub struct Signer {
    key: Vec<u8>,
}

impl Signer {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            key: secret.to_vec(),
        }
    }

    fn mac(&self, id: &str) -> String {
        let mut m = Hmac::<Sha256>::new_from_slice(&self.key).expect("hmac key");
        m.update(id.as_bytes());
        m.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    pub fn sign(&self, id: &str) -> String {
        format!("{id}.{}", self.mac(id))
    }

    pub fn verify(&self, value: &str) -> Option<String> {
        let (id, sig) = value.rsplit_once('.')?;
        let expected = self.mac(id);
        // Constant-time compare.
        if expected.len() != sig.len() {
            return None;
        }
        let diff = expected
            .bytes()
            .zip(sig.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        (diff == 0).then(|| id.to_owned())
    }
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}

fn set_cookie(name: &str, value: &str, max_age: i64, secure: bool) -> HeaderValue {
    let mut s = format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}");
    if secure {
        s.push_str("; Secure");
    }
    HeaderValue::from_str(&s).expect("cookie header")
}

/// The logged-in GitHub login. In dev mode (`RUBATO_DEV_USER`) the header `x-rubato-dev-user`
/// may name another login, so tests can act as two people.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub String);

impl FromRequestParts<App> for CurrentUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, Self::Rejection> {
        if let Some(dev) = &app.config.dev_user {
            let login = parts
                .headers
                .get("x-rubato-dev-user")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
                .unwrap_or_else(|| dev.clone());
            return Ok(CurrentUser(login));
        }
        let value = cookie_value(&parts.headers, COOKIE).ok_or(StatusCode::UNAUTHORIZED)?;
        let id = app.signer.verify(&value).ok_or(StatusCode::UNAUTHORIZED)?;
        match db::web_session_login(&app.db, &id).await {
            Ok(Some(login)) => Ok(CurrentUser(login)),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }
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
    if let Err(e) = db::upsert_user(
        &app.db,
        &user.login,
        Some(user.id),
        user.avatar_url.as_deref(),
    )
    .await
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    let id = match db::create_web_session(&app.db, &user.login).await {
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

pub async fn logout(State(app): State<App>, headers: HeaderMap) -> Response {
    if let Some(v) = cookie_value(&headers, COOKIE)
        && let Some(id) = app.signer.verify(&v)
    {
        let _ = db::delete_web_session(&app.db, &id).await;
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(COOKIE, "", 0, app.config.public_url.starts_with("https")),
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_round_trips_and_rejects_tampering() {
        let s = Signer::new(b"secret");
        let v = s.sign("abc");
        assert_eq!(s.verify(&v).as_deref(), Some("abc"));
        let mut t = v.clone();
        t.replace_range(0..1, "x");
        assert_eq!(s.verify(&t), None);
        assert_eq!(Signer::new(b"other").verify(&v), None);
    }

    #[test]
    fn reads_a_cookie_among_many() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; rubato_session=x.y; b=2"),
        );
        assert_eq!(cookie_value(&h, COOKIE).as_deref(), Some("x.y"));
        assert_eq!(cookie_value(&h, "zzz"), None);
    }
}
