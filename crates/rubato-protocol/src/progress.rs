//! The progress model: one state per issue, derived, never stored by hand.
//!
//! The derivation is pure so the server, its tests and the fixtures agree on it.

use crate::github::DecisionRecord;
use crate::github::{ChecksStatus, PullRequestState};
use crate::ids::{IssueRef, MapRef};
use crate::map::MapResponse;
use crate::pairing::PairingInfo;
use crate::plan::Plan;
use crate::session::{SessionState, SessionStatus};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    /// Issue exists and is pinned to the map.
    Planned,
    /// Session launched for the issue.
    Started,
    /// Session working, no plan file yet.
    Designing,
    /// Plan file on the branch; affected nodes lit.
    Designed,
    /// Session working after the plan.
    Building,
    /// Session blocked (hook) or a cap hit.
    Blocked,
    /// PR open, checks green.
    InReview,
    /// Merged.
    Done,
    /// The signal is not flowing. Trust check first.
    Unknown,
}

/// Everything the derivation looks at. The server collects these; nothing here is stored as a state.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressInputs<'a> {
    pub pins: &'a [MapRef],
    pub launched: bool,
    pub sessions: &'a [SessionState],
    pub has_plan: bool,
    pub cap_hit: bool,
    pub pr: Option<&'a PullRequestState>,
    pub now: DateTime<Utc>,
    pub session_stale_secs: u64,
}

/// Derived progress of one issue, with the time of the newest signal behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Progress {
    pub issue: IssueRef,
    pub state: ProgressState,
    pub last_seen: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned_to: Vec<MapRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<String>,
    /// One line on why, for *Blocked* and *Unknown*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Derive the state. Order of precedence: Done > InReview > Blocked > session-driven states > Started > Planned.
/// A live session whose signal is stale or untrusted makes the issue *Unknown* rather than *working*.
pub fn derive_progress(inputs: &ProgressInputs<'_>) -> (ProgressState, Option<String>) {
    if let Some(pr) = inputs.pr {
        if pr.merged_at.is_some() {
            return (ProgressState::Done, None);
        }
        if pr.open && !pr.draft && pr.checks == ChecksStatus::Green {
            return (ProgressState::InReview, None);
        }
    }
    if inputs.cap_hit {
        return (ProgressState::Blocked, Some("cap hit".into()));
    }
    let live: Vec<&SessionState> = inputs
        .sessions
        .iter()
        .filter(|s| s.status != SessionStatus::Ended)
        .collect();
    if let Some(s) = live
        .iter()
        .find(|s| s.status == SessionStatus::Blocked && s.source.is_trusted())
    {
        return (
            ProgressState::Blocked,
            Some(format!(
                "session {} blocked",
                s.session_id.as_deref().unwrap_or("?")
            )),
        );
    }
    if !live.is_empty() {
        let effective: Vec<SessionStatus> = live
            .iter()
            .map(|s| s.effective_status(inputs.now, inputs.session_stale_secs))
            .collect();
        if effective.contains(&SessionStatus::Working) {
            return (
                if inputs.has_plan {
                    ProgressState::Building
                } else {
                    ProgressState::Designing
                },
                None,
            );
        }
        if effective.iter().all(|e| *e == SessionStatus::Unknown) {
            return (
                ProgressState::Unknown,
                Some("no trusted signal from any live session".into()),
            );
        }
        // Idle sessions: the issue is where the plan says it is.
        return (
            if inputs.has_plan {
                ProgressState::Designed
            } else {
                ProgressState::Started
            },
            None,
        );
    }
    if inputs.has_plan {
        return (ProgressState::Designed, None);
    }
    if inputs.launched {
        return (ProgressState::Started, None);
    }
    if !inputs.pins.is_empty() {
        return (ProgressState::Planned, None);
    }
    (
        ProgressState::Unknown,
        Some("issue is not pinned to the map".into()),
    )
}

/// Everything the map view needs about one issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssueSummary {
    pub issue: IssueRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub pins: Vec<MapRef>,
    pub progress: Progress,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<DecisionRecord>,
    /// Sum over this issue's sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_usd: Option<f64>,
}

/// Map versus territory, and plans that disagree with the map. Shown, never suppressed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Finding {
    /// A plan names ids the map does not declare.
    UnknownIds { issue: IssueRef, ids: Vec<String> },
    /// Two or more open plans touch the same seam.
    SeamCollision { seam: String, issues: Vec<IssueRef> },
    /// A plan file exists but does not parse.
    PlanParseError {
        issue: IssueRef,
        path: String,
        error: String,
    },
    /// A session was launched against a different map version than the server holds.
    MapVersionMismatch {
        issue: IssueRef,
        launched_with: String,
        current: String,
    },
}

