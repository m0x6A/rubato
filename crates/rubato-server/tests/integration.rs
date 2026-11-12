//! The server end to end: pairing, a companion replaying protocol fixtures over the websocket,
//! pins, GitHub rows from a fake, and the derived progress read back. Also the check that no
//! table stores progress.

use futures_util::{SinkExt, StreamExt};
use rubato_protocol::launch::routes as launch_routes;
use rubato_protocol::layout::{MapLayout, Position, routes as layout_routes};
use rubato_protocol::map::routes as map_routes;
use rubato_protocol::map_draft::routes as draft_routes;
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes as pr,
};
use rubato_protocol::pins::routes as pin_routes;
use rubato_protocol::progress::routes as overview_routes;
use rubato_protocol::queue::routes as queue_routes;
use rubato_protocol::web_link::routes as web_routes;
use rubato_protocol::{
    ChecksStatus, CompanionEnvelope, CompanionMessage, Finding, IssueRef, MapRef, Overview,
    PairApproveRequest, PairingInfo, PinRequest, ProgressState, ServerEnvelope, ServerMessage,
};
use rubato_server::kernel::config::Config;
use rubato_server::kernel::db;
use rubato_server::kernel::github::{Comment, FakeGitHub, GitHubApi, IssueInfo, PullInfo};
use rubato_server::kernel::state::{App, AppState};
use rubato_server::{app, map, tasks};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

struct Harness {
    url: String,
    app: App,
    github: Arc<FakeGitHub>,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn harness() -> Harness {
    harness_with(|_| {}).await
}

async fn harness_with(configure: impl FnOnce(&mut Config)) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/test.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let github = Arc::new(FakeGitHub::new());
    let map_yaml = std::fs::read_to_string(repo_root().join("docs/design/rubato-map.yml")).unwrap();
    github.with(|g| {
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/design/rubato-map.yml".into(),
                "main".into(),
            ),
            map_yaml,
        );
        g.branches.insert(
            "m0x6A/rubato".into(),
            vec![("main".into(), "mapsha1".into())],
        );
    });
    let mut cfg = Config::for_tests(db_url, "m0x6A");
    configure(&mut cfg);
    let app = AppState::new(pool, cfg, github.clone());
    map::reload(&app).await.unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Harness {
        url: format!("http://{addr}"),
        app,
        github,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

impl Harness {
    async fn pair(&self, as_user: &str) -> (Uuid, String) {
        let companion_id = Uuid::new_v4();
        let start: PairStartResponse = self
            .http
            .post(format!("{}{}", self.url, pr::PAIR_START))
            .json(&PairStartRequest {
                companion_id,
                hostname: "h".into(),
                companion_version: "t".into(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let pending: PairPollResponse = self
            .http
            .post(format!("{}{}", self.url, pr::PAIR_POLL))
            .json(&PairPollRequest {
                device_code: start.device_code.clone(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(pending, PairPollResponse::Pending);
        let approve = self
            .http
            .post(format!("{}{}", self.url, pr::PAIR_APPROVE))
            .header("x-rubato-dev-user", as_user)
            .json(&PairApproveRequest {
                user_code: start.user_code.to_lowercase(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(approve.status(), 200, "approve as the human");
        let info: PairingInfo = approve.json().await.unwrap();
        assert_eq!(info.user, as_user);
        let approved: PairPollResponse = self
            .http
            .post(format!("{}{}", self.url, pr::PAIR_POLL))
            .json(&PairPollRequest {
                device_code: start.device_code.clone(),
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let PairPollResponse::Approved { token, user } = approved else {
            panic!("{approved:?}")
        };
        assert_eq!(user, as_user);
        // A second poll must not hand the token out again.
        let again: PairPollResponse = self
            .http
            .post(format!("{}{}", self.url, pr::PAIR_POLL))
            .json(&PairPollRequest {
                device_code: start.device_code,
            })
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(again, PairPollResponse::Denied);
        (companion_id, token)
    }

    async fn connect(
        &self,
        token: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut req = format!(
            "ws://{}{}",
            self.url.trim_start_matches("http://"),
            pr::COMPANION_WS
        )
        .into_client_request()
        .unwrap();
        req.headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .expect("websocket upgrade");
        ws
    }

    async fn overview(&self, as_user: &str) -> Overview {
        self.http
            .get(format!("{}{}", self.url, overview_routes::OVERVIEW))
            .header("x-rubato-dev-user", as_user)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    fn fixture(name: &str) -> CompanionEnvelope {
        let p = repo_root()
            .join("crates/rubato-protocol/fixtures")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }
}

async fn send(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    env: &CompanionEnvelope,
) -> ServerMessage {
    ws.send(Message::Text(serde_json::to_string(env).unwrap().into()))
        .await
        .unwrap();
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!("connection closed")
    };
    let env: ServerEnvelope = serde_json::from_str(&t).unwrap();
    env.message
}

fn with_companion(mut env: CompanionEnvelope, companion_id: Uuid) -> CompanionEnvelope {
    match &mut env.message {
        CompanionMessage::Hello {
            companion_id: c, ..
        } => *c = companion_id,
        CompanionMessage::Heartbeat(h) => h.companion_id = companion_id,
        // Recorded fixtures carry a fixed last_seen; the trust check would render them unknown.
        CompanionMessage::Sessions { sessions } => sessions.iter_mut().for_each(|s| {
            s.companion_id = companion_id;
            s.last_seen = chrono::Utc::now();
        }),
        _ => {}
    }
    env
}

#[tokio::test]
async fn protocol_fixtures_replay_derives_progress() {
    let h = harness().await;
    let (companion_id, token) = h.pair("m0x6A").await;
    let mut ws = h.connect(&token).await;

    let welcome = send(
        &mut ws,
        &with_companion(Harness::fixture("companion_hello.json"), companion_id),
    )
    .await;
    assert!(
        matches!(
            welcome,
            ServerMessage::Welcome {
                protocol_version: 1,
                ..
            }
        ),
        "{welcome:?}"
    );
    assert!(matches!(
        send(
            &mut ws,
            &with_companion(Harness::fixture("companion_heartbeat.json"), companion_id)
        )
        .await,
        ServerMessage::Ack { seq: 2 }
    ));

    // Pin issue 1 to the protocol node, then replay the sessions snapshot: working, no plan → Designing.
    let issue = IssueRef::new("m0x6A/rubato", 1);
    let r = h
        .http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("rubato-protocol"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let bad = h
        .http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("rubato-orchestrator"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 422, "unknown ids are refused");
    assert!(matches!(
        send(
            &mut ws,
            &with_companion(Harness::fixture("companion_sessions.json"), companion_id)
        )
        .await,
        ServerMessage::Ack { .. }
    ));
    let ov = h.overview("m0x6A").await;
    let one = ov
        .issues
        .iter()
        .find(|i| i.issue == issue)
        .expect("issue 1 in overview");
    assert_eq!(
        one.progress.state,
        ProgressState::Designing,
        "{:?}",
        one.progress
    );
    assert_eq!(one.pins, vec![MapRef::node("rubato-protocol")]);
    assert_eq!(
        ov.unpinned_sessions.len(),
        1,
        "the scraped goldilocks pane has no issue"
    );
    assert!(
        ov.companions
            .iter()
            .any(|c| c.companion_id == companion_id && c.connected)
    );

    // A plan on the branch → Building (session still working).
    let plan_md = std::fs::read_to_string(repo_root().join("docs/plans/1.md")).unwrap();
    h.github.with(|g| {
        g.branches
            .get_mut("m0x6A/rubato")
            .unwrap()
            .push(("issue/1-protocol".into(), "sha-1".into()));
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/plans/1.md".into(),
                "issue/1-protocol".into(),
            ),
            plan_md,
        );
        g.issues.insert(
            ("m0x6A/rubato".into(), 1),
            IssueInfo {
                title: "Task 1: rubato-protocol".into(),
                state: "open".into(),
                url: "https://github.com/m0x6A/rubato/issues/1".into(),
            },
        );
        g.comments.insert(
            ("m0x6A/rubato".into(), 1),
            vec![Comment {
                id: 7,
                author: "m0x6A".into(),
                body: "Decision: Keep chrono\n\n## Context\nx\n\n## Chosen\nchrono\n\n## Why\nfine"
                    .into(),
                created_at: chrono::Utc::now(),
                url: "u".into(),
            }],
        );
    });
    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    let one = ov.issues.iter().find(|i| i.issue == issue).unwrap();
    assert_eq!(one.progress.state, ProgressState::Building);
    assert_eq!(one.title.as_deref(), Some("Task 1: rubato-protocol"));
    assert_eq!(one.decisions.len(), 1);
    assert_eq!(one.decisions[0].title, "Keep chrono");
    assert!(
        one.plan
            .as_ref()
            .is_some_and(|p| p.affected_parts.len() == 1)
    );

    // PR open with green checks → In review; merged → Done.
    h.github.with(|g| {
        g.pulls.insert(
            ("m0x6A/rubato".into(), "issue/1-protocol".into()),
            PullInfo {
                number: 8,
                open: true,
                draft: false,
                head_sha: "sha-1".into(),
                merged_at: None,
                updated_at: chrono::Utc::now(),
                url: "pr".into(),
            },
        );
        g.checks.insert("sha-1".into(), ChecksStatus::Green);
    });
    tasks::poll_once(&h.app).await.unwrap();
    assert_eq!(
        h.overview("m0x6A")
            .await
            .issues
            .iter()
            .find(|i| i.issue == issue)
            .unwrap()
            .progress
            .state,
        ProgressState::InReview
    );
    h.github.with(|g| {
        g.pulls
            .get_mut(&("m0x6A/rubato".into(), "issue/1-protocol".into()))
            .unwrap()
            .merged_at = Some(chrono::Utc::now());
    });
    tasks::poll_once(&h.app).await.unwrap();
    assert_eq!(
        h.overview("m0x6A")
            .await
            .issues
            .iter()
            .find(|i| i.issue == issue)
            .unwrap()
            .progress
            .state,
        ProgressState::Done
    );

    // A cap hit on another, unmerged issue blocks it.
    let two = IssueRef::new("m0x6A/rubato", 2);
    h.http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: two.clone(),
            target: MapRef::seam("plan-check-plan-file"),
        })
        .send()
        .await
        .unwrap();
    let mut cap = Harness::fixture("companion_cap_hit.json");
    if let CompanionMessage::CapHit(c) = &mut cap.message {
        c.issue = two.clone();
    }
    cap.seq = 9;
    assert!(matches!(
        send(&mut ws, &cap).await,
        ServerMessage::Ack { .. }
    ));
    assert_eq!(
        h.overview("m0x6A")
            .await
            .issues
            .iter()
            .find(|i| i.issue == two)
            .unwrap()
            .progress
            .state,
        ProgressState::Blocked
    );

    // Replaying an already-seen sequence is acknowledged and not recorded twice.
    let again = send(&mut ws, &cap).await;
    assert!(matches!(again, ServerMessage::Ack { seq: 9 }));
}

#[tokio::test]
async fn findings_show_unknown_ids_and_seam_collisions() {
    let h = harness().await;
    let plan_a = "---\nissue: m0x6A/rubato#21\n---\n## Affected parts\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | x | server-github |\n## O-rows\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| server-github | y | z | p | rubato-server |\n";
    let plan_b = plan_a.replace("#21", "#22").replace(
        "| rubato-server | affected |",
        "| rubato-mystery | affected |",
    );
    h.github.with(|g| {
        let b = g.branches.get_mut("m0x6A/rubato").unwrap();
        b.push(("issue/21-a".into(), "s21".into()));
        b.push(("issue/22-b".into(), "s22".into()));
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/plans/21.md".into(),
                "issue/21-a".into(),
            ),
            plan_a.to_owned(),
        );
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/plans/22.md".into(),
                "issue/22-b".into(),
            ),
            plan_b,
        );
    });
    for n in [21, 22] {
        h.http
            .post(format!("{}{}", h.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new("m0x6A/rubato", n),
                target: MapRef::node("rubato-server"),
            })
            .send()
            .await
            .unwrap();
    }
    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    assert!(ov.findings.iter().any(|f| matches!(f, Finding::SeamCollision { seam, issues } if seam == "server-github" && issues.len() == 2)), "{:?}", ov.findings);
    assert!(ov.findings.iter().any(|f| matches!(f, Finding::UnknownIds { issue, ids } if issue.number == 22 && ids == &vec!["rubato-mystery".to_string()])), "{:?}", ov.findings);
    assert!(
        ov.issues
            .iter()
            .all(|i| i.progress.state == ProgressState::Designed)
    );
}

#[tokio::test]
async fn derived_state_is_never_stored() {
    let h = harness().await;
    let cols = db::all_columns(&h.app.db).await.unwrap();
    assert!(cols.len() > 30, "schema has columns");
    let offenders: Vec<_> = cols
        .iter()
        .filter(|(t, c)| {
            c == "progress" || c == "progress_state" || t == "progress" || t == "issue_states"
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "progress must be derived, found {offenders:?}"
    );
    println!(
        "checked {} columns in {} tables",
        cols.len(),
        cols.iter()
            .map(|(t, _)| t)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
}

#[tokio::test]
async fn bad_and_revoked_tokens_are_refused() {
    let h = harness().await;
    let req = format!(
        "ws://{}{}",
        h.url.trim_start_matches("http://"),
        pr::COMPANION_WS
    )
    .into_client_request()
    .unwrap();
    let mut bad = req.clone();
    bad.headers_mut()
        .insert("authorization", "Bearer nope".parse().unwrap());
    let err = tokio_tungstenite::connect_async(bad)
        .await
        .expect_err("refused");
    assert!(err.to_string().contains("401"), "{err}");

    let (companion_id, token) = h.pair("m0x6A").await;
    let ws = h.connect(&token).await;
    drop(ws);
    let r = h
        .http
        .delete(format!("{}{}/{}", h.url, pr::PAIRINGS, companion_id))
        .header("x-rubato-dev-user", "alice")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "alice cannot revoke m0x6A's companion");
    let r = h
        .http
        .delete(format!("{}{}/{}", h.url, pr::PAIRINGS, companion_id))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let mut revoked = req;
    revoked
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    assert!(tokio_tungstenite::connect_async(revoked).await.is_err());
}

#[tokio::test]
async fn launch_goes_only_to_the_callers_own_connected_companion() {
    let h = harness().await;
    let (companion_id, token) = h.pair("m0x6A").await;
    let mut ws = h.connect(&token).await;
    let _ = send(
        &mut ws,
        &with_companion(Harness::fixture("companion_hello.json"), companion_id),
    )
    .await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    h.http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("skill-rubato"),
        })
        .send()
        .await
        .unwrap();
    let body = serde_json::json!({ "issue": issue, "slug": "skill" });

    // Alice has no companion: nothing to launch into.
    let r = h
        .http
        .post(format!("{}{}", h.url, launch_routes::LAUNCH))
        .header("x-rubato-dev-user", "alice")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "{}", r.text().await.unwrap());
    // Alice naming m0x6A's companion is forbidden.
    let r = h
        .http
        .post(format!("{}{}", h.url, launch_routes::LAUNCH))
        .header("x-rubato-dev-user", "alice")
        .json(&serde_json::json!({ "issue": issue, "companion_id": companion_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // The owner launches; the command arrives on the websocket with the right branch and attributes.
    let r = h
        .http
        .post(format!("{}{}", h.url, launch_routes::LAUNCH))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!("no launch on the socket")
    };
    let env: ServerEnvelope = serde_json::from_str(&t).unwrap();
    let ServerMessage::Launch(cmd) = env.message else {
        panic!("{:?}", env.message)
    };
    assert_eq!(cmd.user, "m0x6A");
    assert_eq!(cmd.branch, "issue/3-skill");
    assert_eq!(cmd.node_ids, vec!["skill-rubato".to_string()]);
    assert_eq!(
        cmd.otel_resource.get("rubato.issue").map(String::as_str),
        Some("m0x6A/rubato#3")
    );
    assert_eq!(cmd.attempt, 1);
    assert!(cmd.prompt.contains("docs/plans/3.md"), "{}", cmd.prompt);
    // Progress: launched, no session yet → Started.
    let ov = h.overview("m0x6A").await;
    // The launch result has not arrived, so it does not count as started yet; the pin alone is Planned.
    assert_eq!(
        ov.issues
            .iter()
            .find(|i| i.issue == issue)
            .unwrap()
            .progress
            .state,
        ProgressState::Planned
    );
}

#[tokio::test]
async fn unauthenticated_requests_without_dev_user_are_401() {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let mut cfg = Config::for_tests(db_url, "x");
    cfg.dev_user = None;
    let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()) as Arc<dyn GitHubApi>);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app::router(app)).await.unwrap() });
    let r = reqwest::get(format!("http://{addr}{}", overview_routes::OVERVIEW))
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = reqwest::get(format!("http://{addr}/api/health"))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

// ----- guards for the vertical-slice move (issue #20) ----------------------------------------

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Harness {
    async fn web_ws(&self) -> Ws {
        let req = format!(
            "ws://{}{}",
            self.url.trim_start_matches("http://"),
            web_routes::WEB_WS
        )
        .into_client_request()
        .unwrap();
        tokio_tungstenite::connect_async(req)
            .await
            .expect("web websocket")
            .0
    }
}

/// Wait for the next `WebEvent::Changed` with this reason; other reasons are skipped.
async fn await_reason(web: &mut Ws, reason: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let next = tokio::time::timeout_at(deadline, web.next())
            .await
            .unwrap_or_else(|_| panic!("no web event {reason:?} within 5 s"));
        let Some(Ok(Message::Text(t))) = next else {
            panic!("web socket closed waiting for {reason:?}")
        };
        let ev: rubato_protocol::WebEvent = serde_json::from_str(&t).unwrap();
        let rubato_protocol::WebEvent::Changed { reason: got, .. } = ev;
        if got == reason {
            return;
        }
    }
}

