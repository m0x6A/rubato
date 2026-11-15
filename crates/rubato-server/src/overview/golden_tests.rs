//! Plan #171: the overview a rich fixture builds, byte for byte, against a golden recorded before
//! the build was changed to take owned, grouped inputs. Run with `RUBATO_BLESS=1` to record it
//! again, only when an output change is intended.

use super::build::{
    OtherWorkspace, OverviewInputs, WorkspaceInputs, build, cross_workspace_findings,
};
use crate::github_poll::IssueMeta;
use crate::kernel::state::{LoadedMap, TrafficLatest};
use crate::plans::PlanRow;
use chrono::{DateTime, Duration, TimeZone, Utc};
use rubato_protocol::layout::{MapLayout, Position};
use rubato_protocol::{
    CapKind, Finding, IssueRef, Limits, ObservedTraffic, SessionState, SpendPoint, SystemMap,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/golden/overview_build.json"
);

const HERE_MAP: &str = r#"
version: 1
name: app
repos:
  - name: acme/app
    plan_path: docs/plans/{issue}.md
nodes:
  - { id: app-web, kind: context, repo: acme/app, service: { namespace: acme, name: app-web } }
  - { id: app-api, kind: context, repo: acme/app, service: { namespace: acme, name: app-api } }
  - { id: auth-svc, kind: context, repo: acme/auth }
  - { id: github, kind: external }
seams:
  - id: web-api
    from: app-web
    to: app-api
    owner: app-api
    flows:
      - { kind: data, from: app-api, to: app-web, carries: "issues" }
  - id: api-auth
    from: app-api
    to: auth-svc
    owner: app-api
    flows:
      - { kind: control, from: app-api, to: auth-svc, carries: "token check" }
"#;

const AUTH_MAP: &str = r#"
version: 1
name: auth
repos:
  - name: acme/auth
    plan_path: docs/design/{issue}.md
nodes:
  - { id: auth-svc, kind: context, repo: acme/other }
  - { id: token, kind: context, repo: acme/auth }
seams: []
"#;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
}

fn at(mins: i64) -> String {
    (now() - Duration::minutes(mins)).to_rfc3339()
}

fn de<T: DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn i(repo: &str, n: u64) -> IssueRef {
    IssueRef::new(repo, n)
}

fn system(yaml: &str) -> SystemMap {
    serde_yaml_ng::from_str(yaml).unwrap()
}

fn loaded() -> LoadedMap {
    LoadedMap {
        map: system(HERE_MAP),
        version: "v2".into(),
        loaded_at: now(),
        yaml: HERE_MAP.into(),
    }
}

fn session(
    issue: Option<IssueRef>,
    id: &str,
    status: &str,
    mins: i64,
    spend: Option<f64>,
) -> SessionState {
    de(json!({
        "session_id": id,
        "companion_id": "00000000-0000-0000-0000-000000000001",
        "user": "m0x6A",
        "cwd": format!("/w/{id}"),
        "branch": issue.as_ref().map(|i| format!("issue/{}-x", i.number)),
        "repo": issue.as_ref().map(|i| i.repo.clone()).unwrap_or_else(|| "acme/app".into()),
        "issue": issue,
        "status": status,
        "source": "hook",
        "last_seen": at(mins),
        "spend_usd": spend,
    }))
}

/// A plan affecting `nodes`, touching `seams` with an O-row each, and `flows` as (from, to, seam).
fn plan(issue: &IssueRef, nodes: &[&str], seams: &[&str], flows: &[(&str, &str, &str)]) -> PlanRow {
    let plan = de(json!({
        "issue": issue,
        "title": format!("plan {}", issue.number),
        "path": format!("docs/plans/{}.md", issue.number),
        "affected_parts": nodes.iter().map(|n| json!({
            "node": n, "status": "affected", "change": "x", "seams": seams,
        })).collect::<Vec<_>>(),
        "o_rows": seams.iter().map(|s| json!({
            "seam": s, "signal": "s", "panel": "p", "owner": "app-api",
        })).collect::<Vec<_>>(),
        "flows": flows.iter().map(|(f, t, s)| json!({
            "from": f, "to": t, "seam": s, "kind": "data", "carries": "c",
        })).collect::<Vec<_>>(),
    }));
    PlanRow {
        issue: issue.clone(),
        branch: format!("issue/{}-x", issue.number),
        path: format!("docs/plans/{}.md", issue.number),
        plan: Some(plan),
        parse_error: None,
        approval_sha: Some(format!("sha{}", issue.number)),
        head_sha: None,
    }
}

