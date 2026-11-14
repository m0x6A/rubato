//! The plan's room (ADR 0057, plan 122 §D8): messages everyone reads, refusals by reason, no room
//! without a plan or draft, one seam notice per pair of issue and seam in both rooms, none when
//! the seams differ or one plan has merged.

use super::{SYSTEM_AUTHOR, post, post_seam_notices, room};
use crate::auth::CurrentUser;
use crate::kernel::config::Config;
use crate::kernel::db::{connect, ts};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState, LoadedMap};
use crate::kernel::telemetry::{capture, metric_reader};
use crate::{plan_drafts, plans};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::room::{reason, routes};
use rubato_protocol::{
    DraftBundle, DraftFile, IssueRef, RoomAuthorKind, RoomKey, RoomMessage, RoomPostRequest,
    SeamNotice, SystemMap, WebEvent, WebResource, bundle_hash,
};
use std::sync::Arc;
use uuid::Uuid;

const REPO: &str = "m0x6A/rubato";

fn plan_text(n: u64, seam: &str) -> String {
    format!(
        "---\nissue: m0x6A/rubato#{n}\n---\n# Plan\n\n## Affected parts\n\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | x | {seam} |\n\n## O-rows\n\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| {seam} | x | y | z | rubato-server |\n"
    )
}

struct H {
    app: App,
    reader: metric_reader::Reader,
    github: Arc<FakeGitHub>,
    map: LoadedMap,
    _tmp: tempfile::TempDir,
}

async fn harness() -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let github = Arc::new(FakeGitHub::new());
    let mut app = AppState::new(pool, Config::for_tests(url, "m0x6A"), github.clone());
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let text = include_str!("../../../../docs/design/rubato-map.yml");
    let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
    let map = LoadedMap {
        map,
        version: "v".into(),
        loaded_at: Utc::now(),
        yaml: text.into(),
    };
    app.set_map(rubato_protocol::DEFAULT_WORKSPACE, map.clone())
        .await;
    H {
        app,
        reader,
        github,
        map,
        _tmp: tmp,
    }
}

/// Issue `n`'s plan on `branch`, read by the poll.
async fn plan_on(h: &H, n: u64, branch: &str, seam: &str) {
    let issue = IssueRef::new(REPO, n);
    let path = plans::plan_path(&h.map, &issue, branch);
    h.github.with(|g| {
        g.files
            .insert((REPO.into(), path, branch.into()), plan_text(n, seam));
    });
    plans::fetch_and_store(&h.app, &h.map, &issue, branch, branch)
        .await
        .unwrap();
}

/// Issue `n`'s draft on `branch`, mirrored from its owner's session.
async fn draft_on(h: &H, n: u64, branch: &str, seam: &str) {
    let path = format!("docs/plans/{n}.md");
    let text = plan_text(n, seam);
    plan_drafts::on_draft(
        &h.app,
        &DraftBundle {
            launch_id: Uuid::from_u128(n as u128),
            session_id: None,
            issue: IssueRef::new(REPO, n),
            branch: branch.into(),
            bundle_hash: bundle_hash(&path, &[(path.clone(), text.clone())]),
            files: vec![DraftFile { path, text }],
            missing: vec![],
            refused: None,
            at: Utc::now(),
        },
        "m0x6A",
    )
    .await;
}

/// Issue `n`'s pull request merged.
async fn merged(h: &H, n: u64, branch: &str) {
    let pr = rubato_protocol::PullRequestState {
        repo: REPO.into(),
        number: 100 + n,
        branch: branch.into(),
        open: false,
        draft: false,
        checks: rubato_protocol::ChecksStatus::Green,
        merged_at: Some(Utc::now()),
        updated_at: Utc::now(),
        url: "u".into(),
    };
    sqlx::query("INSERT INTO pull_requests (repo, number, pr, fetched_at) VALUES (?, ?, ?, ?)")
        .bind(REPO)
        .bind(n as i64)
        .bind(serde_json::to_string(&pr).unwrap())
        .bind(ts(Utc::now()))
        .execute(&h.app.db)
        .await
        .unwrap();
}

