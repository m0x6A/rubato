//! The progress model: one state per issue, derived, never stored by hand.
//!
//! The derivation is pure so the server, its tests and the fixtures agree on it.

use crate::github::{ApprovalState, DecisionRecord, ReviewRecord};
use crate::github::{ChecksStatus, PullRequestState};
use crate::ids::{IssueRef, MapRef};
use crate::issue_body::PinSuggestion;
use crate::layout::MapLayout;
use crate::map::MapResponse;
use crate::map_draft::MapDraft;
use crate::pairing::PairingInfo;
use crate::plan::{Plan, PlanFit};
use crate::queue::QueuedIssue;
use crate::session::{SessionState, SessionStatus};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    /// The newest design review a member posted (ADR 0027).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewRecord>,
    /// The newest approval a member posted, and whether it is of the plan as it is now. A
    /// badge, not a progress state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalState>,
    /// Where the plan fits the map, and whether it does (ADR 0029). Absent without a plan file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<PlanFit>,
    /// The plan's approval hash as Rubato read it (ADR 0027): what *Accept plan* approves and
    /// what `Approved: plan <sha>` names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_sha: Option<String>,
    /// Sum over this issue's sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_usd: Option<f64>,
}

/// An open issue of a repo the map names that Rubato does not track yet: not pinned, launched,
/// planned or worked on. It has no progress state (ADR 0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenIssue {
    pub issue: IssueRef,
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    pub updated_at: DateTime<Utc>,
    /// The place its body's `Map:` line names, when its author is not trusted to place it
    /// (ADR 0026); a trusted line pins the issue and it leaves this list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_line: Option<MapRef>,
    /// Where Rubato suggests it belongs. Confirmed by a person, never applied on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<PinSuggestion>,
}

/// How fresh one repo's listing is. The trust check for the backlog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BacklogRepo {
    /// `owner/name`.
    pub repo: String,
    /// The last good listing. Absent until the first one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_at: Option<DateTime<Utc>>,
    /// The listing was cut at the server's cap.
    #[serde(default)]
    pub truncated: bool,
    /// Why the latest listing failed. Absent when it succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Every open issue of the map's repos that is not tracked yet, and how fresh each listing is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Backlog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<OpenIssue>,
    /// One entry per repo the map names, listed or not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<BacklogRepo>,
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
    /// The GitHub App installation does not cover a repo the map or a tracked issue names, so
    /// its branches, plans and pull requests cannot be read (ADR 0022).
    RepoNotCovered { repo: String },
    /// A plan's flow names a seam that does not join the flow's two parts (ADR 0021).
    FlowOffSeam {
        issue: IssueRef,
        seam: String,
        from: String,
        to: String,
    },
    /// The plan was approved, then changed: the approval names an earlier version (ADR 0027).
    ApprovalStale {
        issue: IssueRef,
        approved_sha: String,
        current_sha: String,
    },
    /// An approval names the current plan, but the plan does not fit the map, so it does not
    /// count (ADR 0029).
    ApprovedWithoutFit { issue: IssueRef },
    /// A live session Rubato cannot tie to an issue: its branch is not `issue/<n>-…`, or its
    /// repo is unknown.
    UnattributedSession {
        companion: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
    },
    /// Map versus territory (ADR 0039): a declared seam with no calls in the window of a fresh
    /// report, though both its parts show traffic elsewhere in it.
    SeamQuiet { seam: String, window_secs: u32 },
    /// Map versus territory (ADR 0039): calls between two parts no seam joins, from a fresh
    /// report. `from` calls `to`.
    UndeclaredTraffic {
        from: String,
        to: String,
        calls: u64,
    },
}

/// One repo the map names, and what Rubato sees of it: whether the App covers it, and its open
/// issues, plans and sessions. A repo outside the map with tracked work appears with
/// `in_map: false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConnectedRepo {
    /// `owner/name`, in the map's spelling.
    pub repo: String,
    pub in_map: bool,
    /// The App installation covers it (ADR 0022). False also when coverage is not known yet.
    pub covered: bool,
    /// Open issues in the latest listing.
    pub open_issues: u32,
    /// Tracked issues with a plan file.
    pub plans: u32,
    /// Live sessions tied to an issue of this repo.
    pub sessions: u32,
    /// Live sessions in this repo with no issue.
    pub unattributed: u32,
    /// Open pull requests from its own branches in the latest listing (ADR 0034).
    #[serde(default)]
    pub open_pulls: u32,
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
    /// Open issues not tracked yet (ADR 0019).
    #[serde(default)]
    pub backlog: Backlog,
    /// Where the team has put each part (ADR 0016). Parts without a position are laid out
    /// automatically. Never an input to progress.
    #[serde(default)]
    pub layout: MapLayout,
    /// What each developer intends to run on their companions, lane by lane (ADR 0020).
    /// Never an input to progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<QueuedIssue>,
    /// The shared draft of the map and its proposal (ADR 0018). Never an input to progress.
    #[serde(default)]
    pub draft: MapDraft,
    /// Whether "New issue here…" can create an issue: the GitHub App's installation has
    /// Issues: write, or the server runs on the local stand-in (ADR 0025).
    #[serde(default)]
    pub can_create_issues: bool,
    /// One row per repo the map names, plus repos of tracked work outside the map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<ConnectedRepo>,
    /// Open pull requests per map repo (ADR 0034). Never an input to progress.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pulls: Vec<crate::pulls::RepoPulls>,
    /// Observed calls per seam from the latest traffic report (ADR 0039). Absent from an older
    /// server: not configured. Never an input to progress.
    #[serde(default)]
    pub traffic: crate::traffic::ObservedTraffic,
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
            branch_drift: None,
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
