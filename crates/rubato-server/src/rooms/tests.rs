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
    app.set_map(
        &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
        map.clone(),
    )
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
    sqlx::query("INSERT INTO pull_requests (tenant_id, repo, number, pr, fetched_at) VALUES ('default', ?, ?, ?, ?)")
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
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
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
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
        0
    );
    plan_on(&h, 4, "issue/4-m", "web-server").await;
    merged(&h, 4, "issue/4-m").await;
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
        0,
        "#4 is done"
    );
    // The same pair with #4 open: the check above is not passing by accident.
    sqlx::query("DELETE FROM pull_requests")
        .execute(&h.app.db)
        .await
        .unwrap();
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
        2
    );
}

/// #265: no notice with a plan whose issue is closed on GitHub, merged or not.
#[tokio::test]
async fn no_notice_when_one_issue_is_closed() {
    let h = harness().await;
    plan_on(&h, 1, "issue/1-x", "web-server").await;
    plan_on(&h, 4, "issue/4-m", "web-server").await;
    let set = async |state: &str| {
        sqlx::query(
            "INSERT INTO issue_meta (tenant_id, repo, number, github_state, fetched_at) VALUES ('default', ?, 4, ?, ?) \
             ON CONFLICT(tenant_id, repo, number) DO UPDATE SET github_state = excluded.github_state",
        )
        .bind(REPO)
        .bind(state)
        .bind(ts(Utc::now()))
        .execute(&h.app.db)
        .await
        .unwrap();
    };
    set("closed").await;
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
        0,
        "#4 is closed"
    );
    // The same pair with #4 open: the check above is not passing by accident.
    set("open").await;
    assert_eq!(
        post_seam_notices(&h.app, &crate::kernel::tenancy::TenantId::default_tenant())
            .await
            .unwrap(),
        2
    );
}

// ---- PR 11: calling an agent in, addressed turns, replies, limits (ADR 0057, plan 122 §D8) ----

mod call_in_tests {
    use super::*;
    use crate::rooms::{
        TURN_LIMIT_TEXT, answer, call_in, dismiss, on_hook, on_room_reply, sweep_agents,
    };
    use crate::session_messages::{self, RoomTurnRef};
    use axum::extract::Path;
    use rubato_protocol::{
        CallInRequest, CallInResponse, DismissAgentRequest, HookEvent, LaunchCommand, LeftReason,
        ROOM_TURN_PREAMBLE, RequestAnswer, RequestAnswerBody, RequestState, RoomAgent, RoomReply,
        RoomRequest, RoomTurnMode, capability,
    };
    use std::collections::HashSet;

    const BRANCH: &str = "issue/7-x";
    const OWNER: &str = "m0x6A";

    /// One second later: messages, joins and turns get distinct times.
    fn tick(h: &H) {
        let next = h.app.clock.now() + chrono::Duration::seconds(1);
        h.app.clock.set(next);
    }

    /// A launch of issue `n` on `branch` by `user` through `companion`, which can take turns
    /// when `turns` is set.
    async fn launched(
        h: &H,
        user: &str,
        companion: Uuid,
        n: u64,
        branch: &str,
        turns: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let cmd = LaunchCommand {
            id,
            user: user.into(),
            issue: IssueRef::new(REPO, n),
            repo_url: "https://github.com/m0x6A/rubato.git".into(),
            branch: branch.into(),
            base_ref: "origin/main".into(),
            node_ids: vec![],
            spend_cap_usd: 10.0,
            attempt_limit: 3,
            attempt: 1,
            session_id: Uuid::nil(),
            prompt: "p".into(),
            otel_resource: Default::default(),
            map_version: None,
            plan_path: Some(format!("docs/plans/{n}.md")),
            draft: None,
            planning: Default::default(),
            role: Default::default(),
            lead_launch_id: None,
        };
        sqlx::query("INSERT INTO launches (workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', ?, ?, ?, ?, ?, ?, ?)")
            .bind(id.to_string())
            .bind(companion.to_string())
            .bind(user)
            .bind(REPO)
            .bind(n as i64)
            .bind(serde_json::to_string(&cmd).unwrap())
            .bind(ts(Utc::now()))
            .execute(&h.app.db)
            .await
            .unwrap();
        let mut caps = vec![capability::SESSION_INPUT, capability::HOOK_ALLOWLIST];
        if turns {
            caps.push(capability::ROOM_TURNS);
        }
        h.app
            .set_capabilities(companion, caps.into_iter().map(str::to_owned).collect());
        id
    }

