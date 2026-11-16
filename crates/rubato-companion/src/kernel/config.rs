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
    /// Command used for git in a launch: fetch, worktree list, add and remove (ADR 0070). Tests
    /// point this at a wrapper around the real git.
    pub git_command: String,
    /// Command used for tmux.
    pub tmux_command: String,
    /// Socket directory for a private tmux server (`TMUX_TMPDIR`). None means the default server.
    pub tmux_tmpdir: Option<PathBuf>,
    /// Lowest tmux version an interactive launch accepts (`tmux -V`, ADR 0070).
    pub tmux_min_version: String,
    /// Thresholds; the server may override them in `Welcome`.
    pub limits: Limits,
    /// Prices by model id prefix, longest prefix wins. Empty means spend is not measured.
    pub pricing: BTreeMap<String, ModelPrice>,
    /// Permission mode passed to Claude Code at launch.
    pub permission_mode: String,
    /// Deadline for every git and tmux call of a launch; a launch that hits it is
    /// `Failed` (plan 40 S3, A1).
    pub command_timeout_secs: u64,
    /// Where the rubato skill is installed; None means `~/.claude/skills/rubato`. Its version
    /// decides whether `plan_approved` can be typed (ADR 0055).
    pub skill_dir: Option<PathBuf>,
    /// `interactive` (tmux, the default) or `headless` (ADR 0062, A18): one mode per companion.
    pub mode: rubato_protocol::ClientKind,
    /// A headless turn running longer than this is killed, `failed { timeout }` (A19).
    pub headless_turn_max_secs: u64,
    /// A headless turn with no stream event for this long is killed, `failed { quiet }` (A19).
    pub headless_quiet_max_secs: u64,
    /// Deadline for one read-only fork turn of a room agent (ADR 0057). Its first turn re-reads
    /// the whole session uncached, so it gets longer than other commands.
    pub fork_timeout_secs: u64,
    /// A person's lead agent may run here (ADR 0072): the companion announces `fleet` and serves
    /// the fleet tools to a lead launch. Off by default.
    pub orchestrator: bool,
}

/// Why `run` refuses to start in a container (ADR 0076): the image has no tmux, so only
/// `mode: headless` runs there. `None` outside a container or in headless mode.
pub fn container_refusal(
    mode: rubato_protocol::ClientKind,
    in_container: Option<&str>,
) -> Option<String> {
    let inside = matches!(in_container.map(str::trim), Some("1") | Some("true"));
    (inside && mode != rubato_protocol::ClientKind::Headless).then(|| {
        "headless only in a container: set `companion.mode: headless` in local.yml (ADR 0076)"
            .to_owned()
    })
}

impl CompanionConfig {
    pub fn is_headless(&self) -> bool {
        self.mode == rubato_protocol::ClientKind::Headless
    }

    pub fn command_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.command_timeout_secs)
    }

    /// The installed skill's directory.
    pub fn skill_dir(&self) -> PathBuf {
        self.skill_dir
            .clone()
            .unwrap_or_else(crate::kernel::skill::default_dir)
    }
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            claude_command: "claude".into(),
            git_command: "git".into(),
            tmux_command: "tmux".into(),
            tmux_tmpdir: None,
            tmux_min_version: "3.7".into(),
            limits: Limits::default(),
            pricing: BTreeMap::new(),
            permission_mode: "acceptEdits".into(),
            command_timeout_secs: crate::kernel::proc::DEFAULT_TIMEOUT.as_secs(),
            skill_dir: None,
            mode: rubato_protocol::ClientKind::Interactive,
            headless_turn_max_secs: 4 * 60 * 60,
            headless_quiet_max_secs: 15 * 60,
            fork_timeout_secs: 600,
            orchestrator: false,
        }
    }
}

/// `local.yml` keys under `companion:` that tmux-ide used (ADR 0070). They still parse, since
/// unknown keys are ignored; each is named once at start so the developer can delete it.
pub const RETIRED_KEYS: [&str; 2] = ["tmux_ide_command", "tmux_ide_min_version"];

/// The retired keys `text` still sets. Text that does not parse sets none: `parse` says why.
pub fn retired_keys(text: &str) -> Vec<&'static str> {
    let Ok(v) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text) else {
        return Vec::new();
    };
    let companion = v.get("companion");
    RETIRED_KEYS
        .into_iter()
        .filter(|k| companion.and_then(|c| c.get(*k)).is_some())
        .collect()
}

