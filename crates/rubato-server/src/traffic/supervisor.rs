//! The traffic supervisor (ADR 0075): always running, it follows the kernel's workspace
//! registry and keeps one fetch task per workspace with a traffic source, or one task for the
//! development file feeding every workspace. A changed source restarts only its workspace's
//! task; the generation check keeps a cancelled task's report from landing after.

use super::fetch::{Fetcher, Generations, Targets, ws_attrs};
use super::prometheus::{PrometheusConfig, PrometheusSource, SOURCE};
use super::source::{ConfiguredSource, DevFileSource};
use crate::kernel::config::TRAFFIC_FETCH_SECS;
use crate::kernel::state::{App, TrafficLatest};
use crate::kernel::tenancy::WsKey;
use crate::kernel::workspaces::WorkspaceEntry;
use chrono::Utc;
use rubato_protocol::SourceStatus;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// What one task runs: built from a workspace's settings, or why it cannot be.
#[derive(Debug, Clone, PartialEq)]
struct Want {
    /// The revision of the settings it was built from: a change restarts the task.
    revision: u64,
    fetch_secs: u32,
}

/// The supervisor: an essential task. It ends only on shutdown.
pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) -> anyhow::Result<()> {
    let gens = app.slots.get::<Generations>();
    let mut registry = app.subscribe_workspaces();
    let mut running: HashMap<String, (Want, JoinHandle<()>)> = HashMap::new();
    let local = crate::kernel::config::is_local(app.config.bind, &app.config.public_url);
    if app.config.dev_traffic.is_some() {
        tracing::warn!(
            "RUBATO_DEV_TRAFFIC is set: observed traffic comes from that file for every workspace; development and tests only"
        );
    }
    loop {
        let entries = registry.borrow_and_update().entries.clone();
        reconcile(&app, &gens, &mut running, &entries, local).await;
        tokio::select! {
            changed = registry.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = shutdown.changed() => break,
        }
    }
    for (_, (_, h)) in running {
        h.abort();
    }
    Ok(())
}

