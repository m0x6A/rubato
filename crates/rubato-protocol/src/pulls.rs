//! Open pull requests of the map's repos (issue #82, ADR 0034): listed once per repo per tick,
//! same-repo branches only. A pull request's view, read on demand, and the rule that says whether
//! the person may merge it from Rubato (ADR 0033): [`plan_gate`] and [`merge_refusal`], pure, used
//! by the view and the merge request alike. Facts GitHub reported; nothing here is a progress
//! state.

use crate::github::{ApprovalRecord, ApprovalSource, ChecksStatus};
use crate::ids::IssueRef;
use crate::plan::PlanApprovalRecord;
use crate::plan_hash::approval_names;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One open pull request from a branch of the repo itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenPull {
    /// `owner/name`, in the map's spelling.
    pub repo: String,
    pub number: u64,
    pub title: String,
    /// GitHub login of who opened it.
    pub author: String,
    /// The head branch.
    pub branch: String,
    /// The branch it merges into.
    pub base: String,
    pub draft: bool,
    pub head_sha: String,
    /// The folded check runs at `head_sha`; absent until they have been read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<ChecksStatus>,
    pub updated_at: DateTime<Utc>,
    pub url: String,
    /// The issue the branch names (`issue/<n>-…`, ADR 0006), if it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
}

/// One map repo's open pull requests and how fresh that listing is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RepoPulls {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pulls: Vec<OpenPull>,
    /// When the last listing succeeded; absent until one has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listed_at: Option<DateTime<Utc>>,
    /// The last listing hit its one page: there are more than `pulls` shows.
    #[serde(default)]
    pub truncated: bool,
    /// Why the last listing failed, when it did; `pulls` are from the last good one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OpenPull {
    /// Whether the stored checks need another read on the full lane (ADR 0034): never read,
    /// the head moved since, or not finished.
    pub fn checks_due(checks: Option<ChecksStatus>, checks_sha: Option<&str>, head: &str) -> bool {
        match checks {
            None => true,
            Some(_) if checks_sha != Some(head) => true,
            Some(ChecksStatus::Pending | ChecksStatus::None) => true,
            Some(ChecksStatus::Green | ChecksStatus::Red) => false,
        }
    }
}

/// How a pull request is merged: `merge_method` on GitHub's merge call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

/// Whether the pull request is open, closed without merging, or merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PullState {
    Open,
    Closed,
    Merged,
}

/// One check run at the head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckRun {
    pub name: String,
    /// `queued`, `in_progress` or `completed`, as GitHub says.
    pub status: String,
    /// `success`, `failure`, `neutral`, … once completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    pub url: String,
}