impl Workspace {
    pub fn parse(text: &str) -> Result<Self> {
        let ws: Self = serde_yaml_ng::from_str(text).context("workspace.yml does not parse")?;
        // Zero would time every launch call out at once, with nothing pointing here (plan 40).
        anyhow::ensure!(
            ws.companion.command_timeout_secs > 0,
            "workspace.yml: companion.command_timeout_secs must be at least 1"
        );
        Ok(ws)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        for key in retired_keys(&text) {
            tracing::info!(key, "local.yml sets a retired key; delete it");
        }
        Self::parse(&text)
    }

    /// `$RUBATO_LOCAL`, then `$RUBATO_WORKSPACE`, then `local.yml` in the config directory
    /// (`$XDG_CONFIG_HOME/rubato`, else `~/.config/rubato`), then the old `workspace.yml` there
    /// when only it exists. A new machine gets `local.yml`.
    pub fn default_path() -> PathBuf {
        Self::resolve(None).0
    }

    /// The config path to read, from `--workspace` when given, else as `default_path` finds it,
    /// and whether it was found by an old name, for the one warning at start (#132).
    pub fn resolve(explicit: Option<PathBuf>) -> (PathBuf, bool) {
        match explicit {
            Some(p) => {
                let old = Self::is_old_name(&p);
                (p, old)
            }
            None => Self::path_in(
                std::env::var("RUBATO_LOCAL").ok(),
                std::env::var("RUBATO_WORKSPACE").ok(),
                &config_dir(),
            ),
        }
    }

    /// `default_path` from explicit inputs, and whether it was found by an old name
    /// (`RUBATO_WORKSPACE`, or a file still called `workspace.yml`), so the caller can warn once
    /// as the skill does (ADR 0044, #132).
    pub fn path_in(
        local_env: Option<String>,
        workspace_env: Option<String>,
        dir: &Path,
    ) -> (PathBuf, bool) {
        if let Some(p) = local_env {
            let p = PathBuf::from(p);
            let old = Self::is_old_name(&p);
            return (p, old);
        }
        if let Some(p) = workspace_env {
            return (PathBuf::from(p), true);
        }
        let local = dir.join("local.yml");
        let old = dir.join("workspace.yml");
        if !local.exists() && old.exists() {
            (old, true)
        } else {
            (local, false)
        }
    }

    /// A config file still named `workspace.yml`.
    pub fn is_old_name(path: &Path) -> bool {
        path.file_name().is_some_and(|n| n == "workspace.yml")
    }

    /// The one warning for a config found by its old name, worded as the skill words it.
    pub fn old_name_warning(path: &Path) -> String {
        format!(
            "{}: workspace.yml is now local.yml (and RUBATO_WORKSPACE is RUBATO_LOCAL); rename it",
            path.display()
        )
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

/// Write a state file readable only by the owner, atomically: a temp file with mode 0600,
/// synced, then renamed over `path`, so a reader (or the next start, after a crash) sees the
/// old contents or the new, never a partial file. As `server_link`'s `seq.json`.
pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("opening {}", tmp.display()))?;
    // In case a leftover temp file had wider permissions.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)
        .with_context(|| format!("writing {}", tmp.display()))?;
    f.sync_all()?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming over {}", path.display()))?;
    Ok(())
}

/// Where the hook listener publishes its port and token for `hooks.ts` and the launcher. Kept
/// across restarts: the next start binds the same port with the same token when it can, so
/// sessions launched earlier keep their hooks (plan 122 §D1).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HookEndpoint {
    pub port: u16,
    pub token: String,
}

impl HookEndpoint {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("hook-endpoint.json")
    }

    /// The header file curl reads with `-H @<file>` (ADR 0063): in the state dir for manual
    /// sessions, and one beside each launch's `hooks.json`.
    pub fn header_path(dir: &Path) -> PathBuf {
        dir.join(HOOK_HEADER_FILE)
    }

    /// Write the one `Authorization` line curl sends, mode 0600. The token never goes in argv.
    pub fn write_header(&self, path: &Path) -> Result<()> {
        write_private(
            path,
            format!("Authorization: Bearer {}\n", self.token).as_bytes(),
        )
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        write_atomic_private(&Self::path(dir), serde_json::to_string(self)?.as_bytes())?;
        self.write_header(&Self::header_path(dir))
    }

    /// The endpoint the last run published. `None` when there is none, or it does not parse
    /// (logged): the next start then binds a new one.
    pub fn load(dir: &Path) -> Option<Self> {
        let p = Self::path(dir);
        let text = std::fs::read_to_string(&p).ok()?;
        match serde_json::from_str(&text) {
            Ok(ep) => Some(ep),
            Err(e) => {
                tracing::warn!(path = %p.display(), error = %e, "hook endpoint file does not parse");
                None
            }
        }
    }
}

