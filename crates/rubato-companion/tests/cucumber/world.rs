//! The shared World: a test double of rubato-server, temporary repos, a private tmux server and
//! the stand-in `claude`.

use cucumber::World;
use rubato_companion::hooks::{HOOKS_TEMPLATE, render_hooks};
use rubato_companion::kernel::config::{
    CompanionConfig, HookEndpoint, MapLocation, OtelConfig, State, Workspace, WorkspaceRepo,
};
use rubato_companion::kernel::gitx::default_worktree_path;
use rubato_companion::kernel::tmux::Tmux;
use rubato_companion::runtime::{DaemonConfig, Handles, start};
use rubato_protocol::pairing::{
    PairPollRequest, PairPollResponse, PairStartRequest, PairStartResponse, routes,
};
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, IssueRef, LaunchCommand, Limits, ServerEnvelope,
    ServerMessage, SessionMessage,
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
    /// What the Welcome announces; a scenario may take one away before the daemon starts.
    pub capabilities: Arc<std::sync::Mutex<Vec<String>>>,
    /// Context graph snapshot bodies the server serves, by source key (ADR 0086).
    pub snapshots: Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>>,
    /// Every snapshot request: the source asked for and whether it carried the pairing token.
    pub snapshot_asks: Arc<std::sync::Mutex<Vec<(String, bool)>>>,
}

impl TestServer {
    pub async fn start(user: &str) -> Self {
        Self::start_on(user, std::net::Ipv4Addr::LOCALHOST.into()).await
    }

