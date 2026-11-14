//! Approval in Rubato (ADR 0054, plan 122 §D6): one test per refusal, each with the case that
//! passes beside it so the check cannot pass by accident; self-approval; the second press; the
//! bundle hash stored; *Accept plan* after the cutover; comments after the cutover; the
//! branch the poll prefers.

use super::draft::owner_mid_turn;
use super::{approvals, approve, approve_draft, merge, records};
use crate::auth::CurrentUser;
use crate::kernel::config::Config;
use crate::kernel::db::connect;
use crate::kernel::github::{Comment, FakeGitHub};
use crate::kernel::http::Ws;
use crate::kernel::state::{App, AppState, LoadedMap};
use crate::kernel::telemetry::metric_reader;
use crate::{github_poll, plan_drafts};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, TimeZone, Utc};
use rubato_protocol::approve::{reason, routes};
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::{
    ApprovalSource, ApproveRequest, BlockCause, DraftApproveRequest, DraftApproveResponse,
    DraftBundle, DraftFile, IssueRef, SessionState, SessionStatus, StateSource, SystemMap,
    approval_hash, bundle_hash, git_blob_sha,
};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

const PLAN: &str = "docs/plans/122.md";
const DESIGN: &str = "docs/plans/122-x/design.md";
const ADR: &str = "docs/decisions/0099-x.md";
const ADR_TEXT: &str = "# 0099. X\n\n- **Status:** proposed\n";
const BRANCH: &str = "issue/122-x";
const OWNER: &str = "m0x6A";

fn at(h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, h, 0, 0).unwrap()
}

fn issue() -> IssueRef {
    IssueRef::new("m0x6A/rubato", 122)
}

/// A plan that fits the map, proposing ADR 0099 pinned at `blob` (empty: not pinned).
fn plan_text(change: &str, blob: &str) -> String {
    format!(
        "---\nissue: m0x6A/rubato#122\n---\n# Plan\n\n## Affected parts\n\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | {change} | web-server |\n\n## O-rows\n\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| web-server | x | y | z | rubato-server |\n\n## ADRs proposed\n\n| ADR | File | Blob |\n| --- | --- | --- |\n| 0099 | {ADR} | {blob} |\n"
    )
}

fn pinned(change: &str) -> String {
    plan_text(change, &git_blob_sha(ADR_TEXT.as_bytes()))
}

fn bundle(text: &str, missing: &[&str]) -> DraftBundle {
    let mut files = vec![
        DraftFile {
            path: PLAN.into(),
            text: text.into(),
        },
        DraftFile {
            path: DESIGN.into(),
            text: "design\n".into(),
        },
        DraftFile {
            path: ADR.into(),
            text: ADR_TEXT.into(),
        },
    ];
    files.retain(|f| !missing.contains(&f.path.as_str()));
    let pairs: Vec<(String, String)> = files
        .iter()
        .map(|f| (f.path.clone(), f.text.clone()))
        .collect();
    DraftBundle {
        launch_id: Uuid::from_u128(1),
        session_id: Some("s1".into()),
        issue: issue(),
        branch: BRANCH.into(),
        bundle_hash: bundle_hash(PLAN, &pairs),
        files,
        missing: missing.iter().map(|m| (*m).to_owned()).collect(),
        refused: None,
        at: Utc::now(),
    }
}

struct H {
    app: App,
    reader: metric_reader::Reader,
    github: Arc<FakeGitHub>,
    _tmp: tempfile::TempDir,
}

async fn harness(cutover: Option<DateTime<Utc>>) -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut cfg = Config::for_tests(url, OWNER);
    cfg.comment_approvals_until = cutover;
    let github = Arc::new(FakeGitHub::new());
    let mut app = AppState::new(pool, cfg, github.clone());
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let text = include_str!("../../../../docs/design/rubato-map.yml");
    let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
    app.set_map(
        rubato_protocol::DEFAULT_WORKSPACE,
        LoadedMap {
            map,
            version: "v".into(),
            loaded_at: Utc::now(),
            yaml: text.into(),
        },
    )
    .await;
    H {
        app,
        reader,
        github,
        _tmp: tmp,
    }
}

/// The cutover an hour ago: approval in Rubato on, comments from then on not counted.
fn cut() -> Option<DateTime<Utc>> {
    Some(Utc::now() - Duration::hours(1))
}