    type Called = Result<(StatusCode, CallInResponse), (StatusCode, String)>;

    async fn call(h: &H, user: &str, n: u64, launch: Uuid) -> Called {
        call_in(
            State(h.app.clone()),
            CurrentUser(user.into()),
            Json(CallInRequest {
                repo: REPO.into(),
                number: n,
                branch: BRANCH.into(),
                launch_id: launch,
            }),
        )
        .await
        .map(|(s, j)| (s, j.0))
        .map_err(|(s, j)| (s, j.0["reason"].as_str().unwrap_or("").to_owned()))
    }

    async fn answer_as(
        h: &H,
        user: &str,
        id: Uuid,
        a: RequestAnswer,
    ) -> Result<RoomRequest, (StatusCode, String)> {
        answer(
            State(h.app.clone()),
            crate::kernel::tenancy::default_scope(),
            CurrentUser(user.into()),
            Path(id.into()),
            Json(RequestAnswerBody { answer: a }),
        )
        .await
        .map(|j| j.0)
        .map_err(|(s, j)| (s, j.0["reason"].as_str().unwrap_or("").to_owned()))
    }

    async fn dismiss_as(
        h: &H,
        user: &str,
        n: u64,
        handle: &str,
    ) -> Result<RoomAgent, (StatusCode, String)> {
        dismiss(
            State(h.app.clone()),
            CurrentUser(user.into()),
            Json(DismissAgentRequest {
                repo: REPO.into(),
                number: n,
                branch: BRANCH.into(),
                handle: handle.into(),
            }),
        )
        .await
        .map(|j| j.0)
        .map_err(|(s, j)| (s, j.0["reason"].as_str().unwrap_or("").to_owned()))
    }

    async fn view(h: &H, n: u64) -> rubato_protocol::RoomView {
        room(
            State(h.app.clone()),
            CurrentUser("carol".into()),
            Query(RoomKey {
                repo: REPO.into(),
                number: n,
                branch: BRANCH.into(),
            }),
        )
        .await
        .unwrap()
        .0
    }

    /// Every room turn queued for `launch`, with the session message carrying it.
    async fn turns_of(
        h: &H,
        launch: Uuid,
    ) -> Vec<(rubato_protocol::SessionMessageView, RoomTurnRef)> {
        let views = session_messages::list(
            State(h.app.clone()),
            CurrentUser(OWNER.into()),
            Path(launch.into()),
        )
        .await
        .unwrap()
        .0
        .messages;
        let mut out = Vec::new();
        for v in views.into_iter().filter(|v| v.kind == "room_turn") {
            let t = session_messages::room_turn(&h.app.db, v.id)
                .await
                .unwrap()
                .unwrap();
            out.push((v, t));
        }
        out
    }

    /// The one turn queued for `launch` that is not in `seen`.
    async fn new_turn(
        h: &H,
        launch: Uuid,
        seen: &mut HashSet<Uuid>,
    ) -> (rubato_protocol::SessionMessageView, RoomTurnRef) {
        let fresh: Vec<_> = turns_of(h, launch)
            .await
            .into_iter()
            .filter(|(v, _)| !seen.contains(&v.id))
            .collect();
        assert_eq!(fresh.len(), 1, "one new turn: {fresh:?}");
        let (v, t) = fresh.into_iter().next().unwrap();
        seen.insert(v.id);
        (v, t)
    }

    fn reply(t: &RoomTurnRef, text: &str, fork: Option<&str>, cost: Option<f64>) -> RoomReply {
        RoomReply {
            message_id: t.message_id,
            turn_id: t.turn_id,
            text: text.into(),
            fork_session_id: fork.map(str::to_owned),
            cost_usd: cost,
            error: None,
            at: Utc::now(),
        }
    }

    async fn setup(h: &H) {
        h.app.clock.set(Utc::now());
    }

