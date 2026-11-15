//! `workspace.yml` (shared with the skill) and the companion's own state file.

use anyhow::{Context, Result};
use rubato_protocol::Limits;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The developer's local config, `local.yml` (once `workspace.yml`, ADR 0044). Same shape the skill
/// reads (`skills/rubato/local.example.yml`). The companion never reads a map, so `map:` and
/// `maps:` are both optional here.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Workspace {
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub map: Option<MapLocation>,
    /// Several map clones, one per workspace the developer plans against. Read by the skill.
    #[serde(default)]
    pub maps: Vec<MapLocation>,
    #[serde(default)]
    pub repos: Vec<WorkspaceRepo>,
    #[serde(default)]
    pub otel: Option<OtelConfig>,
    /// Companion-only knobs. All optional.
    #[serde(default)]
    pub companion: CompanionConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct MapLocation {
    pub repo: String,
    pub path: String,
    pub local: PathBuf,
    #[serde(default)]
    pub decisions: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct WorkspaceRepo {
    pub name: String,
    pub local: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct OtelConfig {
    pub endpoint: String,
    #[serde(default)]
    pub protocol: Option<String>,
}

/// Per-model prices in USD per million tokens, for spend measurement (A11). Configured, not guessed.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct CompanionConfig {
    /// Command used to start Claude Code. Tests point this at a stand-in.
    pub claude_command: String,
    /// Command used for tmux-ide.
    pub tmux_ide_command: String,
    /// Command used for tmux.
    pub tmux_command: String,
    /// Socket directory for a private tmux server (`TMUX_TMPDIR`). None means the default server.
    pub tmux_tmpdir: Option<PathBuf>,
    /// Lowest tmux-ide version the companion accepts.
    pub tmux_ide_min_version: String,
    /// Thresholds; the server may override them in `Welcome`.
    pub limits: Limits,
    /// Prices by model id prefix, longest prefix wins. Empty means spend is not measured.
    pub pricing: BTreeMap<String, ModelPrice>,
    /// Permission mode passed to Claude Code at launch.
    pub permission_mode: String,
    /// Deadline for the launch's `git fetch` and every tmux-ide call; a launch that hits it is
    /// `Failed` (plan 40 S3, A1).
    pub command_timeout_secs: u64,
}

impl CompanionConfig {
    pub fn command_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.command_timeout_secs)
    }
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            claude_command: "claude".into(),
            tmux_ide_command: "tmux-ide".into(),
            tmux_command: "tmux".into(),
            tmux_tmpdir: None,
            tmux_ide_min_version: "2.9.0".into(),
            limits: Limits::default(),
            pricing: BTreeMap::new(),
            permission_mode: "acceptEdits".into(),
            command_timeout_secs: crate::kernel::tmux_ide::DEFAULT_TIMEOUT.as_secs(),
        }
    }
}

impl Workspace {
    pub fn parse(text: &str) -> Result<Self> {
        let ws: Self = serde_yaml_ng::from_str(text).context("workspace.yml does not parse")?;
        // Zero would time every tmux-ide call out at once, with nothing pointing here (plan 40).
        anyhow::ensure!(
            ws.companion.command_timeout_secs > 0,
            "workspace.yml: companion.command_timeout_secs must be at least 1"
        );
        Ok(ws)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text)
    }

    /// `$RUBATO_LOCAL`, then `$RUBATO_WORKSPACE`, then `local.yml` in the config directory
    /// (`$XDG_CONFIG_HOME/rubato`, else `~/.config/rubato`), then the old `workspace.yml` there
    /// when only it exists. A new machine gets `local.yml`.
    pub fn default_path() -> PathBuf {
        for var in ["RUBATO_LOCAL", "RUBATO_WORKSPACE"] {
            if let Ok(p) = std::env::var(var) {
                return PathBuf::from(p);
            }
        }
        let local = config_dir().join("local.yml");
        let old = config_dir().join("workspace.yml");
        if !local.exists() && old.exists() {
            old
        } else {
            local
        }
    }

    pub fn repo(&self, name: &str) -> Option<&WorkspaceRepo> {
        self.repos.iter().find(|r| r.name == name)
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("rubato");
    }
    home().join(".config").join("rubato")
}