async fn mirror(h: &H, text: &str, missing: &[&str]) {
    plan_drafts::on_draft(&h.app, &bundle(text, missing), OWNER).await;
}

type Pressed = Result<(StatusCode, DraftApproveResponse), (StatusCode, Value)>;

async fn press_on(h: &H, user: &str, branch: &str, sha: &str, rev: Option<u32>) -> Pressed {
    approve_draft(
        State(h.app.clone()),
        Ws(rubato_protocol::DEFAULT_WORKSPACE.into()),
        CurrentUser(user.into()),
        Json(DraftApproveRequest {
            repo: "m0x6A/rubato".into(),
            number: 122,
            branch: branch.into(),
            plan_sha: sha.into(),
            rev,
            summary: "We agreed on the table.".into(),
        }),
    )
    .await
    .map(|(s, j)| (s, j.0))
    .map_err(|(s, j)| (s, j.0))
}

async fn press(h: &H, user: &str, text: &str) -> Pressed {
    press_on(h, user, BRANCH, &approval_hash(text), None).await
}

/// The refusal's status and reason.
fn refused(p: &Pressed) -> (u16, String) {
    match p {
        Err((s, v)) => (s.as_u16(), v["reason"].as_str().unwrap_or("").to_owned()),
        Ok(ok) => panic!("approved: {ok:?}"),
    }
}

fn rejected(h: &H, route: &str, why: &str) -> u64 {
    h.reader
        .points(metric::WEB_REJECTED)
        .iter()
        .filter(|p| {
            p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(why)
                && p.attrs.get(attr::ROUTE).map(String::as_str) == Some(route)
        })
        .map(|p| p.value)
        .sum()
}

async fn rows(h: &H) -> usize {
    records::all(&h.app.db).await.unwrap().len()
}

fn session(user: &str, branch: &str, status: SessionStatus) -> SessionState {
    SessionState {
        session_id: Some("s1".into()),
        companion_id: Uuid::from_u128(5),
        user: user.into(),
        tmux: None,
        cwd: "/w".into(),
        worktree: None,
        branch: Some(branch.into()),
        repo: Some("m0x6A/rubato".into()),
        issue: Some(issue()),
        status,
        source: StateSource::Hook,
        last_seen: Utc::now(),
        spend_usd: None,
        branch_drift: None,
        blocked_by: None,
        launch_id: Some(Uuid::from_u128(1)),
        hooks_lost: false,
    }
}

#[tokio::test]
async fn approval_in_rubato_is_refused_until_the_cutover_is_set() {
    let h = harness(None).await;
    let text = pinned("one");
    mirror(&h, &text, &[]).await;
    assert_eq!(
        refused(&press(&h, "teammate", &text).await),
        (409, reason::CUTOVER_UNSET.into())
    );
    assert_eq!(rows(&h).await, 0, "nothing recorded");
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::CUTOVER_UNSET),
        1
    );
    // The same draft with a cutover set is approved: the refusal was the cutover.
    let h = harness(cut()).await;
    mirror(&h, &text, &[]).await;
    assert_eq!(
        press(&h, "teammate", &text).await.unwrap().0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn no_draft_on_the_branch_is_refused() {
    let h = harness(cut()).await;
    let text = pinned("one");
    mirror(&h, &text, &[]).await;
    let p = press_on(
        &h,
        "teammate",
        "issue/122-other",
        &approval_hash(&text),
        None,
    )
    .await;
    assert_eq!(refused(&p), (404, reason::NO_DRAFT.into()));
    assert_eq!(rows(&h).await, 0);
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::NO_DRAFT),
        1
    );
}

#[tokio::test]
async fn a_version_that_is_not_the_newest_is_refused() {
    let h = harness(cut()).await;
    let one = pinned("one");
    let two = pinned("two");
    mirror(&h, &one, &[]).await;
    mirror(&h, &two, &[]).await;
    let p = press(&h, "teammate", &one).await;
    assert_eq!(refused(&p), (409, reason::NOT_LATEST.into()));
    let body = p.unwrap_err().1;
    assert_eq!(body["current_sha"], approval_hash(&two));
    assert_eq!(body["current_rev"], 2);
    // The newest plan sha, but the person saw revision 1.
    let p = press_on(&h, "teammate", BRANCH, &approval_hash(&two), Some(1)).await;
    assert_eq!(refused(&p), (409, reason::NOT_LATEST.into()));
    assert_eq!(rows(&h).await, 0);
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::NOT_LATEST),
        2
    );
    let ok = press_on(&h, "teammate", BRANCH, &approval_hash(&two), Some(2)).await;
    assert_eq!(ok.unwrap().0, StatusCode::CREATED);
}

