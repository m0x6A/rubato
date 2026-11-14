//! Composition root: the companion runtime. Owns the channels, spawns each slice's task with an
//! essential or best-effort flag, dispatches the select arms to the slices, and supervises the
//! tasks (design D6, D12):
//!
//! - essential (server link, hook server, heartbeat, reconcile, stale sweep, dropped counter,
//!   launch task, which handles launch commands one at a time off the dispatch loop, plan 40,
//!   session input task, which types session messages one at a time, and plan mirror task,
//!   which reads plan bundles off the hook path, plan 122):
//!   ending while shutdown is not requested logs `rubato.task.exited` and makes `run` return
//!   `Err`, which `main` turns into a non-zero exit;
//! - best-effort (tmux-ide events watcher): ending is normal on a machine with no adopted
//!   session; discovery runs on the timer;
//! - on shutdown every task is aborted and `Cancelled` is clean, so `run` returns `Ok` at once.
//!
//! At start (plan 122 §D1) the hook receiver binds the port and token the last run published
//! when it can, and the launches kept in `launches.json` come back; when the port changed they
//! come back `hooks_lost`, and `hook port changed` is logged at warn.

use crate::hooks::HookServer;
use crate::kernel::config::{HookEndpoint, State, Workspace};
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
use crate::kernel::tmux_ide::TmuxIde;
use crate::launch::Launcher;
use crate::plan_mirror::PlanMirrorTask;
use crate::server_link::{Client, Identity, Reservation};
use crate::session_input::{Deliveries, SessionInputTask};
use crate::sessions::Registry;
use crate::{hooks, launch, plan_mirror, server_link, session_input, sessions, spend, stale};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rubato_protocol::otel::event;
use rubato_protocol::{
    CompanionMessage, HandoffReport, HookEvent, LaunchCommand, ServerMessage, SessionMessage,
    SessionStatus, capability,
};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::{Id, JoinSet};

pub struct DaemonConfig {
    pub workspace: Workspace,
    pub state: State,
    pub state_dir: PathBuf,
}

/// Handles the tests use to observe the daemon.
pub struct Handles {
    pub connected: watch::Receiver<bool>,
    pub hook_endpoint: HookEndpoint,
    pub registry: Arc<Mutex<Registry>>,
}

