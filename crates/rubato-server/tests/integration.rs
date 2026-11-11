//! The server end to end: pairing, a companion replaying protocol fixtures over the websocket,
//! pins, GitHub rows from a fake, and the derived progress read back. Also the check that no
//! table stores progress.

use futures_util::{SinkExt, StreamExt};
use rubato_protocol::launch::routes as launch_routes;
use rubato_protocol::map::routes as map_routes;
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes as pr,
};
use rubato_protocol::pins::routes as pin_routes;
use rubato_protocol::progress::routes as overview_routes;
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
    let cfg = Config::for_tests(db_url, "m0x6A");
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
    // Recorded on main before the move (poller.rs:142-299), commit 88db1f8. Issue #48 adds one
    // listing per map repo before the issue loop, on purpose; nothing else moved.
    let expected = [
        "open_issues m0x6A/rubato",
        "issue m0x6A/rubato#31",
        "branches_with_prefix m0x6A/rubato issue/31-",
        "file m0x6A/rubato docs/plans/31.md issue/31-ok",
        "pull_for_branch m0x6A/rubato issue/31-ok",
        "checks m0x6A/rubato s31",
        "issue_comments m0x6A/rubato#31",
        "issue m0x6A/rubato#32",
        "branches_with_prefix m0x6A/rubato issue/32-",
        "file m0x6A/rubato docs/plans/32.md issue/32-bad",
        "pull_for_branch m0x6A/rubato issue/32-bad",
        "issue_comments m0x6A/rubato#32",
        "issue m0x6A/rubato#33",
        "branches_with_prefix m0x6A/rubato issue/33-",
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
    println!("checked {} GitHub calls over 3 issues", calls.len());

    // As a GitHub App installation, and only then, one more call: the installation's repos,
    // after the listing and before the issue loop.
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
    assert_eq!(
        calls, as_installation,
        "one coverage call, installation only"
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