    /// The owner calls their own agent in at once; a teammate's call is a request the owner
    /// answers. Decline, accept, the asker's withdraw, and the refusals around them.
    #[tokio::test]
    async fn the_owner_calls_their_agent_in_and_a_teammate_asks() {
        let h = harness().await;
        setup(&h).await;
        draft_on(&h, 7, BRANCH, "web-server").await;
        let ca = Uuid::from_u128(0xA);
        let la = launched(&h, OWNER, ca, 7, BRANCH, true).await;

        // A teammate asks: pending, and asking again finds the same request.
        let (s, CallInResponse::Requested { request }) = call(&h, "alice", 7, la).await.unwrap()
        else {
            panic!("a teammate's call is a request")
        };
        assert_eq!(
            (s, request.state, request.owner.as_str()),
            (StatusCode::CREATED, RequestState::Pending, OWNER)
        );
        let (s, CallInResponse::Requested { request: again }) =
            call(&h, "alice", 7, la).await.unwrap()
        else {
            panic!("still a request")
        };
        assert_eq!((s, again.id), (StatusCode::OK, request.id));
        assert!(
            view(&h, 7).await.agents.is_empty(),
            "nobody is in until the owner accepts"
        );

        // Only the owner answers; an answered request stays answered.
        assert_eq!(
            answer_as(&h, "bob", request.id, RequestAnswer::Accept)
                .await
                .unwrap_err(),
            (StatusCode::FORBIDDEN, "not_yours".into())
        );
        assert_eq!(
            answer_as(&h, "alice", request.id, RequestAnswer::Accept)
                .await
                .unwrap_err()
                .1,
            "not_yours",
            "the asker cannot accept"
        );
        let declined = answer_as(&h, OWNER, request.id, RequestAnswer::Decline)
            .await
            .unwrap();
        assert_eq!(declined.state, RequestState::Declined);
        assert_eq!(
            answer_as(&h, OWNER, request.id, RequestAnswer::Accept)
                .await
                .unwrap_err(),
            (StatusCode::CONFLICT, "answered".into())
        );

        // Asked again and withdrawn by the asker; the owner cannot withdraw it for them.
        let (_, CallInResponse::Requested { request: second }) =
            call(&h, "alice", 7, la).await.unwrap()
        else {
            panic!()
        };
        assert_ne!(second.id, request.id);
        assert_eq!(
            answer_as(&h, OWNER, second.id, RequestAnswer::Withdraw)
                .await
                .unwrap_err()
                .1,
            "not_yours"
        );
        assert_eq!(
            answer_as(&h, "alice", second.id, RequestAnswer::Withdraw)
                .await
                .unwrap()
                .state,
            RequestState::Withdrawn
        );

        // Asked a third time and accepted: in, called in by the asker.
        let (_, CallInResponse::Requested { request: third }) =
            call(&h, "alice", 7, la).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(
            answer_as(&h, OWNER, third.id, RequestAnswer::Accept)
                .await
                .unwrap()
                .state,
            RequestState::Accepted
        );
        let v = view(&h, 7).await;
        assert_eq!(v.agents.len(), 1);
        assert_eq!(
            (
                v.agents[0].handle.as_str(),
                v.agents[0].called_by.as_str(),
                v.agents[0].launch_id
            ),
            ("m0x6A-agent", "alice", la)
        );
        assert_eq!(
            call(&h, OWNER, 7, la).await.unwrap_err(),
            (StatusCode::CONFLICT, "already_in".into())
        );

        // In another room the owner's own call joins at once, called in by themselves.
        let h2 = harness().await;
        setup(&h2).await;
        draft_on(&h2, 7, BRANCH, "web-server").await;
        let lb = launched(&h2, OWNER, ca, 7, BRANCH, true).await;
        let (s, CallInResponse::Joined { agent }) = call(&h2, OWNER, 7, lb).await.unwrap() else {
            panic!("the owner's own call joins")
        };
        assert_eq!((s, agent.called_by.as_str()), (StatusCode::CREATED, OWNER));
        // A companion that cannot take turns, and a launch of another branch, are refused.
        let old = launched(&h2, "bob", Uuid::from_u128(0xB0B), 7, BRANCH, false).await;
        assert_eq!(
            call(&h2, "bob", 7, old).await.unwrap_err(),
            (StatusCode::CONFLICT, "companion_cannot".into())
        );
        let elsewhere = launched(&h2, OWNER, ca, 7, "issue/7-y", true).await;
        assert_eq!(
            call(&h2, OWNER, 7, elsewhere).await.unwrap_err(),
            (StatusCode::UNPROCESSABLE_ENTITY, "not_this_plan".into())
        );
    }

