//! Composition root: the companion runtime. Owns the channels, spawns each slice's task with an
//! essential or best-effort flag, dispatches the select arms to the slices, and supervises the
//! tasks (design D6, D12):
//!
//! - essential (server link, hook server, heartbeat, reconcile, stale sweep, dropped counter,
//!   launch task, which handles launch commands one at a time off the dispatch loop, plan 40):
//!   ending while shutdown is not requested logs `rubato.task.exited` and makes `run` return
//!   `Err`, which `main` turns into a non-zero exit;
//! - best-effort (tmux-ide events watcher): ending is normal on a machine with no adopted
//!   session; discovery runs on the timer;
//! - on shutdown every task is aborted and `Cancelled` is clean, so `run` returns `Ok` at once.

use crate::hooks::HookServer;
use crate::kernel::config::{HookEndpoint, State, Workspace};
use crate::kernel::telemetry::Metrics;
use crate::kernel::tmux::Tmux;
use crate::kernel::tmux_ide::TmuxIde;
use crate::launch::Launcher;
use crate::server_link::{Client, Identity, Reservation};
use crate::sessions::Registry;
use crate::{hooks, launch, server_link, sessions, spend, stale};
use anyhow::{Result, anyhow};
use chrono::Utc;
use rubato_protocol::otel::event;
use rubato_protocol::{CompanionMessage, HookEvent, LaunchCommand, ServerMessage, SessionStatus};
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
    let (hook_tx, hook_rx) = mpsc::unbounded_channel::<HookEvent>();
    let (dropped_tx, dropped_rx) = mpsc::unbounded_channel::<&'static str>();
    let server = HookServer::bind(hook_tx, dropped_tx).await?;
    let hook_endpoint = server.endpoint.clone();
    hook_endpoint.save(&cfg.state_dir)?;
    tracing::info!(
        port = hook_endpoint.port,
        "hook receiver listening on 127.0.0.1"
    );

    let registry = Arc::new(Mutex::new(Registry::new(
        cfg.state.companion_id,
        cfg.state.user.clone(),
    )));
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
        hook_rx,
        dropped_rx,
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
    hook_rx: mpsc::UnboundedReceiver<HookEvent>,
    dropped_rx: mpsc::UnboundedReceiver<&'static str>,
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

    let (out_tx, out_rx) = mpsc::unbounded_channel::<CompanionMessage>();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let (poll_tx, poll_rx) = mpsc::unbounded_channel::<()>();
    let client = Client::new(
        cfg.state.server_url.clone(),
        Identity {
            companion_id: cfg.state.companion_id,
            user: cfg.state.user.clone(),
            version: crate::kernel::VERSION.to_owned(),
            token: cfg.state.pairing_token.clone(),
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
        ),
    );
    let dispatch = Dispatch {
        cfg: &cfg,
        registry: &registry,
        out: &out_tx,
        metrics: &metrics,
        tmux: &tmux,
        limits: &limits,
        launch: &launch_tx,
    };
    let supervised = supervise(tasks, shutdown.clone());
    tokio::pin!(supervised);
    tokio::select! {
        r = &mut supervised => return r,
        () = dispatch.run(hook_rx, in_rx, shutdown) => {}
    }
    supervised.await
}

/// The select arms, each one call into a slice (design §9.1 keeps their order).
struct Dispatch<'a> {
    cfg: &'a DaemonConfig,
    registry: &'a Mutex<Registry>,
    out: &'a mpsc::UnboundedSender<CompanionMessage>,
    metrics: &'a Metrics,
    tmux: &'a Tmux,
    limits: &'a std::sync::Mutex<rubato_protocol::Limits>,
    /// The launch task's queue (plan 40 S1).
    launch: &'a mpsc::UnboundedSender<LaunchCommand>,
}

impl Dispatch<'_> {
    async fn run(
        &self,
        mut hook_rx: mpsc::UnboundedReceiver<HookEvent>,
        mut in_rx: mpsc::UnboundedReceiver<ServerMessage>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                Some(ev) = hook_rx.recv() => {
                    let outcome = hooks::on_event(&ev, self.registry, self.out, self.metrics, self.cfg.state.companion_id).await;
                    spend::after_hook(&ev, outcome, self.registry, self.out, &self.cfg.workspace.companion.pricing, self.tmux).await;
                }
                Some(msg) = in_rx.recv() => match msg {
                    ServerMessage::Welcome { limits, .. } => server_link::on_welcome(limits, self.limits),
                    // Queued, not awaited: hooks keep flowing while it runs.
                    ServerMessage::Launch(cmd) => {
                        let _ = self.launch.send(cmd);
                    }
                    ServerMessage::Error { code, .. } if code == "auth" => {
                        tracing::error!("pairing token rejected; re-pair with `rubato-companion pair`");
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
    use std::time::Duration;

    fn exited(events: &capture::Events) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.message == event::TASK_EXITED)
            .count()
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
