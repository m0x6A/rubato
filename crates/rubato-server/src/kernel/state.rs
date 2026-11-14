//! Shared application state: the loaded map, the connected-companion registry and the web
//! notify sender. In the kernel because four or more slices need it (design §9).

use crate::kernel::config::Config;
use crate::kernel::github::GitHubApi;
use crate::kernel::signer::Signer;
use crate::kernel::telemetry::Metrics;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{ServerMessage, SourceStatus, SystemMap, TrafficReport, WebEvent};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{RwLock, broadcast, mpsc};
use uuid::Uuid;

pub struct CompanionConn {
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

#[derive(Debug, Clone)]
pub struct LoadedMap {
    pub map: SystemMap,
    pub version: String,
    pub loaded_at: DateTime<Utc>,
    /// The map file's text as it was loaded: what a proposal must find unchanged on the map
    /// branch, and what it checks for keys a rewrite would drop (ADR 0018).
    pub yaml: String,
}

/// The latest good traffic report and its source's state (slice `traffic`, ADR 0039). In
/// memory only: a restart is *unknown* until the first fetch. The traffic task writes it; the
/// overview reads it.
#[derive(Debug, Clone)]
pub struct TrafficLatest {
    pub report: Option<TrafficReport>,
    pub source: SourceStatus,
}

pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub github: Arc<dyn GitHubApi>,
    /// Companions with a websocket open right now.
    pub companions: Mutex<HashMap<Uuid, CompanionConn>>,
    pub web_tx: broadcast::Sender<WebEvent>,
    pub map: RwLock<Option<LoadedMap>>,
    pub metrics: Metrics,
    pub signer: Signer,
    pub traffic: RwLock<TrafficLatest>,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(db: SqlitePool, config: Config, github: Arc<dyn GitHubApi>) -> App {
        let (web_tx, _) = broadcast::channel(256);
        let signer = Signer::new(config.session_secret.as_bytes());
        let traffic = TrafficLatest {
            report: None,
            source: if config.dev_traffic.is_some() {
                SourceStatus::Configured {
                    fetch_secs: config.traffic_fetch_secs,
                    error: None,
                }
            } else {
                SourceStatus::NotConfigured
            },
        };
        Arc::new(Self {
            db,
            config: Arc::new(config),
            github,
            companions: Mutex::new(HashMap::new()),
            web_tx,
            map: RwLock::new(None),
            metrics: Metrics::new(),
            signer,
            traffic: RwLock::new(traffic),
        })
    }

    /// Tell every browser something changed; they refetch the overview. One `info` line per
    /// change says why and to how many, so a change that reached nobody can be told from one
    /// that was never sent.
    pub fn notify(&self, reason: &str) {
        let subscribers = self.web_tx.receiver_count();
        tracing::info!(reason, rubato.subscribers = subscribers, "web fan-out");
        let _ = self.web_tx.send(WebEvent::Changed {
            at: Utc::now(),
            reason: reason.to_owned(),
        });
    }

    /// A web request refused by a rule: counted by route and reason, and logged with who.
    pub fn reject(&self, route: &str, reason: &str, user: &str) {
        self.metrics.web_rejected.add(
            1,
            &[
                KeyValue::new(attr::ROUTE, route.to_owned()),
                KeyValue::new(attr::DROP_REASON, reason.to_owned()),
            ],
        );
        tracing::info!(
            rubato.route = route,
            rubato.reason = reason,
            rubato.user = user,
            "web write rejected"
        );
    }

    pub fn connected_companions(&self) -> Vec<Uuid> {
        self.companions.lock().unwrap().keys().copied().collect()
    }

    pub fn send_to_companion(&self, companion_id: Uuid, msg: ServerMessage) -> bool {
        match self.companions.lock().unwrap().get(&companion_id) {
            Some(c) => c.tx.send(msg).is_ok(),
            None => false,
        }
    }
}
