//! Server configuration from the environment.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
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
/// The catch-up interval's range and default with webhooks on (ADR 0074).
pub const CATCHUP_SECS: (u64, u64, u64) = (300, 900, 3600);

/// The GitHub App's webhook secret (`RUBATO_GITHUB_WEBHOOK_SECRET`, ADR 0074). Read once into the
/// config; `Debug` never prints it, and nothing logs, stores or sends it.
#[derive(Clone)]
pub struct WebhookSecret(String);

impl WebhookSecret {
    /// Unset or empty is `None`: webhooks are off.
    pub fn from_env_value(v: Option<String>) -> Option<Self> {
        v.filter(|s| !s.is_empty()).map(Self)
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl std::fmt::Debug for WebhookSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// A credential read from the environment (ADR 0073: the LiveKit secret, the speech keys).
/// `Debug` never prints it; nothing logs, stores or sends it but the call it is for.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// A plan room's voice channel (ADR 0073), present only with `RUBATO_VOICE=1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceConfig {
    /// The team's own LiveKit; `None` makes the token route answer `no_livekit`.
    pub livekit: Option<LiveKitConfig>,
    /// The speech provider; `None` is voice for people only.
    pub speech: Option<SpeechConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitConfig {
    /// `wss://…` (or `ws://` on a LAN test).
    pub url: String,
    pub api_key: String,
    pub api_secret: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechConfig {
    /// Tests and the dev server only: echoes text (design 146 §5).
    Fake,
    OpenAiCompatible(OpenAiSpeech),
}

/// The `openai_compatible` adapter's settings, STT and TTS on their own base URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiSpeech {
    pub stt_url: String,
    pub tts_url: String,
    pub stt_key: Option<Secret>,
    pub tts_key: Option<Secret>,
    pub stt_model: String,
    pub tts_model: String,
    pub tts_voice: String,
    pub language: Option<String>,
}

/// The shortest LiveKit secret Rubato takes: LiveKit itself refuses shorter ones.
pub const MIN_LIVEKIT_SECRET_CHARS: usize = 32;

/// Voice settings from the environment (ADR 0073). Off unless `RUBATO_VOICE` is `1` or `true`.
/// A half-configured LiveKit or speech provider stops the start, naming what is missing, and the
/// fake adapter needs `RUBATO_DEV_USER`, which ADR 0064 keeps to loopback.
pub fn voice_from(
    get: impl Fn(&str) -> Option<String>,
    dev_user: bool,
) -> Result<Option<VoiceConfig>> {
    match get("RUBATO_VOICE").as_deref().map(str::trim) {
        None | Some("0") | Some("false") => return Ok(None),
        Some("1") | Some("true") => {}
        Some(other) => anyhow::bail!("RUBATO_VOICE={other:?}: use 1 or 0"),
    }
    let trio = [
        "RUBATO_LIVEKIT_URL",
        "RUBATO_LIVEKIT_API_KEY",
        "RUBATO_LIVEKIT_API_SECRET",
    ];
    let set: Vec<Option<String>> = trio.iter().map(|k| get(k)).collect();
    let livekit = if set.iter().all(Option::is_none) {
        None
    } else {
        let missing: Vec<&str> = trio
            .iter()
            .zip(&set)
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| *k)
            .collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "RUBATO_VOICE=1 with LiveKit half configured: set {}",
                missing.join(", ")
            );
        }
        let url = set[0].clone().unwrap_or_default();
        if !(url.starts_with("wss://") || url.starts_with("ws://")) {
            anyhow::bail!("RUBATO_LIVEKIT_URL must be wss://… (or ws:// on a LAN test)");
        }
        let secret = set[2].clone().unwrap_or_default();
        if secret.chars().count() < MIN_LIVEKIT_SECRET_CHARS {
            anyhow::bail!(
                "RUBATO_LIVEKIT_API_SECRET must be at least {MIN_LIVEKIT_SECRET_CHARS} characters"
            );
        }
        Some(LiveKitConfig {
            url: trim_slash(url),
            api_key: set[1].clone().unwrap_or_default(),
            api_secret: Secret::new(secret),
        })
    };
    let speech = match get("RUBATO_SPEECH").as_deref().map(str::trim) {
        None => None,
        Some("fake") if dev_user => Some(SpeechConfig::Fake),
        Some("fake") => anyhow::bail!(
            "RUBATO_SPEECH=fake is for tests and the dev server: it needs RUBATO_DEV_USER"
        ),
        Some("openai_compatible") => {
            let need = [
                "RUBATO_STT_URL",
                "RUBATO_TTS_URL",
                "RUBATO_STT_MODEL",
                "RUBATO_TTS_MODEL",
                "RUBATO_TTS_VOICE",
            ];
            let missing: Vec<&str> = need.iter().filter(|k| get(k).is_none()).copied().collect();
            if !missing.is_empty() {
                anyhow::bail!(
                    "RUBATO_SPEECH=openai_compatible: set {}",
                    missing.join(", ")
                );
            }
            let v = |k: &str| get(k).unwrap_or_default();
            Some(SpeechConfig::OpenAiCompatible(OpenAiSpeech {
                stt_url: trim_slash(v("RUBATO_STT_URL")),
                tts_url: trim_slash(v("RUBATO_TTS_URL")),
                stt_key: get("RUBATO_STT_API_KEY").map(Secret::new),
                tts_key: get("RUBATO_TTS_API_KEY").map(Secret::new),
                stt_model: v("RUBATO_STT_MODEL"),
                tts_model: v("RUBATO_TTS_MODEL"),
                tts_voice: v("RUBATO_TTS_VOICE"),
                language: get("RUBATO_SPEECH_LANGUAGE"),
            }))
        }
        Some(other) => {
            anyhow::bail!("RUBATO_SPEECH={other:?}: use openai_compatible or fake")
        }
    };
    Ok(Some(VoiceConfig { livekit, speech }))
}