/// The newest settled revision of a draft (ADR 0053) on `issue/<n>-x`, owned by m0x6A.
fn draft(
    issue: &IssueRef,
    plan: Option<rubato_protocol::Plan>,
    conflict: Option<&str>,
) -> crate::plan_drafts::LatestDraft {
    crate::plan_drafts::LatestDraft {
        issue: issue.clone(),
        summary: de(json!({
            "draft_id": format!("00000000-0000-0000-0000-0000000000{:02}", issue.number),
            "branch": format!("issue/{}-x", issue.number),
            "owner": "m0x6A",
            "rev": 2,
            "at": at(issue.number as i64),
            "plan_sha": format!("sha{}", issue.number),
            "bundle_hash": "h",
            "missing": [],
            "live": false,
        })),
        plan,
        conflict: conflict.map(|o| (o.to_owned(), now() - Duration::minutes(5))),
    }
}

/// An approval given in Rubato (ADR 0054) of the draft on `issue/<n>-x`.
fn rubato_approval(issue: &IssueRef, mins: i64, sha: &str) -> rubato_protocol::ApprovalRecord {
    de(json!({
        "issue": issue, "source": "rubato",
        "approval_id": format!("00000000-0000-0000-0000-0000000001{:02}", issue.number),
        "bundle_hash": "h", "author": "teammate", "sha": sha,
        "branch": format!("issue/{}-x", issue.number),
        "created_at": at(mins), "url": "u/rubato",
    }))
}

fn pin(issue: &IssueRef, kind: &str, id: &str, mins: i64) -> rubato_protocol::IssuePin {
    de(json!({
        "issue": issue,
        "target": { "kind": kind, "id": id },
        "pinned_by": "m0x6A",
        "pinned_at": at(mins),
    }))
}

fn decision(issue: &IssueRef, id: u64) -> rubato_protocol::DecisionRecord {
    de(json!({
        "issue": issue, "comment_id": id, "author": "m0x6A", "title": format!("d{id}"),
        "chosen": "a", "created_at": at(100 - id as i64), "url": format!("u/d{id}"),
    }))
}

fn review(issue: &IssueRef, id: u64, outcome: &str) -> rubato_protocol::ReviewRecord {
    de(json!({
        "issue": issue, "comment_id": id, "author": "m0x6A", "track": "small",
        "outcome": outcome, "plan_sha": format!("sha{}", issue.number),
        "created_at": at(100 - id as i64), "url": format!("u/r{id}"),
    }))
}

fn approval(
    issue: &IssueRef,
    id: u64,
    sha: Option<&str>,
    branch: Option<&str>,
) -> rubato_protocol::ApprovalRecord {
    de(json!({
        "issue": issue, "comment_id": id, "author": "m0x6A", "sha": sha, "branch": branch,
        "created_at": at(100 - id as i64), "url": format!("u/a{id}"),
    }))
}

fn pr(
    issue: &IssueRef,
    number: u64,
    open: bool,
    merged: bool,
    checks: &str,
) -> rubato_protocol::PullRequestState {
    de(json!({
        "repo": issue.repo, "number": number, "branch": format!("issue/{}-x", issue.number),
        "open": open, "draft": false, "checks": checks,
        "merged_at": if merged { Some(at(5)) } else { None },
        "updated_at": at(3), "url": format!("u/p{number}"),
    }))
}

fn meta(title: &str, state: &str) -> IssueMeta {
    IssueMeta {
        title: Some(title.into()),
        url: Some(format!("u/{title}")),
        github_state: Some(state.into()),
    }
}

