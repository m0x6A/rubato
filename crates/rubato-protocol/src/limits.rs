//! Thresholds both sides must agree on (A10). Overridable in config; these are the defaults.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Limits {
    /// Companion heartbeat interval.
    pub heartbeat_interval_secs: u64,
    /// A *working* session with no hook event for longer than this renders *unknown*.
    pub session_stale_secs: u64,
    /// A companion silent for longer than this fires `CompanionSilent`.
    pub companion_silent_secs: u64,
    /// Reconciliation poll of `tmux-ide team --json`.
    pub team_poll_interval_secs: u64,
    /// How often the server looks for a plan file on a live branch.
    pub plan_poll_interval_secs: u64,
    /// How often the server polls GitHub for PRs, checks and decision comments per live issue.
    pub github_poll_interval_secs: u64,
    /// The longest session message the companion types (ADR 0051); longer ones are refused.
    pub session_message_max_chars: u64,
    /// The longest prompt or reply the companion forwards for the session log (ADR 0052); longer
    /// text is cut and marked truncated.
    pub session_log_max_chars: u64,
    /// The largest plan bundle the companion mirrors (ADR 0053); a larger one is sent without
    /// its files, refused as too large.
    pub draft_max_bytes: u64,
    /// The longest room message a person may post, in characters (ADR 0057).
    pub room_message_max_chars: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            heartbeat_interval_secs: 30,
            session_stale_secs: 120,
            companion_silent_secs: 600,
            team_poll_interval_secs: 30,
            plan_poll_interval_secs: 60,
            github_poll_interval_secs: 60,
            session_message_max_chars: 8000,
            session_log_max_chars: 16000,
            draft_max_bytes: 1_048_576,
            room_message_max_chars: 8000,
        }
    }
}
