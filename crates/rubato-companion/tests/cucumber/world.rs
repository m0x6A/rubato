//! The shared World: a test double of rubato-server, temporary repos, a private tmux server and
//! the stand-in `claude`.

use cucumber::World;
use rubato_companion::kernel::config::{
    CompanionConfig, MapLocation, OtelConfig, State, Workspace, WorkspaceRepo,
};
use rubato_companion::kernel::tmux::Tmux;
use rubato_companion::kernel::tmux_ide::{Fleet, default_worktree_path};
use rubato_companion::runtime::{DaemonConfig, Handles, start};
use rubato_companion::sessions::Registry;
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes,
};
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, IssueRef, LaunchCommand, Limits, ServerEnvelope,
    ServerMessage,
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
pub struct TestServer {
    pub url: String,
    pub user: String,
    pub token: String,
    pub received: Arc<Mutex<Vec<CompanionEnvelope>>>,
    pub notify: Arc<Notify>,
    pub push: broadcast::Sender<ServerEnvelope>,
    pub seq: Arc<std::sync::atomic::AtomicU64>,
    pub limits: Arc<std::sync::Mutex<Limits>>,
}

impl TestServer {
    pub async fn start(user: &str) -> Self {
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

    pub fn push(&self, msg: ServerMessage) {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let _ = self.push.send(ServerEnvelope {
            seq,
            sent_at: chrono::Utc::now(),
            message: msg,
        });
    }

    /// Wait until some received message satisfies `pred`, returning it.
    pub async fn wait_for<T>(
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

    pub async fn count(&self, pred: impl Fn(&CompanionMessage) -> bool) -> usize {
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
pub struct RubatoWorld {
    pub root: PathBuf,
    pub tmux_tmpdir: PathBuf,
    pub state_dir: PathBuf,
    pub fake_bin: PathBuf,
    pub server: Option<TestServer>,
    pub workspace: Workspace,
    pub repos: HashMap<String, PathBuf>,
    pub handles: Option<Handles>,
    pub shutdown: Option<watch::Sender<bool>>,
    pub join: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    pub run_error: Option<String>,
    pub last_launch: Option<LaunchCommand>,
    pub last_status: Option<u16>,
    pub started_worktree: Option<PathBuf>,
    pub registry: Option<Registry>,
    pub fleet: Option<Fleet>,
    pub team_result: Option<anyhow::Result<(Fleet, String)>>,
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
    pub async fn new() -> Self {
        crate::logs::clear();
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
            map: Some(MapLocation {
                repo: "m0x6A/rubato".into(),
                path: "docs/design/rubato-map.yml".into(),
                local: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
                decisions: None,
            }),
            maps: vec![],
            repos: vec![],
            otel: Some(OtelConfig {
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
            join: None,
            run_error: None,
            last_launch: None,
            last_status: None,
            started_worktree: None,
            registry: None,
            fleet: None,
            team_result: None,
        }
    }

    pub fn tmux(&self) -> Tmux {
        Tmux::new("tmux", Some(self.tmux_tmpdir.clone()))
    }

    pub fn server(&self) -> &TestServer {
        self.server
            .as_ref()
            .expect("a test server; add the Given step")
    }

    pub async fn ensure_server(&mut self, user: &str) {
        if self.server.is_none() {
            self.server = Some(TestServer::start(user).await);
        }
    }

    pub fn git(cwd: &Path, args: &[&str]) {
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

    pub fn make_repo(&mut self, name: &str) -> PathBuf {
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

    pub fn launch_cmd(&self, user: &str, repo: &str, issue: u64, slug: &str) -> LaunchCommand {
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

    pub async fn start_daemon(&mut self) {
        let state = State::load(&self.state_dir).unwrap().expect("paired");
        let (tx, rx) = watch::channel(false);
        let (handles, join) = start(
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
        self.join = Some(join);
    }

    pub async fn post_hook(&self, fixture: &str, with_token: bool) -> u16 {
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

// ----- helpers shared by several features ------------------------------------------------

pub fn worktree_of(w: &RubatoWorld, repo: &str, branch: &str) -> PathBuf {
    default_worktree_path(w.repos.get(repo).unwrap(), branch)
}

pub async fn standin_log(w: &RubatoWorld) -> String {
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