/// The host of a URL, for the consent text: `speaches:8000`, `api.openai.com`.
fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_owned(),
    })
}

/// The traffic adapter's own settings, `RUBATO_TRAFFIC_PROMETHEUS_*` and
/// `RUBATO_TRAFFIC_WINDOW_SECS`, kept as text for the slice to read (ADR 0040): what they mean,
/// and which are credentials, is the adapter's business. Debug prints the names only.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TrafficEnv(pub std::collections::BTreeMap<String, String>);

impl TrafficEnv {
    /// The names read into it: these prefixes.
    pub const PREFIXES: [&str; 2] = ["RUBATO_TRAFFIC_PROMETHEUS_", "RUBATO_TRAFFIC_WINDOW_SECS"];

    pub fn from_vars(vars: impl Iterator<Item = (String, String)>) -> Self {
        Self(
            vars.filter(|(k, v)| !v.is_empty() && Self::PREFIXES.iter().any(|p| k.starts_with(p)))
                .collect(),
        )
    }

    pub fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

impl std::fmt::Debug for TrafficEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

/// A launch's spend cap when neither the plan nor its workspace sets one (ADR 0075): built in,
/// never an application setting.
pub const BUILT_IN_SPEND_CAP_USD: f64 = 25.0;
/// A launch's attempt limit when neither the plan nor its workspace sets one.
pub const BUILT_IN_ATTEMPT_LIMIT: u32 = 3;

/// The workspace-scoped variables as set (ADR 0075): `None` when unset or not a value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeedEnv {
    pub default_spend_cap_usd: Option<f64>,
    pub default_attempt_limit: Option<u32>,
    /// `RUBATO_TRAFFIC_FETCH_SECS`, clamped as the traffic task reads it.
    pub traffic_fetch_secs: Option<u32>,
    /// Which of `RUBATO_MAP_REPO`, `RUBATO_MAP_PATH` and `RUBATO_MAP_REF` are set.
    pub map_vars: Vec<String>,
}

/// One traffic source at most (ADR 0040): both is a startup error, not a silent choice.
fn one_traffic_source(dev_file: bool, traffic: &TrafficEnv) -> Result<()> {
    if dev_file && traffic.get("RUBATO_TRAFFIC_PROMETHEUS_URL").is_some() {
        anyhow::bail!(
            "RUBATO_DEV_TRAFFIC and RUBATO_TRAFFIC_PROMETHEUS_URL are both set; a server reads one traffic source"
        );
    }
    Ok(())
}

/// Whether only this machine can reach the server (ADR 0064): it binds a loopback address and
/// its public URL names a loopback host. A URL whose host does not parse is not local.
pub fn is_local(bind: SocketAddr, public_url: &str) -> bool {
    let url_is_loopback = url::Url::parse(public_url).ok().is_some_and(|u| {
        u.host().is_some_and(|h| match h {
            url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
            url::Host::Ipv4(ip) => ip.is_loopback(),
            url::Host::Ipv6(ip) => ip.is_loopback(),
        })
    });
    bind.ip().is_loopback() && url_is_loopback
}

