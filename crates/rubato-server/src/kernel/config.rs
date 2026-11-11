//! Server configuration from the environment.

use anyhow::{Context, Result};
use rubato_protocol::Limits;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The longest poll interval: the `ServerPollSilent` window is 10 minutes (design D14).
pub const MAX_POLL_SECS: u64 = 300;

/// The GitHub App the server acts as (ADR 0022). All three settings or none.
#[derive(Clone, PartialEq, Eq)]
pub struct GitHubApp {
    pub app_id: u64,
    /// The private key, PEM text.
    pub private_key: String,
    pub installation_id: u64,
}

impl std::fmt::Debug for GitHubApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubApp")
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .finish_non_exhaustive()
    }
}

/// Read the App settings: all of id, key and installation, or none of them.
fn github_app(
    id: Option<String>,
    key: Option<String>,
    key_file: Option<String>,
    installation: Option<String>,
) -> Result<Option<GitHubApp>> {
    let key = match (key, key_file) {
        (Some(k), _) => Some(k.replace("\\n", "\n")),
        (None, Some(path)) => Some(
            std::fs::read_to_string(&path)
                .with_context(|| format!("RUBATO_GITHUB_APP_PRIVATE_KEY_FILE {path}"))?,
        ),
        (None, None) => None,
    };
    match (id, key, installation) {
        (None, None, None) => Ok(None),
        (Some(id), Some(private_key), Some(installation)) => Ok(Some(GitHubApp {
            app_id: id.parse().context("RUBATO_GITHUB_APP_ID is not a number")?,
            private_key,
            installation_id: installation
                .parse()
                .context("RUBATO_GITHUB_APP_INSTALLATION_ID is not a number")?,
        })),
        _ => anyhow::bail!(
            "the GitHub App needs RUBATO_GITHUB_APP_ID, RUBATO_GITHUB_APP_PRIVATE_KEY (or _FILE) and RUBATO_GITHUB_APP_INSTALLATION_ID together"
        ),
    }
}

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
    /// The GitHub App the server acts as. When set it replaces `github_token`, and a session
    /// needs a team record (ADR 0022).
    pub github_app: Option<GitHubApp>,
    /// Development and tests only: a JSON file `{"owner/name": [{number, title, url, labels}]}`
    /// giving the local stand-in and the tokenless fake their open issues.
    pub dev_issues: Option<PathBuf>,
    /// `owner/name=/path/to/repo.git,...`: read plans and branches from local repos instead of GitHub.
    pub local_repos: Option<String>,
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
    /// Problems found while reading the environment. `main` logs each as `warn!` once the
    /// subscriber exists, since `from_env` runs before it does (design D14).
    pub warnings: Vec<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let mut warnings = Vec::new();
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
            limits.github_poll_interval_secs = clamp_poll_secs(v, &mut warnings);
        }
        let session_secret = env("RUBATO_SESSION_SECRET").unwrap_or_else(|| {
            warnings.push(
                "RUBATO_SESSION_SECRET unset; web sessions will not survive a restart".into(),
            );
            uuid::Uuid::new_v4().to_string()
        });
        Ok(Self {
            bind,
            public_url,
            database_url: env("RUBATO_DATABASE_URL")
                .unwrap_or_else(|| "sqlite://rubato.db?mode=rwc".into()),
            github_client_id: env("GITHUB_CLIENT_ID"),
            github_client_secret: env("GITHUB_CLIENT_SECRET"),
            github_token: env("RUBATO_GITHUB_TOKEN"),
            github_app: github_app(
                env("RUBATO_GITHUB_APP_ID"),
                env("RUBATO_GITHUB_APP_PRIVATE_KEY"),
                env("RUBATO_GITHUB_APP_PRIVATE_KEY_FILE"),
                env("RUBATO_GITHUB_APP_INSTALLATION_ID"),
            )?,
            dev_issues: env("RUBATO_DEV_ISSUES").map(PathBuf::from),
            local_repos: env("RUBATO_LOCAL_REPOS"),
            map_repo: env("RUBATO_MAP_REPO").unwrap_or_else(|| "m0x6A/rubato".into()),
            map_path: env("RUBATO_MAP_PATH").unwrap_or_else(|| "docs/design/rubato-map.yml".into()),
            map_ref: env("RUBATO_MAP_REF").unwrap_or_else(|| "main".into()),
            map_file: env("RUBATO_MAP_FILE").map(PathBuf::from),
            session_secret,
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
            warnings,
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
            github_app: None,
            dev_issues: None,
            local_repos: None,
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
            warnings: Vec::new(),
        }
    }
}

/// Clamp the poll interval to `MAX_POLL_SECS`, noting it once in `warnings`.
fn clamp_poll_secs(v: u64, warnings: &mut Vec<String>) -> u64 {
    if v > MAX_POLL_SECS {
        warnings.push(format!(
            "RUBATO_GITHUB_POLL_SECS={v} is above {MAX_POLL_SECS}; polling every {MAX_POLL_SECS} s"
        ));
        MAX_POLL_SECS
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_needs_all_three_settings_or_none() {
        let s = |v: &str| Some(v.to_owned());
        assert_eq!(github_app(None, None, None, None).unwrap(), None);
        let app = github_app(s("12"), s("-----BEGIN\\nKEY"), None, s("34"))
            .unwrap()
            .unwrap();
        assert_eq!((app.app_id, app.installation_id), (12, 34));
        assert_eq!(
            app.private_key, "-----BEGIN\nKEY",
            "escaped newlines are read"
        );
        assert!(
            !format!("{app:?}").contains("BEGIN"),
            "the key is not printed"
        );
        for partial in [
            github_app(s("12"), None, None, None),
            github_app(None, s("k"), None, s("34")),
            github_app(s("12"), s("k"), None, None),
        ] {
            assert!(partial.unwrap_err().to_string().contains("together"));
        }
        assert!(github_app(s("x"), s("k"), None, s("34")).is_err());
    }

    #[test]
    fn poll_interval_above_300_is_clamped_with_a_warning() {
        let mut w = Vec::new();
        assert_eq!(clamp_poll_secs(3600, &mut w), 300);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("3600"), "{w:?}");
        let mut w = Vec::new();
        assert_eq!(clamp_poll_secs(300, &mut w), 300);
        assert_eq!(clamp_poll_secs(30, &mut w), 30);
        assert!(w.is_empty(), "{w:?}");
    }
}