pub fn state_dir() -> PathBuf {
    if let Ok(x) = std::env::var("RUBATO_STATE_DIR") {
        return PathBuf::from(x);
    }
    if let Ok(x) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(x).join("rubato");
    }
    home().join(".local").join("state").join("rubato")
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// What the companion remembers between runs. Written with mode 0600; it holds the pairing token.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct State {
    pub companion_id: Uuid,
    pub server_url: String,
    pub user: String,
    pub pairing_token: String,
    /// Not used: the sequence is reserved in `seq.json`, which the running companion owns. Kept
    /// so existing state files still parse.
    #[serde(default)]
    pub last_seq: u64,
    /// Last server sequence acknowledged.
    #[serde(default)]
    pub last_server_seq: u64,
    /// Set by `pair --insecure`: `server_url` may be http/ws off loopback (ADR 0067). Files
    /// written before the field existed read as false.
    #[serde(default)]
    pub allow_plain_http: bool,
}

impl State {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("companion.json")
    }

    pub fn load(dir: &Path) -> Result<Option<Self>> {
        let p = Self::path(dir);
        if !p.exists() {
            return Ok(None);
        }
        let text =
            std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        Ok(Some(
            serde_json::from_str(&text).context("state file does not parse")?,
        ))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let p = Self::path(dir);
        let text = serde_json::to_string_pretty(self)?;
        write_private(&p, text.as_bytes())
    }
}

/// Write a file readable only by the owner.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    f.write_all(bytes)?;
    // In case the file existed with wider permissions.
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(())
}

/// How a server URL that passed [`check_server_url`] carries the pairing token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// `https`/`wss`, or `http`/`ws` on loopback.
    Protected,
    /// `http`/`ws` off loopback, allowed only because the developer paired with `--insecure`.
    PlainOffLoopback,
}

/// Refuse a server URL that would carry the pairing token or launch commands in clear text off
/// this machine (ADR 0067): `https` and `wss` anywhere, `http` and `ws` on loopback
/// (`localhost`, `127.0.0.0/8`, `::1`), and `http` and `ws` elsewhere only when the developer
/// opted in with `pair --insecure` (`allow_plain_http` in `companion.json`). Checked before
/// pairing and before connecting; the caller warns on [`Transport::PlainOffLoopback`].
pub fn check_server_url(server_url: &str, allow_plain_http: bool) -> Result<Transport> {
    let url = url::Url::parse(server_url)
        .with_context(|| format!("server URL {server_url:?} does not parse"))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" | "wss" => Ok(Transport::Protected),
        "http" | "ws" if loopback => Ok(Transport::Protected),
        "http" | "ws" if allow_plain_http => Ok(Transport::PlainOffLoopback),
        "http" | "ws" => anyhow::bail!(
            "server URL {server_url} is plain text off loopback; use https://, a tunnel to \
             127.0.0.1, or accept an unencrypted pairing token with \
             `rubato-companion pair --insecure --server {server_url}`"
        ),
        other => anyhow::bail!("server URL {server_url} has scheme {other}; use https://"),
    }
}

/// Logged at pair time and at every start while the `--insecure` opt-in is in use (ADR 0067).
pub fn warn_plain_http(server_url: &str) {
    tracing::warn!(
        server = %server_url,
        "server URL {server_url} is plain http off loopback (paired with --insecure): the pairing token and launch commands travel unencrypted"
    );
}

/// Where the hook listener publishes its port and token for `hooks.ts` and the launcher.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HookEndpoint {
    pub port: u16,
    pub token: String,
}

