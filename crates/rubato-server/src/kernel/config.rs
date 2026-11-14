//! Server configuration from the environment.

use anyhow::{Context, Result};
use rubato_protocol::Limits;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The longest poll interval: the `ServerPollSilent` window is 10 minutes (design D14).
pub const MAX_POLL_SECS: u64 = 300;

/// The traffic fetch interval's range and default (plan #104): a report is stale after three.
pub const TRAFFIC_FETCH_SECS: (u32, u32, u32) = (15, 60, 300);
/// The shortest companion stale threshold N (#105): below it a typo would page for every
/// companion and empty every freshness bar.
pub const MIN_COMPANION_SILENT_SECS: u64 = 60;

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

/// The key user-to-server tokens are encrypted with (`RUBATO_TOKEN_KEY`, 64 hex characters,
/// ADR 0030). Never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenKey(pub [u8; 32]);

impl std::fmt::Debug for TokenKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenKey(..)")
    }
}

impl TokenKey {
    pub fn from_hex(text: &str) -> Result<Self> {
        let t = text.trim();
        anyhow::ensure!(
            t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()),
            "RUBATO_TOKEN_KEY must be 64 hex characters (32 bytes)"
        );
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = u8::from_str_radix(&t[2 * i..2 * i + 2], 16)?;
        }
        Ok(Self(key))
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
    /// Encrypts the user-to-server tokens Rubato keeps to act as a person (ADR 0030). Unset:
    /// no token is kept and nobody can accept a plan from Rubato.
    pub token_key: Option<TokenKey>,
    /// Token the server itself uses to read repos, plans, PRs and comments.
    pub github_token: Option<String>,
    /// The GitHub App the server acts as. When set it replaces `github_token`, and a session
    /// needs a team record (ADR 0022).
    pub github_app: Option<GitHubApp>,
    /// Another GitHub than github.com: the REST API root (`RUBATO_GITHUB_API_URL`) and the web
    /// root that serves the OAuth pages (`RUBATO_GITHUB_WEB_URL`). For a GitHub Enterprise
    /// Server, and for the stand-in of the browser tests. Unset means github.com.
    pub github_api_url: Option<String>,
    pub github_web_url: Option<String>,
    /// Development and tests only: a JSON file `{"owner/name": [{number, title, url, labels}]}`
    /// giving the local stand-in and the tokenless fake their open issues.
    pub dev_issues: Option<PathBuf>,
    /// `owner/name=/path/to/repo.git,...`: read plans and branches from local repos instead of GitHub.
    pub local_repos: Option<String>,
    /// Development and tests only: a JSON `TrafficReport` file, read every fetch interval, the
    /// only traffic source so far (ADR 0039, plan #104 Q1 (d)). Unset: traffic not configured.
    pub dev_traffic: Option<PathBuf>,
    /// Seconds between traffic fetches (`RUBATO_TRAFFIC_FETCH_SECS`, 15–300, default 60).
    pub traffic_fetch_secs: u32,
    /// `owner/name` of the repo holding `system-map.yml` (ADR 0010).
    pub map_repo: String,
    pub map_path: String,
    pub map_ref: String,
    /// Local file to load the map from instead of GitHub (local runs and e2e tests).
    pub map_file: Option<PathBuf>,
    /// The workspaces from `RUBATO_WORKSPACES_FILE` (ADR 0044). Empty: the `map_*` fields above
    /// are the one workspace `default`. Set: they are copied from its `default` entry.
    pub workspaces: Vec<crate::kernel::workspaces::WorkspaceEntry>,
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
            limits.companion_silent_secs = floor_companion_silent_secs(v, &mut warnings);
        }
        if let Some(v) = env("RUBATO_GITHUB_POLL_SECS").and_then(|v| v.parse().ok()) {
            limits.github_poll_interval_secs = clamp_poll_secs(v, &mut warnings);
        }
        let workspaces = match env("RUBATO_WORKSPACES_FILE") {
            Some(path) => {
                let legacy: Vec<&str> = [
                    "RUBATO_MAP_REPO",
                    "RUBATO_MAP_PATH",
                    "RUBATO_MAP_REF",
                    "RUBATO_MAP_FILE",
                ]
                .into_iter()
                .filter(|k| env(k).is_some())
                .collect();
                let path = PathBuf::from(path);
                let text = std::fs::read_to_string(&path).with_context(|| {
                    format!("reading RUBATO_WORKSPACES_FILE {}", path.display())
                })?;
                crate::kernel::workspaces::from_file(&text, &path, &legacy)?
            }
            None => vec![],
        };
        let default_entry = workspaces
            .iter()
            .find(|w| w.id == rubato_protocol::DEFAULT_WORKSPACE)
            .cloned();
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
            token_key: env("RUBATO_TOKEN_KEY")
                .map(|k| TokenKey::from_hex(&k))
                .transpose()?,
            github_token: env("RUBATO_GITHUB_TOKEN"),
            github_app: github_app(
                env("RUBATO_GITHUB_APP_ID"),
                env("RUBATO_GITHUB_APP_PRIVATE_KEY"),
                env("RUBATO_GITHUB_APP_PRIVATE_KEY_FILE"),
                env("RUBATO_GITHUB_APP_INSTALLATION_ID"),
            )?,
            github_api_url: env("RUBATO_GITHUB_API_URL").map(trim_slash),
            github_web_url: env("RUBATO_GITHUB_WEB_URL").map(trim_slash),
            dev_issues: env("RUBATO_DEV_ISSUES").map(PathBuf::from),
            local_repos: env("RUBATO_LOCAL_REPOS"),
            dev_traffic: env("RUBATO_DEV_TRAFFIC").map(PathBuf::from),
            traffic_fetch_secs: traffic_fetch_secs(
                env("RUBATO_TRAFFIC_FETCH_SECS").as_deref(),
                &mut warnings,
            ),
            map_repo: default_entry
                .as_ref()
                .map(|w| w.map_repo.clone())
                .or_else(|| env("RUBATO_MAP_REPO"))
                .unwrap_or_else(|| "m0x6A/rubato".into()),
            map_path: default_entry
                .as_ref()
                .map(|w| w.map_path.clone())
                .or_else(|| env("RUBATO_MAP_PATH"))
                .unwrap_or_else(|| "docs/design/rubato-map.yml".into()),
            map_ref: default_entry
                .as_ref()
                .map(|w| w.map_ref.clone())
                .or_else(|| env("RUBATO_MAP_REF"))
                .unwrap_or_else(|| "main".into()),
            map_file: match &default_entry {
                Some(w) => w.map_file.clone(),
                None => env("RUBATO_MAP_FILE").map(PathBuf::from),
            },
            workspaces,
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

    /// The REST API root, without a trailing slash.
    /// The configured workspaces, in switcher order, and where they came from: the file, or
    /// the `map_*` fields as the one workspace `default`.
    pub fn workspace_entries(
        &self,
    ) -> (
        Vec<crate::kernel::workspaces::WorkspaceEntry>,
        crate::kernel::workspaces::Source,
    ) {
        use crate::kernel::workspaces::{Source, WorkspaceEntry};
        if self.workspaces.is_empty() {
            let w = WorkspaceEntry {
                id: rubato_protocol::DEFAULT_WORKSPACE.to_string(),
                name: None,
                map_repo: self.map_repo.clone(),
                map_path: self.map_path.clone(),
                map_ref: self.map_ref.clone(),
                map_file: self.map_file.clone(),
            };
            (vec![w], Source::LegacyEnv)
        } else {
            (self.workspaces.clone(), Source::File)
        }
    }

    pub fn github_api(&self) -> &str {
        self.github_api_url
            .as_deref()
            .unwrap_or("https://api.github.com")
    }

    /// The web root that serves `/login/oauth/*`, without a trailing slash.
    pub fn github_web(&self) -> &str {
        self.github_web_url
            .as_deref()
            .unwrap_or("https://github.com")
    }

    /// A config for tests and local runs with no GitHub at all.
    pub fn for_tests(database_url: String, dev_user: &str) -> Self {
        Self {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_url: "http://test".into(),
            database_url,
            github_client_id: None,
            github_client_secret: None,
            token_key: None,
            github_token: None,
            github_app: None,
            github_api_url: None,
            github_web_url: None,
            dev_issues: None,
            local_repos: None,
            dev_traffic: None,
            traffic_fetch_secs: TRAFFIC_FETCH_SECS.1,
            map_repo: "m0x6A/rubato".into(),
            map_path: "docs/design/rubato-map.yml".into(),
            map_ref: "main".into(),
            map_file: None,
            workspaces: vec![],
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

fn trim_slash(url: String) -> String {
    url.trim_end_matches('/').to_owned()
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

/// `RUBATO_TRAFFIC_FETCH_SECS`, clamped to 15–300; unset or unreadable is 60.
fn traffic_fetch_secs(v: Option<&str>, warnings: &mut Vec<String>) -> u32 {
    let (min, default, max) = TRAFFIC_FETCH_SECS;
    let Some(v) = v else { return default };
    match v.parse::<u32>() {
        Ok(n) if (min..=max).contains(&n) => n,
        Ok(n) => {
            let c = n.clamp(min, max);
            warnings.push(format!(
                "RUBATO_TRAFFIC_FETCH_SECS={n} is outside {min}–{max}; fetching every {c} s"
            ));
            c
        }
        Err(_) => {
            warnings.push(format!(
                "RUBATO_TRAFFIC_FETCH_SECS={v:?} is not a number; fetching every {default} s"
            ));
            default
        }
    }
}

/// Raise the companion stale threshold to `MIN_COMPANION_SILENT_SECS`, noting it once in
/// `warnings`.
fn floor_companion_silent_secs(v: u64, warnings: &mut Vec<String>) -> u64 {
    if v < MIN_COMPANION_SILENT_SECS {
        warnings.push(format!(
            "RUBATO_COMPANION_SILENT_SECS={v} is below {MIN_COMPANION_SILENT_SECS}; \
             a companion is stale after {MIN_COMPANION_SILENT_SECS} s"
        ));
        MIN_COMPANION_SILENT_SECS
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_fetch_interval_is_clamped_with_a_warning() {
        let mut w = Vec::new();
        assert_eq!(traffic_fetch_secs(None, &mut w), 60);
        assert_eq!(traffic_fetch_secs(Some("15"), &mut w), 15);
        assert_eq!(traffic_fetch_secs(Some("300"), &mut w), 300);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(traffic_fetch_secs(Some("5"), &mut w), 15);
        assert_eq!(traffic_fetch_secs(Some("3600"), &mut w), 300);
        assert_eq!(traffic_fetch_secs(Some("soon"), &mut w), 60);
        assert_eq!(w.len(), 3, "{w:?}");
    }

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
    fn github_is_github_com_unless_another_root_is_given() {
        let mut c = Config::for_tests("sqlite::memory:".into(), "m0x6A");
        assert_eq!(c.github_api(), "https://api.github.com");
        assert_eq!(c.github_web(), "https://github.com");
        c.github_api_url = Some(trim_slash("http://127.0.0.1:9/api/v3/".into()));
        c.github_web_url = Some(trim_slash("http://127.0.0.1:9/".into()));
        assert_eq!(c.github_api(), "http://127.0.0.1:9/api/v3");
        assert_eq!(c.github_web(), "http://127.0.0.1:9");
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

    /// #105: a typo cannot page for every companion or empty every bar.
    #[test]
    fn companion_threshold_below_60_is_raised_with_a_warning() {
        let mut w = Vec::new();
        assert_eq!(floor_companion_silent_secs(30, &mut w), 60);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("30"), "{w:?}");
        let mut w = Vec::new();
        assert_eq!(floor_companion_silent_secs(300, &mut w), 300);
        assert_eq!(floor_companion_silent_secs(60, &mut w), 60);
        assert!(w.is_empty(), "{w:?}");
    }
}