/// Three workspaces: this one (`default`), `auth` sharing repo `acme/auth` with a different plan
/// path and a colliding id, and `billing` whose map has never loaded. Every table has rows that
/// match several issues, rows that match none, and the shapes each finding comes from.
fn rich() -> OverviewInputs {
    let (a1, a2, a3, a4) = (
        i("acme/app", 1),
        i("acme/app", 2),
        i("acme/app", 3),
        i("acme/app", 4),
    );
    let (a7, a8) = (i("acme/app", 7), i("acme/app", 8));
    let (auth5, bill6, loose9) = (i("acme/auth", 5), i("acme/billing", 6), i("acme/zzz", 9));
    // Main's drafts (ADR 0053): #10 has a draft and no plan on a branch, #11 a draft that does
    // not parse and a bundle refused from another person.
    let (a10, a11) = (i("acme/app", 10), i("acme/app", 11));
    let mut p4 = plan(&a4, &["app-api"], &[], &[]);
    p4.plan = None;
    p4.parse_error = Some("no Affected parts table".into());
    let mut p7 = plan(&a7, &["app-web"], &["web-api"], &[]);
    p7.approval_sha = Some("sha7-here".into());
    OverviewInputs {
        pins: vec![
            pin(&a1, "node", "app-api", 60),
            pin(&a1, "seam", "web-api", 50),
            pin(&a2, "seam", "web-api", 40),
            pin(&a7, "node", "app-web", 30),
            pin(&a8, "node", "app-api", 20),
        ],
        sessions: vec![
            session(Some(a1.clone()), "s1", "working", 1, Some(2.0)),
            session(Some(a1.clone()), "s1b", "ended", 90, Some(0.5)),
            session(Some(a2.clone()), "s2", "blocked", 2, Some(4.25)),
            session(Some(a3.clone()), "s3", "ended", 30, None),
            session(Some(a7.clone()), "s7", "idle", 1, Some(1.0)),
            session(Some(a7.clone()), "s7b", "working", 45, None),
            session(Some(loose9.clone()), "s9", "working", 1, None),
            session(None, "u1", "working", 1, None),
            session(None, "u2", "ended", 10, Some(0.1)),
        ],
        plans: vec![
            plan(
                &a1,
                &["app-api"],
                &["web-api"],
                &[("app-api", "app-web", "web-api")],
            ),
            plan(
                &a2,
                &["app-api", "ghost-node"],
                &["web-api", "api-auth"],
                &[("app-web", "auth-svc", "web-api")],
            ),
            p4,
            plan(&auth5, &["token"], &[], &[]),
            p7,
            plan(&a8, &["app-api"], &["web-api"], &[]),
        ],
        drafts: vec![
            // On the branch the plan is read from: the plan on the branch is the plan.
            draft(&a1, plan(&a1, &["app-api"], &[], &[]).plan, None),
            draft(&a10, plan(&a10, &["app-api"], &["web-api"], &[]).plan, None),
            draft(&a11, None, Some("alice")),
        ],
        prs: HashMap::from([
            (a1.clone(), pr(&a1, 11, true, false, "green")),
            (a7.clone(), pr(&a7, 17, true, false, "red")),
            (a8.clone(), pr(&a8, 18, false, true, "green")),
        ]),
        decisions: vec![
            decision(&a1, 1),
            decision(&a2, 2),
            decision(&a1, 3),
            decision(&loose9, 4),
        ],
        reviews: vec![
            review(&a1, 10, "passed"),
            review(&a1, 12, "passed after fixes"),
            review(&a2, 11, "blocked"),
        ],
        approvals: vec![
            (approval(&a1, 20, Some("sha1"), None), None),
            (approval(&a2, 21, Some("sha2-old"), None), None),
            (approval(&a2, 19, Some("sha2"), None), None),
            (
                approval(&a7, 22, Some("sha7-there"), Some("issue/7-other")),
                Some("sha7-there".into()),
            ),
            (approval(&auth5, 23, Some("sha5"), None), None),
            (approval(&a3, 24, None, None), None),
            (rubato_approval(&a10, 6, "sha10"), Some("sha10".into())),
        ],
        ignored_approvals: vec![approval(&a2, 30, Some("sha2"), None)],
        approvals_in_rubato: true,
        approval_findings: vec![
            Finding::LinkCommentWaiting {
                issue: a1.clone(),
                approval_id: uuid::Uuid::from_u128(7),
            },
            // About an issue this overview does not hold: left out.
            Finding::AdrsNotAccepted {
                issue: i("acme/elsewhere", 99),
                adrs: vec!["0001".into()],
            },
        ],
        launched: HashSet::from([a1.clone(), a3.clone(), a4.clone(), loose9.clone()]),
        cap_hits: HashMap::from([
            (a2.clone(), CapKind::Spend),
            (a3.clone(), CapKind::Attempts),
        ]),
        meta: HashMap::from([
            (a1.clone(), meta("one", "open")),
            (a2.clone(), meta("two", "open")),
            (a4.clone(), meta("four", "closed")),
            (auth5.clone(), meta("five", "open")),
        ]),
        launch_map_versions: vec![
            (a1.clone(), "v2".into()),
            (a3.clone(), "v1".into()),
            (a3.clone(), "v0".into()),
            (a4.clone(), "v1".into()),
        ],
        backlog: (
            de(json!([
                { "issue": a1, "title": "one", "url": "u/1", "updated_at": at(9) },
                { "issue": i("acme/app", 20), "title": "twenty", "url": "u/20", "updated_at": at(8) },
                { "issue": i("acme/auth", 21), "title": "auth", "url": "u/21", "labels": ["bug"], "updated_at": at(7) },
            ])),
            de(json!([
                { "repo": "acme/app", "listed_at": at(1), "truncated": false },
                { "repo": "acme/gone", "listed_at": at(1), "truncated": true },
            ])),
        ),
        uncovered_repos: vec!["acme/auth".into()],
        layout: MapLayout {
            positions: [
                ("app-api".to_string(), Position { x: 1.0, y: 2.0 }),
                ("removed".to_string(), Position { x: 3.0, y: 4.0 }),
                ("app-web".to_string(), Position { x: -5.5, y: 0.0 }),
            ]
            .into(),
        },
        repo_pulls: de(json!([
            { "repo": "acme/app", "listed_at": at(2), "truncated": false, "pulls": [
                { "repo": "acme/app", "number": 11, "title": "t11", "author": "a", "branch": "issue/1-x",
                  "base": "main", "draft": false, "head_sha": "h11", "updated_at": at(3), "url": "u/11", "issue": a1 },
                { "repo": "acme/app", "number": 30, "title": "t30", "author": "b", "branch": "feature",
                  "base": "main", "draft": true, "head_sha": "h30", "updated_at": at(4), "url": "u/30" },
            ]},
            { "repo": "acme/auth", "listed_at": null, "truncated": false, "pulls": [], "error": "403" },
        ])),
        queue: vec![],
        traffic: TrafficLatest {
            report: Some(de(json!({
                "observed_at": at(1),
                "window_secs": 3600,
                "edges": [
                    { "client": "app-web", "client_namespace": "acme", "server": "app-api", "server_namespace": "acme", "calls": 100, "errors": 2 },
                    { "client": "app-api", "client_namespace": "acme", "server": "ledger", "server_namespace": "acme", "calls": 7 },
                    { "client": "cron", "client_namespace": "ops", "server": "app-web", "server_namespace": "acme", "calls": 1 },
                ],
            }))),
            source: rubato_protocol::SourceStatus::Configured {
                fetch_secs: 60,
                error: None,
            },
        },
        dismissals: de(json!([
            { "id": 1, "target": { "kind": "ghost", "service": { "namespace": "ops", "name": "cron" } },
              "author": "m0x6A", "at": at(30) },
        ])),
        spend: HashMap::from([
            (
                a1.clone(),
                vec![
                    SpendPoint {
                        at: now() - Duration::minutes(30),
                        usd: 0.5,
                    },
                    SpendPoint {
                        at: now() - Duration::minutes(10),
                        usd: 2.0,
                    },
                ],
            ),
            (
                a2.clone(),
                vec![SpendPoint {
                    at: now(),
                    usd: 4.25,
                }],
            ),
            (
                loose9.clone(),
                vec![SpendPoint {
                    at: now(),
                    usd: 9.0,
                }],
            ),
        ]),
        workspaces: WorkspaceInputs {
            here: Some(("default".into(), "app".into())),
            launched_here: HashSet::from([a1.clone(), a3.clone()]),
            others: vec![
                OtherWorkspace {
                    id: "auth".into(),
                    name: "auth".into(),
                    map: Some(std::sync::Arc::new(LoadedMap {
                        map: system(AUTH_MAP),
                        version: "a1".into(),
                        loaded_at: now(),
                        yaml: AUTH_MAP.into(),
                    })),
                    pinned: HashSet::from([auth5.clone(), i("acme/auth", 22)]),
                    launched: HashSet::new(),
                },
                OtherWorkspace {
                    id: "billing".into(),
                    name: "billing".into(),
                    map: None,
                    pinned: HashSet::from([bill6.clone()]),
                    launched: HashSet::from([a4.clone()]),
                },
            ],
        },
    }
}