/// The latest review of one team reviewer (ADR 0027's trusted authors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PullReview {
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED` or `DISMISSED`, as GitHub says.
    pub state: String,
    pub submitted_at: DateTime<Utc>,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// What the pull request changes, from one page of 100 files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FilesSummary {
    /// Files on the page read.
    pub count: u32,
    pub additions: u64,
    pub deletions: u64,
    /// The [`TOP_FILES`] with the most changed lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<ChangedFile>,
    /// The page was full: the pull request changes more files than `count`.
    #[serde(default)]
    pub truncated: bool,
}

/// How many files the summary names.
pub const TOP_FILES: usize = 20;

/// The files read per view: one page; a full page means there may be more (design 82 A3).
pub const FILES_PER_PAGE_HINT: usize = 100;

impl FilesSummary {
    /// Sum `files` and keep the [`TOP_FILES`] with the most changes. `full_page` when the read
    /// hit its page of 100.
    pub fn of(mut files: Vec<ChangedFile>, full_page: bool) -> Self {
        let count = files.len() as u32;
        let additions = files.iter().map(|f| f.additions).sum();
        let deletions = files.iter().map(|f| f.deletions).sum();
        files.sort_by(|a, b| {
            (b.additions + b.deletions)
                .cmp(&(a.additions + a.deletions))
                .then_with(|| a.path.cmp(&b.path))
        });
        files.truncate(TOP_FILES);
        Self {
            count,
            additions,
            deletions,
            top: files,
            truncated: full_page,
        }
    }
}

/// What the plan of the linked issue says about merging (ADR 0033, plan 82 Decisions 1–3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanGate {
    /// The issue has no plan signal at all (a trivial change): GitHub's branch protection alone.
    NoPlan,
    /// A counted approval names the plan at the head being merged.
    Approved { sha: String },
    /// The issue has a plan and no counted approval names the plan at that head.
    NotApproved { why: String },
}

/// Why the plan gate is closed: `PlanGate::NotApproved.why`.
pub mod plan_why {
    /// No counted approval: none at all, or only comments after the cutover (ADR 0054).
    pub const NO_APPROVAL: &str = "no_approval";
    /// The newest counted approval names no version.
    pub const NOT_NAMED: &str = "not_named";
    /// The head being merged has no plan file, while the issue has a plan.
    pub const MISSING_AT_HEAD: &str = "missing_at_head";
    /// The plan at the head being merged is not the version approved.
    pub const CHANGED_AT_HEAD: &str = "changed_at_head";
    /// The plan at the head does not fit the map (ADR 0029).
    pub const DOES_NOT_FIT: &str = "does_not_fit";
    /// The plan at the head could not be read: the gate fails closed (plan 82 Decision 9).
    pub const UNREADABLE: &str = "unreadable";
    /// An approval given in Rubato, and the plan at the head carries no approval record.
    pub const RECORD_MISSING: &str = "record_missing";
    /// An approval given in Rubato, and the record at the head is not Rubato's (ADR 0054).
    pub const RECORD_DIFFERS: &str = "record_differs";
}

/// The plan file at the head being merged, as the gate needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAtHead {
    /// No plan file at the head.
    Absent,
    /// Reading it failed: never taken for "no file".
    Unreadable,
    Present {
        /// Its approval hash (the record left out, ADR 0054).
        hash: String,
        /// It parses and fits the workspace's map (ADR 0029).
        fits: bool,
        /// Its front-matter approval record, when it has one (ADR 0054).
        record: Option<PlanApprovalRecord>,
    },
}

/// The plan's front-matter record is this approval given in Rubato: same approver (case aside),
/// same second, same plan sha (as `approve::keys_match` on the server).
pub fn record_matches(keys: &PlanApprovalRecord, a: &ApprovalRecord) -> bool {
    let at = keys
        .approved_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t.trim()).ok());
    keys.approved_by
        .as_deref()
        .is_some_and(|b| b.trim().eq_ignore_ascii_case(&a.author))
        && at.is_some_and(|t| t.timestamp() == a.created_at.timestamp())
        && keys
            .approved_sha
            .as_deref()
            .zip(a.sha.as_deref())
            .is_some_and(|(k, s)| k.trim().eq_ignore_ascii_case(s))
}

/// The plan gate (ADR 0033, ADR 0054, plan 82 Decisions 1, 3 and 9). `signal`: the issue's
/// summary has a plan row, a draft, a review or an approval. `approval`: the newest counted
/// approval record the overview derives for the issue. Fails closed: once the issue has any plan
/// signal, only an approval whose sha names the plan at the head, with that plan fitting and, for
/// an approval given in Rubato, carrying Rubato's record, opens it.
pub fn plan_gate(signal: bool, head: &PlanAtHead, approval: Option<&ApprovalRecord>) -> PlanGate {
    let not = |why: &str| PlanGate::NotApproved { why: why.into() };
    if !signal && *head == PlanAtHead::Absent && approval.is_none() {
        return PlanGate::NoPlan;
    }
    let (hash, fits, record) = match head {
        PlanAtHead::Unreadable => return not(plan_why::UNREADABLE),
        PlanAtHead::Absent if approval.is_none() => return not(plan_why::NO_APPROVAL),
        PlanAtHead::Absent => return not(plan_why::MISSING_AT_HEAD),
        PlanAtHead::Present { hash, fits, record } => (hash, *fits, record),
    };
    let Some(a) = approval else {
        return not(plan_why::NO_APPROVAL);
    };
    match a.sha.as_deref() {
        None => return not(plan_why::NOT_NAMED),
        Some(approved) if !approval_names(approved, hash) => {
            return not(plan_why::CHANGED_AT_HEAD);
        }
        Some(_) => {}
    }
    if !fits {
        return not(plan_why::DOES_NOT_FIT);
    }
    if a.source == ApprovalSource::Rubato {
        match record {
            None => return not(plan_why::RECORD_MISSING),
            Some(k) if !record_matches(k, a) => return not(plan_why::RECORD_DIFFERS),
            Some(_) => {}
        }
    }
    PlanGate::Approved { sha: hash.clone() }
}

/// What GitHub says about the pull request, read fresh, for [`merge_refusal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFacts {
    pub state: PullState,
    /// The head branch is in another repo.
    pub fork: bool,
    pub draft: bool,
    pub head_sha: String,
    /// GitHub's `mergeable`: `None` while it is still computing.
    pub mergeable: Option<bool>,
    /// The folded check runs at the head.
    pub checks: ChecksStatus,
    /// The folded check runs at the base branch's head, read only when the head has none.
    pub base_checks: Option<ChecksStatus>,
}