#[tokio::test]
async fn web_events_follow_every_action() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let mut checked = 0;

    // pairing approve (pairing.rs)
    let (companion_id, token) = h.pair("m0x6A").await;
    await_reason(&mut web, "pairing").await;
    checked += 1;
    // companion connected (ws_companion.rs)
    let mut ws = h.connect(&token).await;
    await_reason(&mut web, "companion connected").await;
    checked += 1;
    let _ = send(
        &mut ws,
        &with_companion(Harness::fixture("companion_hello.json"), companion_id),
    )
    .await;
    // sessions
    let _ = send(
        &mut ws,
        &with_companion(Harness::fixture("companion_sessions.json"), companion_id),
    )
    .await;
    await_reason(&mut web, "sessions").await;
    checked += 1;
    // pin (api.rs add_pin)
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let pin = PinRequest {
        issue: issue.clone(),
        target: MapRef::node("skill-rubato"),
    };
    let r = h
        .http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&pin)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    await_reason(&mut web, "pin").await;
    checked += 1;
    // launch from the handler (api.rs launch)
    let r = h
        .http
        .post(format!("{}{}", h.url, launch_routes::LAUNCH))
        .json(&serde_json::json!({ "issue": issue, "slug": "skill" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    await_reason(&mut web, "launch").await;
    checked += 1;
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!("no launch command")
    };
    let ServerMessage::Launch(cmd) = serde_json::from_str::<ServerEnvelope>(&t).unwrap().message
    else {
        panic!("expected a launch command")
    };
    // launch result from the companion (ws_companion.rs)
    let started = CompanionEnvelope {
        seq: 20,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Launch(rubato_protocol::LaunchResult::Started {
            command_id: cmd.id,
            session_id: cmd.session_id,
            worktree: "/w".into(),
            tmux_session: "t".into(),
            at: chrono::Utc::now(),
        }),
    };
    let _ = send(&mut ws, &started).await;
    await_reason(&mut web, "launch").await;
    checked += 1;
    // cap hit from the handler: a launch past the attempt limit (launch/handler.rs)
    let r = h
        .http
        .post(format!("{}{}", h.url, launch_routes::LAUNCH))
        .json(&serde_json::json!({ "issue": issue, "slug": "skill", "attempt_limit": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "{}", r.text().await.unwrap());
    await_reason(&mut web, "cap hit").await;
    checked += 1;
    // cap hit from the companion
    let mut cap = Harness::fixture("companion_cap_hit.json");
    cap.seq = 21;
    let _ = send(&mut ws, &cap).await;
    await_reason(&mut web, "cap hit").await;
    checked += 1;
    // stale
    let stale = CompanionEnvelope {
        seq: 22,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Stale(rubato_protocol::StaleSession {
            companion_id,
            session_id: Some("s".into()),
            issue: None,
            last_seen: chrono::Utc::now(),
            silent_for_secs: 900,
            at: chrono::Utc::now(),
        }),
    };
    let _ = send(&mut ws, &stale).await;
    await_reason(&mut web, "stale").await;
    checked += 1;
    // map reload (api.rs map_reload)
    let r = h
        .http
        .post(format!("{}{}", h.url, map_routes::MAP_RELOAD))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    await_reason(&mut web, "map").await;
    checked += 1;
    // unpin (api.rs remove_pin)
    let r = h
        .http
        .delete(format!("{}{}", h.url, pin_routes::PINS))
        .json(&pin)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    await_reason(&mut web, "pin").await;
    checked += 1;
    // poll (tasks.rs)
    tasks::poll_once(&h.app).await.unwrap();
    await_reason(&mut web, "poll").await;
    checked += 1;
    // companion disconnected
    ws.close(None).await.unwrap();
    await_reason(&mut web, "companion disconnected").await;
    checked += 1;
    // pairing revoke (pairing.rs)
    let r = h
        .http
        .delete(format!("{}{}/{}", h.url, pr::PAIRINGS, companion_id))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    await_reason(&mut web, "pairing").await;
    checked += 1;

    assert_eq!(checked, 14);
    println!("checked {checked} notify sites");
}

/// A paired, connected companion and an issue pinned to the map: what a launch needs.
async fn ready_to_launch(h: &Harness, issue: &IssueRef) -> (Uuid, Ws) {
    let (companion_id, token) = h.pair("m0x6A").await;
    let mut ws = h.connect(&token).await;
    let _ = send(
        &mut ws,
        &with_companion(Harness::fixture("companion_hello.json"), companion_id),
    )
    .await;
    let r = h
        .http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("skill-rubato"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    (companion_id, ws)
}

async fn start(h: &Harness, issue: &IssueRef, attempt_limit: u32) -> reqwest::Response {
    h.http
        .post(format!("{}{}", h.url, launch_routes::LAUNCH))
        .json(
            &serde_json::json!({ "issue": issue, "slug": "skill", "attempt_limit": attempt_limit }),
        )
        .send()
        .await
        .unwrap()
}

async fn progress_of(h: &Harness, issue: &IssueRef) -> rubato_protocol::Progress {
    let ov = h.overview("m0x6A").await;
    ov.issues
        .into_iter()
        .find(|i| &i.issue == issue)
        .expect("the issue is in the overview")
        .progress
}

#[tokio::test]
async fn attempt_limit_blocks_the_issue_and_is_an_event() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let (_companion_id, _ws) = ready_to_launch(&h, &issue).await;

    // The one attempt the limit allows.
    let r = start(&h, &issue, 1).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    await_reason(&mut web, "launch").await;
    assert_eq!(progress_of(&h, &issue).await.state, ProgressState::Planned);

    // Past the limit: still a 409 with its message, and now an event.
    let r = start(&h, &issue, 1).await;
    assert_eq!(r.status(), 409);
    assert_eq!(
        r.text().await.unwrap(),
        "attempt limit 1 reached for m0x6A/rubato#3"
    );
    await_reason(&mut web, "cap hit").await;
    let p = progress_of(&h, &issue).await;
    assert_eq!(p.state, ProgressState::Blocked);
    assert_eq!(p.reason.as_deref(), Some("cap hit"));

    // A higher limit lets the next launch through, and the older cap hit no longer blocks.
    let r = start(&h, &issue, 5).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    await_reason(&mut web, "launch").await;
    assert_eq!(progress_of(&h, &issue).await.state, ProgressState::Planned);
}

#[tokio::test]
async fn attempt_limit_does_not_block_a_running_issue() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let (companion_id, mut ws) = ready_to_launch(&h, &issue).await;

    let r = start(&h, &issue, 1).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    // The companion reports a working session for the issue.
    let sessions = CompanionEnvelope {
        seq: 2,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Sessions {
            sessions: vec![rubato_protocol::SessionState {
                session_id: Some("s-3".into()),
                companion_id,
                user: "m0x6A".into(),
                tmux: None,
                cwd: "/w".into(),
                worktree: Some("/w".into()),
                branch: Some("issue/3-skill".into()),
                repo: Some(issue.repo.clone()),
                issue: Some(issue.clone()),
                status: rubato_protocol::SessionStatus::Working,
                source: rubato_protocol::StateSource::Hook,
                last_seen: chrono::Utc::now(),
                spend_usd: None,
            }],
        },
    };
    let _ = send(&mut ws, &sessions).await;
    await_reason(&mut web, "sessions").await;
    assert_eq!(
        progress_of(&h, &issue).await.state,
        ProgressState::Designing
    );

    // Someone presses Start again during the run: refused, an event, and not Blocked.
    let r = start(&h, &issue, 1).await;
    assert_eq!(r.status(), 409, "{}", r.text().await.unwrap());
    await_reason(&mut web, "cap hit").await;
    assert_eq!(
        progress_of(&h, &issue).await.state,
        ProgressState::Designing
    );
}

/// A spend cap hit from the companion: kind `spend`, the given session and companion time.
fn spend_hit(
    issue: &IssueRef,
    seq: u64,
    session_id: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> CompanionEnvelope {
    let mut cap = Harness::fixture("companion_cap_hit.json");
    if let CompanionMessage::CapHit(c) = &mut cap.message {
        c.issue = issue.clone();
        c.session_id = Some(session_id.into());
        c.at = at;
    }
    cap.seq = seq;
    cap
}

/// The companion's `Launch` result for a launch it started.
fn started(seq: u64, launched: &rubato_protocol::LaunchResponse) -> CompanionEnvelope {
    CompanionEnvelope {
        seq,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Launch(rubato_protocol::LaunchResult::Started {
            command_id: launched.command.id,
            session_id: launched.command.session_id,
            worktree: "/w".into(),
            tmux_session: "t".into(),
            at: chrono::Utc::now(),
        }),
    }
}

#[tokio::test]
async fn a_spend_hit_blocks_whatever_the_companion_clock_says() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let (_companion_id, mut ws) = ready_to_launch(&h, &issue).await;

    // Two runs, one after the other, each started by the companion.
    let mut runs = Vec::new();
    for seq in [2, 3] {
        let launched: rubato_protocol::LaunchResponse =
            start(&h, &issue, 3).await.json().await.unwrap();
        await_reason(&mut web, "launch").await;
        let _ = send(&mut ws, &started(seq, &launched)).await;
        await_reason(&mut web, "launch").await;
        runs.push(launched.command.session_id.to_string());
    }

    // A hit from the first run, stamped now: the second run superseded it, so it does not block.
    let _ = send(&mut ws, &spend_hit(&issue, 4, &runs[0], chrono::Utc::now())).await;
    await_reason(&mut web, "cap hit").await;
    assert_ne!(progress_of(&h, &issue).await.state, ProgressState::Blocked);

    // The second run's hit, from a companion whose clock is an hour behind: it blocks.
    let behind = chrono::Utc::now() - chrono::Duration::hours(1);
    let _ = send(&mut ws, &spend_hit(&issue, 5, &runs[1], behind)).await;
    await_reason(&mut web, "cap hit").await;
    let p = progress_of(&h, &issue).await;
    assert_eq!(p.state, ProgressState::Blocked);
    assert_eq!(p.reason.as_deref(), Some("cap hit"));
}

#[tokio::test]
async fn a_spend_hit_blocks_after_a_second_start_was_refused_as_already_running() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let (_companion_id, mut ws) = ready_to_launch(&h, &issue).await;
    let result = |seq: u64, result: rubato_protocol::LaunchResult| CompanionEnvelope {
        seq,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Launch(result),
    };

    let first: rubato_protocol::LaunchResponse = start(&h, &issue, 3).await.json().await.unwrap();
    await_reason(&mut web, "launch").await;
    let _ = send(&mut ws, &started(2, &first)).await;
    await_reason(&mut web, "launch").await;

    // Someone presses Start again during the run; the companion refuses it.
    let second: rubato_protocol::LaunchResponse = start(&h, &issue, 3).await.json().await.unwrap();
    await_reason(&mut web, "launch").await;
    let _ = send(
        &mut ws,
        &result(
            3,
            rubato_protocol::LaunchResult::Refused {
                command_id: second.command.id,
                reason: rubato_protocol::RefusalReason::AlreadyRunning,
                detail: "running".into(),
                at: chrono::Utc::now(),
            },
        ),
    )
    .await;
    await_reason(&mut web, "launch").await;

    // The first run passes its spend cap: the issue is Blocked.
    let session = first.command.session_id.to_string();
    let _ = send(&mut ws, &spend_hit(&issue, 4, &session, chrono::Utc::now())).await;
    await_reason(&mut web, "cap hit").await;
    assert_eq!(progress_of(&h, &issue).await.state, ProgressState::Blocked);
}

#[tokio::test]
async fn a_companion_attempts_hit_blocks_whatever_the_companion_clock_says() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let (_companion_id, mut ws) = ready_to_launch(&h, &issue).await;

    let r = start(&h, &issue, 3).await;
    assert_eq!(r.status(), 200);
    await_reason(&mut web, "launch").await;

    // The companion refuses the launch at its own limit check, with a clock an hour behind.
    let mut cap = spend_hit(
        &issue,
        2,
        "",
        chrono::Utc::now() - chrono::Duration::hours(1),
    );
    if let CompanionMessage::CapHit(c) = &mut cap.message {
        c.kind = rubato_protocol::CapKind::Attempts;
        c.session_id = None;
    }
    let _ = send(&mut ws, &cap).await;
    await_reason(&mut web, "cap hit").await;
    assert_eq!(progress_of(&h, &issue).await.state, ProgressState::Blocked);
}