/// Everything the overview serves from `build`, in the order the `Overview` holds it, and the
/// cross-workspace findings the handler adds.
#[derive(Serialize)]
struct Served {
    issues: Vec<rubato_protocol::IssueSummary>,
    findings: Vec<Finding>,
    cross: Vec<Finding>,
    unpinned_sessions: Vec<SessionState>,
    backlog: rubato_protocol::Backlog,
    layout: MapLayout,
    repos: Vec<rubato_protocol::ConnectedRepo>,
    pulls: Vec<rubato_protocol::RepoPulls>,
    traffic: ObservedTraffic,
}

fn serve(inputs: OverviewInputs) -> String {
    let map = loaded();
    let built = build(inputs.clone(), Some(&map), &Limits::default(), now());
    let auth = system(AUTH_MAP);
    let maps = vec![("app", &map.map), ("auth", &auth)];
    let served = Served {
        issues: built.issues,
        findings: built.findings,
        cross: cross_workspace_findings("app", &maps),
        unpinned_sessions: built.unpinned_sessions,
        backlog: built.backlog,
        layout: built.layout,
        repos: built.repos,
        pulls: built.pulls,
        traffic: built.traffic,
    };
    serde_json::to_string_pretty(&served).unwrap() + "\n"
}

/// The first JSON path where two documents differ, for a readable failure.
fn first_difference(a: &Value, b: &Value, path: String) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()) {
                if let Some(p) = first_difference(
                    x.get(k).unwrap_or(&Value::Null),
                    y.get(k).unwrap_or(&Value::Null),
                    format!("{path}.{k}"),
                ) {
                    return Some(p);
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            for n in 0..x.len().max(y.len()) {
                if let Some(p) = first_difference(
                    x.get(n).unwrap_or(&Value::Null),
                    y.get(n).unwrap_or(&Value::Null),
                    format!("{path}[{n}]"),
                ) {
                    return Some(p);
                }
            }
            None
        }
        _ => (a != b).then_some(path),
    }
}

