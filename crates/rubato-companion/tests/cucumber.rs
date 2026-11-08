//! Gherkin suite for the companion. Real tmux on a private server (`TMUX_TMPDIR`), real git in
//! temporary repositories, real tmux-ide, a stand-in `claude` script, and a test double of the
//! rubato-server end of the websocket. No mocks of tmux or git.

use cucumber::{World, given, then, when};
use rubato_companion::config::{CompanionConfig, MapLocation, State, Workspace, WorkspaceRepo};
use rubato_companion::daemon::{DaemonConfig, Handles, start};
use rubato_companion::sessions::Registry;
use rubato_companion::tmux::{Tmux, session_name};
use rubato_companion::tmux_ide::{Fleet, TmuxIde, default_worktree_path, version_at_least};
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes,
};
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, IssueRef, LaunchCommand, LaunchResult, Limits,
    ServerEnvelope, ServerMessage, SessionStatus, StateSource,
};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, broadcast, watch};
use uuid::Uuid;

// ----- test double of rubato-server --------------------------------------------------------

#[derive(Clone)]
struct TestServer {
    url: String,
    user: String,
    token: String,
    received: Arc<Mutex<Vec<CompanionEnvelope>>>,
    notify: Arc<Notify>,
    push: broadcast::Sender<ServerEnvelope>,
    seq: Arc<std::sync::atomic::AtomicU64>,
    limits: Arc<std::sync::Mutex<Limits>>,
}

impl TestServer {
    async fn start(user: &str) -> Self {
        use axum::extract::State as AxState;
        use axum::extract::ws::{Message, WebSocketUpgrade};
        use axum::http::HeaderMap;
        use axum::routing::{get, post};
        use axum::{Json, Router};
        use futures_util::{SinkExt, StreamExt};
        let token = format!("test-token-{}", Uuid::new_v4());
        let (push, _) = broadcast::channel::<ServerEnvelope>(64);
        let srv = Self {
            url: String::new(),
            user: user.to_owned(),
            token: token.clone(),
            received: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            push,
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            limits: Arc::new(std::sync::Mutex::new(Limits::default())),
        };
        let st = srv.clone();
        let app = Router::new()
            .route(routes::PAIR_START, post(|Json(_req): Json<PairStartRequest>| async move {
                Json(PairStartResponse { device_code: "dc-test".into(), user_code: "ABCD-1234".into(), verification_url: "http://test/pair".into(), interval_secs: 0, expires_in_secs: 30 })
            }))
            .route(routes::PAIR_POLL, post({
                let st = st.clone();
                move |Json(req): Json<PairPollRequest>| {
                    let st = st.clone();
                    async move {
                        if req.device_code == "dc-test" { Json(PairPollResponse::Approved { token: st.token.clone(), user: st.user.clone() }) } else { Json(PairPollResponse::Denied) }
                    }
                }
            }))
            .route(routes::COMPANION_WS, get({
                move |AxState(st): AxState<TestServer>, headers: HeaderMap, ws: WebSocketUpgrade| async move {
                    let ok = headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {}", st.token));
                    if !ok {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    ws.on_upgrade(move |socket| async move {
                        let (mut sink, mut stream) = socket.split();
                        let mut rx = st.push.subscribe();
                        loop {
                            tokio::select! {
                                Some(Ok(msg)) = stream.next() => {
                                    if let Message::Text(t) = msg && let Ok(env) = serde_json::from_str::<CompanionEnvelope>(&t) {
                                        let is_hello = matches!(env.message, CompanionMessage::Hello { .. });
                                        st.received.lock().await.push(env);
                                        st.notify.notify_waiters();
                                        if is_hello {
                                            let seq = st.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                                            let welcome = ServerEnvelope { seq, sent_at: chrono::Utc::now(), message: ServerMessage::Welcome { server_version: "test".into(), protocol_version: 1, last_seen_companion_seq: 0, limits: *st.limits.lock().unwrap() } };
                                            let _ = sink.send(Message::Text(serde_json::to_string(&welcome).unwrap().into())).await;
                                        }
                                    }
                                }
                                Ok(env) = rx.recv() => {
                                    let _ = sink.send(Message::Text(serde_json::to_string(&env).unwrap().into())).await;
                                }
                                else => break,
                            }
                        }
                    })
                }
            }))
            .with_state(st);
        use axum::response::IntoResponse;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url: format!("http://{addr}"),
            ..srv
        }
    }