/// Start, restart or stop tasks so that exactly what the registry says runs.
async fn reconcile(
    app: &App,
    gens: &Arc<Generations>,
    running: &mut HashMap<String, (Want, JoinHandle<()>)>,
    entries: &[WorkspaceEntry],
    local: bool,
) {
    let mut want: BTreeMap<String, (Want, Option<ConfiguredSource>, Targets)> = BTreeMap::new();
    // Written after the stops, so a stopped task cannot overwrite them.
    let mut failing: Vec<(WsKey, u32, String)> = Vec::new();
    if let Some(path) = &app.config.dev_traffic {
        // One task for the development file, feeding every workspace; stored sources ignored.
        if entries.iter().any(|w| w.settings.traffic.is_some()) {
            tracing::warn!(
                "RUBATO_DEV_TRAFFIC is set: the workspaces' stored traffic sources are ignored"
            );
        }
        let fetch_secs = app.config.traffic_fetch_secs;
        want.insert(
            "*".to_owned(),
            (
                Want {
                    revision: 0,
                    fetch_secs,
                },
                Some(ConfiguredSource::DevFile(DevFileSource::new(path.clone()))),
                Targets::All,
            ),
        );
    } else {
        for w in entries {
            let Some(t) = &w.settings.traffic else {
                continue;
            };
            let fetch_secs = fetch_secs_of(&t.fields);
            let built = build(app, w, local, fetch_secs);
            let source = match built {
                Ok(s) => Some(s),
                Err(e) => {
                    failing.push((w.key(), fetch_secs, e));
                    None
                }
            };
            want.insert(
                w.key().to_string(),
                (
                    Want {
                        revision: w.settings.revision,
                        fetch_secs,
                    },
                    source,
                    Targets::One(w.key()),
                ),
            );
        }
    }
    // Stop what is no longer wanted, or wanted differently.
    let stale: Vec<String> = running
        .iter()
        .filter(|(k, (w, _))| want.get(*k).is_none_or(|(n, _, _)| n != w))
        .map(|(k, _)| k.clone())
        .collect();
    for k in stale {
        if let Some((_, h)) = running.remove(&k) {
            gens.bump(&k);
            h.abort();
            let _ = h.await;
            tracing::info!(rubato.workspace = k.as_str(), "traffic task stopped");
        }
    }
    // A source set up wrong is failing, shown as unknown, never as quiet.
    for (ws, fetch_secs, e) in failing {
        tracing::error!(error = %e, rubato.tenant = %ws.tenant, rubato.workspace = ws.ws(), "traffic source failing");
        app.traffic.write().await.insert(
            ws.clone(),
            TrafficLatest {
                report: None,
                source: SourceStatus::Configured {
                    fetch_secs,
                    error: Some(e),
                },
            },
        );
        app.notify_ws("traffic", &ws);
    }
    // A workspace whose source is gone: not configured, and the gauge says so.
    let configured: Vec<WsKey> = match want.contains_key("*") {
        true => entries.iter().map(|w| w.key()).collect(),
        false => want
            .values()
            .filter_map(|(_, _, t)| match t {
                Targets::One(ws) => Some(ws.clone()),
                Targets::All => None,
            })
            .collect(),
    };
    {
        let mut all = app.traffic.write().await;
        let gone: Vec<WsKey> = all
            .keys()
            .filter(|ws| !configured.contains(ws))
            .cloned()
            .collect();
        for ws in gone {
            all.remove(&ws);
            app.notify_ws("traffic", &ws);
        }
    }
    for w in entries {
        app.metrics.traffic_configured.record(
            u64::from(configured.contains(&w.key())),
            &ws_attrs(&w.key()),
        );
    }
    // Start what is wanted and not running.
    for (k, (w, source, targets)) in want {
        if running.contains_key(&k) {
            continue;
        }
        let Some(source) = source else {
            continue;
        };
        let generation = gens.bump(&k);
        let fetcher = Fetcher::new(source, Utc::now())
            .with_timeout(Duration::from_secs(u64::from(
                w.fetch_secs.saturating_sub(1).max(1),
            )))
            .feeding(
                targets.clone(),
                w.fetch_secs,
                Some((gens.clone(), generation)),
            );
        let started_for = targets;
        let secs = u64::from(w.fetch_secs);
        let task_app = app.clone();
        let handle = tokio::spawn(async move {
            let app = task_app;
            let mut fetcher = fetcher;
            loop {
                if fetcher.tick(&app, Utc::now()).await == super::Outcome::Superseded {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(secs)).await;
            }
        });
        tracing::info!(
            rubato.workspace = k.as_str(),
            generation,
            "traffic task started"
        );
        if let Targets::One(ws) = &started_for {
            app.notify_ws("traffic", ws);
        }
        running.insert(k, (w, handle));
    }
}

/// A workspace's fetch interval: its `fetch_secs` field, clamped to the bounds the variable had,
/// else the default.
fn fetch_secs_of(fields: &BTreeMap<String, String>) -> u32 {
    let (min, default, max) = TRAFFIC_FETCH_SECS;
    fields
        .get("fetch_secs")
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(default, |v| v.clamp(min, max))
}

/// The source a workspace's settings describe, with its secrets opened.
fn build(
    app: &App,
    w: &WorkspaceEntry,
    local: bool,
    fetch_secs: u32,
) -> Result<ConfiguredSource, String> {
    let t = w.settings.traffic.as_ref().ok_or("no traffic source")?;
    if t.source != SOURCE {
        return Err(format!("unknown traffic source {:?}", t.source));
    }
    let secrets: BTreeMap<String, String> = match &t.secrets_sealed {
        None => BTreeMap::new(),
        Some(sealed) => {
            let key = app
                .config
                .token_key
                .as_ref()
                .ok_or("secrets_unsealable: RUBATO_TOKEN_KEY is unset")?;
            let plain = crate::kernel::secrets::open(key, sealed).map_err(
                |_| "secrets_unsealable: the stored secrets do not open with RUBATO_TOKEN_KEY",
            )?;
            serde_json::from_str(&plain).map_err(|_| "secrets_unsealable: not a JSON object")?
        }
    };
    match PrometheusConfig::from_fields(&t.fields, &secrets, fetch_secs)? {
        Some(c) => {
            tracing::info!(rubato.workspace = w.id.as_str(), url = %c.url, window_secs = c.window_secs, "traffic from the service graph");
            Ok(ConfiguredSource::Prometheus(PrometheusSource::guarded(
                c, local,
            )))
        }
        None => Err("the traffic source has no url".into()),
    }
}
