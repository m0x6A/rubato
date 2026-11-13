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

impl SessionStatus {
    /// Every status, for the gauges that also report zero. A test checks it against the
    /// JSON Schema, so a new variant fails it until it is listed here.
    pub const ALL: [Self; 5] = [
        Self::Working,
        Self::Idle,
        Self::Blocked,
        Self::Ended,
        Self::Unknown,
    ];

    /// The status as the protocol spells it on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Blocked => "blocked",
            Self::Ended => "ended",
            Self::Unknown => "unknown",
        }
    }
}

impl StateSource {
    /// Every source, for the gauges that also report zero. Checked like `SessionStatus::ALL`.
    pub const ALL: [Self; 4] = [Self::Hook, Self::TmuxIde, Self::Scraped, Self::Launch];

    /// The source as the protocol spells it on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::TmuxIde => "tmux_ide",
            Self::Scraped => "scraped",
            Self::Launch => "launch",
        }
    }

    pub fn is_trusted(self) -> bool {
        !matches!(self, Self::Scraped)
    }
}

/// Which hook blocked a session (ADR 0043): the current cause of the current status, set with
/// a *blocked* status and cleared by the next hook that implies a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockCause {
    /// `Notification` with `permission_prompt`: waiting for a permission, mid-turn.
    PermissionPrompt,
    /// `Notification` with `idle_prompt`: waiting for input after the turn ended.
    IdlePrompt,
    /// `StopFailure`: the turn failed, mid-turn.
    StopFailure,
}

impl BlockCause {
    /// Every cause, checked against the JSON Schema like `SessionStatus::ALL`.
    pub const ALL: [Self; 3] = [Self::PermissionPrompt, Self::IdlePrompt, Self::StopFailure];

    /// The cause as the protocol spells it on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Self::PermissionPrompt => "permission_prompt",
            Self::IdlePrompt => "idle_prompt",
            Self::StopFailure => "stop_failure",
        }
    }

    /// Blocked mid-turn: the session would be working but for the wait.
    pub fn is_mid_turn(self) -> bool {
        !matches!(self, Self::IdlePrompt)
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
    /// A launched session that has moved off its launch branch: the branch it is on now. It
    /// keeps its launch issue (its spend cap and telemetry are that issue's); this says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_drift: Option<String>,
    /// Which hook blocked the session, with `status = blocked` (ADR 0043). Absent from an
    /// older companion and for a block tmux-ide reported: read as mid-turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<BlockCause>,
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
            branch_drift: None,
            blocked_by: None,
        };
        (s, now)
    }

    #[test]
    fn all_block_causes_cover_every_variant_once() {
        let names: std::collections::BTreeSet<String> = BlockCause::ALL
            .iter()
            .map(|c| {
                assert_eq!(
                    serde_json::to_value(c).unwrap(),
                    serde_json::json!(c.name())
                );
                c.name().to_owned()
            })
            .collect();
        assert_eq!(names.len(), BlockCause::ALL.len());
        assert_eq!(names, schema_variants::<BlockCause>());
        println!("checked {} block causes", names.len());
    }

    #[test]
    fn blocked_by_is_skipped_when_absent() {
        let (mut s, _) = state(SessionStatus::Blocked, StateSource::Hook, 1);
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("blocked_by").is_none(), "{v}");
        s.blocked_by = Some(BlockCause::IdlePrompt);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["blocked_by"], "idle_prompt");
        let back: SessionState = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    /// Every string the JSON Schema of `T` allows: its `enum` lists and `const`s.
    fn schema_variants<T: JsonSchema>() -> std::collections::BTreeSet<String> {
        fn walk(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(serde_json::Value::String(s)) = m.get("const") {
                        out.insert(s.clone());
                    }
                    if let Some(serde_json::Value::Array(a)) = m.get("enum") {
                        out.extend(a.iter().filter_map(|x| x.as_str().map(str::to_owned)));
                    }
                    m.values().for_each(|x| walk(x, out));
                }
                serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let mut out = std::collections::BTreeSet::new();
        walk(
            &serde_json::to_value(schemars::schema_for!(T)).unwrap(),
            &mut out,
        );
        out
    }

    #[test]
    fn all_statuses_cover_every_variant_once() {
        let names: Vec<String> = SessionStatus::ALL
            .iter()
            .map(|s| s.name().to_owned())
            .collect();
        for s in SessionStatus::ALL {
            assert_eq!(
                serde_json::to_value(s).unwrap(),
                serde_json::json!(s.name())
            );
        }
        let unique: std::collections::BTreeSet<String> = names.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "a status listed twice: {names:?}"
        );
        assert_eq!(unique, schema_variants::<SessionStatus>());
        println!("checked {} session statuses", names.len());
    }

    #[test]
    fn all_sources_cover_every_variant_once() {
        let names: Vec<String> = StateSource::ALL
            .iter()
            .map(|s| s.name().to_owned())
            .collect();
        for s in StateSource::ALL {
            assert_eq!(
                serde_json::to_value(s).unwrap(),
                serde_json::json!(s.name())
            );
        }
        let unique: std::collections::BTreeSet<String> = names.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "a source listed twice: {names:?}"
        );
        assert_eq!(unique, schema_variants::<StateSource>());
        println!("checked {} state sources", names.len());
    }

    /// The check fails when a variant is missing from `ALL`: the schema of `SessionStatus`
    /// against a list without `unknown`.
    #[test]
    fn a_list_missing_a_variant_fails_the_check() {
        let short: std::collections::BTreeSet<String> = SessionStatus::ALL[..4]
            .iter()
            .map(|s| s.name().to_owned())
            .collect();
        assert_ne!(short, schema_variants::<SessionStatus>());
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