#[tokio::test]
async fn the_owners_session_mid_turn_is_refused_as_not_settled() {
    let h = harness(cut()).await;
    let text = pinned("one");
    mirror(&h, &text, &[]).await;
    let companion = Uuid::from_u128(5);
    h.app.set_sessions(
        companion,
        vec![session(OWNER, BRANCH, SessionStatus::Working)],
    );
    let p = press(&h, "teammate", &text).await;
    assert_eq!(refused(&p), (409, reason::NOT_SETTLED.into()));
    // Blocked on a permission prompt is mid-turn too.
    let mut prompt = session(OWNER, BRANCH, SessionStatus::Blocked);
    prompt.blocked_by = Some(BlockCause::PermissionPrompt);
    h.app.set_sessions(companion, vec![prompt]);
    assert_eq!(
        refused(&press(&h, "teammate", &text).await),
        (409, reason::NOT_SETTLED.into())
    );
    assert_eq!(rows(&h).await, 0);
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::NOT_SETTLED),
        2
    );

    // What does not hold an approval back: another person's session, another branch, a
    // session gone quiet, one waiting for input, an ended one.
    let (draft, _) = plan_drafts::on_branch(&h.app, &issue(), BRANCH)
        .await
        .unwrap()
        .unwrap();
    let now = Utc::now();
    let stale_secs = h.app.config.limits.session_stale_secs;
    let mid_turn = |s: SessionState| owner_mid_turn(&[s], &issue(), &draft, now, stale_secs);
    assert!(
        mid_turn(session("M0X6A", BRANCH, SessionStatus::Working)),
        "logins ignore case"
    );
    assert!(!mid_turn(session("alice", BRANCH, SessionStatus::Working)));
    assert!(!mid_turn(session(
        OWNER,
        "issue/122-y",
        SessionStatus::Working
    )));
    let mut quiet = session(OWNER, BRANCH, SessionStatus::Working);
    quiet.last_seen = now - Duration::seconds(stale_secs as i64 + 1);
    assert!(!mid_turn(quiet.clone()), "unknown, never working");
    quiet.status = SessionStatus::Blocked;
    assert!(!mid_turn(quiet), "a block gone quiet");
    let mut idle = session(OWNER, BRANCH, SessionStatus::Blocked);
    idle.blocked_by = Some(BlockCause::IdlePrompt);
    assert!(!mid_turn(idle.clone()));
    assert!(!mid_turn(session(OWNER, BRANCH, SessionStatus::Idle)));
    assert!(!mid_turn(session(OWNER, BRANCH, SessionStatus::Ended)));
    let mut elsewhere = session(OWNER, BRANCH, SessionStatus::Working);
    elsewhere.repo = Some("acme/other".into());
    assert!(!mid_turn(elsewhere), "the same branch name in another repo");

    h.app.set_sessions(companion, vec![idle]);
    assert_eq!(
        press(&h, "teammate", &text).await.unwrap().0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn a_bundle_with_a_missing_file_is_refused() {
    let h = harness(cut()).await;
    let text = pinned("one");
    mirror(&h, &text, &[DESIGN]).await;
    let p = press(&h, "teammate", &text).await;
    assert_eq!(refused(&p), (409, reason::BUNDLE_INCOMPLETE.into()));
    assert_eq!(p.unwrap_err().1["missing"][0], DESIGN);
    assert_eq!(rows(&h).await, 0);
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::BUNDLE_INCOMPLETE),
        1
    );
    // The design note back: approved.
    mirror(&h, &text, &[]).await;
    assert_eq!(
        press(&h, "teammate", &text).await.unwrap().0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn an_adr_not_pinned_at_its_mirrored_blob_is_refused() {
    let h = harness(cut()).await;
    // No Blob at all.
    let bare = plan_text("one", "");
    mirror(&h, &bare, &[]).await;
    let p = press(&h, "teammate", &bare).await;
    assert_eq!(refused(&p), (409, reason::ADRS_UNPINNED.into()));
    assert_eq!(p.unwrap_err().1["adrs"][0]["adr"], "0099");
    // A Blob, of another text than the one mirrored: the handoff would fail adr_changed.
    let other = plan_text("two", &git_blob_sha(b"an earlier draft of the ADR\n"));
    mirror(&h, &other, &[]).await;
    assert_eq!(
        refused(&press(&h, "teammate", &other).await),
        (409, reason::ADRS_UNPINNED.into())
    );
    assert_eq!(rows(&h).await, 0);
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::ADRS_UNPINNED),
        2
    );
    // Pinned at the mirrored file's blob (upper case is the same blob).
    let ok = plan_text("three", &git_blob_sha(ADR_TEXT.as_bytes()).to_uppercase());
    mirror(&h, &ok, &[]).await;
    assert_eq!(
        press(&h, "teammate", &ok).await.unwrap().0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn a_plan_that_fits_no_workspace_or_does_not_parse_is_refused() {
    let h = harness(cut()).await;
    let unfit = pinned("one").replace("| web-server | x | y | z | rubato-server |\n", "");
    mirror(&h, &unfit, &[]).await;
    let p = press(&h, "teammate", &unfit).await;
    assert_eq!(refused(&p), (409, reason::PLAN_DOES_NOT_FIT.into()));
    assert_eq!(p.unwrap_err().1["reasons"][0]["kind"], "missing_o_row");
    let broken = "---\ntitle: no issue\n---\n";
    mirror(&h, broken, &[]).await;
    let p = press(&h, "teammate", broken).await;
    assert_eq!(refused(&p), (409, reason::PLAN_DOES_NOT_FIT.into()));
    assert_eq!(p.unwrap_err().1["reasons"][0]["kind"], "parse_error");
    assert_eq!(rows(&h).await, 0);
    assert_eq!(
        rejected(&h, routes::PLAN_DRAFT_APPROVE, reason::PLAN_DOES_NOT_FIT),
        2
    );
}

#[tokio::test]
async fn self_approval_is_recorded_and_a_second_press_answers_the_first() {
    let h = harness(cut()).await;
    let text = pinned("one");
    mirror(&h, &text, &[]).await;
    let (draft, rev) = plan_drafts::on_branch(&h.app, &issue(), BRANCH)
        .await
        .unwrap()
        .unwrap();
    let rev = rev.unwrap();
    // Two presses at once by the owner: one row.
    let (a, b) = tokio::join!(press(&h, OWNER, &text), press(&h, OWNER, &text));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.1.approval_id, b.1.approval_id);
    assert_eq!(
        [a.1.created, b.1.created].iter().filter(|c| **c).count(),
        1,
        "one press created it"
    );
    let first = if a.1.created { a.1 } else { b.1 };
    assert!(
        first.self_approved,
        "the owner approving their own agent's plan"
    );
    assert_eq!(first.approved_by, OWNER);
    assert_eq!(first.plan_sha, approval_hash(&text));
    assert_eq!(
        first.bundle_hash, rev.info.bundle_hash,
        "the bundle hash stored"
    );
    assert_eq!(first.bundle_hash, bundle(&text, &[]).bundle_hash);
    // A teammate pressing the same version later finds the first approval.
    let (status, again) = press(&h, "teammate", &text).await.unwrap();
    assert_eq!((status, again.created), (StatusCode::OK, false));
    assert_eq!(again.approval_id, first.approval_id);
    assert_eq!(again.approved_by, OWNER);
    let stored = records::all(&h.app.db).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        (stored[0].draft_id, stored[0].rev, stored[0].branch.as_str()),
        (draft.id, rev.info.rev, BRANCH)
    );
    assert_eq!(stored[0].summary, "We agreed on the table.");
    assert!(stored[0].self_approved);
    // Approving another person's draft is not self.
    let next = pinned("two");
    mirror(&h, &next, &[]).await;
    let (_, other) = press(&h, "teammate", &next).await.unwrap();
    assert!(!other.self_approved);
}