/// What GitHub says about the person and the repo, read with their token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonFacts {
    pub push: bool,
    /// The merge methods the repo allows, in the order merge, squash, rebase.
    pub methods: Vec<MergeMethod>,
}

/// What the person asks for: the head they saw and the method they picked. The view asks
/// without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeAsk<'a> {
    pub head_sha: &'a str,
    pub method: MergeMethod,
}

/// Why Rubato will not merge, first match wins (ADR 0033, design 82 §3); `None` when it may.
pub fn merge_refusal(
    pull: &PullFacts,
    person: Option<&PersonFacts>,
    gate: &PlanGate,
    ask: Option<MergeAsk<'_>>,
) -> Option<&'static str> {
    if pull.state != PullState::Open {
        return Some(reason::NOT_OPEN);
    }
    if pull.fork {
        return Some(reason::FORK);
    }
    if pull.draft {
        return Some(reason::DRAFT);
    }
    if ask.is_some_and(|a| a.head_sha != pull.head_sha) {
        return Some(reason::HEAD_CHANGED);
    }
    let checks_ok = match pull.checks {
        ChecksStatus::Green => true,
        ChecksStatus::Pending | ChecksStatus::Red => false,
        // No runs at the head: passes only when the base head has none either (design A6).
        ChecksStatus::None => pull.base_checks == Some(ChecksStatus::None),
    };
    if !checks_ok {
        return Some(reason::CHECKS_NOT_GREEN);
    }
    if pull.mergeable != Some(true) {
        return Some(reason::NOT_MERGEABLE);
    }
    if matches!(gate, PlanGate::NotApproved { .. }) {
        return Some(reason::PLAN_NOT_APPROVED);
    }
    let Some(person) = person else {
        return Some(reason::NEEDS_GITHUB_AUTHORIZATION);
    };
    if !person.push {
        return Some(reason::NO_PUSH_PERMISSION);
    }
    if ask.is_some_and(|a| !person.methods.contains(&a.method)) {
        return Some(reason::METHOD_NOT_ALLOWED);
    }
    None
}

/// What *Merge* offers the person, from [`merge_refusal`] with no ask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MergeOffer {
    /// The methods the repo allows; empty without the person's facts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<MergeMethod>,
    /// Squash when allowed (ADR 0033), else the first allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_method: Option<MergeMethod>,
    pub can_merge: bool,
    /// A [`reason`] when `can_merge` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
    pub plan_gate: PlanGate,
    /// Open pull requests whose base is this head branch: a squash makes them conflict
    /// (plan 82 Decision 7).
    #[serde(default)]
    pub stacked_above: u32,
}

impl MergeOffer {
    pub fn new(
        pull: &PullFacts,
        person: Option<&PersonFacts>,
        gate: PlanGate,
        stacked_above: u32,
    ) -> Self {
        let why_not = merge_refusal(pull, person, &gate, None);
        let methods = person.map(|p| p.methods.clone()).unwrap_or_default();
        let default_method = methods
            .contains(&MergeMethod::Squash)
            .then_some(MergeMethod::Squash)
            .or_else(|| methods.first().copied());
        Self {
            methods,
            default_method,
            can_merge: why_not.is_none(),
            why_not: why_not.map(str::to_owned),
            plan_gate: gate,
            stacked_above,
        }
    }
}

