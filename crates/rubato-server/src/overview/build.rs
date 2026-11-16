//! Build the overview: progress per issue (derived, never stored), findings, unpinned sessions.
//! Pure over the rows the database holds, so it is testable without a server.

use crate::github_poll::IssueMeta;
use crate::kernel::state::{LoadedMap, TrafficLatest};
use crate::plans::{PlanRow, approval_names};
use chrono::{DateTime, Utc};
use rubato_protocol::QueuedIssue;
use rubato_protocol::layout::{MapLayout, Position};
use rubato_protocol::{
    ApprovalRecord, ApprovalState, ApprovalVersion, Backlog, BacklogRepo, CapKind, ConnectedRepo,
    DecisionRecord, Finding, IssuePin, IssueRef, IssueSummary, Limits, MapRef, ObservedTraffic,
    OpenIssue, Plan, Progress, ProgressInputs, ProgressState, PullRequestState, RepoPulls,
    ReviewRecord, SessionState, SessionStatus, SpendPoint, SystemMap, blocked_from,
    derive_progress, derive_traffic, plan_fit,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// The other workspaces an issue of this overview may also belong to (ADR 0045). Empty, and
/// `here` absent, with one workspace: the overview is then as it was before workspaces.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceInputs {
    /// This overview's workspace, id and name, when there are several.
    pub here: Option<(String, String)>,
    /// Issues launched from this workspace.
    pub launched_here: HashSet<IssueRef>,
    pub others: Vec<OtherWorkspace>,
}

#[derive(Debug, Clone)]
pub struct OtherWorkspace {
    pub id: String,
    pub name: String,
    /// Absent until its map has loaded once. Shared with the server's state, not copied.
    pub map: Option<Arc<LoadedMap>>,
    pub pinned: HashSet<IssueRef>,
    pub launched: HashSet<IssueRef>,
}

impl OtherWorkspace {
    /// An issue belongs to a workspace whose map names its repo, or where it is pinned or
    /// launched (ADR 0045).
    pub fn holds(&self, issue: &IssueRef) -> bool {
        self.pinned.contains(issue)
            || self.launched.contains(issue)
            || self
                .map
                .as_ref()
                .is_some_and(|m| names_repo(&m.map, &issue.repo))
    }
}

/// Whether the map names `repo`, compared without case, as `repos_named` would answer it but
/// without building a set on every call (#171).
fn names_repo(m: &SystemMap, repo: &str) -> bool {
    m.repos
        .iter()
        .map(|r| r.name.as_str())
        .chain(m.nodes.iter().filter_map(|n| n.repo.as_deref()))
        .any(|r| r.eq_ignore_ascii_case(repo))
}

#[derive(Clone)]
pub struct OverviewInputs {
    pub pins: Vec<IssuePin>,
    pub sessions: Vec<SessionState>,
    pub plans: Vec<PlanRow>,
    /// Each draft's newest settled revision (ADR 0053).
    pub drafts: Vec<crate::plan_drafts::LatestDraft>,
    pub prs: HashMap<IssueRef, PullRequestState>,
    pub decisions: Vec<DecisionRecord>,
    /// `Review:` comments from members (ADR 0027).
    pub reviews: Vec<ReviewRecord>,
    /// Counted approvals (ADR 0054): `Approved:` comments from members posted before the
    /// cutover and approvals given in Rubato, each with the plan hash of the branch it names
    /// when that is not the branch the plan is read from (for a Rubato approval, its draft's
    /// newest revision).
    pub approvals: Vec<(ApprovalRecord, Option<String>)>,
    /// `Approved:` comments posted at or after the cutover: findings, never counted.
    pub ignored_approvals: Vec<ApprovalRecord>,
    /// The cutover is set: plans are approved in Rubato, so one with no draft and no counted
    /// approval is outside Rubato (ADR 0058).
    pub approvals_in_rubato: bool,
    /// What the handoffs of Rubato approvals left (ADR 0055): record mismatches, link comments
    /// waiting, ADRs not accepted. Shown for the issues this overview holds.
    pub approval_findings: Vec<Finding>,
    pub launched: HashSet<IssueRef>,
    /// The live cap hit per issue, by kind.
    pub cap_hits: HashMap<IssueRef, CapKind>,
    pub meta: HashMap<IssueRef, IssueMeta>,
    pub launch_map_versions: Vec<(IssueRef, String)>,
    /// Every listed open issue, and each repo's listing state.
    pub backlog: (Vec<OpenIssue>, Vec<BacklogRepo>),
    /// Repos the App installation does not cover.
    pub uncovered_repos: Vec<String>,
    /// Every stored position, rows for parts the map no longer has included.
    pub layout: MapLayout,
    /// Open pull requests per listed map repo (ADR 0034), checks of tracked ones not filled in.
    pub repo_pulls: Vec<RepoPulls>,
    /// Every queued issue, lanes of revoked companions included; the handler leaves those out.
    pub queue: Vec<QueuedIssue>,
    /// The latest traffic report and its source (ADR 0039). Never an input to progress.
    pub traffic: TrafficLatest,
    /// This workspace's dismissed discrepancies (ADR 0060). Never an input to progress.
    pub dismissals: Vec<rubato_protocol::TrafficDismissal>,
    /// Each issue's spend over time, downsampled (ADR 0037). Copied onto its summary only.
    pub spend: HashMap<IssueRef, Vec<SpendPoint>>,
    /// The other workspaces, for an issue's workspaces and where its plan fits (ADR 0045).
    pub workspaces: WorkspaceInputs,
}

pub struct Built {
    pub issues: Vec<IssueSummary>,
    pub findings: Vec<Finding>,
    pub unpinned_sessions: Vec<SessionState>,
    pub backlog: Backlog,
    /// Stored positions of the parts the map has now.
    pub layout: MapLayout,
    /// What Rubato sees of each repo.
    pub repos: Vec<ConnectedRepo>,
    /// Open pull requests per map repo, checks of tracked ones from `pull_requests` (ADR 0034).
    pub pulls: Vec<RepoPulls>,
    /// Observed traffic per seam; its findings are in `findings` (ADR 0039).
    pub traffic: ObservedTraffic,
}

/// Whether the live cap hit blocks the issue. A spend hit always does (its session is being
/// killed). An attempts hit is a refused *Start*: while a session for the issue is live, the
/// state stays what that session says.
fn cap_hit_blocks(kind: Option<CapKind>, sessions: &[SessionState]) -> bool {
    match kind {
        Some(CapKind::Spend) => true,
        Some(CapKind::Attempts) => sessions.iter().all(|s| s.status == SessionStatus::Ended),
        None => false,
    }
}

/// The newest counted approval of an issue and whether it names the plan as it is now (ADR
/// 0027, ADR 0054). `approvals` holds counted ones only: a comment after the cutover never
/// hides a current approval. An approval that names a branch other than the one the plan is
/// read from is compared with the plan on that branch (`target_sha`: read on the full lane for
/// a comment, the draft's newest revision for a Rubato approval); otherwise with the plan as
/// read.
///
/// An approval of the current plan counts only while the plan fits the map (ADR 0029): with
/// `fits == Some(false)` its version is `DoesNotFit`.
pub fn approval_state(
    approvals: &[(ApprovalRecord, Option<String>)],
    plan: Option<&PlanRow>,
    fits: Option<bool>,
) -> Option<ApprovalState> {
    let (record, target_sha) = approvals
        .iter()
        .max_by_key(|(a, _)| (a.created_at, a.comment_id, a.approval_id))?;
    let elsewhere = record
        .branch
        .as_deref()
        .is_some_and(|b| plan.is_none_or(|p| p.branch != b));
    let current_sha = if elsewhere {
        target_sha.clone()
    } else {
        plan.and_then(|p| p.approval_sha.clone())
    };
    let version = match (&record.sha, &current_sha) {
        (None, _) => ApprovalVersion::NotNamed,
        (Some(_), None) => ApprovalVersion::NoPlan,
        (Some(approved), Some(current)) if approval_names(approved, current) => {
            if fits == Some(false) {
                ApprovalVersion::DoesNotFit
            } else {
                ApprovalVersion::Current
            }
        }
        (Some(_), Some(_)) => ApprovalVersion::Earlier,
    };
    Some(ApprovalState {
        record: record.clone(),
        version,
        current_sha,
    })
}

/// Rows of one table by issue, in their order: grouped once, then each issue's rows moved out
/// with `take` (#171), instead of a linear filter and a copy per issue.
struct ByIssue<T>(HashMap<IssueRef, Vec<T>>);

impl<T> ByIssue<T> {
    fn group(rows: impl IntoIterator<Item = (IssueRef, T)>) -> Self {
        let mut by: HashMap<IssueRef, Vec<T>> = HashMap::new();
        for (issue, row) in rows {
            by.entry(issue).or_default().push(row);
        }
        Self(by)
    }

    fn take(&mut self, issue: &IssueRef) -> Vec<T> {
        self.0.remove(issue).unwrap_or_default()
    }
}

