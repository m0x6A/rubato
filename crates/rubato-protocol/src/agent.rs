//! What a launched session's agent says on purpose, and where its plan stands (ADR 0071): the
//! `companion-server` side of the companion's MCP server for agents. `AgentReport` goes from
//! companion to server; `PlanStatus` from server to companion. Both only between peers that
//! announced `capability::AGENT_MCP`.

use crate::github::{ApprovalState, ReviewRecord};
use crate::map::MAP_ID_PATTERN;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a launched session's agent said it is doing, through `report_status` (ADR 0071). A
/// signal with its own time, never a progress state: the server shows the newest per launch and
/// never derives progress from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentReport {
    /// The launch the companion resolved from the call's `X-Rubato-Launch`, never from what the
    /// agent said.
    pub launch_id: Uuid,
    /// One line, at most 200 characters (`limits::AGENT_DOING_MAX_CHARS`).
    #[schemars(length(max = 200))]
    pub doing: String,
    /// Map ids the agent says it is on, at most 8 (`limits::AGENT_PARTS_MAX`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = 8), inner(pattern(MAP_ID_PATTERN)))]
    pub parts: Vec<String>,
    /// When the agent called `report_status`.
    pub at: DateTime<Utc>,
}

/// Where the plan of one launched session stands, as the server derives it (ADR 0071). The
/// companion keeps the newest per launch and answers `plan_status` from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanStatus {
    pub launch_id: Uuid,
    /// When the server derived it.
    pub as_of: DateTime<Utc>,
    /// The newest settled revision of the launch's draft (ADR 0053).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<DraftStatus>,
    /// The issue's newest counted design review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
    /// The newest counted approval and whether it is of the current version; an approval whose
    /// version is not `current` does not cover the plan as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalState>,
    /// The plan on the issue branch, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<BranchStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftStatus {
    /// Revision number, 1-based.
    pub rev: u32,
    /// When the revision settled.
    pub at: DateTime<Utc>,
    /// The revision's plan hash, as an approval names it.
    pub plan_sha: String,
    /// Whether the revision's plan fits the map.
    pub fits: bool,
}

/// What a launched agent last said, as the overview shows it (ADR 0071, plan 226 §3). Joined to
/// a session on `(companion_id, launch_id)`, never on the launch id alone; never a progress
/// state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentLine {
    /// The pairing the report arrived on.
    pub companion_id: Uuid,
    pub launch_id: Uuid,
    pub doing: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// When the server received it (the server's clock).
    pub reported_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BranchStatus {
    /// The plan's hash at the branch head.
    pub sha: String,
    /// Whether the plan on the branch carries the approval record (ADR 0054).
    pub carries_approval: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{ApprovalRecord, ApprovalSource, ApprovalState, ApprovalVersion};
    use crate::ids::IssueRef;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap()
    }

    #[test]
    fn an_agent_report_round_trips() {
        let r = AgentReport {
            launch_id: Uuid::from_u128(7),
            doing: "designing the auth seam".into(),
            parts: vec!["rubato-companion".into(), "companion-mcp".into()],
            at: at(),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["doing"], "designing the auth seam");
        assert_eq!(serde_json::from_value::<AgentReport>(json).unwrap(), r);
    }

    #[test]
    fn a_plan_status_round_trips_with_and_without_its_parts() {
        let empty = PlanStatus {
            launch_id: Uuid::from_u128(7),
            as_of: at(),
            draft: None,
            review: None,
            approval: None,
            branch: None,
        };
        let json = serde_json::to_value(&empty).unwrap();
        assert!(json.get("draft").is_none(), "{json}");
        assert_eq!(serde_json::from_value::<PlanStatus>(json).unwrap(), empty);

        let full = PlanStatus {
            draft: Some(DraftStatus {
                rev: 3,
                at: at(),
                plan_sha: "abc1234".into(),
                fits: true,
            }),
            approval: Some(ApprovalState {
                record: ApprovalRecord {
                    issue: IssueRef::new("m0x6A/rubato", 7),
                    source: ApprovalSource::Rubato,
                    comment_id: None,
                    approval_id: Some(Uuid::from_u128(9)),
                    bundle_hash: None,
                    author: "m0x6A".into(),
                    sha: Some("abc1234".into()),
                    branch: None,
                    note: None,
                    created_at: at(),
                    url: String::new(),
                },
                version: ApprovalVersion::Current,
                current_sha: Some("abc1234".into()),
            }),
            branch: Some(BranchStatus {
                sha: "def5678".into(),
                carries_approval: false,
            }),
            ..empty
        };
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(serde_json::from_value::<PlanStatus>(json).unwrap(), full);
    }
}