fn compare(got: &str, golden: &str) -> Result<(), String> {
    if got == golden {
        return Ok(());
    }
    let (g, w): (Value, Value) = (
        serde_json::from_str(got).unwrap(),
        serde_json::from_str(golden).unwrap(),
    );
    Err(first_difference(&g, &w, "$".into()).unwrap_or_else(|| "$ (formatting only)".into()))
}

#[test]
fn build_matches_the_golden() {
    let got = serve(rich());
    if std::env::var_os("RUBATO_BLESS").is_some() {
        std::fs::write(GOLDEN, &got).unwrap();
    }
    let golden = std::fs::read_to_string(GOLDEN).unwrap();
    let v: Value = serde_json::from_str(&got).unwrap();
    let count = |k: &str| v[k].as_array().map_or(0, Vec::len);
    println!(
        "overview golden: {} issues, {} findings, {} cross-workspace findings, {} repos, {} bytes compared",
        count("issues"),
        count("findings"),
        count("cross"),
        count("repos"),
        got.len()
    );
    // The fixture must exercise what it claims to, or a golden of nothing would pass.
    assert!(count("issues") >= 7, "the fixture lost issues");
    for kind in [
        "plan_parse_error",
        "unknown_ids",
        "flow_off_seam",
        "map_version_mismatch",
        "approval_stale",
        "seam_collision",
        "unattributed_session",
        "repo_not_covered",
        "draft_owner_conflict",
        "comment_approval_ignored",
        "link_comment_waiting",
    ] {
        assert!(
            got.contains(&format!("\"{kind}\"")),
            "the fixture makes no {kind} finding"
        );
    }
    assert!(
        got.contains("\"outside_rubato\": true"),
        "no plan outside Rubato"
    );
    assert!(
        got.contains("\"source\": \"rubato\""),
        "no approval given in Rubato"
    );
    assert!(
        !got.contains("acme/elsewhere"),
        "a handoff finding of an issue not held leaked"
    );
    if let Err(at) = compare(&got, &golden) {
        panic!("the overview differs from the golden at {at}; RUBATO_BLESS=1 records it again");
    }
}

