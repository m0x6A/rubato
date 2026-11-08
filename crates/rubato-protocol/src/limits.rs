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
        }
    }
}