    fn push(&self, msg: ServerMessage) {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let _ = self.push.send(ServerEnvelope {
            seq,
            sent_at: chrono::Utc::now(),
            message: msg,
        });
    }

    /// Wait until some received message satisfies `pred`, returning it.
    async fn wait_for<T>(
        &self,
        timeout: Duration,
        mut pred: impl FnMut(&CompanionMessage) -> Option<T>,
    ) -> Option<T> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let got = self.received.lock().await;
                if let Some(v) = got.iter().find_map(|e| pred(&e.message)) {
                    return Some(v);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), self.notify.notified()).await;
        }
    }

    async fn count(&self, pred: impl Fn(&CompanionMessage) -> bool) -> usize {
        self.received
            .lock()
            .await
            .iter()
            .filter(|e| pred(&e.message))
            .count()
    }
}

// ----- the world -----------------------------------------------------------------------------

#[derive(World)]
#[world(init = Self::new)]
struct RubatoWorld {
    root: PathBuf,
    tmux_tmpdir: PathBuf,
    state_dir: PathBuf,
    fake_bin: PathBuf,
    server: Option<TestServer>,
    workspace: Workspace,
    repos: HashMap<String, PathBuf>,
    handles: Option<Handles>,
    shutdown: Option<watch::Sender<bool>>,
    run_error: Option<String>,
    last_launch: Option<LaunchCommand>,
    last_status: Option<u16>,
    started_worktree: Option<PathBuf>,
    registry: Option<Registry>,
    fleet: Option<Fleet>,
    team_result: Option<anyhow::Result<(Fleet, String)>>,
}

impl std::fmt::Debug for RubatoWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RubatoWorld")
            .field("root", &self.root)
            .finish()
    }
}