impl HookEndpoint {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("hook-endpoint.json")
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        write_private(&Self::path(dir), serde_json::to_string(self)?.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls() {
        let ok = [
            "https://rubato.example",
            "wss://rubato.example",
            "http://127.0.0.1:8080",
            "http://127.1.2.3",
            "http://localhost:8080",
            "http://LOCALHOST:8080/",
            "http://[::1]:8080",
            "ws://127.0.0.1",
        ];
        let refused = [
            "http://rubato.example",
            "ws://10.0.0.5:8080",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example",
            "http://[::ffff:10.0.0.5]",
            "ftp://127.0.0.1",
            "not a url",
        ];
        // The opt-in (`pair --insecure`) lets plain http/ws off loopback through, flagged.
        let opt_in_plain = [
            "http://rubato.example",
            "ws://10.0.0.5:8080",
            "http://192.168.1.20:8080",
        ];
        // ...and changes nothing else: other schemes and bad URLs stay refused.
        let opt_in_refused = ["ftp://127.0.0.1", "not a url"];
        for u in ok {
            assert_eq!(
                check_server_url(u, false).unwrap(),
                Transport::Protected,
                "{u}"
            );
            assert_eq!(
                check_server_url(u, true).unwrap(),
                Transport::Protected,
                "{u}"
            );
        }
        for u in refused {
            assert!(check_server_url(u, false).is_err(), "{u} should be refused");
        }
        let e = format!(
            "{:#}",
            check_server_url("http://rubato.example", false).unwrap_err()
        );
        assert!(e.contains("--insecure"), "{e}");
        for u in opt_in_plain {
            assert_eq!(
                check_server_url(u, true).unwrap(),
                Transport::PlainOffLoopback,
                "{u}"
            );
        }
        for u in opt_in_refused {
            assert!(check_server_url(u, true).is_err(), "{u} should be refused");
        }
        eprintln!(
            "server_urls: {} accepted, {} refused, {} plain with opt-in, {} refused with opt-in",
            ok.len(),
            refused.len(),
            opt_in_plain.len(),
            opt_in_refused.len()
        );
    }

    #[test]
    fn an_older_state_file_does_not_allow_plain_http() {
        let old = r#"{"companion_id":"00000000-0000-0000-0000-000000000001","server_url":"http://x","user":"u","pairing_token":"t"}"#;
        let s: State = serde_json::from_str(old).unwrap();
        assert!(!s.allow_plain_http);
    }

    #[test]
    fn parses_the_example_workspace() {
        let text = include_str!("../../../../skills/rubato/local.example.yml");
        let ws = Workspace::parse(text).unwrap();
        assert_eq!(ws.maps[0].repo, "m0x6A/rubato");
        assert_eq!(ws.companion.claude_command, "claude");
        assert_eq!(ws.companion.limits.session_stale_secs, 120);
        assert_eq!(ws.companion.command_timeout_secs, 120);
    }

    #[test]
    fn a_config_without_a_map_reads_for_the_companion() {
        // The companion never reads a map (plan 89): a `local.yml` with `maps:`, or with none, starts it.
        let ws = Workspace::parse("maps:\n  - { repo: a/m, path: m.yml, local: /m }\nrepos: []\n")
            .unwrap();
        assert!(ws.map.is_none());
        assert_eq!(ws.maps.len(), 1);
        assert!(Workspace::parse("repos: []\n").is_ok());
    }

    #[test]
    fn a_zero_command_timeout_is_refused_by_name() {
        let text = include_str!("../../../../skills/rubato/local.example.yml");
        let zero = format!("{text}\ncompanion:\n  command_timeout_secs: 0\n");
        let e = Workspace::parse(&zero).unwrap_err();
        assert!(
            format!("{e:#}").contains("companion.command_timeout_secs"),
            "{e:#}"
        );
        let five = format!("{text}\ncompanion:\n  command_timeout_secs: 5\n");
        assert_eq!(
            Workspace::parse(&five)
                .unwrap()
                .companion
                .command_timeout_secs,
            5
        );
    }

    #[test]
    fn state_round_trips_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let s = State {
            companion_id: Uuid::nil(),
            server_url: "http://x".into(),
            user: "m0x6A".into(),
            pairing_token: "t".into(),
            last_seq: 3,
            last_server_seq: 1,
            allow_plain_http: false,
        };
        s.save(dir.path()).unwrap();
        let mode = std::fs::metadata(State::path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(State::load(dir.path()).unwrap(), Some(s));
    }
}