/// Refuse a server open to anyone unless it is local (ADR 0064): `RUBATO_DEV_USER` always,
/// OAuth without the App unless `RUBATO_OPEN_LOGIN=1`. Returns the warnings of a server that
/// starts open.
fn exposure(
    bind: SocketAddr,
    public_url: &str,
    dev_user: bool,
    oauth_without_app: bool,
    open_login: Option<&str>,
) -> Result<Vec<String>> {
    let opted_in = match open_login {
        None => false,
        Some("1") => true,
        Some(other) => anyhow::bail!("RUBATO_OPEN_LOGIN must be 1 or unset, not {other:?}"),
    };
    let local = is_local(bind, public_url);
    let mut warnings = Vec::new();
    if dev_user {
        anyhow::ensure!(
            local,
            "RUBATO_DEV_USER is set and the server is not local (bind {bind}, public URL {public_url}): \
             any client could act as anyone. Bind a loopback address with a loopback RUBATO_PUBLIC_URL, \
             or unset RUBATO_DEV_USER (ADR 0064)"
        );
        warnings.push(
            "RUBATO_DEV_USER is set: every request is that user, and x-rubato-dev-user names any other; local only".into(),
        );
    } else if oauth_without_app {
        anyhow::ensure!(
            local || opted_in,
            "OAuth is configured without the GitHub App and the server is not local (bind {bind}, \
             public URL {public_url}): any GitHub account could log in. Configure the App \
             (RUBATO_GITHUB_APP_*), or set RUBATO_OPEN_LOGIN=1 to run open (ADR 0064)"
        );
        warnings.push(if opted_in {
            "no GitHub App configured: login is open to any GitHub account (RUBATO_OPEN_LOGIN=1)"
                .into()
        } else {
            "no GitHub App configured: login is open to any GitHub account; local only".into()
        });
    }
    if opted_in && !oauth_without_app {
        warnings.push(
            "RUBATO_OPEN_LOGIN=1 has no effect: it applies only to OAuth without the GitHub App"
                .into(),
        );
    }
    Ok(warnings)
}

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
    /// Development and tests only: a JSON file `{"owner/name": [{number, branch, base, title,
    /// author}]}` giving the local stand-in open pull requests between its branches (issue #82).
    pub dev_pulls: Option<PathBuf>,
    /// `owner/name=/path/to/repo.git,...`: read plans and branches from local repos instead of GitHub.
    pub local_repos: Option<String>,
    /// Development and tests only: a JSON `TrafficReport` file, read every fetch interval, the
    /// only traffic source so far (ADR 0039, plan #104 Q1 (d)). Unset: traffic not configured.
    pub dev_traffic: Option<PathBuf>,
    /// Seconds between traffic fetches (`RUBATO_TRAFFIC_FETCH_SECS`, 15–300, default 60).
    pub traffic_fetch_secs: u32,
    /// The production source's settings (ADR 0040); configured when
    /// `RUBATO_TRAFFIC_PROMETHEUS_URL` is among them.
    pub traffic_env: TrafficEnv,
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
    /// The workspace-scoped variables as set, unset kept apart from set-to-the-default: what the
    /// first start copies into `workspace_settings`, and what later starts warn about while
    /// they are still set (ADR 0075).
    pub seed: SeedEnv,
    pub pairing_ttl_secs: i64,
    /// Pairing codes that may be pending and unexpired at once; a start above it is 429 (#168).
    pub pairing_max_pending: i64,
    /// The cutover (`RUBATO_COMMENT_APPROVALS_UNTIL`, RFC 3339, ADR 0054): `Approved:` comments
    /// count only when posted before it, *Accept plan* refuses after it, and approval in Rubato
    /// is on once it is set. Unset: comments count as before and approval in Rubato is refused.
    pub comment_approvals_until: Option<DateTime<Utc>>,
    /// The App's webhook secret; `Some` turns `POST /webhooks/github` on (ADR 0074).
    pub github_webhook_secret: Option<WebhookSecret>,
    /// With webhooks on, seconds between catch-up polls (`RUBATO_GITHUB_CATCHUP_SECS`, 300–3600,
    /// default 900). Unused with webhooks off.
    pub github_catchup_secs: u64,
    /// A plan room's voice channel (`RUBATO_VOICE`, ADR 0073); `None` while it is off, the
    /// default.
    pub voice: Option<VoiceConfig>,
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
        let dev_traffic = env("RUBATO_DEV_TRAFFIC").map(PathBuf::from);
        let traffic_env = TrafficEnv::from_vars(std::env::vars());
        one_traffic_source(dev_traffic.is_some(), &traffic_env)?;
        let session_secret = env("RUBATO_SESSION_SECRET").unwrap_or_else(|| {
            warnings.push(
                "RUBATO_SESSION_SECRET unset; web sessions will not survive a restart".into(),
            );
            uuid::Uuid::new_v4().to_string()
        });
        insecure_cookie_warning(&public_url, &mut warnings);
        let mut config = Self {
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
            dev_pulls: env("RUBATO_DEV_PULLS").map(PathBuf::from),
            local_repos: env("RUBATO_LOCAL_REPOS"),
            dev_traffic,
            traffic_fetch_secs: traffic_fetch_secs(
                env("RUBATO_TRAFFIC_FETCH_SECS").as_deref(),
                &mut warnings,
            ),
            traffic_env,
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
            seed: SeedEnv {
                default_spend_cap_usd: env("RUBATO_DEFAULT_SPEND_CAP_USD")
                    .and_then(|v| v.parse().ok()),
                default_attempt_limit: env("RUBATO_DEFAULT_ATTEMPT_LIMIT")
                    .and_then(|v| v.parse().ok()),
                traffic_fetch_secs: env("RUBATO_TRAFFIC_FETCH_SECS").is_some().then(|| {
                    traffic_fetch_secs(env("RUBATO_TRAFFIC_FETCH_SECS").as_deref(), &mut Vec::new())
                }),
                map_vars: ["RUBATO_MAP_REPO", "RUBATO_MAP_PATH", "RUBATO_MAP_REF"]
                    .into_iter()
                    .filter(|k| env(k).is_some())
                    .map(str::to_owned)
                    .collect(),
            },
            pairing_ttl_secs: 900,
            pairing_max_pending: rubato_protocol::pairing::PAIRING_CODES_MAX_PENDING,
            comment_approvals_until: comment_approvals_until(
                env("RUBATO_COMMENT_APPROVALS_UNTIL").as_deref(),
            )?,
            github_webhook_secret: WebhookSecret::from_env_value(env(
                "RUBATO_GITHUB_WEBHOOK_SECRET",
            )),
            github_catchup_secs: catchup_secs(
                env("RUBATO_GITHUB_CATCHUP_SECS").as_deref(),
                &mut warnings,
            ),
            voice: voice_from(env, env("RUBATO_DEV_USER").is_some())?,
            warnings,
        };
        let open = exposure(
            config.bind,
            &config.public_url,
            config.dev_user.is_some(),
            config.oauth_without_app(),
            env("RUBATO_OPEN_LOGIN").as_deref(),
        )?;
        config.warnings.extend(open);
        Ok(config)
    }

    /// The LiveKit URL while voice is on with LiveKit configured: what the CSP allows (ADR 0073).
    pub fn livekit_url(&self) -> Option<&str> {
        self.voice
            .as_ref()
            .and_then(|v| v.livekit.as_ref())
            .map(|l| l.url.as_str())
    }

    /// What `Me.voice` says: `None` while voice is off, so nothing changes for anyone (ADR 0073).
    pub fn voice_availability(&self) -> Option<rubato_protocol::VoiceAvailability> {
        let v = self.voice.as_ref()?;
        let mut hosts: Vec<String> = match &v.speech {
            Some(SpeechConfig::OpenAiCompatible(o)) => [&o.stt_url, &o.tts_url]
                .into_iter()
                .filter_map(|u| host_of(u))
                .collect(),
            _ => vec![],
        };
        hosts.dedup();
        Some(rubato_protocol::VoiceAvailability {
            enabled: v.livekit.is_some(),
            speech: v.speech.is_some(),
            speech_hosts: hosts,
        })
    }

    /// One line at start: voice off, or on with what is configured (never a secret).
    pub fn report_voice(&self) {
        match &self.voice {
            None => tracing::info!(voice = "off", "voice"),
            Some(v) => tracing::info!(
                voice = "on",
                livekit = v
                    .livekit
                    .as_ref()
                    .map(|l| l.url.as_str())
                    .unwrap_or("unset"),
                speech = match &v.speech {
                    None => "unset",
                    Some(SpeechConfig::Fake) => "fake",
                    Some(SpeechConfig::OpenAiCompatible(_)) => "openai_compatible",
                },
                "voice"
            ),
        }
    }

    /// Whether people log in through OAuth with no team check: an OAuth client without the
    /// GitHub App (ADR 0064).
    pub fn oauth_without_app(&self) -> bool {
        self.github_client_id.is_some()
            && self.github_client_secret.is_some()
            && self.github_app.is_none()
    }

    /// The `cutover` line at start, at info: the time, or `unset` (plan 122 §D10).
    pub fn report_cutover(&self) {
        match self.comment_approvals_until {
            Some(at) => tracing::info!(
                cutover = %at.to_rfc3339(),
                means = "approval comments count only before this time; plans are approved in Rubato",
                "cutover"
            ),
            None => tracing::info!(
                cutover = "unset",
                means = "approval comments count; approval in Rubato is off",
                "cutover"
            ),
        }
    }

    /// Whether `POST /webhooks/github` is on (ADR 0074).
    pub fn webhooks_on(&self) -> bool {
        self.github_webhook_secret.is_some()
    }

    /// One line at start: webhooks off (the poll as before), or on with the path and catch-up.
    pub fn report_webhooks(&self) {
        if self.webhooks_on() {
            tracing::info!(
                path = "/webhooks/github",
                catchup_secs = self.github_catchup_secs,
                "github webhooks on"
            );
        } else {
            tracing::info!(
                poll_secs = self.limits.github_poll_interval_secs,
                means = "RUBATO_GITHUB_WEBHOOK_SECRET unset: /webhooks/github answers 404 and the poll reads every tick",
                "github webhooks off"
            );
        }
    }

    /// Whether a traffic source is configured: the development file or the production one.
    pub fn traffic_configured(&self) -> bool {
        self.dev_traffic.is_some()
            || self
                .traffic_env
                .get("RUBATO_TRAFFIC_PROMETHEUS_URL")
                .is_some()
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
                settings: Default::default(),
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
            dev_pulls: None,
            local_repos: None,
            dev_traffic: None,
            traffic_fetch_secs: TRAFFIC_FETCH_SECS.1,
            traffic_env: TrafficEnv::default(),
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
            seed: SeedEnv::default(),
            pairing_ttl_secs: 900,
            pairing_max_pending: rubato_protocol::pairing::PAIRING_CODES_MAX_PENDING,
            comment_approvals_until: None,
            github_webhook_secret: None,
            github_catchup_secs: CATCHUP_SECS.1,
            voice: None,
            warnings: Vec::new(),
        }
    }
}