/// The whole picture in one payload: what the web view renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Overview {
    pub map: MapResponse,
    pub issues: Vec<IssueSummary>,
    pub findings: Vec<Finding>,
    pub companions: Vec<PairingInfo>,
    /// Sessions not attached to any issue (branch outside the convention).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpinned_sessions: Vec<SessionState>,
    pub generated_at: DateTime<Utc>,
}

/// Route path for the overview.
pub mod routes {
    pub const OVERVIEW: &str = "/api/overview";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::StateSource;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
    }

    fn session(status: SessionStatus, source: StateSource, ago: i64) -> SessionState {
        SessionState {
            session_id: Some("s1".into()),
            companion_id: Uuid::nil(),
            user: "m0x6A".into(),
            tmux: None,
            cwd: "/w".into(),
            worktree: None,
            branch: Some("issue/1-x".into()),
            repo: Some("o/r".into()),
            issue: Some(IssueRef::new("o/r", 1)),
            status,
            source,
            last_seen: now() - chrono::Duration::seconds(ago),
            spend_usd: None,
        }
    }

    fn pr(open: bool, checks: ChecksStatus, merged: bool) -> PullRequestState {
        PullRequestState {
            repo: "o/r".into(),
            number: 9,
            branch: "issue/1-x".into(),
            open,
            draft: false,
            checks,
            merged_at: merged.then(now),
            updated_at: now(),
            url: String::new(),
        }
    }

    fn inputs<'a>(
        sessions: &'a [SessionState],
        pins: &'a [MapRef],
        pr: Option<&'a PullRequestState>,
    ) -> ProgressInputs<'a> {
        ProgressInputs {
            pins,
            launched: false,
            sessions,
            has_plan: false,
            cap_hit: false,
            pr,
            now: now(),
            session_stale_secs: 120,
        }
    }

    #[test]
    fn every_state_is_reachable() {
        let pins = [MapRef::node("rubato-protocol")];
        let none: [SessionState; 0] = [];
        assert_eq!(
            derive_progress(&inputs(&none, &pins, None)).0,
            ProgressState::Planned
        );
        let mut i = inputs(&none, &pins, None);
        i.launched = true;
        assert_eq!(derive_progress(&i).0, ProgressState::Started);
        let working = [session(SessionStatus::Working, StateSource::Hook, 5)];
        assert_eq!(
            derive_progress(&inputs(&working, &pins, None)).0,
            ProgressState::Designing
        );
        let mut i = inputs(&working, &pins, None);
        i.has_plan = true;
        assert_eq!(derive_progress(&i).0, ProgressState::Building);
        let idle = [session(SessionStatus::Idle, StateSource::Hook, 5)];
        let mut i = inputs(&idle, &pins, None);
        i.has_plan = true;
        assert_eq!(derive_progress(&i).0, ProgressState::Designed);
        let blocked = [session(SessionStatus::Blocked, StateSource::Hook, 5)];
        assert_eq!(
            derive_progress(&inputs(&blocked, &pins, None)).0,
            ProgressState::Blocked
        );
        let green = pr(true, ChecksStatus::Green, false);
        assert_eq!(
            derive_progress(&inputs(&working, &pins, Some(&green))).0,
            ProgressState::InReview
        );
        let merged = pr(false, ChecksStatus::Green, true);
        assert_eq!(
            derive_progress(&inputs(&working, &pins, Some(&merged))).0,
            ProgressState::Done
        );
    }

    #[test]
    fn stale_or_scraped_session_is_unknown_never_working() {
        let pins = [MapRef::node("n")];
        let stale = [session(SessionStatus::Working, StateSource::Hook, 500)];
        assert_eq!(
            derive_progress(&inputs(&stale, &pins, None)).0,
            ProgressState::Unknown
        );
        let scraped = [session(SessionStatus::Working, StateSource::Scraped, 1)];
        assert_eq!(
            derive_progress(&inputs(&scraped, &pins, None)).0,
            ProgressState::Unknown
        );
    }

    #[test]
    fn red_checks_keep_the_issue_in_building() {
        let pins = [MapRef::node("n")];
        let working = [session(SessionStatus::Working, StateSource::Hook, 5)];
        let red = pr(true, ChecksStatus::Red, false);
        let mut i = inputs(&working, &pins, Some(&red));
        i.has_plan = true;
        assert_eq!(derive_progress(&i).0, ProgressState::Building);
    }

    #[test]
    fn cap_hit_blocks_even_when_working() {
        let pins = [MapRef::node("n")];
        let working = [session(SessionStatus::Working, StateSource::Hook, 5)];
        let mut i = inputs(&working, &pins, None);
        i.cap_hit = true;
        assert_eq!(derive_progress(&i).0, ProgressState::Blocked);
    }

    #[test]
    fn unpinned_issue_is_unknown() {
        let none: [SessionState; 0] = [];
        let (s, reason) = derive_progress(&inputs(&none, &[], None));
        assert_eq!(s, ProgressState::Unknown);
        assert!(reason.unwrap().contains("not pinned"));
    }
}