static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl RubatoWorld {
    async fn new() -> Self {
        // Unix socket paths are short-limited; keep the tmux socket dir under /tmp.
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let root = PathBuf::from(format!("/tmp/rubato-test-{}-{n}", std::process::id()));
        let tmux_tmpdir = root.join("tmux");
        let state_dir = root.join("state");
        let fake_bin = root.join("bin");
        std::fs::create_dir_all(&tmux_tmpdir).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(&fake_bin).unwrap();
        // The stand-in claude: a script that records argv and env, then execs a copy of this very
        // companion binary named `claude` running its hidden `idle` command, so tmux reports the
        // pane's command as `claude` (a script shows as `sh`). Not coreutils: on Ubuntu 26.04
        // `sleep` is a multicall binary that dispatches on its own name and exits as `claude`.
        let real = fake_bin.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_rubato-companion"), real.join("claude")).unwrap();
        let script = format!(
            "#!/bin/sh\n{{ echo \"ARGS: $*\"; env | grep -E '^(OTEL_|CLAUDE_|RUBATO_)' ; }} > \"$PWD/.claude-standin.log\" 2>&1\nexec '{}' idle\n",
            real.join("claude").display()
        );
        let claude = fake_bin.join("claude");
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let workspace = Workspace {
            server_url: None,
            map: MapLocation {
                repo: "m0x6A/rubato".into(),
                path: "docs/design/rubato-map.yml".into(),
                local: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
                decisions: None,
            },
            repos: vec![],
            otel: Some(rubato_companion::config::OtelConfig {
                endpoint: "http://127.0.0.1:4318".into(),
                protocol: Some("http/protobuf".into()),
            }),
            companion: CompanionConfig {
                claude_command: claude.to_string_lossy().into_owned(),
                tmux_tmpdir: Some(tmux_tmpdir.clone()),
                limits: Limits {
                    heartbeat_interval_secs: 1,
                    team_poll_interval_secs: 1,
                    ..Limits::default()
                },
                pricing: BTreeMap::new(),
                ..CompanionConfig::default()
            },
        };
        Self {
            root,
            tmux_tmpdir,
            state_dir,
            fake_bin,
            server: None,
            workspace,
            repos: HashMap::new(),
            handles: None,
            shutdown: None,
            run_error: None,
            last_launch: None,
            last_status: None,
            started_worktree: None,
            registry: None,
            fleet: None,
            team_result: None,
        }
    }

    fn tmux(&self) -> Tmux {
        Tmux::new("tmux", Some(self.tmux_tmpdir.clone()))
    }

    fn server(&self) -> &TestServer {
        self.server
            .as_ref()
            .expect("a test server; add the Given step")
    }

    async fn ensure_server(&mut self, user: &str) {
        if self.server.is_none() {
            self.server = Some(TestServer::start(user).await);
        }
    }

    fn git(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn make_repo(&mut self, name: &str) -> PathBuf {
        let short = name.rsplit('/').next().unwrap();
        let origin = self.root.join("origins").join(format!("{name}.git"));
        std::fs::create_dir_all(&origin).unwrap();
        Self::git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let clone = self.root.join("repos").join(short);
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        Self::git(
            clone.parent().unwrap(),
            &["init", "-q", "-b", "main", short],
        );
        std::fs::write(clone.join("README.md"), format!("# {name}\n")).unwrap();
        Self::git(&clone, &["add", "-A"]);
        Self::git(&clone, &["commit", "-q", "-m", "init"]);
        Self::git(
            &clone,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        Self::git(&clone, &["push", "-q", "-u", "origin", "main"]);
        self.workspace.repos.push(WorkspaceRepo {
            name: name.to_owned(),
            local: clone.clone(),
        });
        self.repos.insert(name.to_owned(), clone.clone());
        clone
    }

    fn launch_cmd(&self, user: &str, repo: &str, issue: u64, slug: &str) -> LaunchCommand {
        let branch = format!("issue/{issue}-{slug}");
        let mut otel = BTreeMap::new();
        otel.insert("rubato.repo".into(), repo.to_owned());
        otel.insert("rubato.issue".into(), format!("{repo}#{issue}"));
        otel.insert("rubato.branch".into(), branch.clone());
        otel.insert("rubato.node".into(), "rubato-companion".into());
        LaunchCommand {
            id: Uuid::new_v4(),
            user: user.to_owned(),
            issue: IssueRef::new(repo, issue),
            repo_url: self
                .repos
                .get(repo)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            branch,
            base_ref: "origin/main".into(),
            node_ids: vec!["rubato-companion".into()],
            spend_cap_usd: 25.0,
            attempt_limit: 3,
            attempt: 1,
            session_id: Uuid::new_v4(),
            prompt: format!("Work on {repo}#{issue}."),
            otel_resource: otel,
            map_version: Some("test".into()),
        }
    }

    async fn start_daemon(&mut self) {
        let state = State::load(&self.state_dir).unwrap().expect("paired");
        let (tx, rx) = watch::channel(false);
        let (handles, _join) = start(
            DaemonConfig {
                workspace: self.workspace.clone(),
                state,
                state_dir: self.state_dir.clone(),
            },
            rx,
        )
        .await
        .unwrap();
        self.handles = Some(handles);
        self.shutdown = Some(tx);
    }

    async fn post_hook(&self, fixture: &str, with_token: bool) -> u16 {
        let h = self.handles.as_ref().expect("daemon running");
        let body = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("features/fixtures/hooks")
                .join(fixture),
        )
        .unwrap();
        // Exactly what Claude Code's hook command does: curl with the bearer and the stdin body.
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "-m",
            "2",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            &format!("http://127.0.0.1:{}/hook", h.hook_endpoint.port),
        ]);
        if with_token {
            cmd.args([
                "-H",
                &format!("Authorization: Bearer {}", h.hook_endpoint.token),
            ]);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }
}

impl Drop for RubatoWorld {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(true);
        }
        self.tmux().kill_server();
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = self.fake_bin;
    }
}

// ----- steps -----------------------------------------------------------------------------------

#[given(expr = "a test server that approves pairing for user {string}")]
async fn given_server(w: &mut RubatoWorld, user: String) {
    w.ensure_server(&user).await;
}

#[given("no state file")]
async fn given_no_state(w: &mut RubatoWorld) {
    let _ = std::fs::remove_file(State::path(&w.state_dir));
}

#[given(expr = "a paired companion for user {string}")]
async fn given_paired(w: &mut RubatoWorld, user: String) {
    w.ensure_server(&user).await;
    let s = w.server().clone();
    State {
        companion_id: Uuid::new_v4(),
        server_url: s.url.clone(),
        user,
        pairing_token: s.token.clone(),
        last_seq: 0,
        last_server_seq: 0,
    }
    .save(&w.state_dir)
    .unwrap();
}

#[given(
    expr = "a paired companion for user {string} with a session stale threshold of {int} second(s)"
)]
async fn given_paired_stale(w: &mut RubatoWorld, user: String, secs: u64) {
    given_paired(w, user).await;
    w.workspace.companion.limits.session_stale_secs = secs;
    w.server().limits.lock().unwrap().session_stale_secs = secs;
}

