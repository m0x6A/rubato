//! Slice `agent_mcp`: the MCP server a session Rubato launched asks about itself (ADR 0071,
//! plan 220 §4). Three tools on `POST /mcp` of the hook receiver: `get_assignment` and
//! `plan_status` read, `report_status` is a signal. The caller is the launch its
//! `X-Rubato-Launch` names, after the receiver's token check (`hooks::LaunchHeader`); nothing an
//! agent sends picks another launch.
//!
//! `report_status` writes only this slice's own map, never the session registry: a report never
//! makes a session look *working* and never feeds progress. A drain sends the newest report per
//! launch every 5 s, and only to a server that announced `agent_mcp`; otherwise it is dropped.
//!
//! Names `sessions` (the registry, `assignment_of`) and `hooks` (`LaunchHeader`). Named only by
//! the composition root, which mounts the routers on the receiver and spawns the two tasks.

mod cache;
mod fleet;
mod report;
mod rpc;
mod tools;
mod watch;

#[cfg(test)]
mod fleet_tests;
#[cfg(test)]
mod tests;

pub use fleet::{FLEET_TOOLS, leads_path, new_lead_secret};
pub use tools::{TOOLS, Tool};

use crate::kernel::telemetry::Metrics;
use crate::sessions::Registry;
use anyhow::Result;
use axum::Router;
use axum::routing::{get, post};
use chrono::Utc;
use rubato_protocol::{CompanionMessage, PlanStatus};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch as tokio_watch};

/// How often the drain sends the newest report per launch.
pub const DRAIN_EVERY: Duration = Duration::from_secs(5);
/// How often the watch looks for launches that never asked.
pub const WATCH_EVERY: Duration = Duration::from_secs(60);

/// What the routes and the two tasks share.
pub struct AgentMcp {
    companion_id: uuid::Uuid,
    registry: Arc<Mutex<Registry>>,
    metrics: Arc<Metrics>,
    reports: report::Pending,
    cache: cache::Cache,
    initialized: watch::Initialized,
    server_capabilities: Arc<std::sync::Mutex<Vec<String>>>,
    out: mpsc::UnboundedSender<CompanionMessage>,
    /// Whether the companion's websocket is up: `plan_status` says so with its answer.
    link: tokio_watch::Receiver<bool>,
    /// The companion's orchestrator flag (ADR 0072): the fleet tools only with it on.
    orchestrator: bool,
    /// Where `leads.json` holds the lead secrets.
    state_dir: std::path::PathBuf,
    fleet_waiters: fleet::Waiters,
}

impl AgentMcp {
    /// Launches already in the registry (restored at start) count as initialized: they asked
    /// under the previous process, and Claude Code does not initialize again.
    pub async fn new(
        registry: Arc<Mutex<Registry>>,
        metrics: Arc<Metrics>,
        server_capabilities: Arc<std::sync::Mutex<Vec<String>>>,
        out: mpsc::UnboundedSender<CompanionMessage>,
        link: tokio_watch::Receiver<bool>,
        orchestrator: bool,
        state_dir: std::path::PathBuf,
    ) -> Arc<Self> {
        let (companion_id, restored) = {
            let reg = registry.lock().await;
            let ids = reg.entries().iter().filter_map(|e| e.launch_id).collect();
            (reg.companion_id, ids)
        };
        Arc::new(Self {
            companion_id,
            registry,
            metrics,
            reports: report::Pending::default(),
            cache: cache::Cache::default(),
            initialized: watch::Initialized::new(restored),
            server_capabilities,
            out,
            link,
            orchestrator,
            state_dir,
            fleet_waiters: fleet::Waiters::default(),
        })
    }

    /// A `PlanStatus` from the server: the newest per launch answers `plan_status`.
    pub fn on_plan_status(&self, s: PlanStatus) {
        self.cache.put(s);
    }

    /// A `FleetResult` from the server: the lead's call waiting for it answers (ADR 0072).
    pub fn on_fleet_result(&self, r: rubato_protocol::FleetResult) {
        fleet::on_result(self, r);
    }
}

/// The routers the composition root hands `hooks::HookServer::serve_with`: `open` (OAuth
/// discovery, no token) and `guarded` (`/mcp`).
pub fn routers(state: Arc<AgentMcp>) -> (Router, Router) {
    let open = Router::new()
        .route("/.well-known/{*rest}", get(rpc::discovery))
        .with_state(state.clone());
    let guarded = Router::new()
        .route("/mcp", post(rpc::post).get(rpc::not_allowed))
        .with_state(state);
    (open, guarded)
}

/// Every [`DRAIN_EVERY`], the newest report per launch goes to the server, or is dropped when
/// the server did not announce `agent_mcp`. Essential: reports would pile up without it.
pub async fn drain(state: Arc<AgentMcp>, mut shutdown: tokio_watch::Receiver<bool>) -> Result<()> {
    let mut tick = tokio::time::interval(DRAIN_EVERY);
    loop {
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = tick.tick() => report::drain_once(&state),
        }
    }
}

/// Every [`WATCH_EVERY`], a launch that made no MCP request in its first five minutes is counted
/// once (`rubato.mcp.not_initialized`). Best effort: only a signal.
pub async fn watch(state: Arc<AgentMcp>, mut shutdown: tokio_watch::Receiver<bool>) -> Result<()> {
    let mut tick = tokio::time::interval(WATCH_EVERY);
    loop {
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            _ = tick.tick() => watch::tick(&state, Utc::now()).await,
        }
    }
}
