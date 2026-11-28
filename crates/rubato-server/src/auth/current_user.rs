//! The logged-in user, from the signed session cookie (or the dev user in development).

use super::store;
use super::team::{self, reason};
use crate::kernel::state::App;
use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, StatusCode, request::Parts};
use chrono::Utc;

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    crate::kernel::http::cookie(headers, name)
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
        let value = cookie_value(&parts.headers, app.config.session_cookie())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let id = app.signer.verify(&value).ok_or(StatusCode::UNAUTHORIZED)?;
        let Ok(Some(session)) = store::web_session(&app.db, &id).await else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        let now = Utc::now();
        // Who the person is; what they may reach is their memberships, read on every tenant
        // request (ADR 0088, superseding ADR 0022's team record). A session ends with its cookie,
        // after 30 days (ADR 0064).
        if !team::session_is_current(session.created_at, now) {
            // The route's template, never the concrete path (ADR 0090): a tenant id or a
            // probe's path does not become a label value.
            let route = parts
                .extensions
                .get::<axum::extract::MatchedPath>()
                .map_or("unmatched", |m| m.as_str())
                .to_owned();
            app.reject(&route, reason::SESSION_EXPIRED, &session.login);
            return Err(StatusCode::UNAUTHORIZED);
        }
        app.limiter.know(&id, &session.login);
        Ok(CurrentUser(session.login))
    }
}

/// At start: tell the write limit whose each stored session is, so a person's sessions share
/// one bucket from the first write after a restart (ADR 0090). Returns how many it told. A
/// read that fails is logged; each session is then learned at its first admitted request.
pub async fn start(app: &App) -> usize {
    match store::web_sessions(&app.db).await {
        Ok(sessions) => {
            for (id, login) in &sessions {
                app.limiter.know(id, login);
            }
            sessions.len()
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the web sessions for the write limit failed");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header};

    #[test]
    fn reads_a_cookie_among_many() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; rubato_session=x.y; b=2"),
        );
        assert_eq!(cookie_value(&h, "rubato_session").as_deref(), Some("x.y"));
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

    /// After a restart the write limit knows whose each stored session is before any request:
    /// two sessions of one person draw on one bucket, whatever the case of the login.
    #[tokio::test]
    async fn at_start_the_write_limit_learns_the_stored_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let app = crate::kernel::state::AppState::new(
            pool,
            crate::kernel::config::Config::for_tests(url, "m0x6A"),
            std::sync::Arc::new(crate::kernel::github::FakeGitHub::new()),
        );
        let s1 = store::create_web_session(&app.db, "Ada", None)
            .await
            .unwrap();
        let s2 = store::create_web_session(&app.db, "ada", None)
            .await
            .unwrap();
        let s3 = store::create_web_session(&app.db, "mia", None)
            .await
            .unwrap();
        assert_eq!(app.limiter.writer(&s1), format!("session:{s1}"));
        assert_eq!(start(&app).await, 3);
        assert_eq!(app.limiter.writer(&s1), "person:ada");
        assert_eq!(app.limiter.writer(&s2), "person:ada");
        assert_eq!(app.limiter.writer(&s3), "person:mia");
        assert_eq!(app.limiter.writer("unknown"), "session:unknown");
        println!("checked 3 stored sessions and 1 unknown one");
    }
}