#[given(expr = "a temporary repository {string} with a bare origin")]
async fn given_repo(w: &mut RubatoWorld, name: String) {
    w.make_repo(&name);
}

#[given("the companion daemon is running and connected")]
async fn given_daemon(w: &mut RubatoWorld) {
    w.start_daemon().await;
    let mut connected = w.handles.as_ref().unwrap().connected.clone();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !*connected.borrow() {
            connected.changed().await.unwrap();
        }
    })
    .await
    .expect("daemon connected to the test server");
}

#[given(
    expr = "the server has launched issue {int} of {string} with slug {string} for user {string}"
)]
async fn given_launched(w: &mut RubatoWorld, issue: u64, repo: String, slug: String, user: String) {
    when_launch(w, user, issue, repo, slug).await;
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started { command_id, .. })
                if *command_id == id =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(got.is_some(), "first launch started");
}

#[given(
    expr = "a worktree for branch {string} of {string} with a tmux pane running the stand-in claude in it"
)]
async fn given_worktree_with_pane(w: &mut RubatoWorld, branch: String, repo: String) {
    let clone = w.repos.get(&repo).cloned().expect("repo created");
    let wt = default_worktree_path(&clone, &branch);
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    RubatoWorld::git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &branch,
            &wt.to_string_lossy(),
            "main",
        ],
    );
    let claude = w.workspace.companion.claude_command.clone();
    w.tmux()
        .new_session(&session_name(&repo, &branch), &wt, &claude)
        .unwrap();
}

#[given("a private tmux server")]
async fn given_private_tmux(w: &mut RubatoWorld) {
    w.tmux().new_session("probe", &w.root, "sh").unwrap();
}

#[given(expr = "the recorded tmux-ide fleet {string}")]
async fn given_fleet(w: &mut RubatoWorld, file: String) {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("features/fixtures")
            .join(file),
    )
    .unwrap();
    w.fleet = Some(Fleet::parse(&text).unwrap());
}

#[when("the companion pairs with the server")]
async fn when_pair(w: &mut RubatoWorld) {
    let s = w.server().clone();
    let paired = rubato_companion::pairing::pair(
        &s.url,
        Uuid::new_v4(),
        "testhost",
        "0.1.0-test",
        |start| assert_eq!(start.user_code, "ABCD-1234"),
    )
    .await
    .unwrap();
    State {
        companion_id: Uuid::new_v4(),
        server_url: s.url.clone(),
        user: paired.user,
        pairing_token: paired.token,
        last_seq: 0,
        last_server_seq: 0,
    }
    .save(&w.state_dir)
    .unwrap();
}

#[when("the companion is asked to run")]
async fn when_run_unpaired(w: &mut RubatoWorld) {
    w.run_error = match State::load(&w.state_dir).unwrap() {
        None => Some("not paired: run `rubato-companion pair --server <url>` first".into()),
        Some(_) => None,
    };
}

#[when("the companion daemon starts")]
async fn when_daemon_starts(w: &mut RubatoWorld) {
    w.start_daemon().await;
}

#[when(
    expr = "the server sends a launch for user {string} on issue {int} of {string} with slug {string}"
)]
async fn when_launch(w: &mut RubatoWorld, user: String, issue: u64, repo: String, slug: String) {
    let cmd = w.launch_cmd(&user, &repo, issue, &slug);
    w.last_launch = Some(cmd.clone());
    w.server().push(ServerMessage::Launch(cmd));
}

#[when(expr = "Claude Code posts the recorded hook {string} to the companion")]
async fn when_post_hook(w: &mut RubatoWorld, fixture: String) {
    w.last_status = Some(w.post_hook(&fixture, true).await);
    assert_eq!(w.last_status, Some(202));
}

#[when(expr = "something posts the recorded hook {string} without the bearer token")]
async fn when_post_hook_no_token(w: &mut RubatoWorld, fixture: String) {
    w.last_status = Some(w.post_hook(&fixture, false).await);
}