/// `RUBATO_COMMENT_APPROVALS_UNTIL`: an RFC 3339 time, or unset. A value that does not read
/// stops the start: a cutover silently unset would keep counting comments the team thinks no
/// longer count.
fn comment_approvals_until(v: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    v.map(|t| {
        DateTime::parse_from_rfc3339(t.trim())
            .map(|d| d.with_timezone(&Utc))
            .with_context(|| {
                format!("RUBATO_COMMENT_APPROVALS_UNTIL={t:?} is not an RFC 3339 time")
            })
    })
    .transpose()
}

fn trim_slash(url: String) -> String {
    url.trim_end_matches('/').to_owned()
}

/// `RUBATO_GITHUB_CATCHUP_SECS`, clamped to 300–3600; unset or unreadable is 900.
fn catchup_secs(v: Option<&str>, warnings: &mut Vec<String>) -> u64 {
    let (min, default, max) = CATCHUP_SECS;
    let Some(v) = v else { return default };
    match v.parse::<u64>() {
        Ok(n) if (min..=max).contains(&n) => n,
        Ok(n) => {
            let c = n.clamp(min, max);
            warnings.push(format!(
                "RUBATO_GITHUB_CATCHUP_SECS={n} is outside {min}–{max}; catching up every {c} s"
            ));
            c
        }
        Err(_) => {
            warnings.push(format!(
                "RUBATO_GITHUB_CATCHUP_SECS={v:?} is not a number; catching up every {default} s"
            ));
            default
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

/// Design 193 D3: an `http` public URL on a host that is not loopback sends the session cookie
/// without `Secure`. Warned, not refused: the team runs on plain http on a LAN (plan 193, Q1).
fn insecure_cookie_warning(public_url: &str, warnings: &mut Vec<String>) {
    let Ok(u) = url::Url::parse(public_url) else {
        return;
    };
    let loopback = match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => true,
    };
    if u.scheme() == "http" && !loopback {
        warnings.push(format!(
            "RUBATO_PUBLIC_URL {public_url} is http on a non-loopback host; the session cookie is sent without Secure"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    /// ADR 0073: voice is off unless `RUBATO_VOICE` says on, and the test config has none.
    #[test]
    fn voice_is_off_by_default() {
        assert_eq!(voice_from(vars(&[]), false).unwrap(), None);
        assert_eq!(
            voice_from(vars(&[("RUBATO_VOICE", "0")]), true).unwrap(),
            None
        );
        // The LiveKit trio alone does not turn it on.
        assert_eq!(
            voice_from(
                vars(&[
                    ("RUBATO_LIVEKIT_URL", "wss://lk.example"),
                    ("RUBATO_LIVEKIT_API_KEY", "k"),
                    ("RUBATO_LIVEKIT_API_SECRET", SECRET),
                ]),
                true
            )
            .unwrap(),
            None
        );
        let c = Config::for_tests("sqlite::memory:".into(), "m0x6A");
        assert!(c.voice.is_none());
        assert!(c.voice_availability().is_none());
        assert!(c.livekit_url().is_none());
        assert!(voice_from(vars(&[("RUBATO_VOICE", "yes")]), true).is_err());
    }

    /// A half-configured LiveKit or speech provider refuses to start, naming what is missing.
    #[test]
    fn half_configured_voice_refuses_to_start() {
        let e = voice_from(
            vars(&[
                ("RUBATO_VOICE", "1"),
                ("RUBATO_LIVEKIT_URL", "wss://lk.example"),
            ]),
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("RUBATO_LIVEKIT_API_KEY") && e.contains("RUBATO_LIVEKIT_API_SECRET"),
            "{e}"
        );
        let e = voice_from(
            vars(&[
                ("RUBATO_VOICE", "1"),
                ("RUBATO_LIVEKIT_URL", "wss://lk.example"),
                ("RUBATO_LIVEKIT_API_KEY", "k"),
                ("RUBATO_LIVEKIT_API_SECRET", "short"),
            ]),
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("at least 32"), "{e}");
        let e = voice_from(
            vars(&[
                ("RUBATO_VOICE", "1"),
                ("RUBATO_SPEECH", "openai_compatible"),
                ("RUBATO_STT_URL", "http://speaches:8000/v1"),
            ]),
            false,
        )
        .unwrap_err()
        .to_string();
        for k in [
            "RUBATO_TTS_URL",
            "RUBATO_STT_MODEL",
            "RUBATO_TTS_MODEL",
            "RUBATO_TTS_VOICE",
        ] {
            assert!(e.contains(k), "{e}");
        }
        assert!(
            voice_from(
                vars(&[("RUBATO_VOICE", "1"), ("RUBATO_SPEECH", "whisper")]),
                true
            )
            .is_err()
        );
    }

    /// The fake adapter echoes uploads: only a dev server (loopback, ADR 0064) may use it.
    #[test]
    fn the_fake_speech_adapter_needs_the_dev_user() {
        let on = vars(&[("RUBATO_VOICE", "1"), ("RUBATO_SPEECH", "fake")]);
        let e = voice_from(&on, false).unwrap_err().to_string();
        assert!(e.contains("RUBATO_DEV_USER"), "{e}");
        let v = voice_from(&on, true).unwrap().unwrap();
        assert_eq!(v.speech, Some(SpeechConfig::Fake));
        assert_eq!(v.livekit, None);
    }

    /// A full configuration, what `Me.voice` says about it, and no secret in `Debug`.
    #[test]
    fn voice_on_names_its_hosts_and_never_prints_a_secret() {
        let v = voice_from(
            vars(&[
                ("RUBATO_VOICE", "1"),
                ("RUBATO_LIVEKIT_URL", "wss://lk.example/"),
                ("RUBATO_LIVEKIT_API_KEY", "APIkey"),
                ("RUBATO_LIVEKIT_API_SECRET", SECRET),
                ("RUBATO_SPEECH", "openai_compatible"),
                ("RUBATO_STT_URL", "https://api.example.com/v1/"),
                ("RUBATO_TTS_URL", "http://kokoro:8880/v1"),
                ("RUBATO_STT_API_KEY", "sk-stt-marker"),
                ("RUBATO_STT_MODEL", "whisper-1"),
                ("RUBATO_TTS_MODEL", "kokoro"),
                ("RUBATO_TTS_VOICE", "af_heart"),
            ]),
            false,
        )
        .unwrap();
        let mut c = Config::for_tests("sqlite::memory:".into(), "m0x6A");
        c.voice = v;
        assert_eq!(c.livekit_url(), Some("wss://lk.example"));
        let a = c.voice_availability().unwrap();
        assert!(a.enabled && a.speech);
        assert_eq!(a.speech_hosts, ["api.example.com", "kokoro:8880"]);
        let shown = format!("{c:?}");
        assert!(
            !shown.contains(SECRET),
            "the LiveKit secret leaked: {shown}"
        );
        assert!(!shown.contains("sk-stt-marker"), "the speech key leaked");
        println!("checked the config's Debug output for 2 secrets");
    }

    /// #169: local is a loopback bind and a loopback public URL, both.
    #[test]
    fn is_local_needs_loopback_bind_and_url() {
        let local = [
            ("127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("127.0.0.1:8080", "http://localhost:8080/"),
            ("127.0.0.2:8080", "http://127.0.0.2:8080"),
            ("[::1]:8080", "http://[::1]:8080"),
            ("127.0.0.1:8080", "https://LOCALHOST"),
        ];
        let open = [
            ("0.0.0.0:8080", "http://127.0.0.1:8080"),
            ("[::]:8080", "http://localhost:8080"),
            ("192.168.1.5:8080", "http://192.168.1.5:8080"),
            ("127.0.0.1:8080", "https://rubato.example.test"),
            ("127.0.0.1:8080", "http://10.0.0.2:8080"),
            ("127.0.0.1:8080", "http://localhost.example.test"),
            ("127.0.0.1:8080", "not a url"),
            ("127.0.0.1:8080", "http://test"),
        ];
        for (b, u) in local {
            assert!(is_local(addr(b), u), "{b} {u} is local");
        }
        for (b, u) in open {
            assert!(!is_local(addr(b), u), "{b} {u} is not local");
        }
        println!("checked {} binds and URLs", local.len() + open.len());
    }

    /// #169: the dev user never starts off loopback, with or without the opt-in.
    #[test]
    fn dev_user_off_loopback_refuses_to_start() {
        let lan = addr("0.0.0.0:8080");
        for open_login in [None, Some("1")] {
            for oauth in [false, true] {
                let e = exposure(lan, "http://0.0.0.0:8080", true, oauth, open_login)
                    .unwrap_err()
                    .to_string();
                assert!(e.contains("RUBATO_DEV_USER"), "{e}");
            }
        }
        let behind_proxy = exposure(
            addr("127.0.0.1:8080"),
            "https://rubato.example.test",
            true,
            false,
            None,
        );
        assert!(behind_proxy.is_err(), "a public URL is not local");
        let w = exposure(
            addr("127.0.0.1:8080"),
            "http://127.0.0.1:8080",
            true,
            false,
            None,
        )
        .unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("RUBATO_DEV_USER"), "{w:?}");
    }

    /// #169: OAuth without the App off loopback needs `RUBATO_OPEN_LOGIN=1`.
    #[test]
    fn oauth_without_app_off_loopback_needs_open_login() {
        let lan = addr("0.0.0.0:8080");
        let url = "https://rubato.example.test";
        let e = exposure(lan, url, false, true, None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("RUBATO_OPEN_LOGIN=1"), "{e}");
        let w = exposure(lan, url, false, true, Some("1")).unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("open to any GitHub account"), "{w:?}");
        let w = exposure(
            addr("127.0.0.1:8080"),
            "http://127.0.0.1:8080",
            false,
            true,
            None,
        )
        .unwrap();
        assert!(w[0].contains("open to any GitHub account"), "{w:?}");
        assert!(
            exposure(lan, url, false, false, None).unwrap().is_empty(),
            "as an App, or with no login at all, nothing is open"
        );
        let w = exposure(lan, url, false, false, Some("1")).unwrap();
        assert!(w[0].contains("no effect"), "{w:?}");
        for bad in ["yes", "true", "0"] {
            let e = exposure(lan, url, false, true, Some(bad))
                .unwrap_err()
                .to_string();
            assert!(e.contains("must be 1 or unset"), "{bad}: {e}");
        }
    }

    #[test]
    fn oauth_without_app_needs_id_and_secret_and_no_app() {
        let mut c = Config::for_tests("sqlite::memory:".into(), "m0x6A");
        assert!(!c.oauth_without_app());
        c.github_client_id = Some("Iv1.x".into());
        assert!(!c.oauth_without_app(), "no secret: login is not configured");
        c.github_client_secret = Some("s".into());
        assert!(c.oauth_without_app());
        c.github_app = Some(GitHubApp {
            app_id: 1,
            private_key: "k".into(),
            installation_id: 2,
        });
        assert!(!c.oauth_without_app());
    }

    #[test]
    fn http_public_url_off_loopback_warns() {
        let warned = |url: &str| {
            let mut w = Vec::new();
            insecure_cookie_warning(url, &mut w);
            !w.is_empty()
        };
        let cases = [
            ("http://rubato.lan:8080", true),
            ("http://192.168.1.20:8081", true),
            ("http://127.0.0.1:8080", false),
            ("http://localhost", false),
            ("http://[::1]:1", false),
            ("https://rubato.example", false),
        ];
        for (url, want) in cases {
            assert_eq!(warned(url), want, "{url}");
        }
        println!("checked {} public URLs", cases.len());
    }

    #[test]
    fn two_traffic_sources_are_a_startup_error_and_settings_never_print() {
        let prom = TrafficEnv::from_vars(
            [
                ("RUBATO_TRAFFIC_PROMETHEUS_URL", "http://prom:9090"),
                (
                    "RUBATO_TRAFFIC_PROMETHEUS_HEADERS",
                    "Authorization=Bearer s3cret",
                ),
                ("RUBATO_TRAFFIC_WINDOW_SECS", "600"),
                ("RUBATO_TRAFFIC_FETCH_SECS", "60"),
                ("RUBATO_OTHER", "x"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );
        assert_eq!(prom.0.len(), 3, "{prom:?}");
        assert!(!format!("{prom:?}").contains("s3cret"));
        assert!(one_traffic_source(true, &prom).is_err());
        assert!(one_traffic_source(false, &prom).is_ok());
        assert!(one_traffic_source(true, &TrafficEnv::default()).is_ok());
    }

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

    /// ADR 0054: the cutover reads as RFC 3339 in any offset, is absent when unset, and a value
    /// that does not read stops the start.
    #[test]
    fn the_cutover_is_an_rfc3339_time_or_unset() {
        assert_eq!(comment_approvals_until(None).unwrap(), None);
        let at = comment_approvals_until(Some("2026-11-20T09:00:00+01:00"))
            .unwrap()
            .unwrap();
        assert_eq!(at.to_rfc3339(), "2026-11-20T08:00:00+00:00");
        for bad in ["2026-11-20", "tomorrow", "2026-11-20 08:00"] {
            let e = comment_approvals_until(Some(bad)).unwrap_err().to_string();
            assert!(e.contains("RUBATO_COMMENT_APPROVALS_UNTIL"), "{e}");
        }
    }

    /// One `cutover` line at info, saying the time or `unset`.
    #[test]
    fn the_cutover_is_logged_once_at_start() {
        let (events, _guard) = crate::kernel::telemetry::capture::start();
        let mut c = Config::for_tests("sqlite::memory:".into(), "m0x6A");
        c.report_cutover();
        c.comment_approvals_until = comment_approvals_until(Some("2026-11-20T08:00:00Z")).unwrap();
        c.report_cutover();
        let events = events.lock().unwrap();
        let lines: Vec<_> = events.iter().filter(|e| e.message == "cutover").collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|e| e.level == tracing::Level::INFO));
        assert_eq!(
            lines[0].fields.get("cutover").map(String::as_str),
            Some("unset")
        );
        assert_eq!(
            lines[1].fields.get("cutover").map(String::as_str),
            Some("2026-11-20T08:00:00+00:00")
        );
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
    /// ADR 0074: the secret turns webhooks on and never prints; the catch-up is clamped.
    #[test]
    fn webhook_secret_and_catchup_from_env() {
        assert!(WebhookSecret::from_env_value(None).is_none());
        assert!(WebhookSecret::from_env_value(Some(String::new())).is_none());
        let s = WebhookSecret::from_env_value(Some("hunter2".into())).unwrap();
        assert_eq!(s.as_bytes(), b"hunter2");
        assert_eq!(format!("{s:?}"), "[redacted]");
        let mut cfg = Config::for_tests("sqlite::memory:".into(), "m0x6A");
        cfg.github_webhook_secret = Some(s);
        assert!(
            !format!("{cfg:?}").contains("hunter2"),
            "Config's Debug hides it"
        );
        let cases: [(Option<&str>, u64, bool); 6] = [
            (None, 900, false),
            (Some("900"), 900, false),
            (Some("300"), 300, false),
            (Some("10"), 300, true),
            (Some("99999"), 3600, true),
            (Some("soon"), 900, true),
        ];
        for (v, want, warns) in cases {
            let mut w = Vec::new();
            assert_eq!(catchup_secs(v, &mut w), want, "{v:?}");
            assert_eq!(!w.is_empty(), warns, "{v:?}: {w:?}");
        }
        println!("checked {} catch-up values", cases.len());
    }
}