/// The check fails on purpose: one session's status changed is a different overview.
#[test]
fn a_changed_input_fails_the_golden() {
    let mut inputs = rich();
    inputs.sessions[0].status = rubato_protocol::SessionStatus::Idle;
    let golden = std::fs::read_to_string(GOLDEN).unwrap();
    let at = compare(&serve(inputs), &golden).expect_err("a changed input still matched");
    assert!(at.starts_with("$.issues[0]"), "{at}");
}

/// Not a benchmark: a timing print over a fixture scaled to many issues, quoted in the PR.
#[test]
fn build_time_over_a_scaled_fixture() {
    let base = rich();
    let mut inputs = rich();
    for k in 1..=60u64 {
        let shift = |r: &IssueRef| i(&r.repo, r.number + 100 * k);
        inputs.pins.extend(base.pins.iter().map(|p| {
            let mut p = p.clone();
            p.issue = shift(&p.issue);
            p
        }));
        inputs.sessions.extend(base.sessions.iter().map(|s| {
            let mut s = s.clone();
            s.issue = s.issue.as_ref().map(shift);
            s
        }));
        inputs.plans.extend(base.plans.iter().map(|p| {
            let mut p = p.clone();
            p.issue = shift(&p.issue);
            p
        }));
        inputs.decisions.extend(base.decisions.iter().map(|d| {
            let mut d = d.clone();
            d.issue = shift(&d.issue);
            d
        }));
        inputs.reviews.extend(base.reviews.iter().map(|r| {
            let mut r = r.clone();
            r.issue = shift(&r.issue);
            r
        }));
        inputs.approvals.extend(base.approvals.iter().map(|(a, t)| {
            let mut a = a.clone();
            a.issue = shift(&a.issue);
            (a, t.clone())
        }));
        inputs.drafts.extend(base.drafts.iter().map(|d| {
            let mut d = d.clone();
            d.issue = shift(&d.issue);
            d
        }));
        inputs.launched.extend(base.launched.iter().map(shift));
        inputs.launch_map_versions.extend(
            base.launch_map_versions
                .iter()
                .map(|(i, v)| (shift(i), v.clone())),
        );
        inputs
            .spend
            .extend(base.spend.iter().map(|(i, v)| (shift(i), v.clone())));
    }
    let map = loaded();
    let rounds = 20;
    // The copies are made before the clock starts: only the build is timed.
    let copies: Vec<OverviewInputs> = (0..rounds).map(|_| inputs.clone()).collect();
    let started = std::time::Instant::now();
    let mut issues = 0;
    for copy in copies {
        issues = build(copy, Some(&map), &Limits::default(), now())
            .issues
            .len();
    }
    println!(
        "overview build: {issues} issues, {:?} per build over {rounds} rounds",
        started.elapsed() / rounds
    );
    assert!(issues > 300);
}