    /// The owner or whoever called it in dismisses an agent, nobody else; one not addressed for
    /// room_agent_idle_secs leaves on its own.
    #[tokio::test]
    async fn dismissal_is_the_owners_or_the_callers_and_an_idle_agent_times_out() {
        let h = harness().await;
        setup(&h).await;
        draft_on(&h, 7, BRANCH, "web-server").await;
        let la = launched(&h, OWNER, Uuid::from_u128(0xA), 7, BRANCH, true).await;
        let (_, CallInResponse::Requested { request }) = call(&h, "alice", 7, la).await.unwrap()
        else {
            panic!()
        };
        answer_as(&h, OWNER, request.id, RequestAnswer::Accept)
            .await
            .unwrap();
        assert_eq!(
            dismiss_as(&h, "carol", 7, "m0x6A-agent").await.unwrap_err(),
            (StatusCode::FORBIDDEN, "not_yours".into())
        );
        let gone = dismiss_as(&h, "alice", 7, "m0x6a-agent").await.unwrap();
        assert_eq!(
            gone.left_reason,
            Some(LeftReason::Dismissed),
            "the caller dismisses it"
        );
        tick(&h);
        let (_, CallInResponse::Joined { .. }) = call(&h, OWNER, 7, la).await.unwrap() else {
            panic!()
        };
        assert_eq!(
            dismiss_as(&h, OWNER, 7, "m0x6A-agent")
                .await
                .unwrap()
                .left_reason,
            Some(LeftReason::Dismissed)
        );

        tick(&h);
        let (_, CallInResponse::Joined { .. }) = call(&h, OWNER, 7, la).await.unwrap() else {
            panic!()
        };
        h.app
            .clock
            .set(h.app.clock.now() + chrono::Duration::seconds(1799));
        assert_eq!(sweep_agents(&h.app).await.unwrap(), 0, "not yet");
        h.app
            .clock
            .set(h.app.clock.now() + chrono::Duration::seconds(2));
        assert_eq!(sweep_agents(&h.app).await.unwrap(), 1);
        let v = view(&h, 7).await;
        let last = v
            .agents
            .iter()
            .find(|a| a.left_reason == Some(LeftReason::TimedOut));
        assert!(last.is_some(), "{:?}", v.agents);
        assert!(v.agents.iter().all(|a| a.left_at.is_some()), "nobody is in");
    }

    /// A person's message naming the agent makes a turn: the room since the agent joined (not
    /// before), quoted with each author, for the owner's launch, authored by the addresser;
    /// typed into the session while drafting, a fork once the plan is on the branch, and the
    /// next turn carries only what came after the last one.
    #[tokio::test]
    async fn an_addressed_turn_carries_the_room_since_the_agent_joined_and_follows_the_phase() {
        let h = harness().await;
        setup(&h).await;
        draft_on(&h, 7, BRANCH, "web-server").await;
        let la = launched(&h, OWNER, Uuid::from_u128(0xA), 7, BRANCH, true).await;
        say(&h, "alice", 7, BRANCH, "before anyone was called in")
            .await
            .unwrap();
        tick(&h);
        let (_, CallInResponse::Joined { .. }) = call(&h, OWNER, 7, la).await.unwrap() else {
            panic!()
        };
        tick(&h);
        say(&h, "bob", 7, BRANCH, "plain message").await.unwrap();
        tick(&h);
        assert!(
            turns_of(&h, la).await.is_empty(),
            "a message that names no agent makes no turn"
        );
        say(&h, "alice", 7, BRANCH, "@m0x6A-agent why b?")
            .await
            .unwrap();
        let mut seen = HashSet::new();
        let (v, t) = new_turn(&h, la, &mut seen).await;
        assert_eq!(
            (v.author.as_str(), t.mode, t.owner.as_str()),
            ("alice", RoomTurnMode::Session, OWNER)
        );
        assert!(v.text.starts_with(ROOM_TURN_PREAMBLE), "{}", v.text);
        assert!(
            v.text.contains("bob: plain message") && v.text.contains("alice: @m0x6A-agent why b?"),
            "{}",
            v.text
        );
        assert!(
            !v.text.contains("before anyone"),
            "nothing from before the call-in: {}",
            v.text
        );

        // The plan reaches the branch: building, so the next turn runs in a fork, and carries
        // only what came after the last turn.
        plan_on(&h, 7, BRANCH, "web-server").await;
        tick(&h);
        say(&h, "bob", 7, BRANCH, "@m0x6a-agent and now?")
            .await
            .unwrap();
        let (v, t) = new_turn(&h, la, &mut seen).await;
        assert_eq!(t.mode, RoomTurnMode::Fork);
        assert!(v.text.contains("bob: @m0x6a-agent and now?"), "{}", v.text);
        assert!(
            !v.text.contains("plain message"),
            "only since the last turn: {}",
            v.text
        );
    }