#[tokio::test]
async fn welcome_carries_last_seen_seq_after_reconnect() {
    let h = harness().await;
    let (companion_id, token) = h.pair("m0x6A").await;
    let mut ws = h.connect(&token).await;
    let hello = with_companion(Harness::fixture("companion_hello.json"), companion_id);
    let ServerMessage::Welcome {
        last_seen_companion_seq,
        ..
    } = send(&mut ws, &hello).await
    else {
        panic!("no welcome")
    };
    assert_eq!(last_seen_companion_seq, 0, "a new pairing has seen nothing");
    let mut beat = with_companion(Harness::fixture("companion_heartbeat.json"), companion_id);
    beat.seq = 7;
    assert!(matches!(
        send(&mut ws, &beat).await,
        ServerMessage::Ack { seq: 7 }
    ));
    ws.close(None).await.unwrap();
    drop(ws);

    let mut ws = h.connect(&token).await;
    let mut hello = hello;
    hello.seq = 8;
    let ServerMessage::Welcome {
        last_seen_companion_seq,
        ..
    } = send(&mut ws, &hello).await
    else {
        panic!("no welcome after reconnect")
    };
    assert_eq!(last_seen_companion_seq, 7);
}

/// Plan 29 S5, N1: a hello at a sequence number the server has already recorded is a collision.
/// It is acknowledged, not welcomed and not processed; a hello above the record is welcomed.
#[tokio::test]
async fn a_replayed_hello_is_counted_and_a_higher_one_is_welcomed() {
    let h = harness().await;
    let (companion_id, token) = h.pair("m0x6A").await;
    let mut ws = h.connect(&token).await;
    let hello = with_companion(Harness::fixture("companion_hello.json"), companion_id);
    assert_eq!(hello.seq, 1);
    assert!(matches!(
        send(&mut ws, &hello).await,
        ServerMessage::Welcome { .. }
    ));
    let beat = with_companion(Harness::fixture("companion_heartbeat.json"), companion_id);
    assert!(matches!(
        send(&mut ws, &beat).await,
        ServerMessage::Ack { seq: 2 }
    ));
    ws.close(None).await.unwrap();
    drop(ws);

    // A companion that restarted its count: the same hello again.
    let mut ws = h.connect(&token).await;
    let replayed = send(&mut ws, &hello).await;
    assert!(
        matches!(replayed, ServerMessage::Ack { seq: 1 }),
        "a replayed hello is acknowledged, not welcomed: {replayed:?}"
    );
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM companion_events")
        .fetch_one(&h.app.db)
        .await
        .unwrap();
    assert_eq!(events, 2, "the replay is not recorded twice");

    let mut higher = hello;
    higher.seq = 3;
    let ServerMessage::Welcome {
        last_seen_companion_seq,
        ..
    } = send(&mut ws, &higher).await
    else {
        panic!("a hello above the record is welcomed")
    };
    assert_eq!(last_seen_companion_seq, 2);
}