type Posted = Result<(StatusCode, RoomMessage), (StatusCode, String)>;

async fn say(h: &H, user: &str, n: u64, branch: &str, text: &str) -> Posted {
    post(
        State(h.app.clone()),
        CurrentUser(user.into()),
        Json(RoomPostRequest {
            repo: REPO.into(),
            number: n,
            branch: branch.into(),
            text: text.into(),
        }),
    )
    .await
    .map(|(s, j)| (s, j.0))
    .map_err(|(s, j)| (s, j.0["reason"].as_str().unwrap_or("").to_owned()))
}

async fn read(h: &H, user: &str, n: u64, branch: &str) -> Vec<RoomMessage> {
    room(
        State(h.app.clone()),
        CurrentUser(user.into()),
        Query(RoomKey {
            repo: REPO.into(),
            number: n,
            branch: branch.into(),
        }),
    )
    .await
    .unwrap()
    .0
    .messages
}

fn rejected(h: &H, why: &str) -> u64 {
    h.reader
        .points(metric::WEB_REJECTED)
        .iter()
        .filter(|p| {
            p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(why)
                && p.attrs.get(attr::ROUTE).map(String::as_str) == Some(routes::ROOM_MESSAGES)
        })
        .map(|p| p.value)
        .sum()
}

/// ADR 0057: what one person posts, another reads, oldest first; a web view of the room is told.
#[tokio::test]
async fn messages_are_read_by_everyone_who_may_log_in() {
    let h = harness().await;
    plan_on(&h, 1, "issue/1-x", "web-server").await;
    let mut web = h.app.web_tx.subscribe();
    let (status, first) = say(&h, "alice", 1, "issue/1-x", "  Split the table?  ")
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        (
            first.author_kind,
            first.author.as_str(),
            first.text.as_str()
        ),
        (RoomAuthorKind::Person, "alice", "Split the table?"),
        "trimmed, as its author"
    );
    assert_eq!(first.notice, None);
    let (_, second) = say(&h, "bob", 1, "issue/1-x", "No, one table.")
        .await
        .unwrap();
    let seen = read(&h, "carol", 1, "issue/1-x").await;
    assert_eq!(seen, vec![first, second], "carol reads both, oldest first");
    assert!(
        read(&h, "carol", 1, "issue/1-y").await.is_empty(),
        "another branch is another room"
    );
    let mut rooms = 0;
    while let Ok(ev) = web.try_recv() {
        if let WebEvent::Updated {
            resource:
                WebResource::Room {
                    repo,
                    number,
                    branch,
                },
            ..
        } = ev
        {
            assert_eq!(
                (repo.as_str(), number, branch.as_str()),
                (REPO, 1, "issue/1-x")
            );
            rooms += 1;
        }
    }
    assert_eq!(rooms, 2, "one Updated per message");
}

/// Refusals by reason, each counted on `rubato.web.rejected` for the route; nothing is stored.
/// A message of exactly the limit is taken, so the length check is the limit, not near it.
#[tokio::test]
async fn refusals_are_counted_by_reason_and_store_nothing() {
    let h = harness().await;
    plan_on(&h, 1, "issue/1-x", "web-server").await;
    let limit = h.app.config.limits.room_message_max_chars as usize;
    assert_eq!(
        say(&h, "alice", 1, "issue/1-x", " \n\t ")
            .await
            .unwrap_err(),
        (StatusCode::UNPROCESSABLE_ENTITY, reason::EMPTY.into())
    );
    assert_eq!(
        say(&h, "alice", 1, "issue/1-x", &"é".repeat(limit + 1))
            .await
            .unwrap_err(),
        (StatusCode::UNPROCESSABLE_ENTITY, reason::TOO_LONG.into())
    );
    assert!(read(&h, "bob", 1, "issue/1-x").await.is_empty());
    assert_eq!(
        (rejected(&h, reason::EMPTY), rejected(&h, reason::TOO_LONG)),
        (1, 1)
    );
    let at_limit = say(&h, "alice", 1, "issue/1-x", &"é".repeat(limit)).await;
    assert_eq!(
        at_limit.unwrap().0,
        StatusCode::CREATED,
        "characters, not bytes"
    );
}

