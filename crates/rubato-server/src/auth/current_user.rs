//! The logged-in user, from the signed session cookie (or the dev user in development).

use super::store;
use super::team::{self, reason};
use crate::kernel::state::App;
use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, StatusCode, header, request::Parts};
use chrono::Utc;

pub const COOKIE: &str = "rubato_session";

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
        let Ok(Some((login, team_checked_at))) = store::web_session(&app.db, &id).await else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        // As a GitHub App, a session counts only with a fresh team record (ADR 0022). This
        // runs on every route and on the websocket upgrade, so a session from before the
        // gate, or of someone who lost access, is refused too: 401, and the web logs in again.
        if app.config.github_app.is_some() && !team::record_is_fresh(team_checked_at, Utc::now()) {
            app.reject(parts.uri.path(), reason::TEAM_RECORD_STALE, &login);
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(CurrentUser(login))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

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

    /// ADR 0062: a client, interactive or headless, never approves. A companion's pairing token
    /// is not a web session: every web route, the plan approval among them, answers 401.
    #[tokio::test]
    async fn a_companions_token_is_not_a_web_session() {
        use crate::kernel::config::Config;
        use crate::kernel::github::FakeGitHub;
        use crate::kernel::state::AppState;
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let mut cfg = Config::for_tests(url, "m0x6A");
        cfg.dev_user = None;
        let app = AppState::new(pool, cfg, std::sync::Arc::new(FakeGitHub::new()));
        let req = axum::http::Request::builder()
            .method("POST")
            .uri(rubato_protocol::approve::routes::PLAN_DRAFT_APPROVE)
            .header("authorization", "Bearer a-companion-pairing-token")
            .body(())
            .unwrap();
        let (mut parts, ()) = req.into_parts();
        let got = CurrentUser::from_request_parts(&mut parts, &app).await;
        assert_eq!(got.unwrap_err(), StatusCode::UNAUTHORIZED);
    }
}