#[tokio::test]
async fn accept_plan_refuses_after_the_cutover_and_posts_nothing() {
    let accept = |h: &H| {
        approve(
            State(h.app.clone()),
            Ws(rubato_protocol::DEFAULT_WORKSPACE.into()),
            CurrentUser("teammate".into()),
            Json(ApproveRequest {
                issue: IssueRef::new("m0x6A/rubato", 71),
                sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9".into(),
            }),
        )
    };
    let h = harness(cut()).await;
    let (status, body) = accept(&h).await.unwrap_err();
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::CONFLICT, Some(reason::APPROVE_IN_RUBATO))
    );
    assert_eq!(
        rejected(&h, routes::PLAN_APPROVE, reason::APPROVE_IN_RUBATO),
        1
    );
    assert!(
        !h.github.calls().iter().any(|(k, _)| *k == "comment_as"),
        "nothing posted"
    );
    // Before the cutover, and with none, it goes on as before (here: no plan yet).
    for cutover in [Some(Utc::now() + Duration::hours(1)), None] {
        let h = harness(cutover).await;
        let (_, body) = accept(&h).await.unwrap_err();
        assert_eq!(body["reason"].as_str(), Some(reason::NO_PLAN));
    }
}

fn approved_comment(id: u64, created_at: DateTime<Utc>, body: &str) -> Comment {
    Comment {
        id,
        author: "teammate".into(),
        author_association: "MEMBER".into(),
        author_is_bot: false,
        body: body.into(),
        created_at,
        url: format!("https://github.com/m0x6A/rubato/issues/71#issuecomment-{id}"),
    }
}