/// The name of the hook header file, in the state dir and in each launch dir.
pub const HOOK_HEADER_FILE: &str = "hook-auth.header";

#[cfg(test)]
mod tests {
    use super::*;

    /// Plan 147 A7: a `local.yml` written for tmux-ide still parses, and each retired key it
    /// sets is named once; one that sets none names none.
    #[test]
    fn a_config_with_retired_tmux_ide_keys_parses_and_names_them() {
        let old = "companion:\n  tmux_ide_command: /usr/local/bin/tmux-ide\n  tmux_ide_min_version: 2.9.0\n  tmux_command: tmux\nrepos: []\n";
        let ws = Workspace::parse(old).unwrap();
        assert_eq!(ws.companion.tmux_min_version, "3.7");
        assert_eq!(ws.companion.git_command, "git");
        assert_eq!(retired_keys(old), RETIRED_KEYS);
        assert!(retired_keys("companion:\n  tmux_command: tmux\nrepos: []\n").is_empty());
        assert!(retired_keys("not: [yaml").is_empty());
        println!("checked {} retired keys", RETIRED_KEYS.len());
    }

    /// The old name is found and flagged: `RUBATO_WORKSPACE`, a `workspace.yml` left alone, or one
    /// named by `RUBATO_LOCAL`; `local.yml` and an empty directory are not (#132).
    #[test]
    fn a_config_found_by_its_old_name_is_flagged_once_and_local_yml_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(
            Workspace::path_in(None, None, d),
            (d.join("local.yml"), false)
        );
        std::fs::write(d.join("workspace.yml"), "").unwrap();
        assert_eq!(
            Workspace::path_in(None, None, d),
            (d.join("workspace.yml"), true)
        );
        std::fs::write(d.join("local.yml"), "").unwrap();
        assert_eq!(
            Workspace::path_in(None, None, d),
            (d.join("local.yml"), false)
        );
        assert_eq!(
            Workspace::path_in(None, Some("/x/any.yml".into()), d),
            (PathBuf::from("/x/any.yml"), true)
        );
        assert_eq!(
            Workspace::path_in(Some("/x/workspace.yml".into()), None, d),
            (PathBuf::from("/x/workspace.yml"), true)
        );
        assert_eq!(
            Workspace::path_in(Some("/x/local.yml".into()), Some("/y.yml".into()), d),
            (PathBuf::from("/x/local.yml"), false)
        );
        assert_eq!(
            Workspace::resolve(Some("/x/workspace.yml".into())),
            (PathBuf::from("/x/workspace.yml"), true)
        );
        assert_eq!(
            Workspace::resolve(Some("/x/local.yml".into())),
            (PathBuf::from("/x/local.yml"), false)
        );
        let w = Workspace::old_name_warning(&d.join("workspace.yml"));
        assert!(w.contains("workspace.yml is now local.yml"), "{w}");
    }

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
    fn an_atomic_write_is_private_and_leaves_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("deliveries.json");
        std::fs::write(&p, "old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic_private(&p, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["deliveries.json"], "no temp file left behind");
    }

    #[test]
    fn the_hook_endpoint_round_trips_and_a_bad_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(HookEndpoint::load(dir.path()), None);
        let ep = HookEndpoint {
            port: 43125,
            token: "t".into(),
        };
        ep.save(dir.path()).unwrap();
        assert_eq!(HookEndpoint::load(dir.path()), Some(ep));
        std::fs::write(HookEndpoint::path(dir.path()), "{not json").unwrap();
        assert_eq!(HookEndpoint::load(dir.path()), None);
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

    /// The state dir gets the header file manual sessions render from (ADR 0063): 0600, the
    /// bearer line and nothing else.
    #[test]
    fn hook_endpoint_save_writes_a_private_header_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ep = HookEndpoint {
            port: 4321,
            token: "abcdefghijklmnopqrstuvwxyz012345".into(),
        };
        ep.save(dir.path()).unwrap();
        let p = HookEndpoint::header_path(dir.path());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345\n"
        );
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;
    use rubato_protocol::ClientKind;

    #[test]
    fn a_container_runs_headless_only() {
        let msg = container_refusal(ClientKind::Interactive, Some("1")).unwrap();
        assert!(msg.starts_with("headless only in a container"), "{msg}");
        assert_eq!(container_refusal(ClientKind::Headless, Some("1")), None);
        assert_eq!(container_refusal(ClientKind::Interactive, None), None);
        assert_eq!(container_refusal(ClientKind::Interactive, Some("0")), None);
        assert!(
            !CompanionConfig::default().orchestrator,
            "the orchestrator is off by default"
        );
    }
}
