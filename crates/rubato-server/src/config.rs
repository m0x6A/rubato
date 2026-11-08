//! Server configuration from the environment.

use anyhow::{Context, Result};
use rubato_protocol::Limits;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// External URL, for OAuth callbacks and the pairing verification link.
    pub public_url: String,
    pub database_url: String,
    pub github_client_id: Option<String>,
    pub github_client_secret: Option<String>,
    /// Token the server itself uses to read repos, plans, PRs and comments.
    pub github_token: Option<String>,
    /// `owner/name` of the repo holding `system-map.yml` (ADR 0010).
    pub map_repo: String,
    pub map_path: String,
    pub map_ref: String,
    /// Local file to load the map from instead of GitHub (local runs and e2e tests).
    pub map_file: Option<PathBuf>,
    pub session_secret: String,
    /// Development only: every request is this login, no OAuth. Never set in production.
    pub dev_user: Option<String>,
    pub web_dist: PathBuf,
    pub limits: Limits,
    pub otel_endpoint: Option<String>,
    /// Default caps for a launch when the plan has none.
    pub default_spend_cap_usd: f64,
    pub default_attempt_limit: u32,
    pub pairing_ttl_secs: i64,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind = env("RUBATO_BIND")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()
            .context("RUBATO_BIND")?;
        let public_url = env("RUBATO_PUBLIC_URL").unwrap_or_else(|| format!("http://{bind}"));
        let mut limits = Limits::default();
        if let Some(v) = env("RUBATO_SESSION_STALE_SECS").and_then(|v| v.parse().ok()) {
            limits.session_stale_secs = v;
        }
        if let Some(v) = env("RUBATO_COMPANION_SILENT_SECS").and_then(|v| v.parse().ok()) {
            limits.companion_silent_secs = v;
        }
        if let Some(v) = env("RUBATO_GITHUB_POLL_SECS").and_then(|v| v.parse().ok()) {
            limits.github_poll_interval_secs = v;
        }
        Ok(Self {
            bind,
            public_url,
            database_url: env("RUBATO_DATABASE_URL")
                .unwrap_or_else(|| "sqlite://rubato.db?mode=rwc".into()),
            github_client_id: env("GITHUB_CLIENT_ID"),
            github_client_secret: env("GITHUB_CLIENT_SECRET"),
            github_token: env("RUBATO_GITHUB_TOKEN"),
            map_repo: env("RUBATO_MAP_REPO").unwrap_or_else(|| "m0x6A/rubato".into()),
            map_path: env("RUBATO_MAP_PATH").unwrap_or_else(|| "docs/design/rubato-map.yml".into()),
            map_ref: env("RUBATO_MAP_REF").unwrap_or_else(|| "main".into()),
            map_file: env("RUBATO_MAP_FILE").map(PathBuf::from),
            session_secret: env("RUBATO_SESSION_SECRET").unwrap_or_else(|| {
                tracing::warn!(
                    "RUBATO_SESSION_SECRET unset; web sessions will not survive a restart"
                );
                uuid::Uuid::new_v4().to_string()
            }),
            dev_user: env("RUBATO_DEV_USER"),
            web_dist: env("RUBATO_WEB_DIST")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("web/dist")),
            limits,
            otel_endpoint: env("OTEL_EXPORTER_OTLP_ENDPOINT"),
            default_spend_cap_usd: env("RUBATO_DEFAULT_SPEND_CAP_USD")
                .and_then(|v| v.parse().ok())
                .unwrap_or(25.0),
            default_attempt_limit: env("RUBATO_DEFAULT_ATTEMPT_LIMIT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            pairing_ttl_secs: 900,
        })
    }

    /// A config for tests and local runs with no GitHub at all.
    pub fn for_tests(database_url: String, dev_user: &str) -> Self {
        Self {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_url: "http://test".into(),
            database_url,
            github_client_id: None,
            github_client_secret: None,
            github_token: None,
            map_repo: "m0x6A/rubato".into(),
            map_path: "docs/design/rubato-map.yml".into(),
            map_ref: "main".into(),
            map_file: None,
            session_secret: "test-secret".into(),
            dev_user: Some(dev_user.into()),
            web_dist: PathBuf::from("web/dist"),
            limits: Limits::default(),
            otel_endpoint: None,
            default_spend_cap_usd: 25.0,
            default_attempt_limit: 3,
            pairing_ttl_secs: 900,
        }
    }
}