/// A room belongs to a plan: a branch with neither a plan nor a draft has none; a draft alone is
/// enough.
#[tokio::test]
async fn a_branch_with_no_plan_and_no_draft_has_no_room() {
    let h = harness().await;
    plan_on(&h, 1, "issue/1-x", "web-server").await;
    assert_eq!(
        say(&h, "alice", 1, "issue/1-y", "hello").await.unwrap_err(),
        (StatusCode::NOT_FOUND, reason::NO_PLAN.into()),
        "the issue has a plan, but on another branch"
    );
    assert_eq!(
        say(&h, "alice", 2, "issue/2-d", "hello").await.unwrap_err(),
        (StatusCode::NOT_FOUND, reason::NO_PLAN.into())
    );
    assert_eq!(rejected(&h, reason::NO_PLAN), 2);
    draft_on(&h, 2, "issue/2-d", "web-server").await;
    assert_eq!(
        say(&h, "alice", 2, "issue/2-d", "hello").await.unwrap().0,
        StatusCode::CREATED
    );
}

fn notices(messages: &[RoomMessage]) -> Vec<SeamNotice> {
    messages
        .iter()
        .filter(|m| m.author_kind == RoomAuthorKind::System && m.author == SYSTEM_AUTHOR)
        .filter_map(|m| m.notice.clone())
        .collect()
}

/// Q6: an open plan and a settled draft touching the same seam get one notice each, about the
/// other, over two ticks; a plan on another seam gets none.
#[tokio::test]
async fn a_seam_notice_is_posted_once_per_pair_in_both_rooms() {
    let h = harness().await;
    plan_on(&h, 1, "issue/1-x", "web-server").await;
    draft_on(&h, 2, "issue/2-d", "web-server").await;
    plan_on(&h, 3, "issue/3-z", "server-github").await;
    let (events, _guard) = capture::start();
    assert_eq!(post_seam_notices(&h.app).await.unwrap(), 2);
    assert_eq!(
        post_seam_notices(&h.app).await.unwrap(),
        0,
        "the second tick posts none"
    );
    let in1 = read(&h, "carol", 1, "issue/1-x").await;
    let in2 = read(&h, "carol", 2, "issue/2-d").await;
    assert_eq!(
        notices(&in1),
        vec![SeamNotice {
            seam: "web-server".into(),
            other: IssueRef::new(REPO, 2),
            other_branch: "issue/2-d".into(),
        }]
    );
    assert_eq!(
        notices(&in2),
        vec![SeamNotice {
            seam: "web-server".into(),
            other: IssueRef::new(REPO, 1),
            other_branch: "issue/1-x".into(),
        }]
    );
    assert_eq!(
        in1[0].text,
        "This plan and m0x6A/rubato#2 on issue/2-d both touch web-server."
    );
    assert!(read(&h, "carol", 3, "issue/3-z").await.is_empty());
    let events = events.lock().unwrap();
    let posted: Vec<_> = events
        .iter()
        .filter(|e| e.message == "seam notice posted")
        .collect();
    assert_eq!(posted.len(), 2);
    assert!(posted.iter().all(|e| e.level == tracing::Level::INFO));
}

/// No notice for plans on different seams, nor with a plan whose pull request merged.
#[tokio::test]
async fn no_notice_when_the_seams_differ_or_one_plan_has_merged() {
    let h = harness().await;
    plan_on(&h, 1, "issue/1-x", "web-server").await;
    plan_on(&h, 3, "issue/3-z", "server-github").await;
    assert_eq!(post_seam_notices(&h.app).await.unwrap(), 0);
    plan_on(&h, 4, "issue/4-m", "web-server").await;
    merged(&h, 4, "issue/4-m").await;
    assert_eq!(post_seam_notices(&h.app).await.unwrap(), 0, "#4 is done");
    // The same pair with #4 open: the check above is not passing by accident.
    sqlx::query("DELETE FROM pull_requests")
        .execute(&h.app.db)
        .await
        .unwrap();
    assert_eq!(post_seam_notices(&h.app).await.unwrap(), 2);
}