/// A pull request's view (`GET /api/pulls/{owner}/{repo}/{number}`, ADR 0034): read when opened,
/// cached 30 s. Titles and bodies are shown as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PullDetail {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub author: String,
    /// The description, as written (plain text, design A4).
    #[serde(default)]
    pub body: String,
    pub state: PullState,
    pub draft: bool,
    /// The head is in another repo (ADR 0034: never merged from Rubato).
    #[serde(default)]
    pub fork: bool,
    pub branch: String,
    pub base: String,
    pub head_sha: String,
    /// GitHub's `mergeable`; absent while GitHub is still computing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mergeable: Option<bool>,
    /// The folded check runs at the head.
    pub checks: ChecksStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub check_runs: Vec<CheckRun>,
    /// The latest review per team reviewer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<PullReview>,
    /// Reviews from outside the team: counted, not shown (design A7).
    #[serde(default)]
    pub outside_reviews: u32,
    pub files: FilesSummary,
    pub url: String,
    /// The issue the branch names (ADR 0006).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<DateTime<Utc>>,
    pub merge: MergeOffer,
    /// When GitHub was read for this view; a cached view is at most 30 s old.
    pub read_at: DateTime<Utc>,
}

/// Why Rubato will not merge, in the order [`merge_refusal`] checks them: the `reason` of a 409
/// or 502 answer, `MergeOffer.why_not` and `rubato.reason` on `rubato.web.rejected`.
pub mod reason {
    pub const NOT_OPEN: &str = "not_open";
    pub const FORK: &str = "fork";
    pub const DRAFT: &str = "draft";
    pub const HEAD_CHANGED: &str = "head_changed";
    /// Pending, red, or none at the head while the base branch has some.
    pub const CHECKS_NOT_GREEN: &str = "checks_not_green";
    /// Conflicts, or GitHub is still checking.
    pub const NOT_MERGEABLE: &str = "not_mergeable";
    pub const PLAN_NOT_APPROVED: &str = "plan_not_approved";
    pub const NEEDS_GITHUB_AUTHORIZATION: &str = "needs_github_authorization";
    pub const NO_PUSH_PERMISSION: &str = "no_push_permission";
    pub const METHOD_NOT_ALLOWED: &str = "method_not_allowed";
    /// GitHub refused the merge; its message is in the answer.
    pub const GITHUB_REFUSED: &str = "github_refused";
    /// The repo is not one the workspace's map names and the installation covers.
    pub const UNKNOWN_REPO: &str = "unknown_repo";
    /// Every reason [`super::merge_refusal`] returns, in its order.
    pub const ORDER: [&str; 10] = [
        NOT_OPEN,
        FORK,
        DRAFT,
        HEAD_CHANGED,
        CHECKS_NOT_GREEN,
        NOT_MERGEABLE,
        PLAN_NOT_APPROVED,
        NEEDS_GITHUB_AUTHORIZATION,
        NO_PUSH_PERMISSION,
        METHOD_NOT_ALLOWED,
    ];
}

/// Map-scoped (ADR 0049): also under `/api/w/{ws}`.
pub mod routes {
    /// `GET`: a pull request's view.
    pub const PULL: &str = "/api/pulls/{owner}/{repo}/{number}";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{ApprovalRecord, ApprovalSource};

    const HEAD_PLAN: &str = "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9";
    const AT: &str = "2026-11-15T10:00:00Z";

    fn approval(sha: Option<&str>, source: ApprovalSource) -> ApprovalRecord {
        ApprovalRecord {
            issue: IssueRef::new("o/r", 7),
            source,
            comment_id: None,
            approval_id: None,
            bundle_hash: None,
            author: "m0x6A".into(),
            sha: sha.map(str::to_owned),
            branch: None,
            note: None,
            created_at: DateTime::parse_from_rfc3339(AT)
                .unwrap()
                .with_timezone(&Utc),
            url: String::new(),
        }
    }

    fn keys(by: &str, at: &str, sha: &str) -> Option<PlanApprovalRecord> {
        Some(PlanApprovalRecord {
            approved_by: Some(by.into()),
            approved_at: Some(at.into()),
            approved_sha: Some(sha.into()),
        })
    }