    /// The same test server listening on `ip`, for a URL that is not loopback.
    pub async fn start_on(user: &str, ip: std::net::IpAddr) -> Self {
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
            // The Welcome replaces the companion's own limits, so it reconciles every second as the
            // suite's config says. tmux-ide's event stream used to wake reconcile and hid the 30 s
            // default; without it a pane is seen at the next reconcile (ADR 0070).
            limits: Arc::new(std::sync::Mutex::new(Limits {
                team_poll_interval_secs: 1,
                ..Limits::default()
            })),
            capabilities: Arc::new(std::sync::Mutex::new(vec![
                rubato_protocol::capability::SESSION_MESSAGES.into(),
                rubato_protocol::capability::DRAFTS.into(),
                rubato_protocol::capability::HANDOFF_REPORTS.into(),
                rubato_protocol::capability::ROOM_REPLIES.into(),
            ])),
            snapshots: Default::default(),
            snapshot_asks: Default::default(),
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
                                            let welcome = ServerEnvelope { seq, sent_at: chrono::Utc::now(), message: ServerMessage::Welcome { capabilities: st.capabilities.lock().unwrap().clone(), server_version: "test".into(), protocol_version: 1, last_seen_companion_seq: 0, limits: *st.limits.lock().unwrap() } };
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
            .route(rubato_protocol::graph::routes::GRAPH_SNAPSHOT, get(
                |AxState(st): AxState<TestServer>, headers: HeaderMap, axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>| async move {
                    let source = q.get("source").cloned().unwrap_or_default();
                    let ok = headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {}", st.token));
                    st.snapshot_asks.lock().unwrap().push((source.clone(), ok));
                    if !ok {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    match st.snapshots.lock().unwrap().get(&source) {
                        Some(body) => body.clone().into_response(),
                        None => axum::http::StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ))
            .with_state(st);
        use axum::response::IntoResponse;
        let listener = tokio::net::TcpListener::bind((ip, 0)).await.unwrap();
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
    /// What the test server announces as its context graph sources (ADR 0086).
    pub graph_announce: Vec<rubato_protocol::graph::GraphSourceInfo>,
    /// A lead's fleet request was checked against its launch (features/fleet.feature).
    pub fleet_request_checked: bool,
    pub last_status: Option<u16>,
    /// What `rubato-companion status` printed (ADR 0078).
    pub status_out: Option<String>,
    pub started_worktree: Option<PathBuf>,
    /// The last session message the server pushed (`session-messages.feature`).
    pub last_message: Option<SessionMessage>,
    /// An id recorded as `typing` in `deliveries.json` before the daemon started.
    pub typing_id: Option<Uuid>,
    /// The session id a launched session moved to.
    pub new_session_id: Option<String>,
    /// The hook endpoint before a restart, and a port held so the restart cannot take it back.
    pub endpoint_before: Option<HookEndpoint>,
    pub held_port: Option<std::net::TcpListener>,
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
        // A headless `-p` run (a room's fork turn, ADR 0057) records its args, its Rubato and tmux
        // environment and its stdin in .claude-fork.log, then prints stream-json: an init and a
        // result whose session is `fork-of-<resumed>` with --fork-session, else the resumed one,
        // at the cost in <root>/fork-cost (0.05 when absent).
        let fork = format!(
            "case \" $* \" in *\" -p \"*)\n  resume=''; fork=''; prev=''\n  for a in \"$@\"; do [ \"$prev\" = --resume ] && resume=\"$a\"; [ \"$a\" = --fork-session ] && fork=1; prev=\"$a\"; done\n  if [ -n \"$fork\" ]; then sid=\"fork-of-$resume\"; else sid=\"$resume\"; fi\n  {{ echo \"ARGS: $*\"; env | grep -E '^(RUBATO_|TMUX=)'; echo 'STDIN:'; cat; echo; echo 'END'; }} >> \"$PWD/.claude-fork.log\"\n  cost=$(cat '{}' 2>/dev/null || echo 0.05)\n  printf '%s\\n' \"{{\\\"type\\\":\\\"system\\\",\\\"subtype\\\":\\\"init\\\",\\\"session_id\\\":\\\"$sid\\\"}}\"\n  printf '%s\\n' \"{{\\\"type\\\":\\\"result\\\",\\\"subtype\\\":\\\"success\\\",\\\"is_error\\\":false,\\\"result\\\":\\\"b keeps it in one place\\\",\\\"session_id\\\":\\\"$sid\\\",\\\"total_cost_usd\\\":$cost}}\"\n  exit 0;;\nesac\n",
            root.join("fork-cost").display()
        );
        let script = format!(
            "#!/bin/sh\n{fork}{{ echo \"ARGS: $*\"; env | grep -E '^(OTEL_|CLAUDE_|RUBATO_)' ; }} > \"$PWD/.claude-standin.log.tmp\" 2>&1 && mv \"$PWD/.claude-standin.log.tmp\" \"$PWD/.claude-standin.log\"\nexec '{}' idle --record \"$PWD/.claude-standin.typed\"\n",
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
                // Never the developer's own skill: a scenario installs one here when it needs it.
                skill_dir: Some(root.join("skill")),
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
            graph_announce: Vec::new(),
            fleet_request_checked: false,
            last_status: None,
            status_out: None,
            started_worktree: None,
            last_message: None,
            typing_id: None,
            new_session_id: None,
            endpoint_before: None,
            held_port: None,
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
            plan_path: Some(format!("docs/plans/{issue}.md")),
            draft: None,
            planning: Default::default(),
            role: Default::default(),
            lead_launch_id: None,
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

    /// Stop the daemon as Ctrl-C does and wait until it has returned, so its hook port is free.
    pub async fn stop_daemon(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(true);
        }
        if let Some(join) = self.join.take() {
            tokio::time::timeout(Duration::from_secs(10), join)
                .await
                .expect("the daemon stops within 10 s")
                .unwrap()
                .unwrap();
        }
        self.handles = None;
    }

    /// Run the launched session's own hook command for `event`, exactly as Claude Code does:
    /// the command from the `--settings` file the stand-in got, through `sh`, with the payload
    /// on stdin and `RUBATO_LAUNCH_ID` exported as `launch.sh` does.
    /// `extra` adds payload keys, such as `prompt` or `last_assistant_message`.
    pub async fn fire_launched_hook(
        &self,
        event: &str,
        session_id: &str,
        extra: serde_json::Value,
    ) {
        let log = standin_log(self).await;
        let args = log.lines().find(|l| l.starts_with("ARGS:")).unwrap();
        let settings = args
            .split_whitespace()
            .skip_while(|a| *a != "--settings")
            .nth(1)
            .expect("--settings <file>");
        let hooks: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let command = hooks["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no {event} hook in {settings}"))
            .to_owned();
        let cmd = self.last_launch.as_ref().unwrap();
        let worktree = worktree_of(self, &cmd.issue.repo, &cmd.branch);
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "hook_event_name": event,
            "cwd": worktree.to_string_lossy(),
            "permission_mode": "acceptEdits",
        });
        if let (Some(p), serde_json::Value::Object(more)) = (payload.as_object_mut(), extra) {
            p.extend(more);
        }
        let mut sh = Command::new("sh");
        sh.arg("-c")
            .arg(&command)
            .env("RUBATO_LAUNCH_ID", cmd.id.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = sh.spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "hook command failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    pub fn hook_fixture(fixture: &str) -> String {
        std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("features/fixtures/hooks")
                .join(fixture),
        )
        .unwrap()
    }

    pub async fn post_hook(&self, fixture: &str, with_token: bool) -> u16 {
        self.post_body(&Self::hook_fixture(fixture), with_token)
            .await
    }

    /// The hook command Claude Code runs for `Stop`, rendered from the embedded template with the
    /// header file the daemon wrote to its state dir (ADR 0063).
    pub fn rendered_stop_command(&self) -> String {
        let h = self.handles.as_ref().expect("daemon running");
        let rendered = render_hooks(
            HOOKS_TEMPLATE,
            h.hook_endpoint.port,
            &HookEndpoint::header_path(&self.state_dir),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        v["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// As `post_hook`, with a body the step built: a hook from a session started by hand.
    pub async fn post_hook_body(&self, body: &str, with_token: bool) -> u16 {
        self.post_body(body, with_token).await
    }

    /// With the token: exactly what Claude Code does, the rendered hook command under `sh -c`
    /// with the body on stdin, plus `-w` to read the status. Without: a bare curl, no header.
    pub async fn post_body(&self, body: &str, with_token: bool) -> u16 {
        let h = self.handles.as_ref().expect("daemon running");
        let mut cmd = if with_token {
            let mut c = Command::new("sh");
            c.args([
                "-c",
                &format!(
                    "{} -o /dev/null -w '%{{http_code}}'",
                    self.rendered_stop_command()
                ),
            ]);
            c
        } else {
            let mut c = Command::new("curl");
            c.args([
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
            c
        };
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

/// What was typed into the stand-in claude's pane, line by line (`idle --record`).
pub fn typed(w: &RubatoWorld) -> String {
    let cmd = w.last_launch.as_ref().unwrap();
    let wt = worktree_of(w, &cmd.issue.repo, &cmd.branch);
    std::fs::read_to_string(wt.join(".claude-standin.typed")).unwrap_or_default()
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
