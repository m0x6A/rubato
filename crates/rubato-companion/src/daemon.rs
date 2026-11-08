//! The companion daemon: hook receiver, websocket client, tmux-ide reconciliation, stale
//! detection, spend caps, heartbeats. Everything the binary does, driven by the Gherkin suite.

use crate::client::{Client, Identity};
use crate::config::{HookEndpoint, State, Workspace};
use crate::hooks::HookServer;
use crate::launch::Launcher;
use crate::sessions::Registry;
use crate::spend;
use crate::telemetry::Metrics;
use crate::tmux::Tmux;
use crate::tmux_ide::TmuxIde;
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    CapHit, CapKind, CompanionMessage, Heartbeat, HookEvent, Limits, ServerMessage, SessionStatus,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch};

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
    server: HookServer,
    mut hook_rx: mpsc::UnboundedReceiver<HookEvent>,
    mut dropped_rx: mpsc::UnboundedReceiver<&'static str>,
    registry: Arc<Mutex<Registry>>,
    connected_tx: watch::Sender<bool>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let metrics = Arc::new(Metrics::new());
    let hook_endpoint = server.endpoint.clone();
    let tmux = Tmux::new(
        cfg.workspace.companion.tmux_command.clone(),
        cfg.workspace.companion.tmux_tmpdir.clone(),
    );
    let tmux_ide = TmuxIde::new(
        cfg.workspace.companion.tmux_ide_command.clone(),
        tmux.clone(),
    );
    let limits = Arc::new(std::sync::Mutex::new(cfg.workspace.companion.limits));

    let (out_tx, out_rx) = mpsc::unbounded_channel::<CompanionMessage>();
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let identity = Identity {
        companion_id: cfg.state.companion_id,
        user: cfg.state.user.clone(),
        version: crate::VERSION.to_owned(),
        token: cfg.state.pairing_token.clone(),
    };
    let client = Client::new(
        cfg.state.server_url.clone(),
        identity,
        cfg.state.last_seq,
        cfg.state.last_server_seq,
    );
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(client.run(out_rx, in_tx, connected_tx, shutdown.clone()));
    tasks.spawn(server.serve());

    // Heartbeat.
    {
        let out = out_tx.clone();
        let cfg = cfg.clone();
        let metrics = metrics.clone();
        let tmux_ide = tmux_ide.clone();
        let limits = limits.clone();
        let mut shutdown = shutdown.clone();
        tasks.spawn(async move {
            let hostname = std::fs::read_to_string("/etc/hostname")
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| "unknown".into());
            let tmux_ide_version = tokio::task::spawn_blocking(move || tmux_ide.version().ok())
                .await
                .ok()
                .flatten();
            loop {
                let now = Utc::now();
                let _ = out.send(CompanionMessage::Heartbeat(Heartbeat {
                    companion_id: cfg.state.companion_id,
                    user: cfg.state.user.clone(),
                    at: now,
                    hostname: hostname.clone(),
                    companion_version: crate::VERSION.to_owned(),
                    tmux_ide_version: tmux_ide_version.clone(),
                    claude_version: None,
                }));
                metrics.heartbeat.record(
                    now.timestamp() as u64,
                    &[
                        KeyValue::new(attr::COMPANION_ID, cfg.state.companion_id.to_string()),
                        KeyValue::new(attr::USER, cfg.state.user.clone()),
                    ],
                );
                let secs = limits.lock().unwrap().heartbeat_interval_secs.max(1);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
                    _ = shutdown.changed() => return Ok(()),
                }
            }
        });
    }

    // Reconciliation: tmux pane metadata + tmux-ide team --json, on a timer and on tmux-ide events.
    let (poll_tx, mut poll_rx) = mpsc::unbounded_channel::<()>();
    {
        let tmux_ide = tmux_ide.clone();
        let poll_tx = poll_tx.clone();
        let mut shutdown = shutdown.clone();
        tasks.spawn(async move {
            // tmux-ide events need an adopted session; when it fails we rely on the timer.
            let child = tokio::task::spawn_blocking(move || tmux_ide.events_follow()).await;
            let Ok(Ok(mut child)) = child else {
                return Ok(());
            };
            let Some(stdout) = child.stdout.take() else {
                return Ok(());
            };
            let stdout = tokio::process::ChildStdout::from_std(stdout)?;
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            loop {
                tokio::select! {
                    line = lines.next_line() => match line {
                        Ok(Some(_)) => { let _ = poll_tx.send(()); }
                        _ => { let _ = child.kill(); return Ok(()); }
                    },
                    _ = shutdown.changed() => { let _ = child.kill(); return Ok(()); }
                }
            }
        });
    }
    {
        let out = out_tx.clone();
        let registry = registry.clone();
        let metrics = metrics.clone();
        let tmux = tmux.clone();
        let tmux_ide = tmux_ide.clone();
        let limits = limits.clone();
        let mut shutdown = shutdown.clone();
        tasks.spawn(async move {
            let mut last_snapshot: Option<String> = None;
            loop {
                let (t, ti) = (tmux.clone(), tmux_ide.clone());
                let (panes, fleet) =
                    tokio::task::spawn_blocking(move || (t.panes(), ti.team())).await?;
                {
                    let mut reg = registry.lock().await;
                    let now = Utc::now();
                    match panes {
                        Ok(p) => reg.apply_panes(&p, now),
                        Err(e) => tracing::warn!(error = %e, "tmux list-panes failed"),
                    }
                    match fleet {
                        Ok(f) => reg.apply_fleet(&f, now),
                        Err(e) => tracing::debug!(error = %e, "tmux-ide team --json failed"),
                    }
                    reg.sweep(now, 300);
                    let snap = reg.snapshot();
                    metrics.sessions_discovered.record(snap.len() as u64, &[]);
                    for (status, n) in reg.counts() {
                        metrics.sessions_by_state.record(
                            n as u64,
                            &[KeyValue::new(
                                attr::SESSION_STATE,
                                format!("{status:?}").to_lowercase(),
                            )],
                        );
                    }
                    let key = serde_json::to_string(&snap).unwrap_or_default();
                    if last_snapshot.as_deref() != Some(key.as_str()) {
                        last_snapshot = Some(key);
                        let _ = out.send(CompanionMessage::Sessions { sessions: snap });
                    }
                }
                let secs = limits.lock().unwrap().team_poll_interval_secs.max(1);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
                    _ = poll_rx.recv() => {}
                    _ = shutdown.changed() => return Ok(()),
                }
            }
        });
    }

    // Stale detection, every second.
    {
        let out = out_tx.clone();
        let registry = registry.clone();
        let limits = limits.clone();
        let mut shutdown = shutdown.clone();
        tasks.spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    _ = shutdown.changed() => return Ok(()),
                }
                let stale_secs = limits.lock().unwrap().session_stale_secs;
                let mut reg = registry.lock().await;
                let stale = reg.stale(Utc::now(), stale_secs);
                for s in stale {
                    tracing::warn!(session = ?s.session_id, silent_for_secs = s.silent_for_secs, "{}", rubato_protocol::otel::event::SESSION_STALE);
                    let _ = out.send(CompanionMessage::Stale(s));
                }
            }
        });
    }

    // Dropped hook requests → counter.
    {
        let metrics = metrics.clone();
        tasks.spawn(async move {
            while let Some(reason) = dropped_rx.recv().await {
                metrics
                    .events_dropped
                    .add(1, &[KeyValue::new(attr::DROP_REASON, reason)]);
            }
            Ok(())
        });
    }

    // Main dispatch: hooks in, server commands in.
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            Some(ev) = hook_rx.recv() => {
                metrics.hook_forwarded.add(1, &[KeyValue::new(attr::HOOK_EVENT, format!("{:?}", ev.hook_event_name)), KeyValue::new(attr::SESSION_ID, ev.session_id.clone())]);
                let (issue, transcript, cap, snapshot) = {
                    let mut reg = registry.lock().await;
                    let issue = reg.apply_hook(&ev);
                    let e = reg.by_session(&ev.session_id).cloned();
                    (issue, e.as_ref().and_then(|e| e.transcript_path.clone()), e.as_ref().and_then(|e| e.spend_cap_usd), reg.snapshot())
                };
                let _ = out_tx.send(CompanionMessage::Hook(ev.clone()));
                let _ = out_tx.send(CompanionMessage::Sessions { sessions: snapshot });
                if let Some(path) = transcript && !cfg.workspace.companion.pricing.is_empty() {
                    let pricing = cfg.workspace.companion.pricing.clone();
                    if let Ok(Ok(s)) = tokio::task::spawn_blocking(move || spend::sum_file(&path, &pricing)).await {
                        let mut reg = registry.lock().await;
                        reg.set_spend(&ev.session_id, s.usd);
                        if let (Some(limit), Some(issue)) = (cap, issue.clone()) && s.usd > limit {
                            let entry = reg.by_session(&ev.session_id).cloned();
                            drop(reg);
                            tracing::warn!(issue = %issue, spend = s.usd, limit, "{}", rubato_protocol::otel::event::CAP_HIT);
                            let _ = out_tx.send(CompanionMessage::CapHit(CapHit { issue, session_id: Some(ev.session_id.clone()), kind: CapKind::Spend, value: s.usd, limit, at: Utc::now() }));
                            if let Some(name) = entry.and_then(|e| e.tmux_session) {
                                let t = tmux.clone();
                                let _ = tokio::task::spawn_blocking(move || t.kill_session(&name)).await;
                            }
                        }
                    }
                }
            }
            Some(msg) = in_rx.recv() => match msg {
                ServerMessage::Welcome { limits: l, .. } => {
                    *limits.lock().unwrap() = Limits { ..l };
                }
                ServerMessage::Launch(cmd) => {
                    let registry2 = registry.clone();
                    let cfg2 = cfg.clone();
                    let (tmux2, tmux_ide2, ep) = (tmux.clone(), tmux_ide.clone(), hook_endpoint.clone());
                    let reg_snapshot = registry.lock().await;
                    // Launcher needs a read of the registry; clone entries' view by holding the lock only for the decision.
                    let (result, started) = {
                        let reg = &*reg_snapshot;
                        tokio::task::block_in_place(|| {
                            let launcher = Launcher { workspace: &cfg2.workspace, paired_user: &cfg2.state.user, state_dir: &cfg2.state_dir, hook_endpoint: &ep, tmux: &tmux2, tmux_ide: &tmux_ide2 };
                            launcher.handle(&cmd, reg)
                        })
                    };
                    drop(reg_snapshot);
                    if let Some(s) = started {
                        registry2.lock().await.apply_launch(&cmd, &s.pane_id, &s.tmux_session, &s.worktree, Utc::now());
                    }
                    tracing::info!(launch_id = %cmd.id, issue = %cmd.issue, outcome = ?std::mem::discriminant(&result), "{}", rubato_protocol::otel::span::LAUNCH);
                    let _ = out_tx.send(CompanionMessage::Launch(result));
                    let snap = registry2.lock().await.snapshot();
                    let _ = out_tx.send(CompanionMessage::Sessions { sessions: snap });
                    let _ = poll_tx.send(());
                }
                ServerMessage::Error { code, .. } if code == "auth" => {
                    tracing::error!("pairing token rejected; re-pair with `rubato-companion pair`");
                }
                _ => {}
            },
            else => break,
        }
    }
    tasks.abort_all();
    Ok(())
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
            };
            st.effective_status(Utc::now(), stale_secs)
        })
    }
}