    fn hook(launch: Uuid, name: &str, prompt_id: &str, field: (&str, &str)) -> HookEvent {
        let mut p = serde_json::json!({"session_id": "s1", "hook_event_name": name, "cwd": "/w", "prompt_id": prompt_id});
        p[field.0] = serde_json::json!(field.1);
        let mut ev = HookEvent::from_payload(p, Utc::now()).unwrap();
        ev.launch_id = Some(launch);
        ev
    }

    /// While drafting, the answer is the Stop of the prompt that carried the turn.
    #[tokio::test]
    async fn a_session_turn_is_answered_by_the_stop_of_its_prompt() {
        let h = harness().await;
        setup(&h).await;
        draft_on(&h, 7, BRANCH, "web-server").await;
        let ca = Uuid::from_u128(0xA);
        let la = launched(&h, OWNER, ca, 7, BRANCH, true).await;
        let (_, CallInResponse::Joined { .. }) = call(&h, OWNER, 7, la).await.unwrap() else {
            panic!()
        };
        tick(&h);
        say(&h, "alice", 7, BRANCH, "@m0x6A-agent why b?")
            .await
            .unwrap();
        let (v, t) = new_turn(&h, la, &mut HashSet::new()).await;
        let mut prompt = hook(la, "UserPromptSubmit", "p1", ("prompt", &v.text));
        prompt.message_id = Some(v.id);
        crate::session_log::on_hook(&h.app, ca, &prompt).await;
        let stop = hook(
            la,
            "Stop",
            "p1",
            ("last_assistant_message", "b keeps the table in one place."),
        );
        // From another companion it is not the agent's answer.
        on_hook(&h.app, Uuid::from_u128(0xBAD), &stop).await;
        assert!(
            view(&h, 7)
                .await
                .messages
                .iter()
                .all(|m| m.author_kind != RoomAuthorKind::Agent)
        );
        on_hook(&h.app, ca, &stop).await;
        let agent: Vec<_> = view(&h, 7)
            .await
            .messages
            .into_iter()
            .filter(|m| m.author_kind == RoomAuthorKind::Agent)
            .collect();
        assert_eq!(agent.len(), 1, "{agent:?}");
        assert_eq!(
            (
                agent[0].author.as_str(),
                agent[0].text.as_str(),
                agent[0].turn_id
            ),
            (
                "m0x6A-agent",
                "b keeps the table in one place.",
                Some(t.turn_id)
            )
        );
    }

    /// While building, the answer is the fork's RoomReply, only from the turn's own companion:
    /// posted, its fork kept for the next turn, its cost counted on the issue; an error is a
    /// system message, not the agent's words.
    #[tokio::test]
    async fn a_fork_turn_is_answered_by_its_reply_with_its_cost() {
        let h = harness().await;
        setup(&h).await;
        plan_on(&h, 7, BRANCH, "web-server").await;
        let ca = Uuid::from_u128(0xA);
        let la = launched(&h, OWNER, ca, 7, BRANCH, true).await;
        let (_, CallInResponse::Joined { .. }) = call(&h, OWNER, 7, la).await.unwrap() else {
            panic!()
        };
        tick(&h);
        say(&h, "alice", 7, BRANCH, "@m0x6A-agent why b?")
            .await
            .unwrap();
        let mut seen = HashSet::new();
        let (_, t) = new_turn(&h, la, &mut seen).await;
        assert_eq!(t.mode, RoomTurnMode::Fork);
        assert!(
            !on_room_reply(
                &h.app,
                Uuid::from_u128(0xBAD),
                &reply(&t, "not mine", Some("f0"), Some(1.0))
            )
            .await,
            "another companion's reply is refused"
        );
        assert!(
            on_room_reply(
                &h.app,
                ca,
                &reply(&t, "b is simpler.", Some("f1"), Some(0.25))
            )
            .await
        );
        let agent: Vec<_> = view(&h, 7)
            .await
            .messages
            .into_iter()
            .filter(|m| m.author_kind == RoomAuthorKind::Agent)
            .collect();
        assert_eq!(
            agent.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["b is simpler."]
        );
        let usd: f64 =
            sqlx::query_scalar("SELECT usd FROM spend_sessions WHERE session_id = 'fork:f1'")
                .fetch_one(&h.app.db)
                .await
                .unwrap();
        assert_eq!(usd, 0.25, "the fork's cost counts on the issue");
        // The next turn resumes that fork.
        tick(&h);
        say(&h, "alice", 7, BRANCH, "@m0x6A-agent and c?")
            .await
            .unwrap();
        let (v, t2) = new_turn(&h, la, &mut seen).await;
        let kind = session_messages::room_turn(&h.app.db, v.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kind.mode, RoomTurnMode::Fork);
        let body: String = sqlx::query_scalar("SELECT body FROM session_messages WHERE id = ?")
            .bind(v.id.to_string())
            .fetch_one(&h.app.db)
            .await
            .unwrap();
        assert!(body.contains("\"fork_session_id\":\"f1\""), "{body}");
        // A failed turn is Rubato's message, not the agent's.
        let mut failed = reply(&t2, "", None, None);
        failed.error = Some("spend cap reached".into());
        assert!(on_room_reply(&h.app, ca, &failed).await);
        let last = view(&h, 7).await.messages.into_iter().last().unwrap();
        assert_eq!(last.author_kind, RoomAuthorKind::System);
        assert!(
            last.text.contains("could not answer: spend cap reached"),
            "{}",
            last.text
        );
    }

