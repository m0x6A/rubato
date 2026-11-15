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
    /// Where the rubato skill is installed; None means `~/.claude/skills/rubato`. Its version
    /// decides whether `plan_approved` can be typed (ADR 0055).
    pub skill_dir: Option<PathBuf>,
    /// `interactive` (tmux, the default) or `headless` (ADR 0059, A18): one mode per companion.
    pub mode: rubato_protocol::ClientKind,
    /// A headless turn running longer than this is killed, `failed { timeout }` (A19).
    pub headless_turn_max_secs: u64,
    /// A headless turn with no stream event for this long is killed, `failed { quiet }` (A19).
    pub headless_quiet_max_secs: u64,
    /// Deadline for one read-only fork turn of a room agent (ADR 0057). Its first turn re-reads
    /// the whole session uncached, so it gets longer than other commands.
    pub fork_timeout_secs: u64,
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
            tmux_ide_command: "tmux-ide".into(),
            tmux_command: "tmux".into(),
            tmux_tmpdir: None,
            tmux_ide_min_version: "2.9.0".into(),
            limits: Limits::default(),
            pricing: BTreeMap::new(),
            permission_mode: "acceptEdits".into(),
            command_timeout_secs: crate::kernel::tmux_ide::DEFAULT_TIMEOUT.as_secs(),
            skill_dir: None,
            mode: rubato_protocol::ClientKind::Interactive,
            headless_turn_max_secs: 4 * 60 * 60,
            headless_quiet_max_secs: 15 * 60,
            fork_timeout_secs: 600,
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

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        write_atomic_private(&Self::path(dir), serde_json::to_string(self)?.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

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