/// What `connected_repos` counts over every row, read before the rows move into the issues.
/// Keys are repo names in lower case: a remote may spell a repo differently from the map.
#[derive(Default)]
struct RepoCounts {
    /// Live sessions with an issue, by the issue's repo.
    sessions: HashMap<String, u32>,
    /// Live sessions without an issue, by the session's repo.
    unattributed: HashMap<String, u32>,
    /// Listed open issues, by repo.
    open_issues: HashMap<String, u32>,
}

impl RepoCounts {
    fn of(sessions: &[SessionState], listed: &[OpenIssue]) -> Self {
        let mut c = Self::default();
        for s in sessions.iter().filter(|s| s.status != SessionStatus::Ended) {
            match (&s.issue, &s.repo) {
                (Some(i), _) => *c.sessions.entry(i.repo.to_ascii_lowercase()).or_default() += 1,
                (None, Some(r)) => *c.unattributed.entry(r.to_ascii_lowercase()).or_default() += 1,
                (None, None) => {}
            }
        }
        for i in listed {
            *c.open_issues
                .entry(i.issue.repo.to_ascii_lowercase())
                .or_default() += 1;
        }
        c
    }
}

/// The issue a handoff finding is about.
fn handoff_issue(f: &Finding) -> Option<&IssueRef> {
    match f {
        Finding::ApprovalRecordMismatch { issue, .. }
        | Finding::LinkCommentWaiting { issue, .. }
        | Finding::LinkWaitingForApprover { issue, .. }
        | Finding::AdrsNotAccepted { issue, .. } => Some(issue),
        _ => None,
    }
}