#[when(expr = "{int} seconds pass")]
async fn when_seconds_pass(_w: &mut RubatoWorld, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

#[when("the companion runs tmux-ide team --json")]
async fn when_team(w: &mut RubatoWorld) {
    let ti = TmuxIde::new("tmux-ide", w.tmux());
    w.team_result = Some(
        tokio::task::spawn_blocking(move || Ok((ti.team()?, ti.version()?)))
            .await
            .unwrap(),
    );
}

#[when("the companion merges it into an empty registry")]
async fn when_merge_fleet(w: &mut RubatoWorld) {
    let mut reg = Registry::new(Uuid::nil(), "m0x6A");
    reg.apply_fleet(w.fleet.as_ref().unwrap(), chrono::Utc::now());
    w.registry = Some(reg);
}

#[then("the state file holds the pairing token with mode 0600")]
async fn then_state_private(w: &mut RubatoWorld) {
    use std::os::unix::fs::PermissionsExt;
    let p = State::path(&w.state_dir);
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let s = State::load(&w.state_dir).unwrap().unwrap();
    assert_eq!(s.pairing_token, w.server().token);
}

#[then(expr = "the state file names user {string}")]
async fn then_state_user(w: &mut RubatoWorld, user: String) {
    assert_eq!(State::load(&w.state_dir).unwrap().unwrap().user, user);
}

#[then(expr = "it fails with an error mentioning {string}")]
async fn then_fails(w: &mut RubatoWorld, word: String) {
    let e = w.run_error.clone().expect("an error");
    assert!(e.contains(&word), "{e}");
}

#[then(expr = "the server receives a hello for user {string} with protocol version {int}")]
async fn then_hello(w: &mut RubatoWorld, user: String, version: u32) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hello {
                user: u,
                protocol_version,
                ..
            } if *u == user && *protocol_version == version => Some(()),
            _ => None,
        })
        .await;
    assert!(got.is_some(), "no matching hello");
}

#[then("the server receives a heartbeat naming the tmux-ide version")]
async fn then_heartbeat(w: &mut RubatoWorld) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Heartbeat(h) => h.tmux_ide_version.clone(),
            _ => None,
        })
        .await;
    let v = got.expect("a heartbeat with tmux-ide version");
    assert!(version_at_least(&v, "2.9.0"), "{v}");
}

#[then(expr = "the server receives a launch result {string} with reason {string}")]
async fn then_launch_refused(w: &mut RubatoWorld, outcome: String, reason: String) {
    assert_eq!(outcome, "refused");
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(30), |m| match m {
            CompanionMessage::Launch(LaunchResult::Refused {
                command_id,
                reason: r,
                ..
            }) if *command_id == id => Some(
                serde_json::to_value(r)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            ),
            CompanionMessage::Launch(LaunchResult::Failed {
                command_id, error, ..
            }) if *command_id == id => Some(format!("FAILED: {error}")),
            _ => None,
        })
        .await;
    assert_eq!(got.as_deref(), Some(reason.as_str()));
}

#[then(expr = "the server receives a launch result {string}")]
async fn then_launch_started(w: &mut RubatoWorld, outcome: String) {
    assert_eq!(outcome, "started");
    let id = w.last_launch.as_ref().unwrap().id;
    let got = w
        .server()
        .wait_for(Duration::from_secs(60), |m| match m {
            CompanionMessage::Launch(LaunchResult::Started {
                command_id,
                worktree,
                ..
            }) if *command_id == id => Some(Ok(PathBuf::from(worktree))),
            CompanionMessage::Launch(LaunchResult::Failed {
                command_id, error, ..
            }) if *command_id == id => Some(Err(error.clone())),
            CompanionMessage::Launch(LaunchResult::Refused {
                command_id, detail, ..
            }) if *command_id == id => Some(Err(detail.clone())),
            _ => None,
        })
        .await;
    match got {
        Some(Ok(wt)) => w.started_worktree = Some(wt),
        other => panic!("launch did not start: {other:?}"),
    }
}

fn worktree_of(w: &RubatoWorld, repo: &str, branch: &str) -> PathBuf {
    default_worktree_path(w.repos.get(repo).unwrap(), branch)
}

#[then(expr = "no worktree exists for branch {string} of {string}")]
async fn then_no_worktree(w: &mut RubatoWorld, branch: String, repo: String) {
    let list = rubato_companion::gitx::worktrees(w.repos.get(&repo).unwrap()).unwrap();
    assert!(
        !list
            .iter()
            .any(|(_, b)| b.as_deref() == Some(branch.as_str())),
        "{list:?}"
    );
    assert!(!worktree_of(w, &repo, &branch).exists());
}

