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
use std::collections::{BTreeMap, BTreeSet, HashMap};
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

/// The server's clock for receipt times on the companion socket. The system clock, unless a
/// test sets a time (plan 103: snapshots received 20 minutes apart).
#[derive(Default)]
pub struct Clock(Mutex<Option<DateTime<Utc>>>);

impl Clock {
    pub fn now(&self) -> DateTime<Utc> {
        self.0.lock().unwrap().unwrap_or_else(Utc::now)
    }

    /// Test-only in use: every later `now` answers `at` until it is set again.
    pub fn set(&self, at: DateTime<Utc>) {
        *self.0.lock().unwrap() = Some(at);
    }
}

pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub github: Arc<dyn GitHubApi>,
    /// Companions with a websocket open right now.
    pub companions: Mutex<HashMap<Uuid, CompanionConn>>,
    pub web_tx: broadcast::Sender<WebEvent>,
    /// The loaded map of each workspace, by workspace id (ADR 0044). A workspace whose map has
    /// never loaded has no entry.
    pub maps: RwLock<BTreeMap<String, Arc<LoadedMap>>>,
    pub metrics: Metrics,
    pub signer: Signer,
    pub traffic: RwLock<TrafficLatest>,
    pub clock: Clock,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(db: SqlitePool, config: Config, github: Arc<dyn GitHubApi>) -> App {
        let (web_tx, _) = broadcast::channel(256);
        let signer = Signer::new(config.session_secret.as_bytes());
        let traffic = TrafficLatest {
            report: None,
            source: if config.traffic_configured() {
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
            maps: RwLock::new(BTreeMap::new()),
            metrics: Metrics::new(),
            signer,
            traffic: RwLock::new(traffic),
            clock: Clock::default(),
        })
    }

    /// The loaded map of workspace `default`: what every route served before workspaces, and
    /// what unprefixed routes serve (ADR 0049).
    pub async fn default_map(&self) -> Option<Arc<LoadedMap>> {
        self.map_of(rubato_protocol::DEFAULT_WORKSPACE).await
    }

    /// A shared pointer to the loaded map, not a copy (#171): a reload swaps the pointer, and a
    /// reader holding the old one keeps a consistent map.
    pub async fn map_of(&self, ws: &str) -> Option<Arc<LoadedMap>> {
        self.maps.read().await.get(ws).cloned()
    }

    pub async fn set_map(&self, ws: &str, map: LoadedMap) -> Arc<LoadedMap> {
        let map = Arc::new(map);
        self.maps
            .write()
            .await
            .insert(ws.to_string(), Arc::clone(&map));
        map
    }

    /// Every distinct repo the loaded maps name, each once (ADR 0048), in order.
    pub async fn union_repos(&self) -> Vec<String> {
        let maps = self.maps.read().await;
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for m in maps.values() {
            for r in m.map.repos_named() {
                if seen.insert(r.to_ascii_lowercase()) {
                    out.push(r.to_string());
                }
            }
        }
        out
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
            workspace: None,
        });
    }

    /// Tell every browser one workspace's map, layout or draft changed (ADR 0049): only clients
    /// of that workspace refetch. Changes that show in other workspaces too use `notify`.
    pub fn notify_ws(&self, reason: &str, ws: &str) {
        let subscribers = self.web_tx.receiver_count();
        tracing::info!(
            reason,
            rubato.workspace = ws,
            rubato.subscribers = subscribers,
            "web fan-out"
        );
        let _ = self.web_tx.send(WebEvent::Changed {
            at: Utc::now(),
            reason: reason.to_owned(),
            workspace: Some(ws.to_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::github::FakeGitHub;

    fn loaded(version: &str) -> LoadedMap {
        LoadedMap {
            map: serde_yaml_ng::from_str("version: 1\nname: m\nnodes: []\nseams: []\n").unwrap(),
            version: version.into(),
            loaded_at: Utc::now(),
            yaml: format!("# {version}"),
        }
    }

    /// #171: a read hands out the loaded map, not a copy; a reload swaps it and leaves a reader
    /// holding the old one with a consistent map.
    #[tokio::test]
    async fn map_of_shares_one_map_and_a_reload_swaps_it() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::for_tests(
            format!("sqlite://{}/s.db?mode=rwc", tmp.path().display()),
            "m0x6A",
        );
        let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
        let app = AppState::new(pool, cfg, Arc::new(FakeGitHub::new()));
        let set = app.set_map("default", loaded("v1")).await;
        let (a, b) = (
            app.map_of("default").await.unwrap(),
            app.default_map().await.unwrap(),
        );
        assert!(
            Arc::ptr_eq(&a, &b) && Arc::ptr_eq(&a, &set),
            "a read copied the map"
        );
        app.set_map("default", loaded("v2")).await;
        assert_eq!((a.version.as_str(), a.yaml.as_str()), ("v1", "# v1"));
        assert_eq!(app.map_of("default").await.unwrap().version, "v2");
        assert!(app.map_of("other").await.is_none());
    }
}
