//! Build the overview: progress per issue (derived, never stored), findings, unpinned sessions.
//! Pure over the rows the database holds, so it is testable without a server.

use crate::github_poll::IssueMeta;
use crate::kernel::state::LoadedMap;
use crate::plans::PlanRow;
use chrono::{DateTime, Utc};
use rubato_protocol::{
    CapKind, DecisionRecord, Finding, IssuePin, IssueRef, IssueSummary, Limits, MapRef, Plan,
    Progress, ProgressInputs, ProgressState, PullRequestState, SessionState, SessionStatus,
    derive_progress,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub struct OverviewInputs {
    pub pins: Vec<IssuePin>,
    pub sessions: Vec<SessionState>,
    pub plans: Vec<PlanRow>,
    pub prs: HashMap<IssueRef, PullRequestState>,
    pub decisions: Vec<DecisionRecord>,
    pub launched: HashSet<IssueRef>,
    /// The live cap hit per issue, by kind.
    pub cap_hits: HashMap<IssueRef, CapKind>,
    pub meta: HashMap<IssueRef, IssueMeta>,
    pub launch_map_versions: Vec<(IssueRef, String)>,
}

pub struct Built {
    pub issues: Vec<IssueSummary>,
    pub findings: Vec<Finding>,
    pub unpinned_sessions: Vec<SessionState>,
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
    Built {
        issues: out,
        findings,
        unpinned_sessions,
    }
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
            }),
            parse_error: None,
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
        }
    }

    fn empty() -> OverviewInputs {
        OverviewInputs {
            pins: vec![],
            sessions: vec![],
            plans: vec![],
            prs: HashMap::new(),
            decisions: vec![],
            launched: HashSet::new(),
            cap_hits: HashMap::new(),
            meta: HashMap::new(),
            launch_map_versions: vec![],
        }
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
}