pub async fn start(
    cfg: DaemonConfig,
    shutdown: watch::Receiver<bool>,
) -> Result<(Handles, tokio::task::JoinHandle<Result<()>>)> {
    let cfg = Arc::new(cfg);
    // Before any task starts: a state dir that cannot be written stops the companion here, with
    // the path in the error (plan 29 S1, O1).
    let seq = Reservation::open(&cfg.state_dir, Utc::now().timestamp().max(0) as u64)?;
    tracing::info!(
        start_seq = seq.start_seq(),
        reserved_to = seq.reserved_to(),
        "sequence reserved"
    );
    // Before any task starts too: a delivery record that cannot be read could type a message
    // twice, so it stops the companion with the path in the error.
    let deliveries = Deliveries::open(&cfg.state_dir)?;
    let (hook_tx, hook_rx) = mpsc::unbounded_channel::<HookEvent>();
    // Headless turns send their own hooks into the same channel (ADR 0059).
    let synthetic_hooks = hook_tx.clone();
    let (dropped_tx, dropped_rx) = mpsc::unbounded_channel::<&'static str>();
    let previous = HookEndpoint::load(&cfg.state_dir);
    let (handoff_tx, handoff_rx) = mpsc::unbounded_channel::<HandoffReport>();
    let server = HookServer::bind_reusing(previous.as_ref(), hook_tx, dropped_tx)
        .await?
        .with_handoff(handoff_tx);
    let hook_endpoint = server.endpoint.clone();
    hook_endpoint.save(&cfg.state_dir)?;
    let kept = previous.as_ref() == Some(&hook_endpoint);
    tracing::info!(
        port = hook_endpoint.port,
        kept,
        "hook receiver listening on 127.0.0.1"
    );

    let mut reg = Registry::new(cfg.state.companion_id, cfg.state.user.clone());
    let records = sessions::load_launches(&cfg.state_dir);
    if !kept && (previous.is_some() || !records.is_empty()) {
        tracing::warn!(
            old_port = previous.as_ref().map(|p| p.port),
            new_port = hook_endpoint.port,
            launches = records.len(),
            "hook port changed"
        );
    }
    if !records.is_empty() {
        let tmux = Tmux::new(
            cfg.workspace.companion.tmux_command.clone(),
            cfg.workspace.companion.tmux_tmpdir.clone(),
        );
        let panes = tokio::task::spawn_blocking(move || tmux.panes())
            .await
            .context("listing tmux panes")?;
        let panes = match panes {
            Ok(p) => Some(p),
            // A wedged tmux is not "no panes" (plan 40 N7): keep the launches, unchecked.
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "tmux did not answer; launches restored unchecked");
                None
            }
        };
        let found = records.len();
        let restored = reg.restore_launches(records, panes.as_deref(), !kept, Utc::now());
        tracing::info!(
            restored,
            dropped = found - restored,
            hooks_lost = !kept,
            "launches restored"
        );
        sessions::save_launches_if_changed(&mut reg, &cfg.state_dir);
    }
    let registry = Arc::new(Mutex::new(reg));
    let (connected_tx, connected_rx) = watch::channel(false);
    let handles = Handles {
        connected: connected_rx,
        hook_endpoint: hook_endpoint.clone(),
        registry: registry.clone(),
    };
    let join = tokio::spawn(run(
        cfg,
        seq,
        server,
        deliveries,
        hook_rx,
        synthetic_hooks,
        dropped_rx,
        handoff_rx,
        registry,
        connected_tx,
        shutdown,
    ));
    Ok((handles, join))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    cfg: Arc<DaemonConfig>,
    seq: Reservation,
    server: HookServer,
    deliveries: Deliveries,
    hook_rx: mpsc::UnboundedReceiver<HookEvent>,
    synthetic_hooks: mpsc::UnboundedSender<HookEvent>,
    dropped_rx: mpsc::UnboundedReceiver<&'static str>,
    handoff_rx: mpsc::UnboundedReceiver<HandoffReport>,
    registry: Arc<Mutex<Registry>>,
    connected_tx: watch::Sender<bool>,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let metrics = Arc::new(Metrics::new());
    hooks::seed_forwarded(&metrics, cfg.state.companion_id);
    let hook_endpoint = server.endpoint.clone();
    let tmux = Tmux::new(
        cfg.workspace.companion.tmux_command.clone(),
        cfg.workspace.companion.tmux_tmpdir.clone(),
    );
    let tmux_ide = TmuxIde::new(
        cfg.workspace.companion.tmux_ide_command.clone(),
        tmux.clone(),
    )
    .with_timeout(cfg.workspace.companion.command_timeout());
    let limits = Arc::new(std::sync::Mutex::new(cfg.workspace.companion.limits));
    // What the server announced in its last Welcome (ADR 0051).
    let server_capabilities = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    let (out_tx, out_rx) = mpsc::unbounded_channel::<CompanionMessage>();
    // Before any task can start a turn: a turn the previous run left running is killed first,
    // so two turns never share one transcript (plan 122 §D14).
    let headless = cfg.workspace.companion.is_headless().then(|| {
        let killed = sessions::reap_previous(&cfg.state_dir, &metrics);
        tracing::info!(
            killed,
            "headless mode; turns left by the previous run reaped"
        );
        sessions::Headless::new(
            sessions::HeadlessConfig {
                state_dir: cfg.state_dir.clone(),
                max_total: std::time::Duration::from_secs(
                    cfg.workspace.companion.headless_turn_max_secs,
                ),
                max_quiet: std::time::Duration::from_secs(
                    cfg.workspace.companion.headless_quiet_max_secs,
                ),
            },
            registry.clone(),
            synthetic_hooks.clone(),
            out_tx.clone(),
            metrics.clone(),
        )
    });
    let (in_tx, in_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let (poll_tx, poll_rx) = mpsc::unbounded_channel::<()>();
    let skill_dir = cfg.workspace.companion.skill_dir();
    let skill = crate::kernel::skill::installed_version(&skill_dir);
    tracing::info!(skill_dir = %skill_dir.display(), skill = ?skill, "rubato skill");
    let client = Client::new(
        cfg.state.server_url.clone(),
        Identity {
            companion_id: cfg.state.companion_id,
            user: cfg.state.user.clone(),
            version: crate::kernel::VERSION.to_owned(),
            token: cfg.state.pairing_token.clone(),
            capabilities: hello_capabilities(skill.as_deref(), cfg.workspace.companion.mode),
            client: cfg.workspace.companion.mode,
        },
        seq,
        cfg.state.last_server_seq,
    );

    let mut tasks = Tasks::default();
    tasks.spawn(
        "server_link",
        Kind::Essential,
        client.run(out_rx, in_tx, connected_tx, shutdown.clone()),
    );
    tasks.spawn("hooks", Kind::Essential, server.serve());
    tasks.spawn(
        "heartbeat",
        Kind::Essential,
        stale::heartbeat(
            out_tx.clone(),
            cfg.state.companion_id,
            cfg.state.user.clone(),
            metrics.clone(),
            tmux_ide.clone(),
            limits.clone(),
            shutdown.clone(),
        ),
    );
    tasks.spawn(
        "events",
        Kind::BestEffort,
        sessions::watch_events(tmux_ide.clone(), poll_tx.clone(), shutdown.clone()),
    );
    tasks.spawn(
        "reconcile",
        Kind::Essential,
        sessions::reconcile(
            out_tx.clone(),
            registry.clone(),
            metrics.clone(),
            tmux.clone(),
            tmux_ide.clone(),
            limits.clone(),
            poll_rx,
            shutdown.clone(),
        ),
    );
    tasks.spawn(
        "stale",
        Kind::Essential,
        stale::sweep(
            out_tx.clone(),
            registry.clone(),
            limits.clone(),
            shutdown.clone(),
        ),
    );
    tasks.spawn(
        "dropped",
        Kind::Essential,
        hooks::count_dropped(dropped_rx, metrics.clone()),
    );

    let launcher = Arc::new(Launcher {
        workspace: cfg.workspace.clone(),
        paired_user: cfg.state.user.clone(),
        state_dir: cfg.state_dir.clone(),
        hook_endpoint: hook_endpoint.clone(),
        tmux: tmux.clone(),
        tmux_ide: tmux_ide.clone(),
    });
    let (launch_tx, launch_rx) = mpsc::unbounded_channel::<LaunchCommand>();
    tasks.spawn(
        "launch",
        Kind::Essential,
        launch::run(
            launch_rx,
            launcher,
            registry.clone(),
            out_tx.clone(),
            poll_tx.clone(),
            headless.clone(),
        ),
    );
    let (input_tx, input_rx) = mpsc::unbounded_channel::<SessionMessage>();
    tasks.spawn(
        "session_input",
        Kind::Essential,
        session_input::run(
            input_rx,
            SessionInputTask {
                paired_user: cfg.state.user.clone(),
                registry: registry.clone(),
                out: out_tx.clone(),
                limits: limits.clone(),
                server_capabilities: server_capabilities.clone(),
                input: Arc::new(tmux.clone()),
                metrics: metrics.clone(),
                deliveries,
                skill_dir,
                headless: headless.clone(),
                tmux: Some(tmux.clone()),
                permission_mode: cfg.workspace.companion.permission_mode.clone(),
            },
        ),
    );
    let (mirror_tx, mirror_rx) = mpsc::unbounded_channel::<String>();
    tasks.spawn(
        "plan_mirror",
        Kind::Essential,
        plan_mirror::run(
            mirror_rx,
            PlanMirrorTask {
                registry: registry.clone(),
                out: out_tx.clone(),
                limits: limits.clone(),
                server_capabilities: server_capabilities.clone(),
                read_timeout: crate::kernel::proc::LOCAL_TIMEOUT,
                last_sent: HashMap::new(),
            },
        ),
    );
    let dispatch = Dispatch {
        cfg: &cfg,
        registry: &registry,
        out: &out_tx,
        metrics: &metrics,
        tmux: &tmux,
        limits: &limits,
        server_capabilities: &server_capabilities,
        launch: &launch_tx,
        session_input: &input_tx,
        plan_mirror: &mirror_tx,
        headless: headless.as_ref(),
    };
    let supervised = supervise(tasks, shutdown.clone());
    tokio::pin!(supervised);
    tokio::select! {
        r = &mut supervised => return r,
        () = dispatch.run(hook_rx, in_rx, handoff_rx, shutdown) => {}
    }
    supervised.await
}

