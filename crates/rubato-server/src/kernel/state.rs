//! Shared application state: the loaded map, the connected-companion registry and the web
//! notify sender. In the kernel because four or more slices need it (design §9).

use crate::kernel::config::Config;
use crate::kernel::github::GitHubApi;
use crate::kernel::signer::Signer;
use crate::kernel::speech::Speech;
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
    /// Silent for `companion_silent_secs` with the socket still open: its sessions are counted
    /// unknown until it sends again (#69).
    pub quiet: bool,
}

impl CompanionConn {
    /// A connection entry with a fresh per-socket id (#161).
    pub fn new(tx: mpsc::UnboundedSender<ServerMessage>, now: DateTime<Utc>) -> Self {
        Self {
            tx,
            conn: Uuid::new_v4(),
            last_seen: now,
            quiet: false,
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

impl TrafficLatest {
    /// A workspace with no traffic source (ADR 0075).
    pub fn not_configured() -> Self {
        Self {
            report: None,
            source: SourceStatus::NotConfigured,
        }
    }
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
    /// Which workspaces this server holds, in switcher order, and where the list came from
    /// (ADR 0044, ADR 0075). `new` fills it from `Config`; the composition root replaces it
    /// from `workspace_settings` at start, and that slice alone writes it after. A `watch`, so
    /// a task can follow changes.
    workspaces: tokio::sync::watch::Sender<crate::kernel::workspaces::Registry>,
    /// The loaded map of each workspace, by workspace id (ADR 0044). A workspace whose map has
    /// never loaded has no entry.
    pub maps: RwLock<BTreeMap<String, Arc<LoadedMap>>>,
    pub metrics: Metrics,
    pub signer: Signer,
    /// Each workspace's latest traffic report and source state, by workspace id (ADR 0075). A
    /// workspace with no entry has no source. The traffic slice writes it; the overview reads
    /// its own workspace's with `traffic_of`.
    pub traffic: RwLock<BTreeMap<String, TrafficLatest>>,
    pub clock: Clock,
    /// Each companion's latest session snapshot, by the pairing's companion id: what
    /// `session_snapshots` holds, kept in memory for slices that may not name `companion_link`
    /// (approval's "is the owner's session working", plan 122 §D6). `companion_link` writes it
    /// on every snapshot; the composition root fills it once at start.
    pub sessions: Mutex<HashMap<Uuid, Vec<SessionState>>>,
    /// Woken by an accepted webhook delivery; the poll loop reads what it names (ADR 0074).
    pub webhook_wake: tokio::sync::Notify,
    /// Lower case: the repos the last poll tick read (the union of workspace repos and tracked
    /// issues' repos), so a delivery for any other repo is ignored. Empty before the first tick.
    pub polled_repos: Mutex<BTreeSet<String>>,
    /// In-memory state a slice keeps for this server, one value per type (the pull request view
    /// cache, issue #82). Never a table; gone on restart.
    pub slots: Slots,
    /// A companion was answered a `Welcome`: a slice that pushes to it right after (plan 226 §4,
    /// `plan_status`) listens here, so `companion_link` names no slice for it.
    welcomed: broadcast::Sender<Uuid>,
    /// The speech adapter while voice is on with a provider configured (ADR 0073). Tests swap it
    /// for a counting or blocking fake with `set_speech`.
    speech: std::sync::RwLock<Option<Arc<dyn Speech>>>,
}

/// One value per type, made on first use: lets a slice keep in-memory state on the `App`
/// without the kernel naming the slice.
#[derive(Default)]
pub struct Slots(Mutex<HashMap<std::any::TypeId, Arc<dyn std::any::Any + Send + Sync>>>);

impl Slots {
    pub fn get<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
        let mut m = self.0.lock().unwrap();
        let v = m
            .entry(std::any::TypeId::of::<T>())
            .or_insert_with(|| Arc::new(T::default()))
            .clone();
        v.downcast::<T>().expect("one value per type")
    }
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(db: SqlitePool, config: Config, github: Arc<dyn GitHubApi>) -> App {
        let (web_tx, _) = broadcast::channel(256);
        let signer = Signer::new(config.session_secret.as_bytes());
        // Before the traffic task runs: a source the environment configures is configured for
        // every workspace held now, the state the seed will copy (ADR 0075, Decision 7).
        let traffic: BTreeMap<String, TrafficLatest> = if config.traffic_configured() {
            config
                .workspace_entries()
                .0
                .into_iter()
                .map(|w| {
                    (
                        w.id,
                        TrafficLatest {
                            report: None,
                            source: SourceStatus::Configured {
                                fetch_secs: config.traffic_fetch_secs,
                                error: None,
                            },
                        },
                    )
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let (entries, source) = config.workspace_entries();
        let (workspaces, _) =
            tokio::sync::watch::channel(crate::kernel::workspaces::Registry { entries, source });
        let speech = config
            .voice
            .as_ref()
            .and_then(|v| v.speech.as_ref())
            .map(crate::kernel::speech::from_config);
        Arc::new(Self {
            speech: std::sync::RwLock::new(speech),
            db,
            config: Arc::new(config),
            github,
            companions: Mutex::new(HashMap::new()),
            companion_caps: Mutex::new(HashMap::new()),
            web_tx,
            workspaces,
            maps: RwLock::new(BTreeMap::new()),
            metrics: Metrics::new(),
            signer,
            traffic: RwLock::new(traffic),
            clock: Clock::default(),
            sessions: Mutex::new(HashMap::new()),
            webhook_wake: tokio::sync::Notify::new(),
            polled_repos: Mutex::new(BTreeSet::new()),
            slots: Slots::default(),
            welcomed: broadcast::channel(64).0,
        })
    }

    /// `companion_link` answered this companion's `Hello` with a `Welcome`.
    pub fn companion_welcomed(&self, companion_id: Uuid) {
        let _ = self.welcomed.send(companion_id);
    }

    /// Companions as they are welcomed, from now on.
    pub fn subscribe_welcomed(&self) -> broadcast::Receiver<Uuid> {
        self.welcomed.subscribe()
    }

    /// The per-socket id of a connected companion (#161): a reconnect is a new one.
    pub fn companion_conn(&self, companion_id: Uuid) -> Option<Uuid> {
        self.companions
            .lock()
            .unwrap()
            .get(&companion_id)
            .map(|c| c.conn)
    }

    /// The workspaces this server holds now, in switcher order, and where the list came from
    /// (ADR 0044, ADR 0075).
    pub fn workspace_entries(
        &self,
    ) -> (
        Vec<crate::kernel::workspaces::WorkspaceEntry>,
        crate::kernel::workspaces::Source,
    ) {
        let r = self.workspaces.borrow();
        (r.entries.clone(), r.source)
    }

    /// Replace the registry: the composition root at start, and slice `workspace_settings` on a
    /// change (ADR 0075). Every reader of `workspace_entries` sees it at once, and a task that
    /// subscribed is woken.
    pub fn set_workspaces(&self, registry: crate::kernel::workspaces::Registry) {
        self.workspaces.send_replace(registry);
    }

    /// Follow the registry: woken on every `set_workspaces`.
    pub fn subscribe_workspaces(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::kernel::workspaces::Registry> {
        self.workspaces.subscribe()
    }

    /// One workspace's latest traffic report and source state; not configured when it has none.
    pub async fn traffic_of(&self, ws: &str) -> TrafficLatest {
        self.traffic
            .read()
            .await
            .get(ws)
            .cloned()
            .unwrap_or_else(TrafficLatest::not_configured)
    }

    /// The speech adapter, if one is configured (ADR 0073).
    pub fn speech(&self) -> Option<Arc<dyn Speech>> {
        self.speech.read().unwrap().clone()
    }

    /// Replace the speech adapter: tests use a counting or blocking fake.
    pub fn set_speech(&self, s: Option<Arc<dyn Speech>>) {
        *self.speech.write().unwrap() = s;
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

    /// The repos a webhook delivery may name (ADR 0074), lower case: what the last tick read, and
    /// every repo the loaded maps name.
    pub async fn read_repos(&self) -> BTreeSet<String> {
        let mut out = self.polled_repos.lock().unwrap().clone();
        out.extend(
            self.union_repos()
                .await
                .into_iter()
                .map(|r| r.to_ascii_lowercase()),
        );
        out
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
    /// Heard from now. Whether it had gone quiet (#69): the caller records its sessions again.
    pub fn touch_companion(&self, companion_id: Uuid, now: DateTime<Utc>) -> bool {
        match self.companions.lock().unwrap().get_mut(&companion_id) {
            Some(c) => {
                c.last_seen = now;
                std::mem::take(&mut c.quiet)
            }
            None => false,
        }
    }

    /// The latest cached snapshot of one companion, if it sent one (or one was stored).
    pub fn sessions_of(&self, companion_id: Uuid) -> Option<Vec<SessionState>> {
        self.sessions.lock().unwrap().get(&companion_id).cloned()
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