    fn head(hash: &str, fits: bool, record: Option<PlanApprovalRecord>) -> PlanAtHead {
        PlanAtHead::Present {
            hash: hash.into(),
            fits,
            record,
        }
    }

    fn why(g: PlanGate) -> String {
        match g {
            PlanGate::NotApproved { why } => why,
            other => panic!("expected a closed gate, got {other:?}"),
        }
    }

    /// Plan 82 Decisions 1, 3 and 9: the gate opens on no plan signal at all, or on a counted
    /// approval naming the plan at the head that fits, with Rubato's record for an approval given
    /// in Rubato; every other case is closed, each with its reason.
    #[test]
    fn the_plan_gate_opens_only_on_no_plan_or_an_approval_of_the_head() {
        use ApprovalSource::{Comment, Rubato};
        assert_eq!(
            plan_gate(false, &PlanAtHead::Absent, None),
            PlanGate::NoPlan
        );
        let approved = PlanGate::Approved {
            sha: HEAD_PLAN.into(),
        };
        let comment = approval(Some(&HEAD_PLAN[..7]), Comment);
        assert_eq!(
            plan_gate(true, &head(HEAD_PLAN, true, None), Some(&comment)),
            approved
        );
        let rubato = approval(Some(HEAD_PLAN), Rubato);
        let record = keys("M0X6A", "2026-11-15T10:00:00+00:00", HEAD_PLAN);
        assert_eq!(
            plan_gate(true, &head(HEAD_PLAN, true, record.clone()), Some(&rubato)),
            approved
        );
        let edited = "abcdef0123456789abcdef0123456789abcdef01";
        let cases: Vec<(&str, PlanGate, &str)> = vec![
            (
                "file at head, no approval",
                plan_gate(false, &head(HEAD_PLAN, true, None), None),
                plan_why::NO_APPROVAL,
            ),
            (
                "plan row, no approval",
                plan_gate(true, &PlanAtHead::Absent, None),
                plan_why::NO_APPROVAL,
            ),
            (
                "unreadable, no signal",
                plan_gate(false, &PlanAtHead::Unreadable, None),
                plan_why::UNREADABLE,
            ),
            (
                "unreadable",
                plan_gate(true, &PlanAtHead::Unreadable, Some(&comment)),
                plan_why::UNREADABLE,
            ),
            (
                "not named",
                plan_gate(
                    true,
                    &head(HEAD_PLAN, true, None),
                    Some(&approval(None, Comment)),
                ),
                plan_why::NOT_NAMED,
            ),
            (
                "edited at head",
                plan_gate(true, &head(edited, true, None), Some(&comment)),
                plan_why::CHANGED_AT_HEAD,
            ),
            (
                "deleted at head",
                plan_gate(true, &PlanAtHead::Absent, Some(&comment)),
                plan_why::MISSING_AT_HEAD,
            ),
            (
                "an approval alone is a signal",
                plan_gate(false, &PlanAtHead::Absent, Some(&comment)),
                plan_why::MISSING_AT_HEAD,
            ),
            (
                "does not fit",
                plan_gate(true, &head(HEAD_PLAN, false, None), Some(&comment)),
                plan_why::DOES_NOT_FIT,
            ),
            (
                "rubato, no record",
                plan_gate(true, &head(HEAD_PLAN, true, None), Some(&rubato)),
                plan_why::RECORD_MISSING,
            ),
            (
                "rubato, other approver",
                plan_gate(
                    true,
                    &head(HEAD_PLAN, true, keys("ann", AT, HEAD_PLAN)),
                    Some(&rubato),
                ),
                plan_why::RECORD_DIFFERS,
            ),
            (
                "rubato, other second",
                plan_gate(
                    true,
                    &head(
                        HEAD_PLAN,
                        true,
                        keys("m0x6A", "2026-11-15T10:00:01Z", HEAD_PLAN),
                    ),
                    Some(&rubato),
                ),
                plan_why::RECORD_DIFFERS,
            ),
            (
                "rubato, other sha",
                plan_gate(
                    true,
                    &head(HEAD_PLAN, true, keys("m0x6A", AT, edited)),
                    Some(&rubato),
                ),
                plan_why::RECORD_DIFFERS,
            ),
        ];
        for (name, gate, want) in &cases {
            assert_eq!(why(gate.clone()), *want, "{name}");
        }
        println!("checked {} plan gate cases", cases.len() + 3);
    }

