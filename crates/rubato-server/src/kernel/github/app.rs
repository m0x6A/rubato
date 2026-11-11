//! The start-up check of the GitHub App settings (ADR 0022). A wrong key, a client id from
//! another registration or an installation id that mints no token would otherwise show up
//! later as "no access" for everyone, which sends on-call to the wrong cause.

use super::api::{READ_TIMEOUT, app_client};
use crate::kernel::config::GitHubApp;
use std::time::Duration;

/// Which setting does not fit. The message names it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppCheckError {
    #[error("the private key is not usable: {0}")]
    Key(String),
    #[error("GitHub did not accept the App id and key: {0}")]
    App(String),
    #[error(
        "GITHUB_CLIENT_ID is {configured:?} but the App's client id is {actual:?}; logins must go through the App's own registration"
    )]
    ClientId { configured: String, actual: String },
    #[error("installation {installation} mints no token for this App: {reason}")]
    Installation { installation: u64, reason: String },
}

/// Check that key, client id and installation belong together. One call as the App
/// (`GET /app`) and one as the installation.
pub async fn check_app(
    app: &GitHubApp,
    client_id: Option<&str>,
    base_uri: Option<&str>,
) -> Result<(), AppCheckError> {
    check_app_with_timeout(app, client_id, base_uri, READ_TIMEOUT).await
}

async fn check_app_with_timeout(
    app: &GitHubApp,
    client_id: Option<&str>,
    base_uri: Option<&str>,
    read_timeout: Duration,
) -> Result<(), AppCheckError> {
    let crab = app_client(app, base_uri, read_timeout)
        .map_err(|e| AppCheckError::Key(format!("{e:#}")))?;
    let me: serde_json::Value = crab
        .get("/app", None::<&()>)
        .await
        .map_err(|e| AppCheckError::App(e.to_string()))?;
    let actual = me
        .get("client_id")
        .and_then(|c| c.as_str())
        .unwrap_or_default();
    if let Some(configured) = client_id
        && configured != actual
    {
        return Err(AppCheckError::ClientId {
            configured: configured.to_owned(),
            actual: actual.to_owned(),
        });
    }
    let installation = |reason: String| AppCheckError::Installation {
        installation: app.installation_id,
        reason,
    };
    let inst = crab
        .installation(octocrab::models::InstallationId(app.installation_id))
        .map_err(|e| installation(e.to_string()))?;
    inst.get::<serde_json::Value, _, ()>("/installation/repositories?per_page=1", None::<&()>)
        .await
        .map_err(|e| installation(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;

    /// A throwaway RSA key in PEM form. Generated per test run, so no key is committed.
    pub(crate) fn test_key() -> String {
        let out = std::process::Command::new("openssl")
            .args(["genrsa", "2048"])
            .output()
            .expect("openssl is needed to make a test key");
        assert!(out.status.success(), "openssl genrsa failed");
        String::from_utf8(out.stdout).unwrap()
    }

    /// A stand-in for api.github.com that knows one App with one installation (34).
    pub(crate) async fn github_standin() -> String {
        let router = Router::new()
            .route(
                "/app",
                get(|| async { Json(json!({"id": 12, "client_id": "Iv1.app"})) }),
            )
            .route(
                "/app/installations/34/access_tokens",
                post(|| async { Json(json!({"token": "t", "permissions": {}})) }),
            )
            .route(
                "/installation/repositories",
                get(|| async {
                    Json(json!({"total_count": 1, "repositories": [{"full_name": "o/covered"}]}))
                }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn startup_check_names_what_is_wrong() {
        let base = github_standin().await;
        let good = GitHubApp {
            app_id: 12,
            private_key: test_key(),
            installation_id: 34,
        };
        let t = Duration::from_secs(2);
        let run = |app: GitHubApp, client: Option<&'static str>| {
            let base = base.clone();
            async move { check_app_with_timeout(&app, client, Some(&base), t).await }
        };

        assert_eq!(run(good.clone(), Some("Iv1.app")).await, Ok(()));
        assert_eq!(run(good.clone(), None).await, Ok(()), "no OAuth client");

        let bad_key = GitHubApp {
            private_key: "not a key".into(),
            ..good.clone()
        };
        let e = run(bad_key, Some("Iv1.app")).await.unwrap_err();
        assert!(matches!(e, AppCheckError::Key(_)), "{e}");
        assert!(e.to_string().contains("private key"), "{e}");

        // The client id of the old OAuth app, not the App's own.
        let e = run(good.clone(), Some("Ov23.oauth")).await.unwrap_err();
        assert_eq!(
            e,
            AppCheckError::ClientId {
                configured: "Ov23.oauth".into(),
                actual: "Iv1.app".into()
            }
        );
        assert!(e.to_string().contains("GITHUB_CLIENT_ID"), "{e}");

        let wrong_installation = GitHubApp {
            installation_id: 99,
            ..good
        };
        let e = run(wrong_installation, Some("Iv1.app")).await.unwrap_err();
        assert!(
            matches!(
                e,
                AppCheckError::Installation {
                    installation: 99,
                    ..
                }
            ),
            "{e}"
        );
        println!("checked 5 App configurations");
    }
}
