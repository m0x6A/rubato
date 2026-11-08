//! `workspace.yml` (shared with the skill) and the companion's own state file.

use anyhow::{Context, Result};
use rubato_protocol::Limits;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The developer's `workspace.yml`. Same shape the skill reads (`skills/rubato/workspace.example.yml`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Workspace {
    #[serde(default)]
    pub server_url: Option<String>,
    pub map: MapLocation,
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
        }
    }
}

impl Workspace {
    pub fn parse(text: &str) -> Result<Self> {
        let ws: Self = serde_yaml_ng::from_str(text).context("workspace.yml does not parse")?;
        Ok(ws)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text)
    }

    /// `$RUBATO_WORKSPACE`, then `$XDG_CONFIG_HOME/rubato/workspace.yml`, then `~/.config/rubato/workspace.yml`.
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("RUBATO_WORKSPACE") {
            return PathBuf::from(p);
        }
        config_dir().join("workspace.yml")
    }

    pub fn repo(&self, name: &str) -> Option<&WorkspaceRepo> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// Which connected repo a path belongs to, if any.
    pub fn repo_for_path(&self, path: &Path) -> Option<&WorkspaceRepo> {
        self.repos
            .iter()
            .find(|r| path == r.local || path.starts_with(&r.local))
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
    /// Last companion sequence number sent; resumes from here.
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
    fn parses_the_example_workspace() {
        let text = include_str!("../../../skills/rubato/workspace.example.yml");
        let ws = Workspace::parse(text).unwrap();
        assert_eq!(ws.map.repo, "m0x6A/rubato");
        assert_eq!(ws.companion.claude_command, "claude");
        assert_eq!(ws.companion.limits.session_stale_secs, 120);
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