/// What this companion announces in Hello (ADR 0051). The trim has shipped since #122 PR 1
/// (ADR 0052), session input since PR 3, the plan mirror since PR 5, the handoff since PR 7
/// (draft restore since PR 8):
/// `plan_handoff` with the `/handoff` route that forwards `HandoffReport`, and `skill:<version>`
/// when the installed skill names its version.
pub fn hello_capabilities(skill: Option<&str>, mode: rubato_protocol::ClientKind) -> Vec<String> {
    let mut caps = vec![
        capability::HOOK_ALLOWLIST.to_owned(),
        capability::SESSION_INPUT.to_owned(),
        capability::DRAFT_MIRROR.to_owned(),
        capability::PLAN_HANDOFF.to_owned(),
        capability::DRAFT_RESTORE.to_owned(),
        capability::SESSION_STOP.to_owned(),
    ];
    if mode == rubato_protocol::ClientKind::Headless {
        caps.push(capability::HEADLESS.to_owned());
    }
    if let Some(v) = skill {
        caps.push(format!("{}{v}", capability::SKILL_PREFIX));
    }
    caps
}

/// The select arms, each one call into a slice (design §9.1 keeps their order).
struct Dispatch<'a> {
    cfg: &'a DaemonConfig,
    registry: &'a Mutex<Registry>,
    out: &'a mpsc::UnboundedSender<CompanionMessage>,
    metrics: &'a Metrics,
    tmux: &'a Tmux,
    limits: &'a std::sync::Mutex<rubato_protocol::Limits>,
    server_capabilities: &'a std::sync::Mutex<Vec<String>>,
    /// The launch task's queue (plan 40 S1).
    launch: &'a mpsc::UnboundedSender<LaunchCommand>,
    /// The session input task's queue (plan 122 §D1).
    session_input: &'a mpsc::UnboundedSender<SessionMessage>,
    /// The plan mirror task's queue: session ids of settle hooks (ADR 0053).
    plan_mirror: &'a mpsc::UnboundedSender<String>,
    /// Headless mode: turns killed on a cap hit or a revoked pairing (ADR 0059).
    headless: Option<&'a sessions::Headless>,
}

