//! Shared application state.

use crate::auth::Signer;
use crate::config::Config;
use crate::github::GitHubApi;
use crate::telemetry::Metrics;
use chrono::{DateTime, Utc};
use rubato_protocol::{ServerMessage, SystemMap, WebEvent};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{RwLock, broadcast, mpsc};
use uuid::Uuid;

pub struct CompanionConn {
    pub user: String,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

#[derive(Debug, Clone)]
pub struct LoadedMap {
    pub map: SystemMap,
    pub version: String,
    pub loaded_at: DateTime<Utc>,
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
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(db: SqlitePool, config: Config, github: Arc<dyn GitHubApi>) -> App {
        let (web_tx, _) = broadcast::channel(256);
        let signer = Signer::new(config.session_secret.as_bytes());
        Arc::new(Self {
            db,
            config: Arc::new(config),
            github,
            companions: Mutex::new(HashMap::new()),
            web_tx,
            map: RwLock::new(None),
            metrics: Metrics::new(),
            signer,
        })
    }

    /// Tell every browser something changed; they refetch the overview.
    pub fn notify(&self, reason: &str) {
        let _ = self.web_tx.send(WebEvent::Changed {
            at: Utc::now(),
            reason: reason.to_owned(),
        });
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