#[then(expr = "no tmux session exists for {string} branch {string}")]
async fn then_no_session(w: &mut RubatoWorld, repo: String, branch: String) {
    assert!(!w.tmux().has_session(&session_name(&repo, &branch)));
}

#[then(expr = "a worktree exists for branch {string} of {string} beside the clone")]
async fn then_worktree(w: &mut RubatoWorld, branch: String, repo: String) {
    let wt = worktree_of(w, &repo, &branch);
    assert_eq!(
        w.started_worktree.as_deref(),
        Some(wt.as_path()),
        "reported worktree is the sibling path"
    );
    assert!(wt.is_dir(), "{}", wt.display());
    assert_eq!(
        rubato_companion::gitx::current_branch(&wt).as_deref(),
        Some(branch.as_str())
    );
}

#[then(expr = "a tmux session exists for {string} branch {string} with a pane in that worktree")]
async fn then_session(w: &mut RubatoWorld, repo: String, branch: String) {
    let name = session_name(&repo, &branch);
    assert!(w.tmux().has_session(&name), "no tmux session {name}");
    let wt = worktree_of(w, &repo, &branch);
    let panes = w.tmux().panes().unwrap();
    assert!(
        panes.iter().any(|p| p.session == name && p.cwd == wt),
        "{panes:?}"
    );
}

async fn standin_log(w: &RubatoWorld) -> String {
    let cmd = w.last_launch.as_ref().unwrap();
    let wt = worktree_of(w, &cmd.issue.repo, &cmd.branch);
    let log = wt.join(".claude-standin.log");
    for _ in 0..50 {
        if let Ok(t) = std::fs::read_to_string(&log)
            && t.contains("ARGS:")
        {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("stand-in claude did not write {}", log.display());
}

#[then("the stand-in claude was started with the session id from the command")]
async fn then_session_id(w: &mut RubatoWorld) {
    let log = standin_log(w).await;
    let id = w.last_launch.as_ref().unwrap().session_id.to_string();
    assert!(log.contains(&format!("--session-id {id}")), "{log}");
}

#[then(expr = "the stand-in claude received {string} in OTEL_RESOURCE_ATTRIBUTES")]
async fn then_otel_attr(w: &mut RubatoWorld, attr: String) {
    let log = standin_log(w).await;
    let line = log
        .lines()
        .find(|l| l.starts_with("OTEL_RESOURCE_ATTRIBUTES="))
        .expect("OTEL_RESOURCE_ATTRIBUTES set");
    assert!(line.contains(&attr), "{line}");
    assert!(log.contains("CLAUDE_CODE_ENABLE_TELEMETRY=1"));
    assert!(log.contains("OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf"));
}

#[then("the stand-in claude received a settings file whose hooks post to the companion's port")]
async fn then_settings(w: &mut RubatoWorld) {
    let log = standin_log(w).await;
    let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
    let path = args
        .split_whitespace()
        .skip_while(|a| *a != "--settings")
        .nth(1)
        .expect("--settings <file>");
    let text = std::fs::read_to_string(path).unwrap();
    let port = w.handles.as_ref().unwrap().hook_endpoint.port;
    assert!(
        text.contains(&format!("http://127.0.0.1:{port}/hook")),
        "{text}"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600,
        "hooks file holds the token"
    );
}

#[then(
    expr = "the server receives a sessions snapshot with issue {int} of {string} from source {string}"
)]
async fn then_snapshot_source(w: &mut RubatoWorld, issue: u64, repo: String, source: String) {
    let want = IssueRef::new(repo, issue);
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.issue.as_ref() == Some(&want))
                .map(|s| {
                    serde_json::to_value(s.source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                }),
            _ => None,
        })
        .await;
    assert_eq!(got.as_deref(), Some(source.as_str()));
}

#[then(
    expr = "the server receives a sessions snapshot with issue {int} of {string} on branch {string}"
)]
async fn then_snapshot_branch(w: &mut RubatoWorld, issue: u64, repo: String, branch: String) {
    let want = IssueRef::new(repo, issue);
    let got = w
        .server()
        .wait_for(Duration::from_secs(15), |m| match m {
            CompanionMessage::Sessions { sessions } => sessions
                .iter()
                .find(|s| s.issue.as_ref() == Some(&want))
                .cloned(),
            _ => None,
        })
        .await;
    let s = got.expect("snapshot with the issue");
    assert_eq!(s.branch.as_deref(), Some(branch.as_str()));
}