/// ADR 0054: an `Approved:` comment posted at or after the cutover is stored, so it shows as a
/// finding, counted on `rubato.plans.comment_after_cutover` the first time it is read, and is
/// never the newest counted approval. Without a cutover the same comment counts.
#[tokio::test]
async fn comments_after_the_cutover_are_seen_counted_once_and_never_newest() {
    let i71 = IssueRef::new("m0x6A/rubato", 71);
    let comments = vec![
        approved_comment(
            1,
            at(9),
            "Approved: plan f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9",
        ),
        approved_comment(
            2,
            at(12),
            "Approved: plan c438e006b75b48508ae0d204788e73e11005a6d3 on issue/71-b",
        ),
    ];
    let counter = |h: &H| -> u64 {
        h.reader
            .points(metric::PLANS_COMMENT_AFTER_CUTOVER)
            .iter()
            .map(|p| p.value)
            .sum()
    };
    let h = harness(Some(at(12))).await;
    h.github.with(|g| {
        drop(
            g.comments
                .insert(("m0x6A/rubato".into(), 71), comments.clone()),
        )
    });
    let read = github_poll::poll_comments(&h.app, &i71).await.unwrap();
    github_poll::poll_comments(&h.app, &i71).await.unwrap();
    assert_eq!(read.newest_approval.and_then(|a| a.comment_id), Some(1));
    assert_eq!(counter(&h), 1, "counted once, on the first read");
    let list = approvals(&h.app).await.unwrap();
    assert_eq!(
        list.counted
            .iter()
            .map(|(a, _)| a.comment_id)
            .collect::<Vec<_>>(),
        [Some(1)]
    );
    assert_eq!(
        list.ignored
            .iter()
            .map(|a| a.comment_id)
            .collect::<Vec<_>>(),
        [Some(2)]
    );
    // The comment at the cutover does not steer which branch the plan is read from.
    assert!(!github_poll::approved_branches(&list.counted).contains_key(&i71));

    let h = harness(None).await;
    h.github
        .with(|g| drop(g.comments.insert(("m0x6A/rubato".into(), 71), comments)));
    let read = github_poll::poll_comments(&h.app, &i71).await.unwrap();
    assert_eq!(read.newest_approval.and_then(|a| a.comment_id), Some(2));
    assert_eq!(counter(&h), 0);
    let list = approvals(&h.app).await.unwrap();
    assert_eq!((list.counted.len(), list.ignored.len()), (2, 0));
    assert_eq!(
        github_poll::approved_branches(&list.counted)
            .get(&i71)
            .map(String::as_str),
        Some("issue/71-b")
    );
}