    /// Agents answering each other make turns, up to room_agent_turn_limit (3) with no person
    /// posting; then Rubato stops them, says so once, and a person's message starts them again.
    #[tokio::test]
    async fn agent_made_turns_stop_at_the_limit_until_a_person_posts() {
        let h = harness().await;
        setup(&h).await;
        plan_on(&h, 7, BRANCH, "web-server").await;
        let (ca, cb) = (Uuid::from_u128(0xA), Uuid::from_u128(0xB));
        let la = launched(&h, OWNER, ca, 7, BRANCH, true).await;
        let lb = launched(&h, "alice", cb, 7, BRANCH, true).await;
        h.app.set_capabilities(
            cb,
            vec![
                capability::SESSION_INPUT.into(),
                capability::ROOM_TURNS.into(),
            ],
        );
        let (_, CallInResponse::Joined { .. }) = call(&h, OWNER, 7, la).await.unwrap() else {
            panic!()
        };
        let (_, CallInResponse::Joined { .. }) = call(&h, "alice", 7, lb).await.unwrap() else {
            panic!()
        };
        tick(&h);
        say(&h, "bob", 7, BRANCH, "@m0x6A-agent start")
            .await
            .unwrap();
        let (mut sa, mut sb) = (HashSet::new(), HashSet::new());
        let (_, t) = new_turn(&h, la, &mut sa).await;
        // Three agent-made turns: A → B, B → A, A → B.
        tick(&h);
        assert!(
            on_room_reply(
                &h.app,
                ca,
                &reply(&t, "@alice-agent your view?", Some("fa"), None)
            )
            .await
        );
        let (_, t) = new_turn(&h, lb, &mut sb).await;
        tick(&h);
        assert!(
            on_room_reply(
                &h.app,
                cb,
                &reply(&t, "@m0x6A-agent back to you", Some("fb"), None)
            )
            .await
        );
        let (_, t) = new_turn(&h, la, &mut sa).await;
        tick(&h);
        assert!(
            on_room_reply(
                &h.app,
                ca,
                &reply(&t, "@alice-agent and?", Some("fa"), None)
            )
            .await
        );
        let (_, t) = new_turn(&h, lb, &mut sb).await;
        // The fourth is stopped: no turn for A, one notice, one limit hit.
        tick(&h);
        assert!(
            on_room_reply(
                &h.app,
                cb,
                &reply(&t, "@m0x6A-agent more", Some("fb"), None)
            )
            .await
        );
        assert_eq!(turns_of(&h, la).await.len(), 2, "no third turn for A");
        let stops = |m: &[RoomMessage]| {
            m.iter()
                .filter(|m| m.text.starts_with(TURN_LIMIT_TEXT))
                .count()
        };
        assert_eq!(stops(&view(&h, 7).await.messages), 1);
        let hits: u64 = h
            .reader
            .points(metric::ROOMS_TURN_LIMIT_HITS)
            .iter()
            .map(|p| p.value)
            .sum();
        assert_eq!(hits, 1);
        // A person posts: turns start again.
        tick(&h);
        say(&h, "carol", 7, BRANCH, "@m0x6A-agent hello")
            .await
            .unwrap();
        new_turn(&h, la, &mut sa).await;
    }
}