impl Dispatch<'_> {
    async fn run(
        &self,
        mut hook_rx: mpsc::UnboundedReceiver<HookEvent>,
        mut in_rx: mpsc::UnboundedReceiver<ServerMessage>,
        mut handoff_rx: mpsc::UnboundedReceiver<HandoffReport>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                Some(ev) = hook_rx.recv() => {
                    let outcome = hooks::on_event(&ev, self.registry, self.out, self.metrics, self.cfg.state.companion_id, &self.cfg.state_dir, self.limits).await;
                    spend::after_hook(&ev, outcome, self.registry, self.out, &self.cfg.workspace.companion.pricing, self.tmux, self.headless).await;
                    plan_mirror::on_hook(&ev, self.plan_mirror);
                }
                // A `plan.ts handoff` report from `/handoff` (ADR 0055).
                Some(report) = handoff_rx.recv() => {
                    hooks::on_handoff(report, self.registry, self.out, self.metrics, self.server_capabilities).await;
                }
                Some(msg) = in_rx.recv() => match msg {
                    ServerMessage::Welcome { limits, capabilities, .. } => {
                        server_link::on_welcome(limits, capabilities, self.limits, self.server_capabilities);
                    }
                    // Queued, not awaited: hooks keep flowing while it runs.
                    ServerMessage::Launch(cmd) => {
                        let _ = self.launch.send(cmd);
                    }
                    // Queued too: one paste at a time, after the Welcome that came before it.
                    ServerMessage::SessionMessage(m) => {
                        let _ = self.session_input.send(m);
                    }
                    ServerMessage::Error { code, .. } if code == "auth" => {
                        tracing::error!("pairing token rejected; re-pair with `rubato-companion pair`");
                        // The owner revoked this companion: nothing it started may go on (ADR 0059).
                        if let Some(h) = self.headless {
                            let n = h.stop_all(crate::kernel::claude::KillReason::Revoke);
                            tracing::warn!(stopped = n, "pairing revoked: headless turns stopped");
                        }
                    }
                    _ => {}
                },
                else => break,
            }
        }
    }
}