#[then(expr = "that snapshot entry has source {string} and status {string}")]
async fn then_snapshot_entry(w: &mut RubatoWorld, source: String, status: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(5), |m| match m {
            CompanionMessage::Sessions { sessions } => {
                sessions.iter().find(|s| s.issue.is_some()).map(|s| {
                    (
                        serde_json::to_value(s.source).unwrap(),
                        serde_json::to_value(s.status).unwrap(),
                    )
                })
            }
            _ => None,
        })
        .await
        .unwrap();
    assert_eq!(got.0.as_str(), Some(source.as_str()));
    assert_eq!(got.1.as_str(), Some(status.as_str()));
}

#[then(expr = "the server receives a hook event {string} for session {string}")]
async fn then_hook(w: &mut RubatoWorld, event: String, session: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Hook(h)
                if h.session_id == session
                    && serde_json::to_value(h.hook_event_name).unwrap().as_str()
                        == Some(event.as_str()) =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(got.is_some(), "no hook {event} for {session}");
}

#[then(expr = "the companion reports that session with status {string} from source {string}")]
async fn then_status(w: &mut RubatoWorld, status: String, source: String) {
    let reg = w.handles.as_ref().unwrap().registry.lock().await;
    let e = reg
        .by_session("0f8fad5b-d9cb-469f-a165-70867728950e")
        .expect("session known");
    assert_eq!(
        serde_json::to_value(e.status).unwrap().as_str(),
        Some(status.as_str())
    );
    assert_eq!(
        serde_json::to_value(e.source).unwrap().as_str(),
        Some(source.as_str())
    );
}

#[then(expr = "the companion reports that session with effective status {string}")]
async fn then_effective(w: &mut RubatoWorld, status: String) {
    let stale = w.workspace.companion.limits.session_stale_secs;
    let s = w
        .handles
        .as_ref()
        .unwrap()
        .status_of("0f8fad5b-d9cb-469f-a165-70867728950e", stale)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(s).unwrap().as_str(),
        Some(status.as_str())
    );
}

#[then(expr = "the companion answers {int}")]
async fn then_answers(w: &mut RubatoWorld, code: u16) {
    assert_eq!(w.last_status, Some(code));
}

#[then(expr = "the server receives no hook event within {int} second(s)")]
async fn then_no_hook(w: &mut RubatoWorld, secs: u64) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(secs), |m| {
            matches!(m, CompanionMessage::Hook(_)).then_some(())
        })
        .await;
    assert!(got.is_none(), "a hook was forwarded");
}

#[then(expr = "the server receives a stale event for session {string}")]
async fn then_stale(w: &mut RubatoWorld, session: String) {
    let got = w
        .server()
        .wait_for(Duration::from_secs(10), |m| match m {
            CompanionMessage::Stale(s) if s.session_id.as_deref() == Some(session.as_str()) => {
                Some(s.silent_for_secs)
            }
            _ => None,
        })
        .await;
    assert!(got.is_some_and(|s| s >= 1), "{got:?}");
}

#[then(expr = "the server receives exactly {int} stale event(s) within {int} more second(s)")]
async fn then_stale_count(w: &mut RubatoWorld, n: usize, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    assert_eq!(
        w.server()
            .count(|m| matches!(m, CompanionMessage::Stale(_)))
            .await,
        n
    );
}

#[then(expr = "it parses and the tmux-ide version is at least {string}")]
async fn then_team_parses(w: &mut RubatoWorld, min: String) {
    let (fleet, version) = w.team_result.take().unwrap().expect("tmux-ide team --json");
    let _ = fleet.agents().count();
    assert!(version_at_least(&version, &min), "{version}");
}

#[then(expr = "pane {string} has status {string} from source {string}")]
async fn then_pane(w: &mut RubatoWorld, pane: String, status: String, source: String) {
    let reg = w.registry.as_ref().unwrap();
    let e = reg
        .entries()
        .iter()
        .find(|e| e.pane_id.as_deref() == Some(pane.as_str()))
        .expect("pane known");
    let want_status: SessionStatus =
        serde_json::from_value(serde_json::Value::String(status)).unwrap();
    let want_source: StateSource =
        serde_json::from_value(serde_json::Value::String(source)).unwrap();
    assert_eq!((e.status, e.source), (want_status, want_source));
}

#[tokio::main]
async fn main() {
    RubatoWorld::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .run_and_exit("features")
        .await;
}
