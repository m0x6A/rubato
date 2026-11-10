//! What the companion knows about sessions.

use crate::ids::IssueRef;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Working,
    Idle,
    Blocked,
    Ended,
    /// No trustworthy signal. Never rendered as working.
    Unknown,
}

/// Where a status came from. Anything not hook-derived is rendered *unknown* (A10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StateSource {
    /// A Claude Code hook forwarded by the companion.
    Hook,
    /// tmux-ide state with hook authority (`since` set).
    TmuxIde,
    /// tmux-ide state from screen matching. Not trusted.
    Scraped,
    /// The companion just launched it; no event yet.
    Launch,
}

impl StateSource {
    pub fn is_trusted(self) -> bool {
        !matches!(self, Self::Scraped)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TmuxLocation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u32>,
    /// tmux pane id such as `%12`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// One session as the companion sees it. Identity comes from the branch (ADR 0006).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionState {
    /// Claude Code session id, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub companion_id: Uuid,
    /// GitHub login of the paired user.
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux: Option<TmuxLocation>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// `owner/name` from `origin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Resolved from the branch, or from the launch when the branch does not follow the convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
    pub status: SessionStatus,
    pub source: StateSource,
    pub last_seen: DateTime<Utc>,
    /// Spend so far, summed from the transcript (A11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_usd: Option<f64>,
}

impl SessionState {
    /// The status a reader may show: `Unknown` when the source is not trusted or the signal is stale.
    pub fn effective_status(&self, now: DateTime<Utc>, stale_after_secs: u64) -> SessionStatus {
        if !self.source.is_trusted() {
            return SessionStatus::Unknown;
        }
        let age = now.signed_duration_since(self.last_seen).num_seconds();
        if self.status == SessionStatus::Working && age > stale_after_secs as i64 {
            return SessionStatus::Unknown;
        }
        self.status
    }
}

/// The companion's liveness signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Heartbeat {
    pub companion_id: Uuid,
    pub user: String,
    pub at: DateTime<Utc>,
    pub hostname: String,
    pub companion_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_ide_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_version: Option<String>,
}

/// Emitted once when a *working* session goes quiet for longer than the stale threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StaleSession {
    pub companion_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
    pub last_seen: DateTime<Utc>,
    pub silent_for_secs: u64,
    pub at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn state(
        status: SessionStatus,
        source: StateSource,
        last_seen_secs_ago: i64,
    ) -> (SessionState, DateTime<Utc>) {
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        let s = SessionState {
            session_id: None,
            companion_id: Uuid::nil(),
            user: "m0x6A".into(),
            tmux: None,
            cwd: "/w".into(),
            worktree: None,
            branch: None,
            repo: None,
            issue: None,
            status,
            source,
            last_seen: now - chrono::Duration::seconds(last_seen_secs_ago),
            spend_usd: None,
        };
        (s, now)
    }

    #[test]
    fn scraped_state_is_unknown() {
        let (s, now) = state(SessionStatus::Working, StateSource::Scraped, 1);
        assert_eq!(s.effective_status(now, 120), SessionStatus::Unknown);
    }

    #[test]
    fn stale_working_is_unknown_but_idle_is_not() {
        let (s, now) = state(SessionStatus::Working, StateSource::Hook, 121);
        assert_eq!(s.effective_status(now, 120), SessionStatus::Unknown);
        let (s, now) = state(SessionStatus::Working, StateSource::Hook, 119);
        assert_eq!(s.effective_status(now, 120), SessionStatus::Working);
        let (s, now) = state(SessionStatus::Idle, StateSource::Hook, 10_000);
        assert_eq!(s.effective_status(now, 120), SessionStatus::Idle);
    }
}