impl Handles {
    /// Effective status of a session as the server would render it.
    pub async fn status_of(&self, session_id: &str, stale_secs: u64) -> Option<SessionStatus> {
        let reg = self.registry.lock().await;
        reg.by_session(session_id).map(|e| {
            let st = rubato_protocol::SessionState {
                session_id: e.session_id.clone(),
                companion_id: reg.companion_id,
                user: reg.user.clone(),
                tmux: None,
                cwd: String::new(),
                worktree: None,
                branch: None,
                repo: None,
                issue: None,
                status: e.status,
                source: e.source,
                last_seen: e.last_seen,
                spend_usd: None,
                branch_drift: None,
                blocked_by: None,
                launch_id: None,
                hooks_lost: false,
            };
            st.effective_status(Utc::now(), stale_secs)
        })
    }
}

// ----- supervision (design D12) --------------------------------------------------------------

/// An essential task ending while the companion runs is fatal; a best-effort one is noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Essential,
    BestEffort,
}

#[derive(Default)]
pub struct Tasks {
    set: JoinSet<Result<()>>,
    names: HashMap<Id, (&'static str, Kind)>,
}

impl Tasks {
    pub fn spawn(
        &mut self,
        name: &'static str,
        kind: Kind,
        fut: impl Future<Output = Result<()>> + Send + 'static,
    ) {
        let handle = self.set.spawn(fut);
        self.names.insert(handle.id(), (name, kind));
    }
}

/// A dropped sender counts as shutdown too.
fn shutdown_requested(rx: &watch::Receiver<bool>) -> bool {
    *rx.borrow() || rx.has_changed().is_err()
}

/// Await the tasks until shutdown. An essential task that returns or panics while shutdown is
/// not requested logs `rubato.task.exited` and returns `Err`. On shutdown every task is aborted
/// and `Cancelled` is clean. No `process::exit`: the Gherkin suite runs this in-process.
pub async fn supervise(mut tasks: Tasks, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    loop {
        if shutdown_requested(&shutdown) {
            tasks.set.abort_all();
            while let Some(r) = tasks.set.join_next_with_id().await {
                if let Err(e) = r
                    && !e.is_cancelled()
                {
                    let name = tasks.names.get(&e.id()).map_or("unknown", |n| n.0);
                    tracing::warn!(rubato.task = name, error = %e, "task failed during shutdown");
                }
            }
            return Ok(());
        }
        tokio::select! {
            _ = shutdown.changed() => {}
            Some(r) = tasks.set.join_next_with_id(), if !tasks.set.is_empty() => {
                let (id, error) = match r {
                    Ok((id, Ok(()))) => (id, "returned".to_owned()),
                    Ok((id, Err(e))) => (id, format!("{e:#}")),
                    Err(e) => (e.id(), e.to_string()),
                };
                let (name, kind) = tasks.names.remove(&id).unwrap_or(("unknown", Kind::Essential));
                if shutdown_requested(&shutdown) {
                    continue;
                }
                match kind {
                    Kind::Essential => {
                        tracing::error!(rubato.task = name, error = %error, "{}", event::TASK_EXITED);
                        tasks.set.abort_all();
                        return Err(anyhow!("essential task {name} exited: {error}"));
                    }
                    Kind::BestEffort => {
                        tracing::info!(rubato.task = name, "best-effort task ended");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;
    use rubato_protocol::ClientKind;
    use std::time::Duration;

    fn exited(events: &capture::Events) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.message == event::TASK_EXITED)
            .count()
    }

    /// ADR 0055: `plan_handoff` always (the `/handoff` route forwards reports), the skill only
    /// when its version is known.
    #[test]
    fn hello_announces_the_handoff_and_the_skill_when_known() {
        let with = hello_capabilities(Some("0.2.0"), ClientKind::Interactive);
        for c in [
            capability::HOOK_ALLOWLIST,
            capability::SESSION_INPUT,
            capability::DRAFT_MIRROR,
            capability::PLAN_HANDOFF,
            capability::DRAFT_RESTORE,
            capability::SESSION_STOP,
            "skill:0.2.0",
        ] {
            assert!(with.iter().any(|x| x == c), "{c} in {with:?}");
        }
        assert!(
            !with.iter().any(|x| x == capability::HEADLESS),
            "an interactive companion does not announce headless"
        );
        let headless = hello_capabilities(None, ClientKind::Headless);
        assert!(headless.iter().any(|x| x == capability::HEADLESS));
        let without = hello_capabilities(None, ClientKind::Interactive);
        assert!(without.iter().any(|x| x == capability::PLAN_HANDOFF));
        assert!(
            without
                .iter()
                .all(|x| !x.starts_with(capability::SKILL_PREFIX))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn essential_exit_while_running_is_an_error_with_one_event() {
        let (events, _g) = capture::start();
        let (_tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("reconcile", Kind::Essential, async { Err(anyhow!("boom")) });
        t.spawn("hooks", Kind::Essential, std::future::pending());
        let r = tokio::time::timeout(Duration::from_secs(1), supervise(t, rx))
            .await
            .expect("supervise returns at once");
        assert!(format!("{:#}", r.unwrap_err()).contains("reconcile"));
        assert_eq!(exited(&events), 1);
        let ev = events.lock().unwrap();
        let e = ev.iter().find(|e| e.message == event::TASK_EXITED).unwrap();
        assert_eq!(
            e.fields.get("rubato.task").map(String::as_str),
            Some("reconcile")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn best_effort_exit_parks_without_an_event() {
        let (events, _g) = capture::start();
        let (tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("events", Kind::BestEffort, async { Ok(()) });
        t.spawn("hooks", Kind::Essential, std::future::pending());
        let sup = tokio::spawn(supervise(t, rx));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !sup.is_finished(),
            "a best-effort exit does not end supervision"
        );
        tx.send(true).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(1), sup).await;
        assert!(r.expect("Ok within 1 s").unwrap().is_ok());
        assert_eq!(exited(&events), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_aborts_tasks_that_never_return() {
        let (events, _g) = capture::start();
        let (tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("hooks", Kind::Essential, std::future::pending());
        t.spawn("dropped", Kind::Essential, std::future::pending());
        let sup = tokio::spawn(supervise(t, rx));
        tx.send(true).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(1), sup).await;
        assert!(r.expect("Ok within 1 s").unwrap().is_ok());
        assert_eq!(exited(&events), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_dropped_sender_is_a_shutdown() {
        let (tx, rx) = watch::channel(false);
        let mut t = Tasks::default();
        t.spawn("hooks", Kind::Essential, std::future::pending());
        drop(tx);
        let r = tokio::time::timeout(Duration::from_secs(1), supervise(t, rx)).await;
        assert!(r.expect("Ok within 1 s").is_ok());
    }
}
