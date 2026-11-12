//! Build the overview: progress per issue (derived, never stored), findings, unpinned sessions.
//! Pure over the rows the database holds, so it is testable without a server.

use crate::github_poll::IssueMeta;
use crate::kernel::state::LoadedMap;
use crate::plans::{PlanRow, approval_names};
use chrono::{DateTime, Utc};
use rubato_protocol::QueuedIssue;
use rubato_protocol::layout::MapLayout;
use rubato_protocol::{
    ApprovalRecord, ApprovalState, ApprovalVersion, Backlog, BacklogRepo, CapKind, ConnectedRepo,
    DecisionRecord, Finding, IssuePin, IssueRef, IssueSummary, Limits, MapRef, OpenIssue, Plan,
    Progress, ProgressInputs, ProgressState, PullRequestState, RepoPulls, ReviewRecord,
    SessionState, SessionStatus, derive_progress, plan_fit,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub struct OverviewInputs {
    pub pins: Vec<IssuePin>,
    pub sessions: Vec<SessionState>,
    pub plans: Vec<PlanRow>,
    pub prs: HashMap<IssueRef, PullRequestState>,
    pub decisions: Vec<DecisionRecord>,
    /// `Review:` comments from members (ADR 0027).
    pub reviews: Vec<ReviewRecord>,
    /// `Approved:` comments from members, each with the plan hash of the branch it names when
    /// that is not the branch the plan is read from.
    pub approvals: Vec<(ApprovalRecord, Option<String>)>,
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
}

fn seams_touched(plan: &Plan) -> BTreeSet<&str> {
    plan.affected_parts
        .iter()
        .flat_map(|p| p.seams.iter().map(String::as_str))
        .chain(plan.o_rows.iter().map(|o| o.seam.as_str()))
        .collect()
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

/// The newest approval of an issue and whether it names the plan as it is now (ADR 0027). An
/// approval that names a branch other than the one the plan is read from is compared with the
/// plan on that branch (`target_sha`, read on the full lane); otherwise with the plan as read.
///
/// An approval of the current plan counts only while the plan fits the map (ADR 0029): with
/// `fits == Some(false)` its version is `DoesNotFit`.
pub fn approval_state(
    approvals: &[&(ApprovalRecord, Option<String>)],
    plan: Option<&PlanRow>,
    fits: Option<bool>,
) -> Option<ApprovalState> {
    let (record, target_sha) = approvals
        .iter()
        .max_by_key(|(a, _)| (a.created_at, a.comment_id))?;
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

pub fn build(
    inputs: &OverviewInputs,
    map: Option<&LoadedMap>,
    limits: &Limits,
    now: DateTime<Utc>,
) -> Built {
    // Issues of interest: pinned, launched, planned, or with a session.
    let mut issues: BTreeSet<IssueRef> = inputs.pins.iter().map(|p| p.issue.clone()).collect();
    issues.extend(inputs.launched.iter().cloned());
    issues.extend(inputs.plans.iter().map(|p| p.issue.clone()));
    issues.extend(inputs.sessions.iter().filter_map(|s| s.issue.clone()));

    let mut out = Vec::new();
    let mut findings = Vec::new();
    for issue in &issues {
        let pins: Vec<MapRef> = inputs
            .pins
            .iter()
            .filter(|p| &p.issue == issue)
            .map(|p| p.target.clone())
            .collect();
        let sessions: Vec<SessionState> = inputs
            .sessions
            .iter()
            .filter(|s| s.issue.as_ref() == Some(issue))
            .cloned()
            .collect();
        let plan_row = inputs.plans.iter().find(|p| &p.issue == issue);
        let plan = plan_row.and_then(|p| p.plan.clone());
        if let Some(row) = plan_row
            && let Some(err) = &row.parse_error
        {
            findings.push(Finding::PlanParseError {
                issue: issue.clone(),
                path: row.path.clone(),
                error: err.clone(),
            });
        }
        let pr = inputs.prs.get(issue);
        let (state, reason) = derive_progress(&ProgressInputs {
            pins: &pins,
            launched: inputs.launched.contains(issue),
            sessions: &sessions,
            has_plan: plan.is_some(),
            cap_hit: cap_hit_blocks(inputs.cap_hits.get(issue).copied(), &sessions),
            pr,
            now,
            session_stale_secs: limits.session_stale_secs,
        });
        let last_seen = sessions
            .iter()
            .map(|s| s.last_seen)
            .chain(pr.map(|p| p.updated_at))
            .chain(
                inputs
                    .pins
                    .iter()
                    .filter(|p| &p.issue == issue)
                    .map(|p| p.pinned_at),
            )
            .max()
            .unwrap_or(now);
        if let (Some(p), Some(m)) = (&plan, map) {
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
        if let Some(m) = map {
            for (i, v) in &inputs.launch_map_versions {
                if i == issue && *v != m.version && state != ProgressState::Done {
                    findings.push(Finding::MapVersionMismatch {
                        issue: issue.clone(),
                        launched_with: v.clone(),
                        current: m.version.clone(),
                    });
                    break;
                }
            }
        }
        let review = inputs
            .reviews
            .iter()
            .filter(|r| &r.issue == issue)
            .max_by_key(|r| (r.created_at, r.comment_id))
            .cloned();
        let approvals: Vec<_> = inputs
            .approvals
            .iter()
            .filter(|(a, _)| &a.issue == issue)
            .collect();
        let fit = match (plan_row, map) {
            (Some(row), Some(m)) => plan_fit(plan.as_ref(), row.parse_error.as_deref(), &m.map),
            _ => None,
        };
        let approval = approval_state(&approvals, plan_row, fit.as_ref().map(|f| f.fits));
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
        let meta = inputs.meta.get(issue);
        let spend: Option<f64> = {
            let v: Vec<f64> = sessions.iter().filter_map(|s| s.spend_usd).collect();
            if v.is_empty() {
                None
            } else {
                Some(v.iter().sum())
            }
        };
        out.push(IssueSummary {
            issue: issue.clone(),
            title: meta.and_then(|m| m.title.clone()),
            url: meta.and_then(|m| m.url.clone()),
            pins,
            progress: Progress {
                issue: issue.clone(),
                state,
                last_seen,
                pinned_to: vec![],
                session_ids: sessions
                    .iter()
                    .filter_map(|s| s.session_id.clone())
                    .collect(),
                reason,
            },
            sessions,
            plan,
            pull_request: pr.cloned(),
            decisions: inputs
                .decisions
                .iter()
                .filter(|d| &d.issue == issue)
                .cloned()
                .collect(),
            review,
            approval,
            fit,
            plan_sha: plan_row.and_then(|p| p.approval_sha.clone()),
            spend_usd: spend,
        });
    }
    for s in &mut out {
        s.progress.pinned_to = s.pins.clone();
    }

    // Two or more open plans on one seam.
    let mut by_seam: BTreeMap<&str, Vec<IssueRef>> = BTreeMap::new();
    for s in &out {
        if s.progress.state == ProgressState::Done {
            continue;
        }
        if let Some(p) = &s.plan {
            for seam in seams_touched(p) {
                by_seam.entry(seam).or_default().push(s.issue.clone());
            }
        }
    }
    for (seam, issues) in by_seam {
        if issues.len() > 1 {
            findings.push(Finding::SeamCollision {
                seam: seam.to_owned(),
                issues,
            });
        }
    }

    let unpinned_sessions = inputs
        .sessions
        .iter()
        .filter(|s| s.issue.is_none())
        .cloned()
        .collect();
    findings.extend(
        inputs
            .sessions
            .iter()
            .filter(|s| s.issue.is_none() && s.status != SessionStatus::Ended)
            .map(|s| Finding::UnattributedSession {
                companion: s.companion_id,
                branch: s.branch.clone(),
                repo: s.repo.clone(),
            }),
    );
    findings.extend(
        inputs
            .uncovered_repos
            .iter()
            .map(|repo| Finding::RepoNotCovered { repo: repo.clone() }),
    );
    let pulls = pulls_with_checks(inputs);
    let repos = connected_repos(inputs, map, &out, &pulls);

    // The backlog: listed open issues that are not tracked above, and one entry per repo the
    // map names, listed or not, so "not listed yet" is said and not shown as empty.
    // An issue tracked only because its branch carries a plan is still not pinned: it stays
    // here until a person or a `Map:` line places it (ADR 0032).
    let placed_or_worked: BTreeSet<&IssueRef> = inputs
        .pins
        .iter()
        .map(|p| &p.issue)
        .chain(inputs.launched.iter())
        .chain(inputs.sessions.iter().filter_map(|s| s.issue.as_ref()))
        .collect();
    let (listed, listings) = &inputs.backlog;
    let backlog = Backlog {
        issues: listed
            .iter()
            .filter(|i| !placed_or_worked.contains(&i.issue))
            .cloned()
            .collect(),
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
            None => listings.clone(),
        },
    };
    // Positions only for parts the map has now: a row for a removed part is not served (and
    // is deleted by the next layout write). With no map loaded there is nothing to place.
    let layout = MapLayout {
        positions: match map {
            Some(m) => {
                let parts = m.map.node_ids();
                inputs
                    .layout
                    .positions
                    .iter()
                    .filter(|(id, _)| parts.contains(id.as_str()))
                    .map(|(id, p)| (id.clone(), *p))
                    .collect()
            }
            None => Default::default(),
        },
    };
    Built {
        issues: out,
        findings,
        unpinned_sessions,
        backlog,
        layout,
        repos,
        pulls,
    }
}

/// The stored listings, with each tracked pull request's checks taken from `pull_requests`
/// (ADR 0023 reads them every tick), so the two never disagree and nothing is copied.
fn pulls_with_checks(inputs: &OverviewInputs) -> Vec<RepoPulls> {
    let tracked: HashMap<(&str, u64), &PullRequestState> = inputs
        .prs
        .values()
        .map(|p| ((p.repo.as_str(), p.number), p))
        .collect();
    inputs
        .repo_pulls
        .iter()
        .cloned()
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
    inputs: &OverviewInputs,
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
    let live = |s: &&SessionState| s.status != SessionStatus::Ended;
    names
        .into_iter()
        .map(|(repo, in_map)| ConnectedRepo {
            open_pulls: pulls
                .iter()
                .filter(|p| same(&p.repo, &repo))
                .map(|p| p.pulls.len() as u32)
                .sum(),
            covered: !inputs.uncovered_repos.iter().any(|u| same(u, &repo)),
            open_issues: inputs
                .backlog
                .0
                .iter()
                .filter(|i| same(&i.issue.repo, &repo))
                .count() as u32,
            plans: issues
                .iter()
                .filter(|i| same(&i.issue.repo, &repo) && i.plan.is_some())
                .count() as u32,
            sessions: inputs
                .sessions
                .iter()
                .filter(live)
                .filter(|s| s.issue.as_ref().is_some_and(|i| same(&i.repo, &repo)))
                .count() as u32,
            unattributed: inputs
                .sessions
                .iter()
                .filter(live)
                .filter(|s| s.issue.is_none() && s.repo.as_deref().is_some_and(|r| same(r, &repo)))
                .count() as u32,
            in_map,
            repo,
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
            for s in seams_touched(p) {
                *by_seam.entry(s.to_owned()).or_insert(0) += 1;
            }
        }
    }
    (by_state, by_seam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rubato_protocol::{AffectedPart, ORow, PartStatus, StateSource, SystemMap};
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
            }),
            parse_error: None,
            approval_sha: Some("f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9".into()),
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
        }
    }

    fn empty() -> OverviewInputs {
        OverviewInputs {
            pins: vec![],
            sessions: vec![],
            plans: vec![],
            prs: HashMap::new(),
            decisions: vec![],
            reviews: vec![],
            approvals: vec![],
            launched: HashSet::new(),
            cap_hits: HashMap::new(),
            meta: HashMap::new(),
            launch_map_versions: vec![],
            backlog: (vec![], vec![]),
            uncovered_repos: vec![],
            layout: MapLayout::default(),
            queue: vec![],
            repo_pulls: vec![],
        }
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
        assert!(built.findings.iter().any(|f| matches!(f, Finding::SeamCollision { seam, issues } if seam == "companion-server" && issues.len() == 2)));
        assert!(built.findings.iter().any(|f| matches!(f, Finding::UnknownIds { issue, ids } if *issue == b && ids == &vec!["rubato-orchestrator".to_string()])));
        assert!(
            built
                .issues
                .iter()
                .all(|i| i.progress.state == ProgressState::Designed)
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
            let built = build(&inputs, None, &Limits::default(), now());
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
        let before = build(&inputs, Some(&map()), &Limits::default(), now());
        assert!(before.layout.positions.is_empty());

        let at = |x: f64| Position { x, y: 1.0 };
        inputs.layout.positions = [
            ("rubato-server".to_owned(), at(5.0)),
            ("gone".to_owned(), at(9.0)),
        ]
        .into();
        let after = build(&inputs, Some(&map()), &Limits::default(), now());
        let placed: Vec<&str> = after.layout.positions.keys().map(String::as_str).collect();
        assert_eq!(
            placed,
            ["rubato-server"],
            "a part the map lacks is left out"
        );
        assert_eq!(
            after.issues, before.issues,
            "a position is not a progress signal"
        );
        assert_eq!(after.findings, before.findings);
        // With no map there is nothing to place.
        let none = build(&inputs, None, &Limits::default(), now());
        assert!(none.layout.positions.is_empty());
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
        let before = build(&inputs, Some(&map()), &Limits::default(), now());

        // The pinned issue and one nobody tracks are queued, and a part is moved.
        let queued = |issue: IssueRef, position: u32| QueuedIssue {
            issue,
            companion_id: Uuid::nil(),
            queued_by: "m0x6A".into(),
            queued_at: now(),
            position,
            last_start_error: Some("missing_dependency: tmux-ide".into()),
        };
        inputs.queue = vec![queued(pinned.clone(), 0), queued(IssueRef::new(repo, 2), 1)];
        inputs.layout.positions =
            [("rubato-server".to_owned(), Position { x: 1.0, y: 2.0 })].into();
        let after = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, Some(&map()), &Limits::default(), now());
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
        let built = build(&inputs, None, &Limits::default(), now());
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
            comment_id: id,
            author: "m0x6A".into(),
            sha: sha.map(str::to_owned),
            branch: branch.map(str::to_owned),
            note: None,
            created_at: now() + chrono::Duration::seconds(id as i64),
            url: "u".into(),
        }
    }

    fn approval_of(inputs: &OverviewInputs, issue: &IssueRef) -> (ApprovalVersion, Vec<Finding>) {
        let built = build(inputs, None, &Limits::default(), now());
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
        let built = build(&inputs, None, &Limits::default(), now());
        let one = built.issues.iter().find(|i| i.issue == a).unwrap();
        assert_eq!(one.review.as_ref().unwrap().outcome, "passed after fixes");
        assert!(one.approval.is_none());
    }
}
