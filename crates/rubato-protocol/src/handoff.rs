//! The handoff (ADR 0055, plan 122 design §D7): after approval in Rubato the agent runs
//! `plan.ts handoff`, which writes the approval record into the plan, accepts the plan's ADRs,
//! makes one commit and pushes it, then reports its outcome to the companion's loopback endpoint.
//! The companion forwards that report, so the reason reaches Rubato whatever the agent says.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The first skill version with `plan.ts handoff`. A companion whose installed skill is older
/// refuses `plan_approved` with `skill_outdated` instead of typing a command the session lacks.
pub const SKILL_HANDOFF_VERSION: &str = "0.2.0";

/// How a launched session plans (ADR 0053): `git` is today's flow (commit, push, `Review:` and
/// `Approved:` comments); `rubato` drafts in Rubato and hands off at approval. The server sends
/// `rubato` only after the cutover and to a companion and skill that can carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanningMode {
    #[default]
    Git,
    Rubato,
}

impl PlanningMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Rubato => "rubato",
        }
    }

    pub fn is_git(&self) -> bool {
        *self == Self::Git
    }
}

/// What `plan.ts handoff` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandoffOutcome {
    /// One commit with the approval record and the accepted ADRs, pushed.
    Committed,
    /// The plan no longer hashes to the approved sha: nothing was written.
    PlanChanged,
    /// The bundle no longer hashes to the approved bundle: nothing was written.
    BundleChanged,
    /// A pinned ADR changed after it was pinned: nothing was written.
    AdrChanged,
    /// Committed, but the push failed; a rerun pushes the same commit.
    PushFailed,
    /// Anything else, with the line's `error`.
    Error,
}

impl HandoffOutcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::PlanChanged => "plan_changed",
            Self::BundleChanged => "bundle_changed",
            Self::AdrChanged => "adr_changed",
            Self::PushFailed => "push_failed",
            Self::Error => "error",
        }
    }
}

/// The `rubato.plan.handoff` line `plan.ts handoff` prints and posts to the companion's
/// loopback `/handoff` route (the companion adds the launch from its own records).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandoffLine {
    /// Always `rubato.plan.handoff`.
    pub event: String,
    pub outcome: HandoffOutcome,
    /// The Rubato approval id the handoff was run for.
    pub approval: Uuid,
    /// The commit made, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// ADR numbers moved to accepted in this commit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adrs_accepted: Vec<String>,
    /// ADR numbers the approval covers that this repo could not accept (they live in the map
    /// repo; finding `AdrsNotAccepted`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adrs_left: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Companion → server: a handoff's outcome, for one launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandoffReport {
    pub launch_id: Uuid,
    #[serde(flatten)]
    pub line: HandoffLine,
    pub at: DateTime<Utc>,
}

/// Whether an installed skill version (`major.minor.patch`) has `plan.ts handoff`.
pub fn skill_has_handoff(version: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let mut it = v.trim().split('.').map(|p| p.parse::<u64>().ok());
        Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
    }
    match (parse(version), parse(SKILL_HANDOFF_VERSION)) {
        (Some(have), Some(need)) => have >= need,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handoff_line_round_trips_as_the_skill_prints_it() {
        let text = r#"{"event":"rubato.plan.handoff","outcome":"committed","approval":"5d3b1f0e-2c4a-4b8e-9f1d-7a6c5e4b3a21","commit":"abc1234","adrs_accepted":["0050"],"adrs_left":[]}"#;
        let line: HandoffLine = serde_json::from_str(text).unwrap();
        assert_eq!(line.outcome, HandoffOutcome::Committed);
        assert_eq!(line.adrs_accepted, ["0050"]);
        let r = HandoffReport {
            launch_id: Uuid::nil(),
            line,
            at: DateTime::from_timestamp(0, 0).unwrap(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["outcome"], "committed");
        assert_eq!(serde_json::from_value::<HandoffReport>(v).unwrap(), r);
    }

    #[test]
    fn skill_versions_compare_by_number() {
        assert!(skill_has_handoff("0.2.0"));
        assert!(skill_has_handoff("0.10.0"));
        assert!(skill_has_handoff("1.0"));
        assert!(!skill_has_handoff("0.1.9"));
        assert!(!skill_has_handoff("dev"));
    }
}