pub fn build(
    inputs: OverviewInputs,
    map: Option<&LoadedMap>,
    limits: &Limits,
    now: DateTime<Utc>,
) -> Built {
    let OverviewInputs {
        pins,
        sessions,
        plans,
        drafts,
        mut prs,
        decisions,
        reviews,
        approvals,
        ignored_approvals,
        approvals_in_rubato,
        approval_findings,
        launched,
        cap_hits,
        mut meta,
        launch_map_versions,
        backlog: (listed, listings),
        uncovered_repos,
        layout,
        repo_pulls,
        queue: _,
        traffic,
        dismissals,
        mut spend,
        workspaces: ws,
    } = inputs;

    // Issues of interest: pinned, launched, planned, or with a session.
    let mut issues: BTreeSet<IssueRef> = pins.iter().map(|p| p.issue.clone()).collect();
    issues.extend(launched.iter().cloned());
    issues.extend(plans.iter().map(|p| p.issue.clone()));
    issues.extend(drafts.iter().map(|d| d.issue.clone()));
    issues.extend(sessions.iter().filter_map(|s| s.issue.clone()));
    // With several workspaces, only the issues this one holds: pinned or launched here, or in a
    // repo its map names. An issue no workspace holds shows in `default`, so none is lost.
    if let Some((here_id, _)) = &ws.here {
        // Pinned in another workspace, in a repo this map names: shown here with "pinned in".
        if let Some(m) = map {
            for o in &ws.others {
                issues.extend(
                    o.pinned
                        .iter()
                        .filter(|i| names_repo(&m.map, &i.repo))
                        .cloned(),
                );
            }
        }
        let pinned_here: HashSet<&IssueRef> = pins.iter().map(|p| &p.issue).collect();
        issues.retain(|i| {
            pinned_here.contains(i)
                || ws.launched_here.contains(i)
                || map.is_some_and(|m| names_repo(&m.map, &i.repo))
                || (here_id == rubato_protocol::DEFAULT_WORKSPACE
                    && !ws.others.iter().any(|o| o.holds(i)))
        });
    }

    // Read over every row before the rows move into the issues: the backlog leaves out every
    // issue that is pinned, launched or worked on, shown here or not, and the repo counts
    // count every session and listed issue.
    let counts = RepoCounts::of(&sessions, &listed);
    // An issue tracked only because its branch carries a plan is still not pinned: it stays
    // in the backlog until a person or a `Map:` line places it (ADR 0032).
    let backlog_issues: Vec<OpenIssue> = {
        let placed_or_worked: HashSet<&IssueRef> = pins
            .iter()
            .map(|p| &p.issue)
            .chain(launched.iter())
            .chain(sessions.iter().filter_map(|s| s.issue.as_ref()))
            .collect();
        listed
            .into_iter()
            .filter(|i| !placed_or_worked.contains(&i.issue))
            .collect()
    };
    let pulls = pulls_with_checks(&prs, repo_pulls);

    let mut unpinned_sessions: Vec<SessionState> = Vec::new();
    let mut sessions_by = ByIssue::group(sessions.into_iter().filter_map(|s| match &s.issue {
        Some(i) => Some((i.clone(), s)),
        None => {
            unpinned_sessions.push(s);
            None
        }
    }));
    let mut pins_by = ByIssue::group(pins.into_iter().map(|p| (p.issue.clone(), p)));
    // The first plan row of an issue is the one shown, as a `find` over the rows would pick.
    let mut plans_by: HashMap<IssueRef, PlanRow> = HashMap::new();
    for p in plans {
        plans_by.entry(p.issue.clone()).or_insert(p);
    }
    let mut decisions_by = ByIssue::group(decisions.into_iter().map(|d| (d.issue.clone(), d)));
    let mut reviews_by = ByIssue::group(reviews.into_iter().map(|r| (r.issue.clone(), r)));
    let mut approvals_by = ByIssue::group(approvals.into_iter().map(|a| (a.0.issue.clone(), a)));
    let mut versions_by = ByIssue::group(launch_map_versions);
    let mut drafts_by = ByIssue::group(drafts.into_iter().map(|d| (d.issue.clone(), d)));
    let mut ignored_by =
        ByIssue::group(ignored_approvals.into_iter().map(|a| (a.issue.clone(), a)));

    let mut out = Vec::with_capacity(issues.len());
    let mut findings = Vec::new();
    // The handoffs' findings, for the issues this overview holds (ADR 0055); pushed after the
    // per-issue findings, as before.
    let handoff_findings: Vec<Finding> = approval_findings
        .into_iter()
        .filter(|f| handoff_issue(f).is_some_and(|i| issues.contains(i)))
        .collect();
    // Plans written as drafts and not on a branch: they count on seams too (ADR 0053).
    let mut draft_plans: Vec<(IssueRef, Plan)> = Vec::new();
    for issue in issues {
        let issue_pins = pins_by.take(&issue);
        let pinned_at = issue_pins.iter().map(|p| p.pinned_at).max();
        let pins: Vec<MapRef> = issue_pins.into_iter().map(|p| p.target).collect();
        let sessions = sessions_by.take(&issue);
        let mut plan_row = plans_by.remove(&issue);
        // The plan moves onto the summary; the row keeps what the approval reads.
        let plan = plan_row.as_mut().and_then(|p| p.plan.take());
        // The newest draft of the issue (ADR 0053); with several branches, the newest revision.
        let latest_draft = drafts_by
            .take(&issue)
            .into_iter()
            .max_by_key(|d| d.summary.at);
        if let Some(d) = &latest_draft
            && let Some((other, at)) = &d.conflict
            && now.signed_duration_since(*at) < chrono::Duration::hours(24)
        {
            findings.push(Finding::DraftOwnerConflict {
                issue: issue.clone(),
                branch: d.summary.branch.clone(),
                owner: d.summary.owner.clone(),
                other: other.clone(),
            });
        }
        let (draft, draft_plan) = match latest_draft {
            Some(d) => {
                let mut summary = d.summary;
                summary.live = sessions.iter().any(|s| {
                    s.status != SessionStatus::Ended
                        && s.branch.as_deref() == Some(summary.branch.as_str())
                        && s.user.eq_ignore_ascii_case(&summary.owner)
                });
                (Some(summary), d.plan)
            }
            None => (None, None),
        };
        let plan_on_branch = plan.is_some();
        if let Some(row) = &plan_row
            && let Some(err) = &row.parse_error
        {
            findings.push(Finding::PlanParseError {
                issue: issue.clone(),
                path: row.path.clone(),
                error: err.clone(),
            });
        }
        let pr = prs.remove(&issue);
        let cap_kind = cap_hits.get(&issue).copied();
        let progress_inputs = ProgressInputs {
            pins: &pins,
            launched: launched.contains(&issue),
            sessions: &sessions,
            plan_on_branch,
            draft_plan: draft_plan.is_some(),
            cap_hit: cap_kind.filter(|_| cap_hit_blocks(cap_kind, &sessions)),
            pr: pr.as_ref(),
            now,
            session_stale_secs: limits.session_stale_secs,
        };
        let (state, reason) = derive_progress(&progress_inputs);
        // Same inputs, so the station never disagrees with the state (ADR 0043).
        let blocked_from = blocked_from(&progress_inputs);
        let last_seen = sessions
            .iter()
            .map(|s| s.last_seen)
            .chain(pr.as_ref().map(|p| p.updated_at))
            .chain(pinned_at)
            .max()
            .unwrap_or(now);
        // Where the plan fits this workspace's map: read once, for "fits in" and for the fit.
        let fit_here = match (&plan_row, map) {
            (Some(row), Some(m)) => plan_fit(plan.as_ref(), row.parse_error.as_deref(), &m.map),
            _ => None,
        };
        // Where else the plan fits (ADR 0045): an approval counts when it fits any of the
        // issue's workspaces, and unknown ids are a finding only when it fits none.
        let mut fits_in: Vec<String> = Vec::new();
        let mut fit_elsewhere: Option<rubato_protocol::PlanFit> = None;
        if let (Some((_, here_name)), Some(row)) = (&ws.here, &plan_row) {
            if fit_here.as_ref().is_some_and(|f| f.fits) {
                fits_in.push(here_name.clone());
            }
            for o in ws.others.iter().filter(|o| o.holds(&issue)) {
                if let Some(f) = o
                    .map
                    .as_ref()
                    .and_then(|m| plan_fit(plan.as_ref(), row.parse_error.as_deref(), &m.map))
                    .filter(|f| f.fits)
                {
                    fits_in.push(o.name.clone());
                    fit_elsewhere.get_or_insert(f);
                }
            }
        }
        let pinned_in: Vec<String> = ws
            .others
            .iter()
            .filter(|o| o.pinned.contains(&issue))
            .map(|o| o.name.clone())
            .collect();
        if let (Some(p), Some(m), true) =
            (&plan, map, fits_in.is_empty() || fit_elsewhere.is_none())
        {
            let unknown: Vec<String> = p
                .unknown_ids(&m.map)
                .into_iter()
                .map(str::to_owned)
                .collect();
            if !unknown.is_empty() {
                findings.push(Finding::UnknownIds {
                    issue: issue.clone(),
                    ids: unknown,
                });
            }
            for f in p.flows_off_seam(&m.map) {
                findings.push(Finding::FlowOffSeam {
                    issue: issue.clone(),
                    seam: f.seam.clone(),
                    from: f.from.clone(),
                    to: f.to.clone(),
                });
            }
        }
        let meta = meta.remove(&issue).unwrap_or_default();
        // Only while the issue is open on GitHub (#135): a closed issue whose work merged elsewhere
        // never reaches Done, and its old launch would be flagged forever. Unknown is not closed.
        let closed = meta.github_state.as_deref() == Some("closed");
        let versions = versions_by.take(&issue);
        if let (Some(m), false) = (map, closed)
            && state != ProgressState::Done
            && let Some(v) = versions.into_iter().find(|v| *v != m.version)
        {
            findings.push(Finding::MapVersionMismatch {
                issue: issue.clone(),
                launched_with: v,
                current: m.version.clone(),
            });
        }
        let review = reviews_by
            .take(&issue)
            .into_iter()
            .max_by_key(|r| (r.created_at, r.comment_id));
        // Not fitting here but in another of its workspaces: that fit is shown and counts there.
        let fit = match (fit_here, fit_elsewhere) {
            (Some(here), Some(there)) if !here.fits => Some(there),
            (here, _) => here,
        };
        for a in ignored_by.take(&issue) {
            findings.push(Finding::CommentApprovalIgnored {
                issue: issue.clone(),
                author: a.author,
                created_at: a.created_at,
            });
        }
        let approval = approval_state(
            &approvals_by.take(&issue),
            plan_row.as_ref(),
            fit.as_ref().map(|f| f.fits),
        );
        if approval
            .as_ref()
            .is_some_and(|a| a.version == ApprovalVersion::DoesNotFit)
        {
            findings.push(Finding::ApprovedWithoutFit {
                issue: issue.clone(),
            });
        }
        if let Some(a) = &approval
            && a.version == ApprovalVersion::Earlier
            && let (Some(approved), Some(current)) = (&a.record.sha, &a.current_sha)
        {
            findings.push(Finding::ApprovalStale {
                issue: issue.clone(),
                approved_sha: approved.clone(),
                current_sha: current.clone(),
            });
        }
        let spend_usd: Option<f64> = {
            let v: Vec<f64> = sessions.iter().filter_map(|s| s.spend_usd).collect();
            if v.is_empty() {
                None
            } else {
                Some(v.iter().sum())
            }
        };
        let session_ids = sessions
            .iter()
            .filter_map(|s| s.session_id.clone())
            .collect();
        // ADR 0058: a plan on a branch with no draft and no counted approval, once plans are
        // approved in Rubato.
        let outside_rubato = approvals_in_rubato
            && approval.is_none()
            && plan_row
                .as_ref()
                .is_some_and(|p| draft.as_ref().is_none_or(|d| d.branch != p.branch));
        if !plan_on_branch && let Some(p) = draft_plan {
            draft_plans.push((issue.clone(), p));
        }
        out.push(IssueSummary {
            title: meta.title,
            url: meta.url,
            progress: Progress {
                issue: issue.clone(),
                state,
                last_seen,
                pinned_to: pins.clone(),
                session_ids,
                reason,
                blocked_from,
            },
            pins,
            sessions,
            plan,
            pull_request: pr,
            decisions: decisions_by.take(&issue),
            review,
            approval,
            fit,
            plan_sha: plan_row.and_then(|p| p.approval_sha),
            spend_usd,
            spend_history: spend.remove(&issue).unwrap_or_default(),
            fits_in,
            pinned_in,
            outside_rubato,
            draft,
            issue,
        });
    }
    findings.extend(handoff_findings);

    // Two or more open plans on one seam: plans on a branch, then drafts not on one (the same
    // grouping a room's seam notices read, `seam_groups`).
    let done: HashSet<&IssueRef> = out
        .iter()
        .filter(|s| s.progress.state == ProgressState::Done)
        .map(|s| &s.issue)
        .collect();
    let open_plans = out
        .iter()
        .filter(|s| s.progress.state != ProgressState::Done)
        .filter_map(|s| s.plan.as_ref().map(|p| (s.issue.clone(), p)));
    let open_drafts = draft_plans
        .iter()
        .filter(|(issue, _)| !done.contains(issue))
        .map(|(issue, p)| (issue.clone(), p));
    for (seam, issues) in rubato_protocol::seam_groups(open_plans.chain(open_drafts)) {
        if issues.len() > 1 {
            findings.push(Finding::SeamCollision {
                seam: seam.to_owned(),
                issues,
            });
        }
    }

    findings.extend(
        unpinned_sessions
            .iter()
            .filter(|s| s.status != SessionStatus::Ended)
            .map(|s| Finding::UnattributedSession {
                companion: s.companion_id,
                branch: s.branch.clone(),
                repo: s.repo.clone(),
            }),
    );
    findings.extend(
        uncovered_repos
            .iter()
            .map(|repo| Finding::RepoNotCovered { repo: repo.clone() }),
    );
    // Map versus territory (ADR 0039): only a fresh report makes findings. Read after progress
    // is derived and never fed back into it.
    let traffic = match map {
        Some(m) => {
            let (t, f) = derive_traffic(
                &m.map,
                traffic.report.as_ref(),
                &traffic.source,
                &dismissals,
                now,
            );
            findings.extend(f);
            t
        }
        None => ObservedTraffic::default(),
    };
    let repos = connected_repos(&counts, &uncovered_repos, map, &out, &pulls);

    // The backlog: listed open issues that are not tracked above, and one entry per repo the
    // map names, listed or not, so "not listed yet" is said and not shown as empty.
    let backlog = Backlog {
        issues: backlog_issues,
        repos: match map {
            Some(m) => m
                .map
                .repos_named()
                .into_iter()
                .map(|repo| {
                    listings
                        .iter()
                        .find(|l| l.repo == repo)
                        .cloned()
                        .unwrap_or(BacklogRepo {
                            repo: repo.to_owned(),
                            listed_at: None,
                            truncated: false,
                            error: None,
                        })
                })
                .collect(),
            None => listings,
        },
    };
    // Positions only for parts the map has now, and boxes only for repos its parts name (ADR
    // 0068): a row for a removed part or repo is not served (and is deleted by the next layout
    // write). With no map loaded there is nothing to place.
    let layout = match map {
        Some(m) => {
            let parts = m.map.node_ids();
            let repos: BTreeSet<&str> = m
                .map
                .nodes
                .iter()
                .filter_map(|n| n.repo.as_deref())
                .collect();
            let keep = |from: BTreeMap<String, Position>, known: &dyn Fn(&str) -> bool| {
                from.into_iter()
                    .filter(|(id, _)| known(id.as_str()))
                    .collect()
            };
            MapLayout {
                positions: keep(layout.positions, &|id| parts.contains(id)),
                grouped: keep(layout.grouped, &|id| parts.contains(id)),
                boxes: keep(layout.boxes, &|id| repos.contains(id)),
            }
        }
        None => MapLayout::default(),
    };
    Built {
        issues: out,
        findings,
        unpinned_sessions,
        backlog,
        layout,
        repos,
        pulls,
        traffic,
    }
}