/// ADR 0054: the poll prefers the branch of the newest Rubato approval, even over a newer
/// counted comment naming another branch, so the handoff commit is read.
#[test]
fn the_branch_of_the_newest_rubato_approval_is_preferred() {
    let i = IssueRef::new("m0x6A/rubato", 71);
    let comment = |id: u64, created_at: DateTime<Utc>, branch: &str| {
        (
            rubato_protocol::ApprovalRecord {
                issue: i.clone(),
                source: ApprovalSource::Comment,
                comment_id: Some(id),
                approval_id: None,
                bundle_hash: None,
                author: "teammate".into(),
                sha: Some("f3c5e84".into()),
                branch: Some(branch.into()),
                note: None,
                created_at,
                url: String::new(),
            },
            None,
        )
    };
    let rubato = |n: u128, approved_at: DateTime<Utc>, branch: &str| {
        (
            records::RubatoApproval {
                id: Uuid::from_u128(n),
                issue: i.clone(),
                branch: branch.into(),
                draft_id: Uuid::from_u128(9),
                rev: 1,
                plan_sha: "f3c5e84".into(),
                bundle_hash: "h".into(),
                approved_by: OWNER.into(),
                approved_at,
                self_approved: true,
                summary: String::new(),
                handoff: None,
            },
            None,
        )
    };
    let prefer = |comments, rubato| {
        github_poll::approved_branches(&merge(comments, rubato, Some(at(12))).counted)
            .get(&i)
            .cloned()
    };
    assert_eq!(
        prefer(vec![comment(1, at(11), "issue/71-b")], vec![]).as_deref(),
        Some("issue/71-b"),
        "with no Rubato approval, the newest counted comment"
    );
    assert_eq!(
        prefer(
            vec![comment(1, at(11), "issue/71-b")],
            vec![rubato(1, at(10), "issue/71-a")]
        )
        .as_deref(),
        Some("issue/71-a")
    );
    assert_eq!(
        prefer(
            vec![],
            vec![
                rubato(1, at(13), "issue/71-a"),
                rubato(2, at(14), "issue/71-c")
            ]
        )
        .as_deref(),
        Some("issue/71-c"),
        "the newest Rubato approval"
    );
}

/// A version is the plan and its whole bundle: a revision that changes only the design note is a
/// new version. Its approval is a new row (not the first one with the old bundle, which the
/// handoff would refuse as bundle_changed), and until then the first approval reads as for an
/// earlier version, so the new one can be approved at all.
#[tokio::test]
async fn a_revision_that_changes_only_the_design_note_is_a_new_version_to_approve() {
    let h = harness(cut()).await;
    let text = pinned("one");
    mirror(&h, &text, &[]).await;
    let (_, first) = press(&h, "teammate", &text).await.unwrap();
    // The same plan, another design note.
    let mut b = bundle(&text, &[]);
    for f in &mut b.files {
        if f.path == DESIGN {
            f.text = "design, revised\n".into();
        }
    }
    let pairs: Vec<(String, String)> = b
        .files
        .iter()
        .map(|f| (f.path.clone(), f.text.clone()))
        .collect();
    b.bundle_hash = bundle_hash(PLAN, &pairs);
    plan_drafts::on_draft(&h.app, &b, OWNER).await;
    let (_, rev) = plan_drafts::on_branch(&h.app, &issue(), BRANCH)
        .await
        .unwrap()
        .unwrap();
    let rev = rev.unwrap();
    assert_eq!(rev.info.rev, 2, "a new revision");
    assert_eq!(rev.info.plan_sha, first.plan_sha, "with the same plan");

    // Before it is approved, the first approval is for an earlier version.
    let list = approvals(&h.app).await.unwrap();
    let (rec, target) = list
        .counted
        .iter()
        .find(|(a, _)| a.approval_id == Some(first.approval_id))
        .unwrap();
    assert_eq!(rec.bundle_hash.as_deref(), Some(first.bundle_hash.as_str()));
    assert_eq!(
        target.as_deref(),
        Some(b.bundle_hash.as_str()),
        "compared with the new bundle"
    );
    assert_ne!(
        target.as_deref(),
        rec.sha.as_deref(),
        "so it is not current"
    );

    let (status, second) = press(&h, "teammate", &text).await.unwrap();
    assert_eq!((status, second.created), (StatusCode::CREATED, true));
    assert_ne!(second.approval_id, first.approval_id);
    assert_eq!(second.bundle_hash, b.bundle_hash);
    assert_eq!(rows(&h).await, 2);
    // The new approval is current: its target is the plan's hash again.
    let list = approvals(&h.app).await.unwrap();
    let (rec, target) = list
        .counted
        .iter()
        .find(|(a, _)| a.approval_id == Some(second.approval_id))
        .unwrap();
    assert_eq!(target.as_deref(), rec.sha.as_deref());
}