    fn open_green() -> PullFacts {
        PullFacts {
            state: PullState::Open,
            fork: false,
            draft: false,
            head_sha: "h1".into(),
            mergeable: Some(true),
            checks: ChecksStatus::Green,
            base_checks: None,
        }
    }

    fn person() -> PersonFacts {
        PersonFacts {
            push: true,
            methods: vec![MergeMethod::Merge, MergeMethod::Squash],
        }
    }

    /// Design 82 §3: each reason on its own case, the passing case beside it, and the order:
    /// a pull request failing every rule answers with the first.
    #[test]
    fn merge_refusal_names_each_reason_in_order() {
        let ok_gate = PlanGate::NoPlan;
        let ask = |h: &'static str, m| {
            Some(MergeAsk {
                head_sha: h,
                method: m,
            })
        };
        let squash = ask("h1", MergeMethod::Squash);
        assert_eq!(
            merge_refusal(&open_green(), Some(&person()), &ok_gate, squash),
            None
        );
        let with = |f: fn(&mut PullFacts)| {
            let mut p = open_green();
            f(&mut p);
            p
        };
        let closed = PlanGate::NotApproved {
            why: plan_why::CHANGED_AT_HEAD.into(),
        };
        let no_push = PersonFacts {
            push: false,
            ..person()
        };
        let cases: Vec<(&str, Option<&str>, &str)> = vec![
            (
                "closed",
                merge_refusal(
                    &with(|p| p.state = PullState::Closed),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::NOT_OPEN,
            ),
            (
                "merged",
                merge_refusal(
                    &with(|p| p.state = PullState::Merged),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::NOT_OPEN,
            ),
            (
                "fork",
                merge_refusal(&with(|p| p.fork = true), Some(&person()), &ok_gate, squash),
                reason::FORK,
            ),
            (
                "draft",
                merge_refusal(&with(|p| p.draft = true), Some(&person()), &ok_gate, squash),
                reason::DRAFT,
            ),
            (
                "head moved",
                merge_refusal(
                    &open_green(),
                    Some(&person()),
                    &ok_gate,
                    ask("h0", MergeMethod::Squash),
                ),
                reason::HEAD_CHANGED,
            ),
            (
                "pending",
                merge_refusal(
                    &with(|p| p.checks = ChecksStatus::Pending),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::CHECKS_NOT_GREEN,
            ),
            (
                "red",
                merge_refusal(
                    &with(|p| p.checks = ChecksStatus::Red),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::CHECKS_NOT_GREEN,
            ),
            (
                "not started",
                merge_refusal(
                    &with(|p| {
                        p.checks = ChecksStatus::None;
                        p.base_checks = Some(ChecksStatus::Green)
                    }),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::CHECKS_NOT_GREEN,
            ),
            (
                "base unread",
                merge_refusal(
                    &with(|p| p.checks = ChecksStatus::None),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::CHECKS_NOT_GREEN,
            ),
            (
                "conflicts",
                merge_refusal(
                    &with(|p| p.mergeable = Some(false)),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::NOT_MERGEABLE,
            ),
            (
                "still checking",
                merge_refusal(
                    &with(|p| p.mergeable = None),
                    Some(&person()),
                    &ok_gate,
                    squash,
                ),
                reason::NOT_MERGEABLE,
            ),
            (
                "plan",
                merge_refusal(&open_green(), Some(&person()), &closed, squash),
                reason::PLAN_NOT_APPROVED,
            ),
            (
                "no token",
                merge_refusal(&open_green(), None, &ok_gate, squash),
                reason::NEEDS_GITHUB_AUTHORIZATION,
            ),
            (
                "no push",
                merge_refusal(&open_green(), Some(&no_push), &ok_gate, squash),
                reason::NO_PUSH_PERMISSION,
            ),
            (
                "rebase not allowed",
                merge_refusal(
                    &open_green(),
                    Some(&person()),
                    &ok_gate,
                    ask("h1", MergeMethod::Rebase),
                ),
                reason::METHOD_NOT_ALLOWED,
            ),
        ];
        for (name, got, want) in &cases {
            assert_eq!(*got, Some(*want), "{name}");
        }
        // No runs at the head nor at the base: the repo runs no checks, and that passes.
        let quiet = with(|p| {
            p.checks = ChecksStatus::None;
            p.base_checks = Some(ChecksStatus::None);
        });
        assert_eq!(
            merge_refusal(&quiet, Some(&person()), &ok_gate, squash),
            None
        );
        // The view asks without a head or a method: neither is checked.
        assert_eq!(
            merge_refusal(&open_green(), Some(&person()), &ok_gate, None),
            None
        );
        // Order: failing everything answers with the first rule, and so on down the list.
        let mut worst = PullFacts {
            state: PullState::Closed,
            fork: true,
            draft: true,
            head_sha: "h1".into(),
            mergeable: Some(false),
            checks: ChecksStatus::Red,
            base_checks: None,
        };
        let bad_ask = ask("h0", MergeMethod::Rebase);
        let mut seen = vec![];
        let steps: [fn(&mut PullFacts); 6] = [
            |p| p.state = PullState::Open,
            |p| p.fork = false,
            |p| p.draft = false,
            |p| p.head_sha = "h0".into(),
            |p| p.checks = ChecksStatus::Green,
            |p| p.mergeable = Some(true),
        ];
        seen.push(merge_refusal(&worst, None, &closed, bad_ask).unwrap());
        for step in steps {
            step(&mut worst);
            seen.push(merge_refusal(&worst, Some(&no_push), &closed, bad_ask).unwrap_or("none"));
        }
        assert_eq!(seen, reason::ORDER[..7], "{seen:?}");
        println!(
            "checked {} refusal cases and the order of {}",
            cases.len() + 2,
            seen.len()
        );
    }

    #[test]
    fn the_offer_defaults_to_squash_and_lists_only_allowed_methods() {
        let o = MergeOffer::new(&open_green(), Some(&person()), PlanGate::NoPlan, 2);
        assert!(o.can_merge && o.why_not.is_none());
        assert_eq!(o.default_method, Some(MergeMethod::Squash));
        assert_eq!(o.methods, [MergeMethod::Merge, MergeMethod::Squash]);
        assert_eq!(o.stacked_above, 2);
        let rebase_only = PersonFacts {
            push: true,
            methods: vec![MergeMethod::Rebase],
        };
        let o = MergeOffer::new(&open_green(), Some(&rebase_only), PlanGate::NoPlan, 0);
        assert_eq!(o.default_method, Some(MergeMethod::Rebase));
        let o = MergeOffer::new(&open_green(), None, PlanGate::NoPlan, 0);
        assert!(!o.can_merge && o.methods.is_empty() && o.default_method.is_none());
        assert_eq!(
            o.why_not.as_deref(),
            Some(reason::NEEDS_GITHUB_AUTHORIZATION)
        );
    }

    #[test]
    fn files_summary_keeps_the_twenty_largest() {
        let files: Vec<ChangedFile> = (0..30)
            .map(|i| ChangedFile {
                path: format!("f{i:02}"),
                additions: i,
                deletions: 1,
            })
            .collect();
        let s = FilesSummary::of(files, false);
        assert_eq!((s.count, s.additions, s.deletions), (30, (0..30).sum(), 30));
        assert_eq!(s.top.len(), TOP_FILES);
        assert_eq!(s.top[0].path, "f29");
        assert!(!s.truncated);
        assert!(FilesSummary::of(vec![], true).truncated);
    }

    #[test]
    fn checks_are_read_again_until_finished_at_the_same_head() {
        let due = |c, s| OpenPull::checks_due(c, s, "abc");
        assert!(due(None, None), "never read");
        assert!(due(Some(ChecksStatus::Green), Some("old")), "head moved");
        assert!(
            due(Some(ChecksStatus::Pending), Some("abc")),
            "still running"
        );
        assert!(
            due(Some(ChecksStatus::None), Some("abc")),
            "not started yet"
        );
        assert!(!due(Some(ChecksStatus::Green), Some("abc")));
        assert!(!due(Some(ChecksStatus::Red), Some("abc")));
    }
}