/// The stored listings, with each tracked pull request's checks taken from `pull_requests`
/// (ADR 0023 reads them every tick), so the two never disagree and nothing is copied.
fn pulls_with_checks(
    prs: &HashMap<IssueRef, PullRequestState>,
    repo_pulls: Vec<RepoPulls>,
) -> Vec<RepoPulls> {
    let tracked: HashMap<(&str, u64), &PullRequestState> = prs
        .values()
        .map(|p| ((p.repo.as_str(), p.number), p))
        .collect();
    repo_pulls
        .into_iter()
        .map(|mut r| {
            for p in &mut r.pulls {
                if let Some(t) = tracked.get(&(p.repo.as_str(), p.number)) {
                    p.checks = Some(t.checks);
                }
            }
            r
        })
        .collect()
}

/// One row per repo the map names, then repos of tracked work outside the map. Repo names are
/// compared without case: a remote may spell a repo differently from the map.
fn connected_repos(
    counts: &RepoCounts,
    uncovered_repos: &[String],
    map: Option<&LoadedMap>,
    issues: &[IssueSummary],
    pulls: &[RepoPulls],
) -> Vec<ConnectedRepo> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let mut names: Vec<(String, bool)> = map
        .map(|m| {
            m.map
                .repos_named()
                .into_iter()
                .map(|r| (r.to_owned(), true))
                .collect()
        })
        .unwrap_or_default();
    for repo in issues.iter().map(|i| i.issue.repo.as_str()) {
        if !names.iter().any(|(n, _)| same(n, repo)) {
            names.push((repo.to_owned(), false));
        }
    }
    names
        .into_iter()
        .map(|(repo, in_map)| {
            let key = repo.to_ascii_lowercase();
            let count = |by: &HashMap<String, u32>| by.get(&key).copied().unwrap_or(0);
            ConnectedRepo {
                open_pulls: pulls
                    .iter()
                    .filter(|p| same(&p.repo, &repo))
                    .map(|p| p.pulls.len() as u32)
                    .sum(),
                covered: !uncovered_repos.iter().any(|u| same(u, &repo)),
                open_issues: count(&counts.open_issues),
                plans: issues
                    .iter()
                    .filter(|i| same(&i.issue.repo, &repo) && i.plan.is_some())
                    .count() as u32,
                sessions: count(&counts.sessions),
                unattributed: count(&counts.unattributed),
                in_map,
                repo,
            }
        })
        .collect()
}

/// Counts for the `rubato.issues.by_state` and `rubato.seam.open_plans` gauges.
pub fn counts(built: &Built) -> (BTreeMap<ProgressState, usize>, BTreeMap<String, usize>) {
    let mut by_state = BTreeMap::new();
    for i in &built.issues {
        *by_state.entry(i.progress.state).or_insert(0) += 1;
    }
    let mut by_seam = BTreeMap::new();
    for i in &built.issues {
        if i.progress.state == ProgressState::Done {
            continue;
        }
        if let Some(p) = &i.plan {
            for s in p.seams_touched() {
                *by_seam.entry(s.to_owned()).or_insert(0) += 1;
            }
        }
    }
    (by_state, by_seam)
}

/// Blocked issues by the station they left from, for `rubato.issues.blocked_from`: all five
/// values, zeros included (A6), so the sum always equals the blocked count of [`counts`].
pub fn blocked_from_counts(built: &Built) -> BTreeMap<&'static str, usize> {
    let mut by: BTreeMap<&'static str, usize> =
        BLOCKED_FROM_VALUES.iter().map(|v| (*v, 0)).collect();
    for i in built
        .issues
        .iter()
        .filter(|i| i.progress.state == ProgressState::Blocked)
    {
        *by.entry(blocked_from_label(i.progress.blocked_from))
            .or_insert(0) += 1;
    }
    by
}

/// The values of `rubato.issue.blocked_from`.
pub const BLOCKED_FROM_VALUES: [&str; 5] =
    ["started", "designing", "designed", "building", "off_rail"];