fn names(calls: &[(&'static str, String)]) -> Vec<&'static str> {
    calls.iter().map(|(m, _)| *m).collect()
}

/// Wait until the fake has seen `rate_limit`, which is the last GitHub call of a poll tick.
async fn until_tick_done(github: &FakeGitHub) -> Vec<(&'static str, String)> {
    for _ in 0..200 {
        let calls = github.calls();
        if calls.iter().any(|(m, _)| *m == "rate_limit") {
            return calls;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("no poll tick within 5 s: {:?}", github.calls());
}

#[tokio::test]
async fn first_tick_reloads_then_polls() {
    // Empty database: the first tick fetches the map, then polls.
    let h = harness().await;
    h.github.with(|g| g.calls.clear());
    *h.app.map.write().await = None;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(tasks::run(h.app.clone(), rx));
    let calls = until_tick_done(&h.github).await;
    assert_eq!(
        names(&calls)[..2],
        ["branch_sha", "file"],
        "map first: {calls:?}"
    );
    assert_eq!(names(&calls).last(), Some(&"rate_limit"));
    tx.send(true).unwrap();
    task.await.unwrap();

    // Warm database (a map stored seconds ago): a restart still reloads on its first tick.
    let warm = AppState::new(
        h.app.db.clone(),
        h.app.config.as_ref().clone(),
        h.github.clone(),
    );
    map::warm(&warm).await.unwrap();
    assert!(
        warm.map.read().await.is_some(),
        "warm_map loads the stored map"
    );
    h.github.with(|g| g.calls.clear());
    let (tx, rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(tasks::run(warm, rx));
    let calls = until_tick_done(&h.github).await;
    assert_eq!(
        names(&calls)[..2],
        ["branch_sha", "file"],
        "reload on the first tick after a restart: {calls:?}"
    );
    tx.send(true).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn poll_makes_the_same_github_calls_per_issue() {
    let h = harness().await;
    let good = "---\nissue: m0x6A/rubato#31\n---\n## Affected parts\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | x | server-github |\n## O-rows\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| server-github | y | z | p | rubato-server |\n";
    h.github.with(|g| {
        let b = g.branches.get_mut("m0x6A/rubato").unwrap();
        b.push(("issue/31-ok".into(), "s31".into()));
        b.push(("issue/32-bad".into(), "s32".into()));
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/plans/31.md".into(),
                "issue/31-ok".into(),
            ),
            good.into(),
        );
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/plans/32.md".into(),
                "issue/32-bad".into(),
            ),
            "---\nissue: not an issue\n---\n".into(),
        );
        g.pulls.insert(
            ("m0x6A/rubato".into(), "issue/31-ok".into()),
            PullInfo {
                number: 41,
                open: true,
                draft: false,
                head_sha: "s31".into(),
                merged_at: None,
                updated_at: chrono::Utc::now(),
                url: "pr41".into(),
            },
        );
        g.pulls.insert(
            ("m0x6A/rubato".into(), "issue/32-bad".into()),
            PullInfo {
                number: 42,
                open: false,
                draft: false,
                head_sha: "s32".into(),
                merged_at: Some(chrono::Utc::now()),
                updated_at: chrono::Utc::now(),
                url: "pr42".into(),
            },
        );
        g.checks.insert("s31".into(), ChecksStatus::Green);
        g.open.insert(
            "m0x6A/rubato".into(),
            vec![
                open_issue(31, "ok"),
                open_issue(32, "bad"),
                open_issue(33, "none"),
            ],
        );
        g.comments.insert(
            ("m0x6A/rubato".into(), 33),
            vec![Comment {
                id: 9,
                author: "m0x6A".into(),
                body: "Decision: No branch yet\n\n## Context\nx\n\n## Chosen\ny\n\n## Why\nz"
                    .into(),
                created_at: chrono::Utc::now(),
                url: "c9".into(),
            }],
        );
    });
    for n in [31, 32, 33] {
        let r = h
            .http
            .post(format!("{}{}", h.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new("m0x6A/rubato", n),
                target: MapRef::node("rubato-server"),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }
    h.github.with(|g| g.calls.clear());
    tasks::poll_once(&h.app).await.unwrap();
    let calls: Vec<String> = h
        .github
        .calls()
        .into_iter()
        .map(|(m, a)| format!("{m} {a}"))
        .collect();
    // Issue #49 (design 49-poll-cost S1–S4): branches and open pull requests once per repo,
    // title and state from the open listing, the plan read when the head moves, comments on the
    // full lane. Was 16 calls here (six per issue with an open pull request).
    let expected = [
        "open_issues m0x6A/rubato",
        "branches m0x6A/rubato issue/",
        "file m0x6A/rubato docs/plans/31.md issue/31-ok",
        "open_pulls m0x6A/rubato",
        "checks m0x6A/rubato s31",
        "issue_comments m0x6A/rubato#31",
        "file m0x6A/rubato docs/plans/32.md issue/32-bad",
        "pull_for_branch m0x6A/rubato issue/32-bad",
        "issue_comments m0x6A/rubato#32",
        "issue_comments m0x6A/rubato#33",
        "rate_limit ",
    ];
    assert_eq!(calls, expected, "GitHub calls per issue changed");
    let ov = h.overview("m0x6A").await;
    let get = |n| ov.issues.iter().find(|i| i.issue.number == n).unwrap();
    assert_eq!(get(31).progress.state, ProgressState::InReview);
    assert_eq!(get(32).progress.state, ProgressState::Done);
    assert_eq!(get(33).decisions.len(), 1);
    assert!(
        ov.findings
            .iter()
            .any(|f| matches!(f, Finding::PlanParseError { issue, .. } if issue.number == 32)),
        "parse failure recorded, not skipped: {:?}",
        ov.findings
    );
    assert_eq!(
        get(31).title.as_deref(),
        Some("ok"),
        "title from the listing"
    );
    println!("checked {} GitHub calls over 3 issues", calls.len());

    // A fast tick after a full one: the listings and the checks of the open pull request only.
    // The merged pull request (#32) is final; no comments; the plan heads have not moved.
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    h.github.with(|g| g.calls.clear());
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    let fast: Vec<String> = h
        .github
        .calls()
        .into_iter()
        .map(|(m, a)| format!("{m} {a}"))
        .collect();
    assert_eq!(
        fast,
        [
            "open_issues m0x6A/rubato",
            "branches m0x6A/rubato issue/",
            "open_pulls m0x6A/rubato",
            "checks m0x6A/rubato s31",
            "rate_limit ",
        ],
        "a fast tick"
    );
    let ov = h.overview("m0x6A").await;
    let get = |n| ov.issues.iter().find(|i| i.issue.number == n).unwrap();
    assert_eq!(get(31).progress.state, ProgressState::InReview);
    assert_eq!(get(32).progress.state, ProgressState::Done);
    println!("checked {} GitHub calls on a fast tick", fast.len());

    // As a GitHub App installation, and only then, two more calls on a full tick: the
    // installation's repos, and its permissions (Issues: write, ADR 0025), after the listing and
    // before the issue loop.
    h.github.with(|g| {
        g.installation = Some(vec!["m0x6A/rubato".into()]);
        g.calls.clear();
    });
    tasks::poll_once(&h.app).await.unwrap();
    let calls: Vec<String> = h
        .github
        .calls()
        .into_iter()
        .map(|(m, a)| format!("{m} {a}"))
        .collect();
    let mut as_installation: Vec<&str> = expected.to_vec();
    as_installation.insert(1, "installation_repos ");
    as_installation.insert(2, "installation ");
    // #32's pull request is stored as merged by now: final, not looked up again (S5).
    as_installation.retain(|c| *c != "pull_for_branch m0x6A/rubato issue/32-bad");
    assert_eq!(
        calls, as_installation,
        "one coverage call and one permission read, installation only"
    );
    println!("checked {} GitHub calls as an installation", calls.len());
}

#[tokio::test]
async fn shutdown_ends_the_supervised_poll_task_cleanly() {
    let h = harness().await;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let background = tasks::spawn(h.app.clone(), rx.clone());
    let supervised = tokio::spawn(tasks::supervise(background, rx));
    until_tick_done(&h.github).await;
    tx.send(true).unwrap();
    let r = tokio::time::timeout(std::time::Duration::from_secs(1), supervised)
        .await
        .expect("supervise returns within 1 s of shutdown");
    assert!(r.unwrap().is_ok());
}

// ----- issue #48, PR 1: open issues, pin kinds, the team record ------------------------------

fn open_issue(n: u64, title: &str) -> rubato_server::kernel::github::OpenIssueInfo {
    rubato_server::kernel::github::OpenIssueInfo {
        number: n,
        title: title.into(),
        url: format!("https://github.com/m0x6A/rubato/issues/{n}"),
        labels: vec!["enhancement".into()],
        updated_at: chrono::Utc::now(),
        ..Default::default()
    }
}

#[tokio::test]
async fn open_issues_reach_the_overview_until_they_are_pinned() {
    let h = harness().await;
    h.github.with(|g| {
        g.open.insert(
            "m0x6A/rubato".into(),
            vec![
                open_issue(47, "Show open issues"),
                open_issue(48, "Interactive map"),
            ],
        );
    });

    // Before the first listing: the repo is named, with no time and no error.
    let ov = h.overview("m0x6A").await;
    assert!(ov.backlog.issues.is_empty());
    assert_eq!(ov.backlog.repos.len(), 1);
    assert_eq!(ov.backlog.repos[0].repo, "m0x6A/rubato");
    assert!(ov.backlog.repos[0].listed_at.is_none());

    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    let numbers: Vec<u64> = ov.backlog.issues.iter().map(|i| i.issue.number).collect();
    assert_eq!(numbers, [47, 48]);
    assert_eq!(ov.backlog.issues[0].title, "Show open issues");
    assert_eq!(ov.backlog.issues[0].labels, ["enhancement"]);
    assert!(ov.backlog.repos[0].listed_at.is_some());
    assert!(
        ov.issues.is_empty(),
        "a backlog issue has no progress state"
    );

    // Pinning one moves it from the backlog to the tracked issues, as Planned.
    let r = h
        .http
        .post(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: IssueRef::new("m0x6A/rubato", 48),
            target: MapRef::seam("web-server"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let ov = h.overview("m0x6A").await;
    let numbers: Vec<u64> = ov.backlog.issues.iter().map(|i| i.issue.number).collect();
    assert_eq!(numbers, [47]);
    assert_eq!(ov.issues.len(), 1);
    assert_eq!(ov.issues[0].progress.state, ProgressState::Planned);

    // A failed listing is said, with the last good time, and the rows stay.
    h.github.with(|g| {
        g.open_errors
            .insert("m0x6A/rubato".into(), "rate limited".into());
    });
    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    assert_eq!(ov.backlog.repos[0].error.as_deref(), Some("rate limited"));
    assert!(ov.backlog.repos[0].listed_at.is_some());
    assert_eq!(ov.backlog.issues.len(), 1);
}

/// ADR 0026, issue #71 PR 3: a member's `Map:` line pins its issue without a drag, a
/// non-member's only suggests, and an issue without a line gets a scored suggestion. The bodies
/// come with the listing: the GitHub calls are the same as for issues without bodies.
#[tokio::test]
async fn map_lines_place_member_issues_and_suggest_for_the_rest() {
    let with_body = |n: u64, title: &str, body: &str, association: &str| {
        rubato_server::kernel::github::OpenIssueInfo {
            body: body.into(),
            author_association: association.into(),
            author: "voice-tool".into(),
            ..open_issue(n, title)
        }
    };
    let plain = harness().await;
    plain.github.with(|g| {
        g.open.insert(
            "m0x6A/rubato".into(),
            vec![
                open_issue(71, "Voice: plan lens"),
                open_issue(72, "Voice: stranger"),
                open_issue(73, "rubato-server is slow"),
            ],
        );
    });
    tasks::poll_once(&plain.app).await.unwrap();

    let h = harness().await;
    h.github.with(|g| {
        g.open.insert(
            "m0x6A/rubato".into(),
            vec![
                with_body(
                    71,
                    "Voice: plan lens",
                    "## Summary\nx\n\nMap: seam web-server\n",
                    "MEMBER",
                ),
                with_body(72, "Voice: stranger", "Map: node web\n", "NONE"),
                open_issue(73, "rubato-server is slow"),
            ],
        );
    });
    tasks::poll_once(&h.app).await.unwrap();
    assert_eq!(
        h.github.calls(),
        plain.github.calls(),
        "placement costs no GitHub call"
    );

    let ov = h.overview("m0x6A").await;
    let tracked: Vec<u64> = ov.issues.iter().map(|i| i.issue.number).collect();
    assert_eq!(tracked, [71], "the member's line pinned #71");
    assert_eq!(ov.issues[0].progress.state, ProgressState::Planned);
    let backlog: Vec<u64> = ov.backlog.issues.iter().map(|i| i.issue.number).collect();
    assert_eq!(backlog, [72, 73]);
    let stranger = &ov.backlog.issues[0];
    assert_eq!(stranger.map_line, Some(MapRef::node("web")));
    assert_eq!(
        stranger.suggestion.as_ref().map(|s| s.target.clone()),
        Some(MapRef::node("web"))
    );
    let scored = &ov.backlog.issues[1];
    assert_eq!(scored.map_line, None);
    assert_eq!(
        scored
            .suggestion
            .as_ref()
            .map(|s| (s.target.clone(), s.score)),
        Some((MapRef::node("rubato-server"), 100))
    );

    // Unpinned by a person: the next tick does not pin it again.
    let r = h
        .http
        .delete(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: IssueRef::new("m0x6A/rubato", 71),
            target: MapRef::seam("web-server"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    assert!(ov.issues.is_empty(), "the unpin sticks");
}

#[tokio::test]
async fn pin_kind_must_match_id() {
    let h = harness().await;
    let pin = |target: MapRef| {
        h.http
            .post(format!("{}{}", h.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new("m0x6A/rubato", 48),
                target,
            })
            .send()
    };
    // `rubato-server` is a part and `web-server` a seam in docs/design/rubato-map.yml.
    let cases = [
        (MapRef::node("rubato-server"), 201),
        (MapRef::seam("web-server"), 201),
        (MapRef::seam("rubato-server"), 422),
        (MapRef::node("web-server"), 422),
        (MapRef::node("no-such-id"), 422),
    ];
    for (target, want) in &cases {
        let r = pin(target.clone()).await.unwrap();
        let status = r.status();
        let body = r.text().await.unwrap();
        assert_eq!(status.as_u16(), *want, "{target:?}: {body}");
        if *want == 422 {
            assert!(body.contains("is not"), "the answer says why: {body}");
        }
    }
    let ov = h.overview("m0x6A").await;
    assert_eq!(
        ov.issues[0].pins.len(),
        2,
        "nothing stored for the refused ones"
    );
    println!("checked {} pin requests", cases.len());
}

#[tokio::test]
async fn old_mismatched_pin_still_listed() {
    let h = harness().await;
    // A pin stored before the kind check: a part's id kept as a seam.
    sqlx::query("INSERT INTO pins (repo, number, kind, target, pinned_by, pinned_at) VALUES ('m0x6A/rubato', 5, 'seam', 'rubato-server', 'm0x6A', '2026-11-01T00:00:00Z')")
        .execute(&h.app.db)
        .await
        .unwrap();
    let ov = h.overview("m0x6A").await;
    assert_eq!(ov.issues.len(), 1);
    assert_eq!(ov.issues[0].pins, vec![MapRef::seam("rubato-server")]);
}

/// A server acting as a GitHub App, with **no dev user**: the dev-user branch of `CurrentUser`
/// returns before any session lookup, so a harness with one would pass for the wrong reason.
async fn app_harness(as_app: bool) -> (String, App, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let db_url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = db::connect(&db_url).await.unwrap();
    let mut cfg = Config::for_tests(db_url, "unused");
    cfg.dev_user = None;
    if as_app {
        cfg.github_app = Some(rubato_server::kernel::config::GitHubApp {
            app_id: 12,
            private_key: "unused: no call is made as the App here".into(),
            installation_id: 34,
        });
    }
    let github = Arc::new(FakeGitHub::new());
    let map_yaml = std::fs::read_to_string(repo_root().join("docs/design/rubato-map.yml")).unwrap();
    github.with(|g| {
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/design/rubato-map.yml".into(),
                "main".into(),
            ),
            map_yaml,
        );
        g.branches.insert(
            "m0x6A/rubato".into(),
            vec![("main".into(), "mapsha1".into())],
        );
    });
    let app = AppState::new(pool, cfg, github);
    map::reload(&app).await.unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), app, tmp)
}

/// Insert a web session and return its signed cookie header value.
async fn session_cookie(app: &App, login: &str, team_checked_days_ago: Option<i64>) -> String {
    let id = Uuid::new_v4().to_string();
    let checked = team_checked_days_ago
        .map(|d| (chrono::Utc::now() - chrono::Duration::days(d)).to_rfc3339());
    sqlx::query(
        "INSERT INTO web_sessions (id, login, created_at, team_checked_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(login)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(checked)
    .execute(&app.db)
    .await
    .unwrap();
    format!("rubato_session={}", app.signer.sign(&id))
}

/// Status of the three kinds of entry a session is checked on: a read, a write, the websocket.
async fn statuses(url: &str, cookie: &str) -> [u16; 3] {
    let http = reqwest::Client::new();
    let read = http
        .get(format!("{url}{}", overview_routes::OVERVIEW))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    let write = http
        .post(format!("{url}{}", pin_routes::PINS))
        .header("cookie", cookie)
        .json(&PinRequest {
            issue: IssueRef::new("m0x6A/rubato", 48),
            target: MapRef::node("rubato-server"),
        })
        .send()
        .await
        .unwrap()
        .status()
        .as_u16();
    let mut req = format!(
        "ws://{}{}",
        url.trim_start_matches("http://"),
        web_routes::WEB_WS
    )
    .into_client_request()
    .unwrap();
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    let ws = match tokio_tungstenite::connect_async(req).await {
        Ok((_, response)) => response.status().as_u16(),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => response.status().as_u16(),
        Err(e) => panic!("websocket: {e}"),
    };
    [read, write, ws]
}

#[tokio::test]
async fn session_needs_a_fresh_team_record_when_the_app_is_configured() {
    let (url, app, _tmp) = app_harness(true).await;
    // A session from before the gate, one whose record is 8 days old, and a fresh one.
    let before_the_gate = session_cookie(&app, "old-timer", None).await;
    let stale = session_cookie(&app, "lapsed", Some(8)).await;
    let fresh = session_cookie(&app, "m0x6A", Some(1)).await;
    assert_eq!(statuses(&url, &before_the_gate).await, [401, 401, 401]);
    assert_eq!(statuses(&url, &stale).await, [401, 401, 401]);
    assert_eq!(statuses(&url, &fresh).await, [200, 201, 101]);
    assert_eq!(
        statuses(&url, "rubato_session=forged.sig").await,
        [401, 401, 401]
    );
    println!("checked 4 sessions on 3 entry points");
}

#[tokio::test]
async fn without_the_app_login_stays_open() {
    // Not a GitHub App (a personal token or none): a session needs no team record, as before.
    let (url, app, _tmp) = app_harness(false).await;
    let plain = session_cookie(&app, "anyone", None).await;
    assert_eq!(statuses(&url, &plain).await, [200, 201, 101]);
    assert!(
        !app.github.can_write(),
        "and such a server cannot propose map changes"
    );
}

fn layout_of(pairs: &[(&str, f64, f64)]) -> MapLayout {
    MapLayout {
        positions: pairs
            .iter()
            .map(|(id, x, y)| ((*id).to_owned(), Position { x: *x, y: *y }))
            .collect(),
    }
}

#[tokio::test]
async fn layout_put_is_in_overview_and_notifies() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let put = |layout: MapLayout| {
        h.http
            .put(format!("{}{}", h.url, layout_routes::LAYOUT))
            .json(&layout)
            .send()
    };
    assert!(h.overview("m0x6A").await.layout.positions.is_empty());

    // One moved part: stored, every browser told, and in the next overview.
    let r = put(layout_of(&[("rubato-server", 10.5, -20.0)]))
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    await_reason(&mut web, "layout").await;
    assert_eq!(
        h.overview("m0x6A").await.layout,
        layout_of(&[("rubato-server", 10.5, -20.0)])
    );

    // A write carries only what moved: the other part's position stays.
    let r = put(layout_of(&[("web", 300.0, 40.0)])).await.unwrap();
    assert_eq!(r.status(), 204);
    // The last write wins per part.
    let r = put(layout_of(&[("rubato-server", 1.0, 2.0)]))
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(
        h.overview("m0x6A").await.layout,
        layout_of(&[("rubato-server", 1.0, 2.0), ("web", 300.0, 40.0)])
    );

    // Refused whole: an id the map does not have, a seam's id, a position off the canvas.
    let refused = [
        layout_of(&[("web", 0.0, 0.0), ("no-such-part", 1.0, 1.0)]),
        layout_of(&[("web-server", 1.0, 1.0)]),
        layout_of(&[("web", 1.0e9, 0.0)]),
    ];
    for bad in &refused {
        let r = put(bad.clone()).await.unwrap();
        let status = r.status();
        let body = r.text().await.unwrap();
        assert_eq!(status, 422, "{bad:?}: {body}");
        assert!(!body.is_empty(), "the answer says why");
    }
    assert_eq!(
        h.overview("m0x6A").await.layout,
        layout_of(&[("rubato-server", 1.0, 2.0), ("web", 300.0, 40.0)]),
        "a refused write stores nothing, not even its good half"
    );

    // A row for a part the map no longer has is not served, and the next write removes it.
    sqlx::query("INSERT INTO map_layout (node_id, x, y, updated_by, updated_at) VALUES ('gone', 1, 1, 'm0x6A', '2026-11-01T00:00:00Z')")
        .execute(&h.app.db)
        .await
        .unwrap();
    assert!(
        !h.overview("m0x6A")
            .await
            .layout
            .positions
            .contains_key("gone")
    );
    put(layout_of(&[("web", 5.0, 5.0)])).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM map_layout WHERE node_id = 'gone'")
        .fetch_one(&h.app.db)
        .await
        .unwrap();
    assert_eq!(left, 0);
    println!(
        "checked 4 accepted and {} refused layout writes",
        refused.len()
    );
}

#[tokio::test]
async fn layout_delete_returns_removed_positions() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    let url = format!("{}{}", h.url, layout_routes::LAYOUT);
    let stored = layout_of(&[("rubato-server", 10.0, 20.0), ("web", 30.0, 40.0)]);
    let r = h.http.put(&url).json(&stored).send().await.unwrap();
    assert_eq!(r.status(), 204);
    await_reason(&mut web, "layout").await;

    // Auto-arrange: everything is removed for the team, and the answer is what was removed.
    let r = h.http.delete(&url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let removed: MapLayout = r.json().await.unwrap();
    assert_eq!(removed, stored);
    await_reason(&mut web, "layout").await;
    assert!(h.overview("m0x6A").await.layout.positions.is_empty());

    // Undo is a PUT of the answer.
    let r = h.http.put(&url).json(&removed).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(h.overview("m0x6A").await.layout, stored);

    // Clearing an empty layout is not an error.
    h.http.delete(&url).send().await.unwrap();
    let r = h.http.delete(&url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(
        r.json::<MapLayout>().await.unwrap().positions.is_empty(),
        "nothing left to remove"
    );
}

impl Harness {
    /// A queue write as `user`: the status and the body.
    async fn queue(
        &self,
        method: reqwest::Method,
        user: &str,
        body: serde_json::Value,
    ) -> (u16, String) {
        let r = self
            .http
            .request(method, format!("{}{}", self.url, queue_routes::QUEUE))
            .header("x-rubato-dev-user", user)
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    /// The numbers queued on a companion, in order, as everyone sees them.
    async fn queued(&self, companion_id: Uuid) -> Vec<u64> {
        self.overview("m0x6A")
            .await
            .queue
            .iter()
            .filter(|q| q.companion_id == companion_id)
            .map(|q| q.issue.number)
            .collect()
    }
}

fn queue_body(companion_id: Uuid, number: u64) -> serde_json::Value {
    serde_json::json!({ "issue": { "repo": "m0x6A/rubato", "number": number }, "companion_id": companion_id })
}

#[tokio::test]
async fn queue_add_reorder_remove() {
    use reqwest::Method;
    let h = harness().await;
    let mut web = h.web_ws().await;
    let (lane, _token) = h.pair("m0x6A").await;
    assert!(h.overview("m0x6A").await.queue.is_empty());

    // Add three; adding one twice changes nothing.
    for n in [5, 6, 7, 6] {
        let (status, body) = h.queue(Method::POST, "m0x6A", queue_body(lane, n)).await;
        assert_eq!(status, 201, "{body}");
    }
    await_reason(&mut web, "queue").await;
    assert_eq!(h.queued(lane).await, [5, 6, 7]);
    let ov = h.overview("alice").await;
    assert_eq!(ov.queue.len(), 3, "everyone sees every queue");
    assert_eq!(ov.queue[0].queued_by, "m0x6A");
    assert_eq!(
        ov.queue.iter().map(|q| q.position).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    // An issue nobody pinned can be queued; it is not a tracked issue and has no progress.
    assert!(ov.issues.is_empty());

    // Reorder with the full order of the lane.
    let order = |numbers: &[u64]| {
        serde_json::json!({
            "companion_id": lane,
            "issues": numbers.iter().map(|n| serde_json::json!({ "repo": "m0x6A/rubato", "number": n })).collect::<Vec<_>>(),
        })
    };
    let (status, body) = h.queue(Method::PUT, "m0x6A", order(&[7, 5, 6])).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(h.queued(lane).await, [7, 5, 6]);
    // An order that drops or adds an entry is refused: a reorder never deletes what another
    // tab added.
    for stale in [order(&[7, 5]), order(&[7, 5, 6, 8]), order(&[7, 7, 5])] {
        let (status, body) = h.queue(Method::PUT, "m0x6A", stale).await;
        assert_eq!(status, 409, "{body}");
        assert!(body.contains("differs"), "{body}");
    }
    assert_eq!(h.queued(lane).await, [7, 5, 6]);

    // Remove one; the rest close up. Removing it again is 404.
    let (status, _) = h.queue(Method::DELETE, "m0x6A", queue_body(lane, 5)).await;
    assert_eq!(status, 204);
    assert_eq!(h.queued(lane).await, [7, 6]);
    assert_eq!(
        h.overview("m0x6A")
            .await
            .queue
            .iter()
            .map(|q| q.position)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let (status, _) = h.queue(Method::DELETE, "m0x6A", queue_body(lane, 5)).await;
    assert_eq!(status, 404);

    // A lane holds at most 50.
    for n in 100..148 {
        let (status, body) = h.queue(Method::POST, "m0x6A", queue_body(lane, n)).await;
        assert_eq!(status, 201, "{n}: {body}");
    }
    let (status, body) = h.queue(Method::POST, "m0x6A", queue_body(lane, 999)).await;
    assert_eq!(status, 409, "{body}");
    assert!(body.contains("50"), "{body}");
    println!("checked 53 adds, 4 reorders, 2 removes");
}

#[tokio::test]
async fn queue_refuses_another_users_and_revoked_companions() {
    use reqwest::Method;
    let h = harness().await;
    let (mine, _t1) = h.pair("m0x6A").await;
    let (alices, _t2) = h.pair("alice").await;
    let (status, _) = h.queue(Method::POST, "alice", queue_body(alices, 1)).await;
    assert_eq!(status, 201);
    let order = |c: Uuid| serde_json::json!({ "companion_id": c, "issues": [{ "repo": "m0x6A/rubato", "number": 1 }] });

    // m0x6A on alice's lane: all three methods are forbidden, and nothing changes.
    let attempts = [
        (Method::POST, queue_body(alices, 2)),
        (Method::PUT, order(alices)),
        (Method::DELETE, queue_body(alices, 1)),
    ];
    for (method, body) in &attempts {
        let (status, text) = h.queue(method.clone(), "m0x6A", body.clone()).await;
        assert_eq!(status, 403, "{method}: {text}");
    }
    assert_eq!(h.queued(alices).await, [1]);
    // A companion nobody has is refused the same way, not revealed as missing.
    let (status, _) = h
        .queue(Method::POST, "m0x6A", queue_body(Uuid::new_v4(), 2))
        .await;
    assert_eq!(status, 403);

    // My own lane, then revoked: its entries are no longer shown and it takes no writes.
    let (status, _) = h.queue(Method::POST, "m0x6A", queue_body(mine, 3)).await;
    assert_eq!(status, 201);
    let r = h
        .http
        .delete(format!("{}{}/{}", h.url, pr::PAIRINGS, mine))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    assert!(
        h.queued(mine).await.is_empty(),
        "a revoked lane shows no queue"
    );
    for (method, body) in [
        (Method::POST, queue_body(mine, 4)),
        (
            Method::PUT,
            serde_json::json!({ "companion_id": mine, "issues": [{ "repo": "m0x6A/rubato", "number": 3 }] }),
        ),
        (Method::DELETE, queue_body(mine, 3)),
    ] {
        let (status, text) = h.queue(method.clone(), "m0x6A", body).await;
        assert_eq!(status, 403, "{method}: {text}");
    }
    // The next accepted queue write clears the revoked lane's rows.
    let (status, _) = h.queue(Method::POST, "alice", queue_body(alices, 9)).await;
    assert_eq!(status, 201);
    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM companion_queue WHERE companion_id = ?")
            .bind(mine.to_string())
            .fetch_one(&h.app.db)
            .await
            .unwrap();
    assert_eq!(left, 0);
    println!("checked 7 refused queue writes");
}

#[tokio::test]
async fn queue_entry_leaves_on_started_and_stays_on_refused() {
    use reqwest::Method;
    let h = harness().await;
    let issue = IssueRef::new("m0x6A/rubato", 3);
    let (lane, mut ws) = ready_to_launch(&h, &issue).await;
    let (status, _) = h.queue(Method::POST, "m0x6A", queue_body(lane, 3)).await;
    assert_eq!(status, 201);
    let before = progress_of(&h, &issue).await;

    // Start from the queue: the web sends only the issue and the lane's companion.
    let launch = |h: &Harness| {
        h.http
            .post(format!("{}{}", h.url, launch_routes::LAUNCH))
            .json(&serde_json::json!({ "issue": issue, "companion_id": lane }))
            .send()
    };
    let command_id = |t: String| {
        let env: ServerEnvelope = serde_json::from_str(&t).unwrap();
        let ServerMessage::Launch(cmd) = env.message else {
            panic!("{:?}", env.message)
        };
        cmd.id
    };
    assert_eq!(launch(&h).await.unwrap().status(), 200);
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!("no launch on the socket")
    };
    let first = command_id(t.to_string());
    assert_eq!(
        h.queued(lane).await,
        [3],
        "sent, not yet started: still queued"
    );

    // The companion refuses: the entry stays, with the reason, and progress is what it was.
    let refused = CompanionEnvelope {
        seq: 30,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Launch(rubato_protocol::LaunchResult::Refused {
            command_id: first,
            reason: rubato_protocol::RefusalReason::MissingDependency,
            detail: "tmux-ide 2.9 is needed".into(),
            at: chrono::Utc::now(),
        }),
    };
    send(&mut ws, &refused).await;
    let ov = h.overview("m0x6A").await;
    assert_eq!(ov.queue.len(), 1);
    assert_eq!(
        ov.queue[0].last_start_error.as_deref(),
        Some("missing_dependency: tmux-ide 2.9 is needed")
    );
    assert_eq!(progress_of(&h, &issue).await.state, before.state);

    // The second start is reported as started: the entry leaves the queue.
    assert_eq!(launch(&h).await.unwrap().status(), 200);
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!("no second launch on the socket")
    };
    let started = CompanionEnvelope {
        seq: 31,
        sent_at: chrono::Utc::now(),
        message: CompanionMessage::Launch(rubato_protocol::LaunchResult::Started {
            command_id: command_id(t.to_string()),
            session_id: Uuid::new_v4(),
            worktree: "/w".into(),
            tmux_session: "s".into(),
            at: chrono::Utc::now(),
        }),
    };
    send(&mut ws, &started).await;
    assert!(h.queued(lane).await.is_empty());
}

fn draft_node(id: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "kind": "context" })
}

fn draft_seam(id: &str, from: &str, to: &str, owner: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "from": from, "to": to, "owner": owner })
}

impl Harness {
    /// Add one operation to the draft as `user`: the status and the body.
    async fn draft_op(&self, user: &str, op: &str, body: serde_json::Value) -> (u16, String) {
        let mut req = body;
        req["op"] = op.into();
        req["op_id"] = Uuid::new_v4().to_string().into();
        let r = self
            .http
            .post(format!("{}{}", self.url, draft_routes::DRAFT_OPS))
            .header("x-rubato-dev-user", user)
            .json(&req)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn draft_post(&self, route: &str) -> (u16, String) {
        let r = self
            .http
            .post(format!("{}{route}", self.url))
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn draft(&self) -> rubato_protocol::MapDraft {
        self.overview("m0x6A").await.draft
    }
}

#[tokio::test]
async fn draft_ops_build_a_preview() {
    let h = harness().await;
    let mut web = h.web_ws().await;
    assert!(h.draft().await.ops.is_empty());
    assert!(h.draft().await.preview.is_none());

    // Add a part, describe it, join it to the server with a seam, give the seam a flow and a
    // description, then remove an existing seam. Two people, one draft.
    let mut described = draft_node("billing");
    described["description"] = "Invoices and payments.".into();
    let mut seam = draft_seam("server-billing", "rubato-server", "billing", "billing");
    seam["description"] = "The server asks billing what an issue cost.".into();
    seam["flows"] = serde_json::json!([{ "kind": "data", "from": "billing", "to": "rubato-server", "carries": "cost per issue" }]);
    let steps = [
        (
            "m0x6A",
            "add_node",
            serde_json::json!({ "node": draft_node("billing") }),
        ),
        (
            "alice",
            "update_node",
            serde_json::json!({ "node": described }),
        ),
        (
            "m0x6A",
            "add_seam",
            serde_json::json!({ "seam": draft_seam("server-billing", "rubato-server", "billing", "billing") }),
        ),
        ("alice", "update_seam", serde_json::json!({ "seam": seam })),
        (
            "m0x6A",
            "remove_seam",
            serde_json::json!({ "id": "skill-plan-file" }),
        ),
    ];
    for (user, op, body) in &steps {
        let (status, text) = h.draft_op(user, op, body.clone()).await;
        assert_eq!(status, 201, "{op}: {text}");
    }
    await_reason(&mut web, "draft").await;

    let draft = h.draft().await;
    assert_eq!(draft.ops.len(), 5);
    assert_eq!(
        draft
            .ops
            .iter()
            .map(|o| o.author.as_str())
            .collect::<Vec<_>>(),
        ["m0x6A", "alice", "m0x6A", "alice", "m0x6A"]
    );
    assert!(draft.ops.iter().all(|o| o.stale.is_none()));
    assert!(draft.proposal.is_none());
    let preview = draft
        .preview
        .expect("a preview while the draft has operations");
    assert_eq!(
        preview.node("billing").unwrap().description.as_deref(),
        Some("Invoices and payments.")
    );
    let s = preview.seam("server-billing").unwrap();
    assert_eq!(s.flows.len(), 1);
    assert_eq!(
        s.description.as_deref(),
        Some("The server asks billing what an issue cost.")
    );
    assert!(preview.seam("skill-plan-file").is_none());
    assert_eq!(preview.validate(), Ok(()));
    // The merged map is untouched: the draft is an overlay.
    let ov = h.overview("m0x6A").await;
    assert!(ov.map.map.node("billing").is_none());
    assert!(ov.map.map.seam("skill-plan-file").is_some());

    // An invalid operation says why and stores nothing; so does one that conflicts with an
    // earlier operation of the draft.
    let refused = [
        (
            "add_node",
            serde_json::json!({ "node": draft_node("billing") }),
            "already exists",
        ),
        (
            "add_seam",
            serde_json::json!({ "seam": draft_seam("web-nowhere", "web", "nowhere", "web") }),
            "unknown node",
        ),
        (
            "remove_node",
            serde_json::json!({ "id": "billing" }),
            "still an end of server-billing",
        ),
    ];
    for (op, body, word) in &refused {
        let (status, text) = h.draft_op("m0x6A", op, body.clone()).await;
        assert_eq!(status, 422, "{op}: {text}");
        assert!(text.contains(word), "{op}: {text}");
    }
    assert_eq!(h.draft().await.ops.len(), 5);

    // A draft part can be placed on the shared layout; an id in neither map nor draft cannot.
    let put = |id: &str| {
        h.http
            .put(format!("{}{}", h.url, layout_routes::LAYOUT))
            .json(&layout_of(&[(id, 40.0, 50.0)]))
            .send()
    };
    assert_eq!(put("billing").await.unwrap().status(), 204);
    assert_eq!(put("no-such-part").await.unwrap().status(), 422);
    assert!(
        h.overview("m0x6A")
            .await
            .layout
            .positions
            .contains_key("billing")
    );

    // The same request sent twice (a retry) adds the operation once.
    let again = serde_json::json!({ "op": "add_node", "op_id": Uuid::new_v4(), "node": draft_node("ledger") });
    for _ in 0..2 {
        let r = h
            .http
            .post(format!("{}{}", h.url, draft_routes::DRAFT_OPS))
            .json(&again)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }
    assert_eq!(h.draft().await.ops.len(), 6);
    println!(
        "checked 6 accepted and {} refused draft operations",
        refused.len()
    );
}

#[tokio::test]
async fn draft_remove_with_pins_is_409() {
    let h = harness().await;
    for (n, target) in [
        (7, MapRef::node("plan-check")),
        (8, MapRef::seam("web-server")),
    ] {
        let r = h
            .http
            .post(format!("{}{}", h.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new("m0x6A/rubato", n),
                target,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }
    let (status, text) = h
        .draft_op(
            "m0x6A",
            "remove_seam",
            serde_json::json!({ "id": "web-server" }),
        )
        .await;
    assert_eq!(status, 409, "{text}");
    assert!(
        text.contains("m0x6A/rubato#8"),
        "the answer names the issue: {text}"
    );
    let (status, text) = h
        .draft_op(
            "m0x6A",
            "remove_node",
            serde_json::json!({ "id": "plan-check" }),
        )
        .await;
    assert_eq!(status, 409, "{text}");
    assert!(text.contains("m0x6A/rubato#7"), "{text}");
    assert!(h.draft().await.ops.is_empty());
    // A seam nobody pinned can go.
    let (status, text) = h
        .draft_op(
            "m0x6A",
            "remove_seam",
            serde_json::json!({ "id": "skill-plan-file" }),
        )
        .await;
    assert_eq!(status, 201, "{text}");
}

#[tokio::test]
async fn draft_ids_cannot_be_pinned() {
    let h = harness().await;
    let (status, _) = h
        .draft_op(
            "m0x6A",
            "add_node",
            serde_json::json!({ "node": draft_node("billing") }),
        )
        .await;
    assert_eq!(status, 201);
    let (status, _) = h
        .draft_op(
            "m0x6A",
            "add_seam",
            serde_json::json!({ "seam": draft_seam("web-billing", "web", "billing", "billing") }),
        )
        .await;
    assert_eq!(status, 201);
    for target in [MapRef::node("billing"), MapRef::seam("web-billing")] {
        let r = h
            .http
            .post(format!("{}{}", h.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new("m0x6A/rubato", 48),
                target: target.clone(),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422, "{target:?} is a draft id, not a map id");
    }
    assert!(h.overview("m0x6A").await.issues.is_empty());
}

#[tokio::test]
async fn removing_an_operation_marks_dependants_stale() {
    let h = harness().await;
    h.draft_op(
        "m0x6A",
        "add_node",
        serde_json::json!({ "node": draft_node("billing") }),
    )
    .await;
    h.draft_op(
        "alice",
        "add_seam",
        serde_json::json!({ "seam": draft_seam("web-billing", "web", "billing", "billing") }),
    )
    .await;
    h.draft_op(
        "alice",
        "add_node",
        serde_json::json!({ "node": draft_node("ledger") }),
    )
    .await;
    let draft = h.draft().await;
    let first = draft.ops[0].seq;

    // Alice removes m0x6A's operation: anyone may. The seam that needed the part no longer
    // applies and says why; the unrelated part still does.
    let r = h
        .http
        .delete(format!("{}{}/{first}", h.url, draft_routes::DRAFT_OPS))
        .header("x-rubato-dev-user", "alice")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let draft = h.draft().await;
    assert_eq!(draft.ops.len(), 2);
    let stale = draft.ops[0].stale.as_deref().expect("the seam is stale");
    assert!(stale.contains("billing"), "{stale}");
    assert!(draft.ops[1].stale.is_none());
    let preview = draft.preview.unwrap();
    assert!(
        preview.seam("web-billing").is_none(),
        "a stale operation is not in the preview"
    );
    assert!(preview.node("ledger").is_some());
    // Removing it again is 404.
    let r = h
        .http
        .delete(format!("{}{}/{first}", h.url, draft_routes::DRAFT_OPS))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn discard_then_undo_restores_every_operation_and_author() {
    let h = harness().await;
    h.draft_op(
        "m0x6A",
        "add_node",
        serde_json::json!({ "node": draft_node("billing") }),
    )
    .await;
    h.draft_op(
        "alice",
        "add_seam",
        serde_json::json!({ "seam": draft_seam("web-billing", "web", "billing", "billing") }),
    )
    .await;
    let before = h.draft().await;
    assert!(!before.can_restore);

    let (status, _) = h.draft_post(draft_routes::DRAFT_DISCARD).await;
    assert_eq!(status, 200);
    let emptied = h.draft().await;
    assert!(emptied.ops.is_empty() && emptied.preview.is_none());
    assert!(emptied.can_restore, "Undo is offered to everyone");

    let (status, _) = h.draft_post(draft_routes::DRAFT_RESTORE).await;
    assert_eq!(status, 200);
    let after = h.draft().await;
    assert_eq!(
        after.ops, before.ops,
        "every operation, author, time and place"
    );
    assert!(!after.can_restore);

    // Discard, then a new operation: the discarded draft can no longer come back.
    h.draft_post(draft_routes::DRAFT_DISCARD).await;
    h.draft_op(
        "m0x6A",
        "add_node",
        serde_json::json!({ "node": draft_node("ledger") }),
    )
    .await;
    let (status, text) = h.draft_post(draft_routes::DRAFT_RESTORE).await;
    assert_eq!(status, 409, "{text}");
    assert_eq!(h.draft().await.ops.len(), 1);
    // With nothing discarded at all, restore is 409 too.
    let (status, _) = h.draft_post(draft_routes::DRAFT_RESTORE).await;
    assert_eq!(status, 409);
}

#[tokio::test]
async fn discard_is_409_while_proposing() {
    let h = harness().await;
    h.draft_op(
        "m0x6A",
        "add_node",
        serde_json::json!({ "node": draft_node("billing") }),
    )
    .await;
    // A proposal exists (written as the propose route writes it).
    sqlx::query("INSERT INTO map_proposal (id, branch, base_sha, file_hash, last_seq, state, state_since, proposed_by) VALUES (1, 'map/draft-1', 'a', 'b', 1, 'opening', '2026-11-11T00:00:00Z', 'm0x6A')")
        .execute(&h.app.db)
        .await
        .unwrap();
    let (status, text) = h.draft_post(draft_routes::DRAFT_DISCARD).await;
    assert_eq!(status, 409, "{text}");
    assert!(text.contains("Withdraw"), "{text}");
    // Nor does it take operations or removals.
    let (status, _) = h
        .draft_op(
            "m0x6A",
            "add_node",
            serde_json::json!({ "node": draft_node("ledger") }),
        )
        .await;
    assert_eq!(status, 409);
    let seq = h.draft().await.ops[0].seq;
    let r = h
        .http
        .delete(format!("{}{}/{seq}", h.url, draft_routes::DRAFT_OPS))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    let draft = h.draft().await;
    assert_eq!(draft.ops.len(), 1);
    assert_eq!(
        draft.proposal.map(|p| p.state),
        Some(rubato_protocol::ProposalState::Opening)
    );
}

const MAP_PATH: &str = "docs/design/rubato-map.yml";

impl Harness {
    /// A draft of two operations by two people, with GitHub writes turned on.
    async fn drafted(&self) {
        self.github.with(|g| g.writes = true);
        let (status, text) = self
            .draft_op(
                "m0x6A",
                "add_node",
                serde_json::json!({ "node": draft_node("billing") }),
            )
            .await;
        assert_eq!(status, 201, "{text}");
        let (status, text) = self
            .draft_op(
                "alice",
                "add_seam",
                serde_json::json!({ "seam": draft_seam("server-billing", "rubato-server", "billing", "billing") }),
            )
            .await;
        assert_eq!(status, 201, "{text}");
    }

    fn write_calls(&self) -> Vec<&'static str> {
        const WRITES: [&str; 5] = [
            "create_ref",
            "put_file",
            "create_pull",
            "close_pull",
            "delete_ref",
        ];
        self.github
            .calls()
            .into_iter()
            .map(|(name, _)| name)
            .filter(|n| WRITES.contains(n))
            .collect()
    }

    fn call_args(&self, name: &str) -> Vec<String> {
        self.github
            .calls()
            .into_iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, args)| args)
            .collect()
    }

    async fn proposal_state(&self) -> Option<rubato_protocol::ProposalState> {
        self.draft().await.proposal.map(|p| p.state)
    }
}

#[tokio::test]
async fn propose_opens_one_pr_and_freezes_the_draft() {
    use rubato_protocol::ProposalState;
    let h = harness().await;
    h.drafted().await;
    let mut web = h.web_ws().await;

    let (status, text) = h.draft_post(draft_routes::DRAFT_PROPOSE).await;
    assert_eq!(status, 200, "{text}");
    await_reason(&mut web, "draft").await;
    let draft = h.draft().await;
    let p = draft.proposal.clone().expect("a proposal");
    assert_eq!(p.state, ProposalState::Proposed);
    assert_eq!(p.proposed_by, "m0x6A");
    assert!(p.branch.starts_with("map/draft-"), "{}", p.branch);
    assert_eq!(p.pr_number, Some(101));
    assert_eq!(
        p.pr_url.as_deref(),
        Some("https://github.com/m0x6A/rubato/pull/101")
    );
    assert_eq!(draft.ops.len(), 2, "the operations stay, frozen");

    // The branch starts at the map branch's head; one commit rewrites the map file; one pull
    // request into the map branch, whose body lists each operation with its author.
    assert_eq!(h.write_calls(), ["create_ref", "put_file", "create_pull"]);
    assert_eq!(
        h.call_args("create_ref"),
        [format!("m0x6A/rubato {} mapsha1", p.branch)]
    );
    let committed = h
        .github
        .file("m0x6A/rubato", MAP_PATH, &p.branch)
        .await
        .unwrap()
        .expect("the file on the proposal's branch");
    let proposed: rubato_protocol::SystemMap = serde_yaml_ng::from_str(&committed).unwrap();
    assert!(proposed.node("billing").is_some());
    assert_eq!(proposed.seam("server-billing").unwrap().owner, "billing");
    assert_eq!(proposed.validate(), Ok(()));
    assert_eq!(proposed.nodes.len(), 12, "everything else is kept");
    let pull = &h.call_args("create_pull")[0];
    assert!(
        pull.starts_with(&format!("m0x6A/rubato {} main Map: 2 changes", p.branch)),
        "{pull}"
    );
    assert!(
        pull.contains("| 1 | add part billing | `m0x6A` |"),
        "{pull}"
    );
    assert!(
        pull.contains("| 2 | add seam server-billing (rubato-server to billing) | `alice` |"),
        "{pull}"
    );
    assert!(
        !pull.contains('@'),
        "no login is written as a mention: {pull}"
    );

    // Frozen for everyone: operations, removals and discard answer 409.
    let (status, text) = h
        .draft_op(
            "alice",
            "add_node",
            serde_json::json!({ "node": draft_node("ledger") }),
        )
        .await;
    assert_eq!(status, 409, "{text}");
    assert_eq!(h.draft_post(draft_routes::DRAFT_DISCARD).await.0, 409);

    // Asked again, the answer is the same proposal; nothing more is written.
    let (status, text) = h.draft_post(draft_routes::DRAFT_PROPOSE).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(h.write_calls(), ["create_ref", "put_file", "create_pull"]);

    // The proposed file is also offered as text.
    let yaml = h
        .http
        .get(format!("{}{}", h.url, draft_routes::DRAFT_YAML))
        .send()
        .await
        .unwrap();
    assert_eq!(yaml.status(), 200);
    assert_eq!(yaml.text().await.unwrap(), committed);
    // With `?base=true`: the loaded map written the same way, to compare the proposal with.
    let base = h
        .http
        .get(format!("{}{}?base=true", h.url, draft_routes::DRAFT_YAML))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!base.contains("billing") && committed.contains("billing"));
    assert!(base.contains("rubato-server"));
}

#[tokio::test]
async fn propose_twice_resumes_and_opens_one_pr() {
    use rubato_protocol::ProposalState;
    let h = harness().await;
    h.drafted().await;
    h.github.with(|g| {
        g.fail_write = Some(("put_file", "Resource not accessible by integration".into()))
    });

    // The write of the file fails: 502 naming the step, and the draft is frozen in opening.
    let (status, text) = h.draft_post(draft_routes::DRAFT_PROPOSE).await;
    assert_eq!(status, 502, "{text}");
    assert!(text.contains("step contents"), "{text}");
    assert!(
        text.contains("Resource not accessible by integration"),
        "{text}"
    );
    assert!(text.contains("Propose again resumes"), "{text}");
    assert_eq!(h.proposal_state().await, Some(ProposalState::Opening));
    let (status, _) = h
        .draft_op(
            "m0x6A",
            "add_node",
            serde_json::json!({ "node": draft_node("ledger") }),
        )
        .await;
    assert_eq!(status, 409, "opening is frozen too");
    let branch = h.draft().await.proposal.unwrap().branch;

    // Propose again resumes on the same branch and opens the one pull request.
    h.github.with(|g| g.fail_write = None);
    let (status, text) = h.draft_post(draft_routes::DRAFT_PROPOSE).await;
    assert_eq!(status, 200, "{text}");
    let p = h.draft().await.proposal.unwrap();
    assert_eq!(p.state, ProposalState::Proposed);
    assert_eq!(p.branch, branch, "the same branch, not a second one");
    assert_eq!(
        h.write_calls(),
        [
            "create_ref",
            "put_file",
            "create_ref",
            "put_file",
            "create_pull"
        ],
        "the branch that exists is reused, and there is one pull request"
    );
    assert!(
        h.call_args("create_ref")
            .iter()
            .all(|a| a.ends_with("mapsha1"))
    );
}

async fn assert_draft_unchanged(h: &Harness, before: &rubato_protocol::MapDraft, what: &str) {
    let now = h.draft().await;
    assert_eq!(now.ops, before.ops, "{what}: the operations are kept");
    assert!(now.proposal.is_none(), "{what}: still editable");
}

#[tokio::test]
async fn propose_refusals_keep_the_draft() {
    use rubato_protocol::ProposalState;
    let h = harness().await;
    h.drafted().await;
    let before = h.draft().await;
    let original = h
        .github
        .file("m0x6A/rubato", MAP_PATH, "main")
        .await
        .unwrap()
        .unwrap();
    let propose = || h.draft_post(draft_routes::DRAFT_PROPOSE);

    // A connection that cannot write (a personal token, or none).
    h.github.with(|g| g.writes = false);
    let (status, text) = propose().await;
    assert_eq!(status, 409, "{text}");
    assert!(
        text.contains("cannot write to GitHub") && text.contains("Copy YAML"),
        "{text}"
    );
    assert_draft_unchanged(&h, &before, "cannot write").await;
    h.github.with(|g| g.writes = true);

    // The map file on the map branch is not the one that is loaded.
    h.github.with(|g| {
        g.files.insert(
            ("m0x6A/rubato".into(), MAP_PATH.into(), "main".into()),
            format!("{original}\n# changed by hand\n"),
        );
    });
    let (status, text) = propose().await;
    assert_eq!(status, 409, "{text}");
    assert!(text.contains("Reload the map first"), "{text}");
    assert_draft_unchanged(&h, &before, "map changed").await;
    h.github.with(|g| {
        g.files.insert(
            ("m0x6A/rubato".into(), MAP_PATH.into(), "main".into()),
            original.clone(),
        );
    });

    // An operation that no longer applies: named, so it can be removed first.
    let first = before.ops[0].seq;
    let removed = h
        .http
        .delete(format!("{}{}/{first}", h.url, draft_routes::DRAFT_OPS))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 204);
    let (status, text) = propose().await;
    assert_eq!(status, 409, "{text}");
    assert!(
        text.contains("add seam server-billing") && text.contains("no longer applies"),
        "{text}"
    );
    assert!(h.draft().await.proposal.is_none());
    let stale = h.draft().await.ops[0].seq;
    h.http
        .delete(format!("{}{}/{stale}", h.url, draft_routes::DRAFT_OPS))
        .send()
        .await
        .unwrap();

    // An empty draft.
    let (status, text) = propose().await;
    assert_eq!(status, 409, "{text}");
    assert!(text.contains("nothing to propose"), "{text}");
    assert_eq!(
        h.write_calls(),
        Vec::<&str>::new(),
        "nothing was written by any refusal"
    );

    // A map file with a key the model does not know: a rewrite would drop it.
    h.github.with(|g| {
        g.files.insert(
            ("m0x6A/rubato".into(), MAP_PATH.into(), "main".into()),
            format!("{original}\nteam: payments\n"),
        );
    });
    map::reload(&h.app).await.unwrap();
    h.draft_op(
        "m0x6A",
        "add_node",
        serde_json::json!({ "node": draft_node("billing") }),
    )
    .await;
    let (status, text) = propose().await;
    assert_eq!(status, 409, "{text}");
    assert!(
        text.contains("keys Rubato does not know") && text.contains("team"),
        "{text}"
    );
    assert!(h.draft().await.proposal.is_none());
    h.github.with(|g| {
        g.files.insert(
            ("m0x6A/rubato".into(), MAP_PATH.into(), "main".into()),
            original.clone(),
        );
    });
    map::reload(&h.app).await.unwrap();

    // An unrelated commit on the map branch does not refuse: the proposal starts at the new head.
    h.github.with(|g| {
        g.branches.insert(
            "m0x6A/rubato".into(),
            vec![("main".into(), "mapsha2".into())],
        );
        g.fail_write = Some(("create_ref", "boom".into()));
    });
    // ...and a failing GitHub step answers 502 naming the step, frozen in opening.
    let (status, text) = propose().await;
    assert_eq!(status, 502, "{text}");
    assert!(text.contains("step ref"), "{text}");
    assert_eq!(h.proposal_state().await, Some(ProposalState::Opening));
    assert!(h.call_args("create_ref")[0].ends_with("mapsha2"));

    // While frozen in opening the map file changes: Propose again refuses and it stays frozen;
    // Withdraw is the way out, and the draft is editable again with its operation kept.
    h.github.with(|g| {
        g.fail_write = None;
        g.files.insert(
            ("m0x6A/rubato".into(), MAP_PATH.into(), "main".into()),
            format!("{original}\n# changed again\n"),
        );
    });
    let (status, text) = propose().await;
    assert_eq!(status, 409, "{text}");
    assert_eq!(h.proposal_state().await, Some(ProposalState::Opening));
    let (status, text) = h.draft_post(draft_routes::DRAFT_WITHDRAW).await;
    assert_eq!(status, 200, "{text}");
    let after = h.draft().await;
    assert!(after.proposal.is_none());
    assert_eq!(after.ops.len(), 1);

    // A server that reads the map from a local file has no repository to propose to.
    let file = repo_root().join(MAP_PATH);
    let local = harness_with(|cfg| cfg.map_file = Some(file)).await;
    local.drafted().await;
    let (status, text) = local.draft_post(draft_routes::DRAFT_PROPOSE).await;
    assert_eq!(status, 409, "{text}");
    assert!(
        text.contains("local file") && text.contains("Copy YAML"),
        "{text}"
    );
    assert!(local.draft().await.proposal.is_none());
    println!("checked 8 refused proposals");
}

#[tokio::test]
async fn merged_proposal_clears_only_after_a_load_that_follows_the_merge() {
    use rubato_protocol::ProposalState;
    let h = harness().await;
    h.drafted().await;
    assert_eq!(h.draft_post(draft_routes::DRAFT_PROPOSE).await.0, 200);
    let p = h.draft().await.proposal.unwrap();

    // While proposed: an unrelated commit lands and the map is loaded again. The draft stays.
    h.github.with(|g| {
        let b = g.branches.get_mut("m0x6A/rubato").unwrap();
        b.iter_mut().find(|(n, _)| n == "main").unwrap().1 = "mapsha2".into();
    });
    map::reload(&h.app).await.unwrap();
    tasks::poll_once(&h.app).await.unwrap();
    assert_eq!(h.proposal_state().await, Some(ProposalState::Proposed));
    assert_eq!(h.draft().await.ops.len(), 2);

    // The pull request is merged, but the map cannot be loaded (the branch read fails): the
    // draft shows merged and keeps its operations, because no load has followed the merge.
    let merged_file = h
        .github
        .file("m0x6A/rubato", MAP_PATH, &p.branch)
        .await
        .unwrap()
        .unwrap();
    h.github.with(|g| {
        g.pulls
            .get_mut(&("m0x6A/rubato".to_owned(), p.branch.clone()))
            .unwrap()
            .merged_at = Some(chrono::Utc::now());
        g.pulls
            .get_mut(&("m0x6A/rubato".to_owned(), p.branch.clone()))
            .unwrap()
            .open = false;
        g.branches
            .get_mut("m0x6A/rubato")
            .unwrap()
            .retain(|(n, _)| n != "main");
    });
    tasks::poll_once(&h.app).await.unwrap();
    let draft = h.draft().await;
    assert_eq!(
        draft.proposal.as_ref().map(|p| p.state),
        Some(ProposalState::Merged)
    );
    assert_eq!(
        draft.ops.len(),
        2,
        "not cleared before a load that follows the merge"
    );
    assert_eq!(
        h.draft_post(draft_routes::DRAFT_WITHDRAW).await.0,
        409,
        "a merged proposal cannot be withdrawn"
    );

    // The map branch is readable again with the merged file: the next tick loads it and the
    // draft empties. The new part is now a part of the map.
    h.github.with(|g| {
        g.branches
            .get_mut("m0x6A/rubato")
            .unwrap()
            .push(("main".into(), "merged1".into()));
        g.files.insert(
            ("m0x6A/rubato".into(), MAP_PATH.into(), "main".into()),
            merged_file,
        );
    });
    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    assert!(ov.draft.ops.is_empty() && ov.draft.proposal.is_none() && ov.draft.preview.is_none());
    assert!(ov.map.map.node("billing").is_some());
    assert_eq!(ov.map.version, "merged1");
    assert!(
        h.write_calls().contains(&"delete_ref"),
        "the merged branch is deleted"
    );
    // And the draft takes operations again.
    let (status, _) = h
        .draft_op(
            "m0x6A",
            "add_node",
            serde_json::json!({ "node": draft_node("ledger") }),
        )
        .await;
    assert_eq!(status, 201);
}

#[tokio::test]
async fn closed_proposal_reopens_the_draft() {
    let h = harness().await;
    h.drafted().await;
    assert_eq!(h.draft_post(draft_routes::DRAFT_PROPOSE).await.0, 200);
    let p = h.draft().await.proposal.unwrap();
    // Someone closes the pull request on GitHub without merging.
    h.github.with(|g| {
        g.pulls
            .get_mut(&("m0x6A/rubato".to_owned(), p.branch.clone()))
            .unwrap()
            .open = false;
    });
    tasks::poll_once(&h.app).await.unwrap();
    let draft = h.draft().await;
    assert!(draft.proposal.is_none());
    assert_eq!(draft.ops.len(), 2, "every operation is kept");
    let (status, _) = h
        .draft_op(
            "alice",
            "add_node",
            serde_json::json!({ "node": draft_node("ledger") }),
        )
        .await;
    assert_eq!(status, 201, "editable again");
    assert!(h.write_calls().contains(&"delete_ref"));
}

#[tokio::test]
async fn a_pr_created_during_a_timed_out_call_is_found_from_opening() {
    use rubato_protocol::ProposalState;
    let h = harness().await;
    h.drafted().await;
    // GitHub creates the pull request, but the answer never arrives.
    h.github.with(|g| g.pull_times_out = true);
    let (status, text) = h.draft_post(draft_routes::DRAFT_PROPOSE).await;
    assert_eq!(status, 502, "{text}");
    assert!(
        text.contains("step pull") && text.contains("timed out"),
        "{text}"
    );
    let draft = h.draft().await;
    assert_eq!(
        draft.proposal.as_ref().map(|p| p.state),
        Some(ProposalState::Opening)
    );
    assert_eq!(draft.proposal.unwrap().pr_number, None);

    // The next poll tick finds it by its branch: proposed, with its number.
    tasks::poll_once(&h.app).await.unwrap();
    let p = h.draft().await.proposal.unwrap();
    assert_eq!(p.state, ProposalState::Proposed);
    assert_eq!(p.pr_number, Some(101));
    assert_eq!(
        h.call_args("create_pull").len(),
        1,
        "one pull request, not two"
    );
    // The proposal's lookup comes after the per-issue reads and before the rate-limit read,
    // which stays the last call of a tick.
    let tick = names(&h.github.calls());
    let lookup = tick.iter().rposition(|n| *n == "pull_for_branch").unwrap();
    assert_eq!(tick.last(), Some(&"rate_limit"));
    assert!(lookup < tick.len() - 1);

    // The same on Propose again without waiting for the tick.
    let other = harness().await;
    other.drafted().await;
    other.github.with(|g| g.pull_times_out = true);
    assert_eq!(other.draft_post(draft_routes::DRAFT_PROPOSE).await.0, 502);
    assert_eq!(other.draft_post(draft_routes::DRAFT_PROPOSE).await.0, 200);
    assert_eq!(other.call_args("create_pull").len(), 1);
    assert_eq!(other.proposal_state().await, Some(ProposalState::Proposed));
}

#[tokio::test]
async fn withdraw_closes_the_pr_and_reopens_the_draft() {
    let h = harness().await;
    h.drafted().await;
    // Nothing to withdraw yet.
    assert_eq!(h.draft_post(draft_routes::DRAFT_WITHDRAW).await.0, 409);
    assert_eq!(h.draft_post(draft_routes::DRAFT_PROPOSE).await.0, 200);
    let p = h.draft().await.proposal.unwrap();

    // GitHub will not close it: 502, and the proposal stays.
    h.github
        .with(|g| g.fail_write = Some(("close_pull", "boom".into())));
    let (status, text) = h.draft_post(draft_routes::DRAFT_WITHDRAW).await;
    assert_eq!(status, 502, "{text}");
    assert!(text.contains("step close"), "{text}");
    assert!(h.draft().await.proposal.is_some());

    h.github.with(|g| g.fail_write = None);
    let (status, text) = h.draft_post(draft_routes::DRAFT_WITHDRAW).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(
        h.call_args("close_pull").last().map(String::as_str),
        Some("m0x6A/rubato 101")
    );
    assert_eq!(
        h.call_args("delete_ref"),
        [format!("m0x6A/rubato {}", p.branch)]
    );
    let draft = h.draft().await;
    assert!(draft.proposal.is_none());
    assert_eq!(draft.ops.len(), 2, "every operation is kept");
    let (status, _) = h
        .draft_op(
            "alice",
            "add_node",
            serde_json::json!({ "node": draft_node("ledger") }),
        )
        .await;
    assert_eq!(status, 201, "editable again");
}

// ----- issue #49: fewer GitHub calls per tracked issue ---------------------------------------

const PLAN_49: &str = "---\nissue: m0x6A/rubato#1\n---\n## Affected parts\n| Node | Status | What changes | Seams |\n| --- | --- | --- | --- |\n| rubato-server | affected | x | server-github |\n## O-rows\n| Seam | Breaks like | Signal | Panel | Owner |\n| --- | --- | --- | --- | --- |\n| server-github | y | z | p | rubato-server |\n";

fn open_pull(number: u64, sha: &str) -> PullInfo {
    PullInfo {
        number,
        open: true,
        draft: false,
        head_sha: sha.into(),
        merged_at: None,
        updated_at: chrono::Utc::now(),
        url: format!("pr{number}"),
    }
}

impl Harness {
    async fn pin_issue(&self, n: u64) {
        let r = self
            .http
            .post(format!("{}{}", self.url, pin_routes::PINS))
            .json(&PinRequest {
                issue: IssueRef::new("m0x6A/rubato", n),
                target: MapRef::node("rubato-server"),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }

    /// Issue `n` open, with branch `issue/<n>-x` at `s<n>`, a plan on it, and optionally an open
    /// green pull request.
    fn working_issue(&self, n: u64, with_pr: bool) {
        self.github.with(|g| {
            let branch = format!("issue/{n}-x");
            let sha = format!("s{n}");
            g.branches
                .get_mut("m0x6A/rubato")
                .unwrap()
                .push((branch.clone(), sha.clone()));
            g.files.insert(
                (
                    "m0x6A/rubato".into(),
                    format!("docs/plans/{n}.md"),
                    branch.clone(),
                ),
                PLAN_49.replace("#1", &format!("#{n}")),
            );
            if with_pr {
                g.pulls
                    .insert(("m0x6A/rubato".into(), branch), open_pull(1000 + n, &sha));
                g.checks.insert(sha, ChecksStatus::Green);
            }
            g.open
                .entry("m0x6A/rubato".into())
                .or_default()
                .push(open_issue(n, &format!("issue {n}")));
        });
    }

    async fn state_of(&self, n: u64) -> ProgressState {
        let ov = self.overview("m0x6A").await;
        ov.issues
            .iter()
            .find(|i| i.issue.number == n)
            .unwrap()
            .progress
            .state
    }
}

/// AC2: 30 tracked issues at the default tick for one hour stay above 10% of the budget.
#[tokio::test]
async fn thirty_tracked_issues_stay_inside_the_rate_limit_budget() {
    let h = harness().await;
    for n in 1..=30 {
        h.working_issue(n, true);
        h.pin_issue(n).await;
    }
    let budget = 5000;
    h.github.with(|g| {
        g.calls.clear();
        g.budget = Some(budget);
    });
    let secs = rubato_protocol::Limits::default().github_poll_interval_secs;
    let ticks = 3600 / secs;
    let mut poller = tasks::Poller::default();
    let mut clock = tasks::LaneClock::new(secs);
    let mut full = 0;
    for _ in 0..ticks {
        let lane = clock.next_lane();
        if lane == tasks::Lane::Full {
            full += 1;
        }
        poller.tick(&h.app, lane).await.unwrap();
    }
    let spent = h.github.calls().len() as u64;
    let (remaining, limit) = h.github.rate_limit().await.unwrap();
    println!(
        "checked {ticks} ticks ({full} full) over 30 issues: {spent} GitHub calls, {remaining} of {limit} left"
    );
    assert_eq!(full, 12, "a full tick every 300 s");
    assert!(
        remaining * 10 > limit,
        "rate limit under 10% after an hour: {spent} calls"
    );
    assert_eq!(h.state_of(7).await, ProgressState::InReview);
}

/// S5, N2: a merge is seen on the next tick, even a fast one.
#[tokio::test]
async fn a_pull_request_that_leaves_the_open_listing_is_read_at_once() {
    let h = harness().await;
    h.working_issue(5, true);
    h.pin_issue(5).await;
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert_eq!(h.state_of(5).await, ProgressState::InReview);
    h.github.with(|g| {
        let pr = g
            .pulls
            .get_mut(&("m0x6A/rubato".to_string(), "issue/5-x".to_string()))
            .unwrap();
        pr.open = false;
        pr.merged_at = Some(chrono::Utc::now());
        g.calls.clear();
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(h.call_args("pull_for_branch"), ["m0x6A/rubato issue/5-x"]);
    assert!(h.call_args("checks").is_empty(), "no checks once merged");
    assert_eq!(h.state_of(5).await, ProgressState::Done);
    // Final now: no more look-ups, on either lane.
    h.github.with(|g| g.calls.clear());
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert!(h.call_args("pull_for_branch").is_empty());
    println!("checked {} calls after the merge", h.github.calls().len());
}

/// S6: a closed issue gets one last pass, then no calls of its own until it reopens.
#[tokio::test]
async fn a_closed_issue_is_settled_until_it_reopens() {
    let h = harness().await;
    h.working_issue(6, false);
    h.pin_issue(6).await;
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    // Closed on GitHub: gone from the open listing.
    h.github.with(|g| {
        g.open.get_mut("m0x6A/rubato").unwrap().clear();
        g.issues.insert(
            ("m0x6A/rubato".into(), 6),
            IssueInfo {
                title: "issue 6".into(),
                state: "closed".into(),
                url: "u6".into(),
            },
        );
        g.calls.clear();
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(h.call_args("issue"), ["m0x6A/rubato#6"]);
    assert_eq!(
        h.call_args("issue_comments"),
        ["m0x6A/rubato#6"],
        "the last pass reads the comments"
    );
    let calls_of_6 = |h: &Harness| {
        h.github
            .calls()
            .into_iter()
            .filter(|(_, a)| a.contains("#6") || a.contains("issue/6-"))
            .count()
    };
    h.github.with(|g| g.calls.clear());
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(calls_of_6(&h), 0, "settled: {:?}", h.github.calls());
    // Reopened: back in the listing, polled again.
    h.github.with(|g| {
        g.open
            .get_mut("m0x6A/rubato")
            .unwrap()
            .push(open_issue(6, "issue 6"));
        g.calls.clear();
    });
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert_eq!(h.call_args("issue_comments"), ["m0x6A/rubato#6"]);
    println!("checked a close and a reopen");
}

/// S3, N3: the plan file is read again only when the branch head moves.
#[tokio::test]
async fn the_plan_file_is_read_again_only_when_the_branch_moves() {
    let h = harness().await;
    h.working_issue(8, false);
    h.pin_issue(8).await;
    h.github.with(|g| g.calls.clear());
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert_eq!(h.call_args("file").len(), 1);
    h.github.with(|g| g.calls.clear());
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert!(h.call_args("file").is_empty(), "head did not move");
    h.github.with(|g| {
        let b = g.branches.get_mut("m0x6A/rubato").unwrap();
        b.iter_mut().find(|(n, _)| n == "issue/8-x").unwrap().1 = "s8b".into();
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(
        h.call_args("file"),
        ["m0x6A/rubato docs/plans/8.md issue/8-x"]
    );
    println!("checked 4 ticks of plan reads");
}

/// S5: a merge that deletes the head branch still reaches Done on the next tick.
#[tokio::test]
async fn a_merge_that_deletes_the_branch_is_done_on_the_next_tick() {
    let h = harness().await;
    h.working_issue(11, true);
    h.pin_issue(11).await;
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert_eq!(h.state_of(11).await, ProgressState::InReview);
    h.github.with(|g| {
        g.branches
            .get_mut("m0x6A/rubato")
            .unwrap()
            .retain(|(n, _)| n != "issue/11-x");
        let pr = g
            .pulls
            .get_mut(&("m0x6A/rubato".to_string(), "issue/11-x".to_string()))
            .unwrap();
        pr.open = false;
        pr.merged_at = Some(chrono::Utc::now());
        g.calls.clear();
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(h.call_args("pull_for_branch"), ["m0x6A/rubato issue/11-x"]);
    assert_eq!(h.state_of(11).await, ProgressState::Done);
    println!("checked a merge with the branch deleted");
}

/// S6: an issue closed while its pull request is open is not settled; the merge still lands.
#[tokio::test]
async fn a_closed_issue_with_an_open_pull_request_still_reaches_done() {
    let h = harness().await;
    h.working_issue(12, true);
    h.pin_issue(12).await;
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    h.github.with(|g| {
        g.open.get_mut("m0x6A/rubato").unwrap().clear();
        g.issues.insert(
            ("m0x6A/rubato".into(), 12),
            IssueInfo {
                title: "issue 12".into(),
                state: "closed".into(),
                url: "u12".into(),
            },
        );
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    h.github.with(|g| {
        let pr = g
            .pulls
            .get_mut(&("m0x6A/rubato".to_string(), "issue/12-x".to_string()))
            .unwrap();
        pr.open = false;
        pr.merged_at = Some(chrono::Utc::now());
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(h.state_of(12).await, ProgressState::Done);
    println!("checked a close before the merge");
}

/// §9.1: a last pass that fails is retried on the next tick, not lost to the settled rule.
#[tokio::test]
async fn a_failed_last_pass_is_retried() {
    let h = harness().await;
    h.working_issue(13, false);
    h.pin_issue(13).await;
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    h.github.with(|g| {
        g.open.get_mut("m0x6A/rubato").unwrap().clear();
        g.issues.insert(
            ("m0x6A/rubato".into(), 13),
            IssueInfo {
                title: "issue 13".into(),
                state: "closed".into(),
                url: "u13".into(),
            },
        );
        g.comments_error = Some("boom".into());
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    h.github.with(|g| {
        g.comments_error = None;
        g.calls.clear();
    });
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(
        h.call_args("issue_comments"),
        ["m0x6A/rubato#13"],
        "retried"
    );
    h.github.with(|g| g.calls.clear());
    poller.tick(&h.app, tasks::Lane::Full).await.unwrap();
    assert!(h.call_args("issue_comments").is_empty(), "settled after it");
    println!("checked a failed last pass");
}

/// N5: a failed open-pull-request listing falls back to one look-up per issue.
#[tokio::test]
async fn poll_falls_back_when_the_pull_listing_fails() {
    let h = harness().await;
    h.working_issue(9, true);
    h.pin_issue(9).await;
    h.github.with(|g| {
        g.open_pulls_error = Some("boom".into());
        g.calls.clear();
    });
    let mut poller = tasks::Poller::default();
    poller.tick(&h.app, tasks::Lane::Fast).await.unwrap();
    assert_eq!(h.call_args("open_pulls").len(), 1, "once per repo");
    assert_eq!(h.call_args("pull_for_branch"), ["m0x6A/rubato issue/9-x"]);
    assert_eq!(h.state_of(9).await, ProgressState::InReview);
    println!("checked the fallback");
}

// ----- issue #71, PR 4: issues created from Rubato (ADR 0025) --------------------------------

fn new_issue(request_id: &str, target: MapRef) -> rubato_protocol::NewIssueRequest {
    rubato_protocol::NewIssueRequest {
        request_id: request_id.into(),
        repo: "m0x6A/rubato".into(),
        target,
        title: "Poll less".into(),
        summary: "The poll costs too many calls.".into(),
        does: vec!["Lists branches once per repo.".into()],
        know: vec!["The call-order test asserts fewer calls.".into()],
        must_not_break: vec!["Progress latency.".into()],
        context: None,
        open_questions: vec![],
    }
}

impl Harness {
    async fn create_issue(
        &self,
        req: &rubato_protocol::NewIssueRequest,
    ) -> (u16, rubato_protocol::NewIssueResponse) {
        let r = self
            .http
            .post(format!(
                "{}{}",
                self.url,
                rubato_protocol::issue_new::routes::ISSUES
            ))
            .json(req)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        let text = r.text().await.unwrap();
        let body = serde_json::from_str(&text).unwrap_or(rubato_protocol::NewIssueResponse {
            state: rubato_protocol::CreationState::Failed,
            issue: None,
            url: None,
            message: Some(text),
        });
        (status, body)
    }

    fn creates(&self) -> usize {
        self.call_args("create_issue").len()
    }

    fn can_create(&self, yes: bool) {
        self.github.with(|g| {
            g.writes = true;
            g.issues_write = yes;
        });
    }
}

#[tokio::test]
async fn an_issue_created_from_a_part_is_pinned_there_once() {
    use rubato_protocol::CreationState;
    let h = harness().await;
    h.can_create(true);
    let ov = h.overview("m0x6A").await;
    assert!(ov.can_create_issues);
    let req = new_issue("r-1", MapRef::node("rubato-server"));
    let (status, r) = h.create_issue(&req).await;
    assert_eq!(status, 201, "{r:?}");
    assert_eq!(r.state, CreationState::Created);
    let issue = r.issue.clone().unwrap();
    // The body GitHub got: the pattern, the creator in backticks, the two lines.
    let body = h.github.inner.lock().unwrap().open["m0x6A/rubato"]
        .iter()
        .find(|i| i.number == issue.number)
        .unwrap()
        .body
        .clone();
    assert!(body.starts_with("## Summary\n"), "{body}");
    assert!(body.contains("Opened in Rubato by `m0x6A`."), "{body}");
    assert!(
        body.ends_with("Rubato-Request: r-1\nMap: node rubato-server\n"),
        "{body}"
    );
    let ov = h.overview("m0x6A").await;
    let s = ov
        .issues
        .iter()
        .find(|i| i.issue == issue)
        .expect("tracked");
    assert_eq!(s.progress.state, ProgressState::Planned);

    // The same request again: the same answer, no second issue.
    let (status, again) = h.create_issue(&req).await;
    assert_eq!(status, 200);
    assert_eq!(again.issue, Some(issue.clone()));
    assert_eq!(h.creates(), 1, "one POST for one request id");

    // A person unpins it; the next tick does not pin it again from its Map: line.
    let r = h
        .http
        .delete(format!("{}{}", h.url, pin_routes::PINS))
        .json(&PinRequest {
            issue: issue.clone(),
            target: MapRef::node("rubato-server"),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    tasks::poll_once(&h.app).await.unwrap();
    let ov = h.overview("m0x6A").await;
    assert!(
        ov.issues.iter().all(|i| i.issue != issue),
        "the unpin sticks"
    );
}

/// The gate: without Issues: write nothing is sent. Remove the `can_create_issues` check in the
/// handler and this fails on the `create_issue` call count.
#[tokio::test]
async fn creation_is_refused_without_the_permission_and_for_unknown_targets() {
    let h = harness().await;
    let (status, r) = h
        .create_issue(&new_issue("r-1", MapRef::node("rubato-server")))
        .await;
    assert_eq!(status, 409, "a port that cannot write: {r:?}");
    h.can_create(false);
    let (status, r) = h
        .create_issue(&new_issue("r-2", MapRef::node("rubato-server")))
        .await;
    assert_eq!(status, 409, "{r:?}");
    assert!(r.message.unwrap().contains("Issues: write"));
    assert!(!h.overview("m0x6A").await.can_create_issues);
    h.can_create(true);
    let (status, _) = h
        .create_issue(&new_issue("r-3", MapRef::node("billing")))
        .await;
    assert_eq!(status, 422);
    let (status, _) = h
        .create_issue(&new_issue("r-4", MapRef::seam("rubato-server")))
        .await;
    assert_eq!(status, 422, "a node id as a seam");
    let mut no_know = new_issue("r-5", MapRef::node("web"));
    no_know.know.clear();
    let (status, r) = h.create_issue(&no_know).await;
    assert_eq!(status, 422);
    assert!(r.message.unwrap().contains("How we'll know it works"));
    assert_eq!(h.creates(), 0, "nothing was sent");
    println!("checked 6 refusals");
}

#[tokio::test]
async fn a_403_fails_the_request_and_clears_the_capability() {
    use rubato_protocol::CreationState;
    let h = harness().await;
    h.can_create(true);
    h.github.with(|g| {
        g.fail_write = Some((
            "create_issue",
            "403 Resource not accessible by integration".into(),
        ))
    });
    let (status, r) = h.create_issue(&new_issue("r-1", MapRef::node("web"))).await;
    assert_eq!(status, 502, "{r:?}");
    assert_eq!(r.state, CreationState::Failed);
    assert!(!h.overview("m0x6A").await.can_create_issues);
    let (status, _) = h.create_issue(&new_issue("r-2", MapRef::node("web"))).await;
    assert_eq!(status, 409, "refused until the next permission read");
    assert_eq!(h.creates(), 1);
}

/// A create that does not answer: the row stays opening, a resubmit sends nothing, and the next
/// listing finds the issue by its `Rubato-Request:` line and pins it.
#[tokio::test]
async fn a_create_that_timed_out_is_found_by_its_request_line() {
    use rubato_protocol::CreationState;
    let h = harness().await;
    h.can_create(true);
    h.github.with(|g| g.issue_times_out = true);
    let req = new_issue("r-1", MapRef::seam("web-server"));
    let (status, r) = h.create_issue(&req).await;
    assert_eq!(status, 202, "{r:?}");
    assert_eq!(r.state, CreationState::Opening);
    assert!(r.message.unwrap().contains("may exist"));
    let (status, r) = h.create_issue(&req).await;
    assert_eq!(status, 202);
    assert!(r.message.unwrap().starts_with("Still opening"));
    assert_eq!(h.creates(), 1, "a resubmit while opening sends nothing");

    tasks::poll_once(&h.app).await.unwrap();
    let (status, r) = h.create_issue(&req).await;
    assert_eq!(status, 200);
    assert_eq!(r.state, CreationState::Created);
    let issue = r.issue.unwrap();
    let ov = h.overview("m0x6A").await;
    assert!(
        ov.issues.iter().any(|i| i.issue == issue),
        "found and pinned"
    );
    assert_eq!(h.creates(), 1);
}

#[tokio::test]
async fn a_create_unseen_for_ten_minutes_fails() {
    use rubato_protocol::CreationState;
    let h = harness().await;
    h.can_create(true);
    // GitHub answers with an error and creates nothing.
    h.github
        .with(|g| g.fail_write = Some(("create_issue", "connection reset".into())));
    let req = new_issue("r-1", MapRef::node("web"));
    let (status, r) = h.create_issue(&req).await;
    assert_eq!((status, r.state), (202, CreationState::Opening));
    // Within ten minutes the listing does not give up on it.
    tasks::poll_once(&h.app).await.unwrap();
    assert_eq!(h.create_issue(&req).await.1.state, CreationState::Opening);
    sqlx::query("UPDATE issue_creations SET created_at = '2026-01-01T00:00:00+00:00'")
        .execute(&h.app.db)
        .await
        .unwrap();
    tasks::poll_once(&h.app).await.unwrap();
    let (status, r) = h.create_issue(&req).await;
    assert_eq!((status, r.state), (409, CreationState::Failed));
    assert_eq!(h.creates(), 1);
}
