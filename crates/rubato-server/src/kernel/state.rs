//! Shared application state: the loaded map, the connected-companion registry and the web
//! notify sender. In the kernel because four or more slices need it (design §9).

use crate::kernel::config::Config;
use crate::kernel::github::GitHubApi;
use crate::kernel::signer::Signer;
use crate::kernel::telemetry::Metrics;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    ServerMessage, SessionState, SourceStatus, SystemMap, TrafficReport, WebEvent, WebResource,
};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use tokio::sync::{RwLock, broadcast, mpsc};
use uuid::Uuid;

pub struct CompanionConn {
    pub tx: mpsc::UnboundedSender<ServerMessage>,
    /// New per socket: only the socket that inserted the entry removes it (#161).
    pub conn: Uuid,
    /// When this connection last sent an envelope: a socket left open by a laptop that went to
    /// sleep looks connected, so "live" also needs a recent envelope (ADR 0051).
    pub last_seen: DateTime<Utc>,
}

impl CompanionConn {
    /// A connection entry with a fresh per-socket id (#161).
    pub fn new(tx: mpsc::UnboundedSender<ServerMessage>, now: DateTime<Utc>) -> Self {
        Self {
            tx,
            conn: Uuid::new_v4(),
            last_seen: now,
        }
    }
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
    /// What each companion announced in its last Hello (ADR 0051), kept after it disconnects so
    /// the web can say "upgrade the companion" while it is away. In memory only.
    pub companion_caps: Mutex<HashMap<Uuid, Vec<String>>>,
    pub web_tx: broadcast::Sender<WebEvent>,
    /// The loaded map of each workspace, by workspace id (ADR 0044). A workspace whose map has
    /// never loaded has no entry.
    pub maps: RwLock<BTreeMap<String, LoadedMap>>,
    pub metrics: Metrics,
    pub signer: Signer,
    pub traffic: RwLock<TrafficLatest>,
    pub clock: Clock,
    /// Each companion's latest session snapshot, by the pairing's companion id: what
    /// `session_snapshots` holds, kept in memory for slices that may not name `companion_link`
    /// (approval's "is the owner's session working", plan 122 §D6). `companion_link` writes it
    /// on every snapshot; the composition root fills it once at start.
    pub sessions: Mutex<HashMap<Uuid, Vec<SessionState>>>,
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
            companion_caps: Mutex::new(HashMap::new()),
            web_tx,
            maps: RwLock::new(BTreeMap::new()),
            metrics: Metrics::new(),
            signer,
            traffic: RwLock::new(traffic),
            clock: Clock::default(),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// A companion's latest snapshot replaces its earlier one.
    pub fn set_sessions(&self, companion_id: Uuid, sessions: Vec<SessionState>) {
        self.sessions.lock().unwrap().insert(companion_id, sessions);
    }

    /// Every session of every companion's latest snapshot.
    pub fn latest_sessions(&self) -> Vec<SessionState> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// The loaded map of workspace `default`: what every route served before workspaces, and
    /// what unprefixed routes serve (ADR 0049).
    pub async fn default_map(&self) -> Option<LoadedMap> {
        self.map_of(rubato_protocol::DEFAULT_WORKSPACE).await
    }

    pub async fn map_of(&self, ws: &str) -> Option<LoadedMap> {
        self.maps.read().await.get(ws).cloned()
    }

    pub async fn set_map(&self, ws: &str, map: LoadedMap) {
        self.maps.write().await.insert(ws.to_string(), map);
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

    /// Record what a companion announced in its Hello.
    pub fn set_capabilities(&self, companion_id: Uuid, caps: Vec<String>) {
        self.companion_caps
            .lock()
            .unwrap()
            .insert(companion_id, caps);
    }

    /// Everything the companion announced the last time it said Hello.
    pub fn capabilities(&self, companion_id: Uuid) -> Vec<String> {
        self.companion_caps
            .lock()
            .unwrap()
            .get(&companion_id)
            .cloned()
            .unwrap_or_default()
    }

    /// The companion announced `cap` the last time it said Hello.
    pub fn announced(&self, companion_id: Uuid, cap: &str) -> bool {
        self.companion_caps
            .lock()
            .unwrap()
            .get(&companion_id)
            .is_some_and(|c| c.iter().any(|x| x == cap))
    }

    /// An envelope arrived from this companion.
    pub fn touch_companion(&self, companion_id: Uuid, now: DateTime<Utc>) {
        if let Some(c) = self.companions.lock().unwrap().get_mut(&companion_id) {
            c.last_seen = now;
        }
    }

    /// Connected, announcing `cap`, and heard from within two heartbeat intervals (ADR 0051).
    pub fn companion_live(&self, companion_id: Uuid, cap: &str, now: DateTime<Utc>) -> bool {
        let window =
            chrono::Duration::seconds(2 * self.config.limits.heartbeat_interval_secs as i64);
        let recent = self
            .companions
            .lock()
            .unwrap()
            .get(&companion_id)
            .is_some_and(|c| now.signed_duration_since(c.last_seen) <= window);
        recent && self.announced(companion_id, cap)
    }

    /// Connected companions per announced capability (`rubato.companions.by_capability`).
    pub fn capability_counts(&self, known: &[&str]) -> BTreeMap<String, u64> {
        let connected: Vec<Uuid> = self.connected_companions();
        let caps = self.companion_caps.lock().unwrap();
        known
            .iter()
            .map(|k| {
                let n = connected
                    .iter()
                    .filter(|id| caps.get(id).is_some_and(|c| c.iter().any(|x| x == k)))
                    .count() as u64;
                ((*k).to_owned(), n)
            })
            .collect()
    }

    /// Tell browsers one resource the overview does not carry changed: only a view showing it
    /// refetches it (plan 122).
    pub fn notify_resource(&self, resource: WebResource) {
        let subscribers = self.web_tx.receiver_count();
        tracing::info!(?resource, rubato.subscribers = subscribers, "web fan-out");
        let _ = self.web_tx.send(WebEvent::Updated {
            at: Utc::now(),
            resource,
        });
    }

    pub fn send_to_companion(&self, companion_id: Uuid, msg: ServerMessage) -> bool {
        match self.companions.lock().unwrap().get(&companion_id) {
            Some(c) => c.tx.send(msg).is_ok(),
            None => false,
        }
    }
}