fn blocked_from_label(at: Option<ProgressState>) -> &'static str {
    match at {
        Some(ProgressState::Started) => "started",
        Some(ProgressState::Designing) => "designing",
        Some(ProgressState::Designed) => "designed",
        Some(ProgressState::Building) => "building",
        // `blocked_from` names only the four rail stations; anything else is off the rail.
        _ => "off_rail",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rubato_protocol::{
        AffectedPart, BlockCause, ORow, PartStatus, Plan, StateSource, SystemMap,
    };
    use uuid::Uuid;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
    }

    fn map() -> LoadedMap {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/design/rubato-map.yml"
        ))
        .unwrap();
        let map: SystemMap = serde_yaml_ng::from_str(&text).unwrap();
        LoadedMap {
            map,
            version: "abc".into(),
            loaded_at: now(),
            yaml: text,
        }
    }

    fn plan(issue: &IssueRef, seam: &str, node: &str) -> PlanRow {
        PlanRow {
            issue: issue.clone(),
            branch: format!("issue/{}-x", issue.number),
            path: "docs/plans/x.md".into(),
            plan: Some(Plan {
                issue: issue.clone(),
                title: None,
                path: "docs/plans/x.md".into(),
                git_ref: None,
                spend_cap_usd: None,
                attempt_limit: None,
                affected_parts: vec![AffectedPart {
                    node: node.into(),
                    status: PartStatus::Affected,
                    change: "x".into(),
                    seams: vec![seam.into()],
                }],
                o_rows: vec![ORow {
                    seam: seam.into(),
                    breaks_like: None,
                    signal: "s".into(),
                    panel: "p".into(),
                    owner: "rubato-server".into(),
                }],
                assumed: vec![],
                flows: vec![],
                data_models: vec![],
                decisions: vec![],
                approval: None,
            }),
            parse_error: None,
            approval_sha: Some("f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9".into()),
            head_sha: None,
        }
    }

    fn session(issue: &IssueRef, status: SessionStatus, source: StateSource) -> SessionState {
        SessionState {
            session_id: Some("s".into()),
            companion_id: Uuid::nil(),
            user: "m0x6A".into(),
            tmux: None,
            cwd: "/w".into(),
            worktree: None,
            branch: None,
            repo: Some(issue.repo.clone()),
            issue: Some(issue.clone()),
            status,
            source,
            last_seen: now(),
            spend_usd: Some(1.5),
            branch_drift: None,
            blocked_by: None,
            launch_id: None,
            hooks_lost: false,
        }
    }

    pub(super) fn empty_inputs() -> OverviewInputs {
        empty()
    }

    fn empty() -> OverviewInputs {
        OverviewInputs {
            pins: vec![],
            sessions: vec![],
            plans: vec![],
            drafts: vec![],
            prs: HashMap::new(),
            decisions: vec![],
            reviews: vec![],
            approvals: vec![],
            ignored_approvals: vec![],
            approvals_in_rubato: false,
            approval_findings: vec![],
            launched: HashSet::new(),
            cap_hits: HashMap::new(),
            meta: HashMap::new(),
            launch_map_versions: vec![],
            backlog: (vec![], vec![]),
            uncovered_repos: vec![],
            layout: MapLayout::default(),
            queue: vec![],
            repo_pulls: vec![],
            traffic: TrafficLatest {
                report: None,
                source: rubato_protocol::SourceStatus::NotConfigured,
            },
            dismissals: vec![],
            spend: HashMap::new(),
            workspaces: Default::default(),
        }
    }

    /// Plan #104: traffic is never an input to progress. The same inputs with and without a
    /// fresh report give the same summary for every issue; only the traffic findings differ.
    #[test]
    fn traffic_does_not_change_any_issue_summary() {
        use rubato_protocol::{SourceStatus, TrafficEdge, TrafficReport, TrafficState};
        let a = IssueRef::new("m0x6A/rubato", 1);
        let b = IssueRef::new("m0x6A/rubato", 2);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "web-server", "web"), plan(&b, "web-server", "web")];
        inputs.sessions = vec![session(&a, SessionStatus::Working, StateSource::Hook)];
        inputs.pins = vec![IssuePin {
            issue: b.clone(),
            target: MapRef::node("web"),
            pinned_by: "m0x6A".into(),
            pinned_at: now(),
        }];
        // Each part declares the namespaceless service it is (ADR 0059), so the report matches.
        let mut map = map();
        for n in &mut map.map.nodes {
            n.service = Some(rubato_protocol::ServiceId::new(None, &n.id));
        }
        let map = &map;
        let without = build(inputs.clone(), Some(map), &Limits::default(), now());
        let edge = |client: &str, server: &str, calls| TrafficEdge {
            client: client.into(),
            server: server.into(),
            calls,
            errors: 0,
            client_namespace: None,
            server_namespace: None,
        };
        inputs.traffic = TrafficLatest {
            report: Some(TrafficReport {
                observed_at: now(),
                window_secs: 3600,
                edges: vec![
                    edge("web", "rubato-server", 100),
                    edge("rubato-companion", "github", 3),
                    TrafficEdge {
                        client_namespace: Some("shop".into()),
                        ..edge("checkout", "rubato-server", 4)
                    },
                ],
                truncated: false,
            }),
            source: SourceStatus::Configured {
                fetch_secs: 60,
                error: None,
            },
        };
        // A dismissal is no input to progress either (ADR 0060).
        inputs.dismissals = vec![rubato_protocol::TrafficDismissal {
            id: 1,
            target: rubato_protocol::DiscrepancyKey::QuietSeam {
                seam: "server-github".into(),
            },
            author: "m0x6A".into(),
            at: now(),
        }];
        let with = build(inputs.clone(), Some(map), &Limits::default(), now());
        assert_eq!(with.traffic.ghosts.len(), 1, "{:?}", with.traffic);
        assert_eq!(with.traffic.dismissed.len(), 1);
        assert_eq!(with.traffic.state, TrafficState::Fresh);
        assert_eq!(without.traffic.state, TrafficState::NotConfigured);
        assert!(!with.issues.is_empty());
        assert_eq!(
            with.issues, without.issues,
            "every IssueSummary is unchanged"
        );
        let traffic_findings = with.findings.len() - without.findings.len();
        assert!(traffic_findings >= 2, "{:?}", with.findings);
        println!(
            "compared {} issue summaries with and without a report",
            with.issues.len()
        );
    }

    #[test]
    fn sessions_without_an_issue_are_findings_and_repos_count_them() {
        let a = IssueRef::new("m0x6A/rubato", 1);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "web-server", "web")];
        let mut loose = session(&a, SessionStatus::Working, StateSource::Hook);
        loose.issue = None;
        loose.branch = Some("main".into());
        let mut ended = loose.clone();
        ended.status = SessionStatus::Ended;
        inputs.sessions = vec![
            session(&a, SessionStatus::Working, StateSource::Hook),
            loose,
            ended,
        ];
        inputs.uncovered_repos = vec!["other/repo".into()];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let unattributed: Vec<_> = built
            .findings
            .iter()
            .filter(|f| matches!(f, Finding::UnattributedSession { branch, .. } if branch.as_deref() == Some("main")))
            .collect();
        assert_eq!(unattributed.len(), 1, "an ended session is not a finding");
        let repo = built
            .repos
            .iter()
            .find(|r| r.repo.eq_ignore_ascii_case("m0x6A/rubato"))
            .expect("the map's repo");
        assert_eq!(
            (
                repo.in_map,
                repo.covered,
                repo.plans,
                repo.sessions,
                repo.unattributed
            ),
            (true, true, 1, 1, 1)
        );
    }

    #[test]
    fn seam_collision_and_unknown_ids_are_findings() {
        let a = IssueRef::new("o/r", 1);
        let b = IssueRef::new("o/r", 2);
        let mut inputs = empty();
        inputs.plans = vec![
            plan(&a, "companion-server", "rubato-companion"),
            plan(&b, "companion-server", "rubato-orchestrator"),
        ];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert!(built.findings.iter().any(|f| matches!(f, Finding::SeamCollision { seam, issues } if seam == "companion-server" && issues.len() == 2)));
        assert!(built.findings.iter().any(|f| matches!(f, Finding::UnknownIds { issue, ids } if *issue == b && ids == &vec!["rubato-orchestrator".to_string()])));
        assert!(
            built
                .issues
                .iter()
                .all(|i| i.progress.state == ProgressState::Designed)
        );
    }

    /// ADR 0053: drafts not on a branch count on seams, an idle session with a draft is Designed,
    /// and another person's refused bundle is a finding.
    #[test]
    fn drafts_count_on_seams_and_as_written_plans() {
        let a = IssueRef::new("o/r", 1);
        let b = IssueRef::new("o/r", 2);
        let draft = |issue: &IssueRef, conflict: bool| crate::plan_drafts::LatestDraft {
            issue: issue.clone(),
            summary: rubato_protocol::DraftSummary {
                draft_id: uuid::Uuid::new_v4(),
                branch: format!("issue/{}-x", issue.number),
                owner: "m0x6A".into(),
                rev: 1,
                at: now(),
                plan_sha: "s".into(),
                bundle_hash: "h".into(),
                missing: vec![],
                parse_error: None,
                live: false,
                imported_from: None,
                original_author: None,
            },
            plan: plan(issue, "companion-server", "rubato-companion").plan,
            conflict: conflict.then(|| ("alice".to_owned(), now())),
        };
        let mut inputs = empty();
        inputs.drafts = vec![draft(&a, false), draft(&b, true)];
        let mut s = session(&a, SessionStatus::Idle, StateSource::Hook);
        s.branch = Some("issue/1-x".into());
        inputs.sessions = vec![s];
        let built = build(inputs, Some(&map()), &Limits::default(), now());
        assert!(built.findings.iter().any(|f| matches!(f, Finding::SeamCollision { seam, issues } if seam == "companion-server" && issues.len() == 2)), "two drafts on one seam");
        assert!(built.findings.iter().any(|f| matches!(f, Finding::DraftOwnerConflict { issue, other, .. } if *issue == b && other == "alice")));
        let ia = built.issues.iter().find(|i| i.issue == a).unwrap();
        assert_eq!(ia.progress.state, ProgressState::Designed);
        assert!(
            ia.draft.as_ref().unwrap().live,
            "the owner's session is on the draft's branch"
        );
        let ib = built.issues.iter().find(|i| i.issue == b).unwrap();
        assert!(!ib.draft.as_ref().unwrap().live, "parked");
        assert_eq!(
            ib.progress.state,
            ProgressState::Designed,
            "a parked draft is written"
        );
    }

    #[test]
    fn working_session_with_plan_is_building_and_spend_sums() {
        let a = IssueRef::new("o/r", 1);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "companion-server", "rubato-companion")];
        inputs.sessions = vec![
            session(&a, SessionStatus::Working, StateSource::Hook),
            session(&a, SessionStatus::Idle, StateSource::Hook),
        ];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert_eq!(built.issues[0].progress.state, ProgressState::Building);
        assert_eq!(built.issues[0].spend_usd, Some(3.0));
        let (by_state, by_seam) = counts(&built);
        assert_eq!(by_state[&ProgressState::Building], 1);
        assert_eq!(by_seam["companion-server"], 1);
    }

    #[test]
    fn an_attempts_cap_hit_blocks_only_without_a_live_session_and_spend_always() {
        let a = IssueRef::new("o/r", 1);
        let state = |kind: CapKind, status: Option<SessionStatus>| {
            let mut inputs = empty();
            inputs.launched = HashSet::from([a.clone()]);
            inputs.cap_hits = HashMap::from([(a.clone(), kind)]);
            inputs.sessions = status
                .map(|s| session(&a, s, StateSource::Hook))
                .into_iter()
                .collect();
            let built = build(inputs.clone(), None, &Limits::default(), now());
            built.issues[0].progress.state
        };
        let working = Some(SessionStatus::Working);
        let ended = Some(SessionStatus::Ended);
        let cases = [
            (CapKind::Attempts, working, ProgressState::Designing),
            (CapKind::Attempts, ended, ProgressState::Blocked),
            (CapKind::Attempts, None, ProgressState::Blocked),
            (CapKind::Spend, working, ProgressState::Blocked),
            (CapKind::Spend, ended, ProgressState::Blocked),
            (CapKind::Spend, None, ProgressState::Blocked),
        ];
        for (kind, status, want) in cases {
            assert_eq!(
                state(kind, status),
                want,
                "{kind:?} with session {status:?}"
            );
        }
        println!("checked {} cap hit cases", cases.len());
    }

    /// Issue #106: a session blocked mid-turn after the plan leaves from Building.
    #[test]
    fn blocked_while_building_says_building() {
        let a = IssueRef::new("o/r", 1);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "web-server", "web")];
        let mut s = session(&a, SessionStatus::Blocked, StateSource::Hook);
        s.blocked_by = Some(BlockCause::PermissionPrompt);
        inputs.sessions = vec![s];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let p = &built.issues[0].progress;
        assert_eq!(p.state, ProgressState::Blocked);
        assert_eq!(p.blocked_from, Some(ProgressState::Building));
        // Waiting for input after its turn, the same session leaves from Designed.
        inputs.sessions[0].blocked_by = Some(BlockCause::IdlePrompt);
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert_eq!(
            built.issues[0].progress.blocked_from,
            Some(ProgressState::Designed)
        );
    }

    /// Issue #106: blocked before anything ran (a refused Start, no session) is off the rail,
    /// and the field is not on the wire.
    #[test]
    fn blocked_before_any_session_is_off_the_rail() {
        let a = IssueRef::new("o/r", 1);
        let mut inputs = empty();
        inputs.pins = vec![IssuePin {
            issue: a.clone(),
            target: MapRef::node("web"),
            pinned_by: "m0x6A".into(),
            pinned_at: now(),
        }];
        inputs.cap_hits = HashMap::from([(a.clone(), CapKind::Attempts)]);
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let p = &built.issues[0].progress;
        assert_eq!(p.state, ProgressState::Blocked);
        assert_eq!(p.blocked_from, None);
        let v = serde_json::to_value(p).unwrap();
        assert!(v.get("blocked_from").is_none(), "{v}");
        let by = blocked_from_counts(&built);
        assert_eq!(by["off_rail"], 1);
    }

    /// An older companion sends no `blocked_by`: the block reads as mid-turn.
    #[test]
    fn a_block_without_a_cause_reads_as_mid_turn() {
        let a = IssueRef::new("o/r", 1);
        let mut inputs = empty();
        inputs.sessions = vec![session(&a, SessionStatus::Blocked, StateSource::Hook)];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert_eq!(
            built.issues[0].progress.blocked_from,
            Some(ProgressState::Designing)
        );
        inputs.plans = vec![plan(&a, "web-server", "web")];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert_eq!(
            built.issues[0].progress.blocked_from,
            Some(ProgressState::Building)
        );
    }

    /// The gauge's five series always exist and add up to the blocked bar of issues by state.
    #[test]
    fn blocked_from_counts_cover_every_value_and_sum_to_blocked() {
        let mut inputs = empty();
        let a = IssueRef::new("o/r", 1);
        let b = IssueRef::new("o/r", 2);
        let c = IssueRef::new("o/r", 3);
        let mut sa = session(&a, SessionStatus::Blocked, StateSource::Hook);
        sa.blocked_by = Some(BlockCause::StopFailure);
        let sb = session(&b, SessionStatus::Working, StateSource::Hook);
        inputs.sessions = vec![sa, sb];
        inputs.plans = vec![plan(&a, "web-server", "web")];
        inputs.launched = HashSet::from([c.clone()]);
        inputs.cap_hits = HashMap::from([(c.clone(), CapKind::Attempts)]);
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let by = blocked_from_counts(&built);
        let keys: Vec<&str> = by.keys().copied().collect();
        assert_eq!(
            keys,
            ["building", "designed", "designing", "off_rail", "started"]
        );
        assert_eq!((by["building"], by["started"]), (1, 1));
        let (by_state, _) = counts(&built);
        assert_eq!(
            by.values().sum::<usize>(),
            by_state[&ProgressState::Blocked]
        );
        println!("checked {} blocked_from values", by.len());
    }

    fn open(repo: &str, n: u64) -> OpenIssue {
        OpenIssue {
            issue: IssueRef::new(repo, n),
            title: format!("Issue {n}"),
            url: String::new(),
            labels: vec![],
            updated_at: now(),
            map_line: None,
            suggestion: None,
        }
    }

    #[test]
    fn layout_is_served_for_known_parts_and_changes_no_progress() {
        use rubato_protocol::layout::Position;
        let repo = "m0x6A/rubato";
        let mut inputs = empty();
        inputs.pins = vec![IssuePin {
            issue: IssueRef::new(repo, 1),
            target: MapRef::node("rubato-server"),
            pinned_by: "m0x6A".into(),
            pinned_at: now(),
        }];
        let before = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert!(before.layout.positions.is_empty());

        let at = |x: f64| Position { x, y: 1.0 };
        inputs.layout.positions = [
            ("rubato-server".to_owned(), at(5.0)),
            ("gone".to_owned(), at(9.0)),
        ]
        .into();
        // The By repo grouping (ADR 0068): the same rule, and a box only for a repo a part names.
        inputs.layout.grouped = [
            ("rubato-server".to_owned(), at(6.0)),
            ("gone".to_owned(), at(9.0)),
        ]
        .into();
        inputs.layout.boxes = [
            ("m0x6A/rubato".to_owned(), at(7.0)),
            ("acme/gone".to_owned(), at(9.0)),
        ]
        .into();
        let after = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let placed: Vec<&str> = after.layout.positions.keys().map(String::as_str).collect();
        assert_eq!(
            placed,
            ["rubato-server"],
            "a part the map lacks is left out"
        );
        let grouped: Vec<&str> = after.layout.grouped.keys().map(String::as_str).collect();
        assert_eq!(grouped, ["rubato-server"]);
        let boxes: Vec<&str> = after.layout.boxes.keys().map(String::as_str).collect();
        assert_eq!(boxes, ["m0x6A/rubato"], "a repo no part names is left out");
        assert_eq!(
            after.issues, before.issues,
            "a position is not a progress signal"
        );
        assert_eq!(after.findings, before.findings);
        // With no map there is nothing to place.
        let none = build(inputs.clone(), None, &Limits::default(), now());
        assert!(none.layout.is_empty());
    }

    #[test]
    fn queue_layout_and_draft_do_not_change_progress() {
        use rubato_protocol::layout::Position;
        let repo = "m0x6A/rubato";
        let pinned = IssueRef::new(repo, 1);
        let mut inputs = empty();
        inputs.pins = vec![IssuePin {
            issue: pinned.clone(),
            target: MapRef::node("rubato-server"),
            pinned_by: "m0x6A".into(),
            pinned_at: now(),
        }];
        let before = build(inputs.clone(), Some(&map()), &Limits::default(), now());

        // The pinned issue and one nobody tracks are queued, and a part is moved.
        let queued = |issue: IssueRef, position: u32| QueuedIssue {
            issue,
            companion_id: Uuid::nil(),
            queued_by: "m0x6A".into(),
            queued_at: now(),
            position,
            last_start_error: Some("missing_dependency: tmux 3.7 is needed".into()),
        };
        inputs.queue = vec![queued(pinned.clone(), 0), queued(IssueRef::new(repo, 2), 1)];
        inputs.layout.positions =
            [("rubato-server".to_owned(), Position { x: 1.0, y: 2.0 })].into();
        inputs.layout.grouped = [("web".to_owned(), Position { x: 3.0, y: 4.0 })].into();
        inputs.layout.boxes = [("m0x6A/rubato".to_owned(), Position { x: 5.0, y: 6.0 })].into();
        let after = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert_eq!(
            after.issues, before.issues,
            "state and reason are unchanged"
        );
        assert_eq!(after.findings, before.findings);
        assert_eq!(
            after.issues.len(),
            1,
            "a queued issue nobody pinned does not become a tracked issue"
        );
    }

    #[test]
    fn flow_off_seam_is_a_finding() {
        use rubato_protocol::{FlowKind, PlanFlow};
        let a = IssueRef::new("o/r", 1);
        let flow = |from: &str, to: &str, seam: &str| PlanFlow {
            from: from.into(),
            to: to.into(),
            seam: seam.into(),
            kind: FlowKind::Data,
            carries: "x".into(),
        };
        let mut row = plan(&a, "web-server", "rubato-server");
        row.plan.as_mut().unwrap().flows = vec![
            // Along the seam, either way: fine.
            flow("web", "rubato-server", "web-server"),
            flow("rubato-server", "web", "web-server"),
            // The seam does not join these two parts.
            flow("web", "github", "web-server"),
            // An unknown seam is an unknown id, reported once as that.
            flow("web", "rubato-server", "no-such-seam"),
        ];
        let mut inputs = empty();
        inputs.plans = vec![row];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let off: Vec<&Finding> = built
            .findings
            .iter()
            .filter(|f| matches!(f, Finding::FlowOffSeam { .. }))
            .collect();
        assert_eq!(
            off,
            [&Finding::FlowOffSeam {
                issue: a.clone(),
                seam: "web-server".into(),
                from: "web".into(),
                to: "github".into(),
            }]
        );
        assert!(built.findings.iter().any(|f| matches!(f, Finding::UnknownIds { ids, .. } if ids == &vec!["no-such-seam".to_string()])));
    }

    #[test]
    fn backlog_excludes_tracked_issues() {
        let repo = "m0x6A/rubato";
        let mut inputs = empty();
        inputs.backlog = (vec![open(repo, 1), open(repo, 2), open(repo, 3)], vec![]);
        // #1 is pinned and #2 has a session: both are tracked and so not in the backlog.
        inputs.pins = vec![IssuePin {
            issue: IssueRef::new(repo, 1),
            target: MapRef::node("rubato-server"),
            pinned_by: "m0x6A".into(),
            pinned_at: now(),
        }];
        inputs.sessions = vec![session(
            &IssueRef::new(repo, 2),
            SessionStatus::Working,
            StateSource::Hook,
        )];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let backlog: Vec<u64> = built
            .backlog
            .issues
            .iter()
            .map(|i| i.issue.number)
            .collect();
        assert_eq!(backlog, [3]);
        let tracked: Vec<u64> = built.issues.iter().map(|i| i.issue.number).collect();
        assert_eq!(tracked, [1, 2]);
        // A backlog issue has no progress: it is not among the issues at all.
        assert!(built.issues.iter().all(|i| i.issue.number != 3));
    }

    #[test]
    fn every_map_repo_has_a_listing_entry_even_before_its_first_listing() {
        let mut inputs = empty();
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        let loaded = map();
        let named: Vec<&str> = loaded.map.repos_named().into_iter().collect();
        assert!(!named.is_empty(), "the committed map names a repo");
        assert_eq!(
            built
                .backlog
                .repos
                .iter()
                .map(|r| r.repo.as_str())
                .collect::<Vec<_>>(),
            named
        );
        assert!(
            built
                .backlog
                .repos
                .iter()
                .all(|r| r.listed_at.is_none() && r.error.is_none()),
            "not listed yet is said, not shown as an empty list"
        );
        // A failed listing carries its error and the last good time through.
        inputs.backlog.1 = vec![BacklogRepo {
            repo: named[0].to_owned(),
            listed_at: Some(now()),
            truncated: true,
            error: Some("rate limited".into()),
        }];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert_eq!(
            built.backlog.repos[0].error.as_deref(),
            Some("rate limited")
        );
        assert!(built.backlog.repos[0].truncated);
        assert_eq!(built.backlog.repos[0].listed_at, Some(now()));
    }

    #[test]
    fn an_uncovered_repo_is_a_finding() {
        let mut inputs = empty();
        inputs.uncovered_repos = vec!["o/left-out".into()];
        let built = build(inputs.clone(), Some(&map()), &Limits::default(), now());
        assert!(
            built
                .findings
                .iter()
                .any(|f| matches!(f, Finding::RepoNotCovered { repo } if repo == "o/left-out")),
            "{:?}",
            built.findings
        );
    }

    #[test]
    fn unpinned_sessions_are_listed_separately() {
        let mut inputs = empty();
        let mut s = session(
            &IssueRef::new("o/r", 1),
            SessionStatus::Working,
            StateSource::Scraped,
        );
        s.issue = None;
        inputs.sessions = vec![s];
        let built = build(inputs.clone(), None, &Limits::default(), now());
        assert!(built.issues.is_empty());
        assert_eq!(built.unpinned_sessions.len(), 1);
    }

    const CURRENT: &str = "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9";
    const EARLIER: &str = "c438e006b75b48508ae0d204788e73e11005a6d3";

    fn approval(
        issue: &IssueRef,
        id: u64,
        sha: Option<&str>,
        branch: Option<&str>,
    ) -> ApprovalRecord {
        ApprovalRecord {
            issue: issue.clone(),
            source: rubato_protocol::ApprovalSource::Comment,
            comment_id: Some(id),
            approval_id: None,
            bundle_hash: None,
            author: "m0x6A".into(),
            sha: sha.map(str::to_owned),
            branch: branch.map(str::to_owned),
            note: None,
            created_at: now() + chrono::Duration::seconds(id as i64),
            url: "u".into(),
        }
    }

    fn approval_of(inputs: &OverviewInputs, issue: &IssueRef) -> (ApprovalVersion, Vec<Finding>) {
        let built = build(inputs.clone(), None, &Limits::default(), now());
        let one = built.issues.iter().find(|i| &i.issue == issue).unwrap();
        let stale = built
            .findings
            .iter()
            .filter(|f| matches!(f, Finding::ApprovalStale { .. }))
            .cloned()
            .collect();
        (one.approval.as_ref().unwrap().version, stale)
    }

    #[test]
    fn an_approval_of_the_plan_as_read_is_current() {
        let a = IssueRef::new("o/r", 71);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "companion-server", "rubato-companion")];
        inputs.approvals = vec![(approval(&a, 1, Some(&CURRENT[..7]), None), None)];
        assert_eq!(approval_of(&inputs, &a), (ApprovalVersion::Current, vec![]));
    }

    #[test]
    fn an_approval_of_an_earlier_version_is_stale_and_a_finding() {
        let a = IssueRef::new("o/r", 71);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "companion-server", "rubato-companion")];
        inputs.approvals = vec![(approval(&a, 1, Some(EARLIER), None), None)];
        let (version, stale) = approval_of(&inputs, &a);
        assert_eq!(version, ApprovalVersion::Earlier);
        assert_eq!(
            stale,
            vec![Finding::ApprovalStale {
                issue: a.clone(),
                approved_sha: EARLIER.into(),
                current_sha: CURRENT.into(),
            }]
        );
        // A newer approval of the current version wins.
        inputs
            .approvals
            .push((approval(&a, 2, Some(CURRENT), None), None));
        assert_eq!(approval_of(&inputs, &a), (ApprovalVersion::Current, vec![]));
    }

    /// Two `issue/71-*` branches: the plan is read from the first, the approval names the
    /// other. It is compared with the plan on the branch it names, not the one read.
    #[test]
    fn an_approval_on_another_branch_is_compared_with_that_branch() {
        let a = IssueRef::new("o/r", 71);
        let mut inputs = empty();
        let mut row = plan(&a, "companion-server", "rubato-companion");
        row.branch = "issue/71-create-issue".into();
        row.approval_sha = Some(EARLIER.into());
        inputs.plans = vec![row];
        inputs.approvals = vec![(
            approval(&a, 1, Some(CURRENT), Some("issue/71-plan")),
            Some(CURRENT.into()),
        )];
        assert_eq!(approval_of(&inputs, &a), (ApprovalVersion::Current, vec![]));
        // That branch moved on since: stale against it, whatever the branch read says.
        inputs.approvals[0].1 = Some(EARLIER.into());
        inputs.plans[0].approval_sha = Some(CURRENT.into());
        assert_eq!(approval_of(&inputs, &a).0, ApprovalVersion::Earlier);
        // That branch has no plan file.
        inputs.approvals[0].1 = None;
        assert_eq!(approval_of(&inputs, &a).0, ApprovalVersion::NoPlan);
        // Naming the branch the plan is read from compares with the plan as read.
        inputs.approvals[0].0.branch = Some("issue/71-create-issue".into());
        assert_eq!(approval_of(&inputs, &a).0, ApprovalVersion::Current);
    }

    /// A Rubato approval of a draft not on a branch yet (ADR 0054), as `approve::approvals`
    /// gives it: compared with the draft's newest revision.
    fn rubato_approval(
        issue: &IssueRef,
        at_secs: i64,
        sha: &str,
        newest: &str,
    ) -> (crate::approve::RubatoApproval, Option<String>) {
        (
            crate::approve::RubatoApproval {
                id: Uuid::from_u128(at_secs as u128),
                issue: issue.clone(),
                branch: format!("issue/{}-x", issue.number),
                draft_id: Uuid::from_u128(99),
                rev: 1,
                plan_sha: sha.into(),
                bundle_hash: "h".into(),
                approved_by: "teammate".into(),
                approved_at: now() + chrono::Duration::seconds(at_secs),
                self_approved: false,
                summary: String::new(),
                handoff: None,
            },
            Some(newest.into()),
        )
    }

    /// The draft that puts an issue with no plan on a branch in the overview.
    fn draft_of(issue: &IssueRef, sha: &str) -> crate::plan_drafts::LatestDraft {
        crate::plan_drafts::LatestDraft {
            issue: issue.clone(),
            summary: rubato_protocol::DraftSummary {
                draft_id: Uuid::from_u128(99),
                branch: format!("issue/{}-x", issue.number),
                owner: "m0x6A".into(),
                rev: 1,
                at: now(),
                plan_sha: sha.into(),
                bundle_hash: "h".into(),
                missing: vec![],
                parse_error: None,
                live: false,
                imported_from: None,
                original_author: None,
            },
            plan: None,
            conflict: None,
        }
    }

    /// ADR 0054: one list, the newest counted approval decides. A comment posted after the
    /// cutover is newer than the Rubato approval but never hides it, and is a finding; without a
    /// cutover the same comment wins, so the check cannot pass by accident.
    #[test]
    fn a_comment_after_the_cutover_never_hides_a_current_rubato_approval() {
        let a = IssueRef::new("o/r", 71);
        let comment = (approval(&a, 30, Some(EARLIER), None), None);
        let rubato = rubato_approval(&a, 10, CURRENT, CURRENT);
        let cutover = Some(now() + chrono::Duration::seconds(20));
        let merged = crate::approve::merge(vec![comment.clone()], vec![rubato.clone()], cutover);
        let mut inputs = empty();
        inputs.drafts = vec![draft_of(&a, CURRENT)];
        inputs.approvals = merged.counted;
        inputs.ignored_approvals = merged.ignored;
        let built = build(inputs, None, &Limits::default(), now());
        let one = built
            .issues
            .iter()
            .find(|i| i.issue == a)
            .expect("issue 71");
        let state = one.approval.as_ref().unwrap();
        assert_eq!(state.version, ApprovalVersion::Current);
        assert_eq!(state.record.source, rubato_protocol::ApprovalSource::Rubato);
        assert_eq!(state.record.approval_id, Some(Uuid::from_u128(10)));
        assert_eq!(
            built
                .findings
                .iter()
                .filter(|f| matches!(f, Finding::CommentApprovalIgnored { .. }))
                .cloned()
                .collect::<Vec<_>>(),
            vec![Finding::CommentApprovalIgnored {
                issue: a.clone(),
                author: "m0x6A".into(),
                created_at: now() + chrono::Duration::seconds(30),
            }]
        );
        // No cutover: the later comment counts and decides.
        let merged = crate::approve::merge(vec![comment], vec![rubato], None);
        let mut inputs = empty();
        inputs.drafts = vec![draft_of(&a, CURRENT)];
        inputs.approvals = merged.counted;
        inputs.ignored_approvals = merged.ignored;
        let built = build(inputs, None, &Limits::default(), now());
        let one = built.issues.iter().find(|i| i.issue == a).unwrap();
        assert_eq!(
            one.approval.as_ref().unwrap().record.source,
            rubato_protocol::ApprovalSource::Comment
        );
        assert!(
            !built
                .findings
                .iter()
                .any(|f| matches!(f, Finding::CommentApprovalIgnored { .. }))
        );
    }

    /// Before handoff a Rubato approval is current while its plan sha is the newest revision's;
    /// a newer revision makes it stale, with the finding the gauge counts. Once the plan is on
    /// the branch it names, the plan as read decides.
    #[test]
    fn a_rubato_approval_follows_the_newest_revision_then_the_plan_on_the_branch() {
        let a = IssueRef::new("o/r", 71);
        let mut inputs = empty();
        inputs.drafts = vec![draft_of(&a, CURRENT)];
        inputs.approvals =
            crate::approve::merge(vec![], vec![rubato_approval(&a, 1, CURRENT, CURRENT)], None)
                .counted;
        assert_eq!(approval_of(&inputs, &a), (ApprovalVersion::Current, vec![]));
        inputs.approvals =
            crate::approve::merge(vec![], vec![rubato_approval(&a, 1, EARLIER, CURRENT)], None)
                .counted;
        assert_eq!(
            approval_of(&inputs, &a),
            (
                ApprovalVersion::Earlier,
                vec![Finding::ApprovalStale {
                    issue: a.clone(),
                    approved_sha: EARLIER.into(),
                    current_sha: CURRENT.into(),
                }]
            )
        );
        // Handed off: the plan on `issue/71-x` is read, and it hashes to the approved sha.
        let mut row = plan(&a, "companion-server", "rubato-companion");
        row.approval_sha = Some(EARLIER.into());
        inputs.plans = vec![row];
        assert_eq!(approval_of(&inputs, &a).0, ApprovalVersion::Current);
    }

    #[test]
    fn an_approval_without_a_sha_names_no_version() {
        let a = IssueRef::new("o/r", 71);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "companion-server", "rubato-companion")];
        inputs.approvals = vec![(approval(&a, 1, None, None), None)];
        assert_eq!(
            approval_of(&inputs, &a),
            (ApprovalVersion::NotNamed, vec![])
        );
    }

    #[test]
    fn the_newest_review_is_shown() {
        let a = IssueRef::new("o/r", 71);
        let mut inputs = empty();
        inputs.plans = vec![plan(&a, "companion-server", "rubato-companion")];
        let review = |id: u64, outcome: &str| ReviewRecord {
            issue: a.clone(),
            comment_id: id,
            author: "m0x6A".into(),
            track: "full".into(),
            outcome: outcome.into(),
            plan_sha: None,
            left_open: None,
            created_at: now() + chrono::Duration::seconds(id as i64),
            url: "u".into(),
        };
        inputs.reviews = vec![review(2, "passed after fixes"), review(1, "2 blockers")];
        let built = build(inputs.clone(), None, &Limits::default(), now());
        let one = built.issues.iter().find(|i| i.issue == a).unwrap();
        assert_eq!(one.review.as_ref().unwrap().outcome, "passed after fixes");
        assert!(one.approval.is_none());
    }
}

/// Findings about a repo two workspaces' maps name (ADR 0045), for the overview of `here`: the
/// maps give it different plan paths, or an id that both name is a part with a different repo in
/// each. `maps` is every workspace's map by name, in config order. Shown, never refused.
pub fn cross_workspace_findings(here: &str, maps: &[(&str, &SystemMap)]) -> Vec<Finding> {
    let Some((_, mine)) = maps.iter().find(|(n, _)| *n == here) else {
        return vec![];
    };
    let mut out = Vec::new();
    for repo in mine.repos_named() {
        let sharing: Vec<&(&str, &SystemMap)> =
            maps.iter().filter(|(_, m)| names_repo(m, repo)).collect();
        if sharing.len() < 2 {
            continue;
        }
        let path = |m: &rubato_protocol::SystemMap| m.plan_path_for(repo, 0, "x");
        if sharing.iter().any(|(_, m)| path(m) != path(mine)) {
            out.push(Finding::PlanPathConflict {
                repo: repo.to_string(),
                workspaces: sharing.iter().map(|(n, _)| (*n).to_owned()).collect(),
            });
        }
        for node in mine.nodes.iter().filter(|n| {
            n.repo
                .as_deref()
                .is_some_and(|r| r.eq_ignore_ascii_case(repo))
        }) {
            let differs: Vec<String> = sharing
                .iter()
                .filter(|(_, m)| {
                    m.node(&node.id).is_some_and(|o| {
                        !o.repo
                            .as_deref()
                            .unwrap_or("")
                            .eq_ignore_ascii_case(node.repo.as_deref().unwrap_or(""))
                    })
                })
                .map(|(n, _)| (*n).to_owned())
                .collect();
            if !differs.is_empty() {
                let mut workspaces = vec![here.to_string()];
                workspaces.extend(differs);
                out.push(Finding::IdCollision {
                    id: node.id.clone(),
                    workspaces,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod cross_workspace_tests {
    use super::*;

    fn borrowed(maps: &[(String, SystemMap)]) -> Vec<(&str, &SystemMap)> {
        maps.iter().map(|(n, m)| (n.as_str(), m)).collect()
    }
    use rubato_protocol::{MapNode, NodeKind, RepoRef, SystemMap};

    fn map(plan_path: Option<&str>, nodes: &[(&str, &str)]) -> SystemMap {
        SystemMap {
            version: 1,
            name: "m".into(),
            owner_rule: None,
            repos: vec![RepoRef {
                name: "acme/auth".into(),
                plan_path: plan_path.map(str::to_string),
            }],
            nodes: nodes
                .iter()
                .map(|(id, repo)| MapNode {
                    id: (*id).into(),
                    kind: NodeKind::Context,
                    repo: Some((*repo).into()),
                    path: None,
                    description: None,
                    service: None,
                })
                .collect(),
            seams: vec![],
            scenarios: vec![],
        }
    }

    #[test]
    fn a_shared_repo_with_two_plan_paths_or_an_id_meaning_two_parts_is_a_finding() {
        let owned = vec![
            ("admin".to_string(), map(None, &[("auth-api", "acme/auth")])),
            (
                "shop".to_string(),
                map(
                    Some("plans/{issue}.md"),
                    &[("auth-api", "acme/billing"), ("x", "acme/auth")],
                ),
            ),
        ];
        let maps = borrowed(&owned);
        let f = cross_workspace_findings("admin", &maps);
        assert!(
            f.contains(&Finding::PlanPathConflict {
                repo: "acme/auth".into(),
                workspaces: vec!["admin".into(), "shop".into()]
            }),
            "{f:?}"
        );
        assert!(
            f.contains(&Finding::IdCollision {
                id: "auth-api".into(),
                workspaces: vec!["admin".into(), "shop".into()]
            }),
            "{f:?}"
        );
        // The same plan path and the same part: nothing to say.
        let quiet = vec![
            ("admin".to_string(), map(None, &[("auth-api", "acme/auth")])),
            ("shop".to_string(), map(None, &[("auth-api", "acme/auth")])),
        ];
        assert!(cross_workspace_findings("admin", &borrowed(&quiet)).is_empty());
        // One workspace: never.
        assert!(cross_workspace_findings("admin", &maps[..1]).is_empty());
    }
}

#[cfg(test)]
mod closed_mismatch_tests {
    //! #135: a launch at an older map version is a finding only while its issue is open.
    use super::*;
    use rubato_protocol::Limits;

    #[test]
    fn a_closed_issue_raises_no_map_version_mismatch_and_an_open_or_unknown_one_does() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/design/rubato-map.yml"
        ))
        .unwrap();
        let map = LoadedMap {
            map: serde_yaml_ng::from_str(&text).unwrap(),
            version: "new".into(),
            loaded_at: Utc::now(),
            yaml: text,
        };
        let closed = IssueRef::new("m0x6A/rubato", 47);
        let open = IssueRef::new("m0x6A/rubato", 48);
        let unknown = IssueRef::new("m0x6A/rubato", 49);
        let mut inputs = tests::empty_inputs();
        for i in [&closed, &open, &unknown] {
            inputs.launched.insert(i.clone());
            inputs.launch_map_versions.push((i.clone(), "old".into()));
        }
        for (i, state) in [(&closed, "closed"), (&open, "open")] {
            inputs.meta.insert(
                i.clone(),
                IssueMeta {
                    title: None,
                    url: None,
                    github_state: Some(state.into()),
                },
            );
        }
        let built = build(inputs.clone(), Some(&map), &Limits::default(), Utc::now());
        let mut flagged: Vec<u64> = built
            .findings
            .iter()
            .filter_map(|f| match f {
                Finding::MapVersionMismatch { issue, .. } => Some(issue.number),
                _ => None,
            })
            .collect();
        flagged.sort();
        assert_eq!(
            flagged,
            [48, 49],
            "closed #47 is not flagged; open #48 and unread #49 are"
        );
    }
}
