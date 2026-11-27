//! Keeping the copy in step with what the server announced (ADR 0086). A snapshot is believed
//! only after it hashed to its own header and to the announced hash; anything else leaves the
//! copy as it was. One writer: this task.

use super::{Fetched, Graph};
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_graph::{Replaced, Snapshot, Store};
use rubato_protocol::limits;
use rubato_protocol::otel::{attr, metric, span};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::Instrument;

/// A failed fetch is tried again on the next announce, and after this long without one.
const RETRY: Duration = Duration::from_secs(300);

/// What one pass over the announced list did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    /// Sources fetched, checked and applied.
    pub applied: Vec<String>,
    /// Sources left as they were, with why (`rubato.reason`).
    pub kept: Vec<(String, &'static str)>,
    /// Sources the list no longer names, removed from the copy.
    pub removed: Vec<String>,
    /// The server said the pairing is gone: the copy was deleted.
    pub deleted_copy: bool,
}

struct Instruments {
    syncs: opentelemetry::metrics::Counter<u64>,
}

fn instruments() -> &'static Instruments {
    static I: OnceLock<Instruments> = OnceLock::new();
    I.get_or_init(|| Instruments {
        syncs: opentelemetry::global::meter("rubato-companion")
            .u64_counter(metric::GRAPH_SYNCS)
            .build(),
    })
}

/// The gauge the alert reads: observed when collected, so it rises while the copy stays behind.
fn observe_behind(graph: &Graph, companion_id: uuid::Uuid) {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    let graph = graph.clone();
    REGISTERED.get_or_init(move || {
        let who = [KeyValue::new(attr::COMPANION_ID, companion_id.to_string())];
        let _gauge = opentelemetry::global::meter("rubato-companion")
            .u64_observable_gauge(metric::GRAPH_COPY_BEHIND)
            .with_callback(move |g| g.observe(graph.behind_secs(Utc::now()), &who))
            .build();
    });
}

fn count(source: &str, outcome: &'static str, reason: &'static str) {
    let mut attrs = vec![
        KeyValue::new(attr::GRAPH_SOURCE, source.to_owned()),
        KeyValue::new(attr::OUTCOME, outcome),
    ];
    if !reason.is_empty() {
        attrs.push(KeyValue::new(attr::DROP_REASON, reason));
    }
    instruments().syncs.add(1, &attrs);
}

/// Remove the copy and SQLite's side files.
pub(super) async fn delete_copy(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = tokio::fs::remove_file(std::path::PathBuf::from(name)).await;
    }
}

/// The copy and SQLite's side files are the owner's alone, as the token beside them is.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for suffix in ["", "-wal", "-shm"] {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            let _ = std::fs::set_permissions(
                std::path::PathBuf::from(name),
                std::fs::Permissions::from_mode(0o600),
            );
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Why a fetched source was not applied.
async fn take(
    store: &Store,
    announced_hash: &str,
    source: &str,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<(), (&'static str, String)> {
    let snapshot = Snapshot::parse(body, limits::GRAPH_SNAPSHOT_READ_MAX_BYTES).map_err(|e| {
        let text = e.to_string();
        (
            if text.contains("hash") {
                "hash_mismatch"
            } else {
                "parse"
            },
            text,
        )
    })?;
    if snapshot.header.source != source {
        return Err((
            "wrong_source",
            format!("asked for {source}, got {}", snapshot.header.source),
        ));
    }
    if snapshot.header.content_hash != announced_hash {
        // The server built a newer one meanwhile: the next announce is already on its way.
        return Err((
            "superseded",
            "the server has a newer one than it announced".to_owned(),
        ));
    }
    match store.apply(snapshot, now).await {
        Ok(Replaced::Written { .. } | Replaced::Unchanged { .. }) => Ok(()),
        Ok(Replaced::Lost) => Err(("apply", "the copy moved on".to_owned())),
        Err(e) => Err(("apply", e.to_string())),
    }
}

/// One pass: fetch what differs from the announced list, apply what checks out, remove what the
/// list no longer names.
pub async fn pass(graph: &Graph, now: DateTime<Utc>) -> Pass {
    let mut pass = Pass::default();
    let Some(announced) = graph.0.state.lock().unwrap().announced.clone() else {
        return pass;
    };
    if let Some(dir) = graph.0.path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    let store = match Store::open_path(&graph.0.path).await {
        Ok(store) => store,
        Err(e) => {
            tracing::warn!(error = %e, rubato.reason = "apply", "context graph copy not opened");
            return pass;
        }
    };
    restrict(&graph.0.path);
    let local = store.sources().await.unwrap_or_default();
    let has = |source: &str| local.iter().find(|r| r.source == source);
    let differs = announced
        .iter()
        .any(|a| has(&a.source).and_then(|r| r.content_hash.as_ref()) != Some(&a.content_hash));
    if differs {
        graph
            .0
            .state
            .lock()
            .unwrap()
            .behind_since
            .get_or_insert(now);
    }
    let mut refused = false;
    for a in &announced {
        if let Some(row) = has(&a.source)
            && row.content_hash.as_ref() == Some(&a.content_hash)
        {
            // The same graph, perhaps at a newer head: say where it stands, fetch nothing.
            if row.head_sha != a.head_sha || row.map_version != a.map_version {
                let _ = store
                    .set_head(&a.source, a.head_sha.as_deref(), a.map_version.as_deref())
                    .await;
            }
            continue;
        }
        if refused {
            pass.kept.push((a.source.clone(), "team_record_stale"));
            continue;
        }
        let sync = tracing::info_span!(
            span::GRAPH_SYNC,
            rubato.graph.source = %a.source,
            rubato.outcome = tracing::field::Empty,
            rubato.reason = tracing::field::Empty,
        );
        let fetched = (graph.0.fetch)(a.source.clone())
            .instrument(sync.clone())
            .await;
        let outcome: Result<(), (&'static str, String)> = match fetched {
            Fetched::Body(body) => take(&store, &a.content_hash, &a.source, &body, now).await,
            Fetched::Unauthorized => {
                // The pairing is unknown or revoked: nothing of the team's stays here.
                sync.record("rubato.outcome", "refused");
                sync.record("rubato.reason", "unauthorized");
                count(&a.source, "refused", "unauthorized");
                store.close().await;
                tracing::warn!(
                    rubato.reason = "unauthorized",
                    "context graph copy deleted: the pairing is gone"
                );
                graph.forget().await;
                pass.deleted_copy = true;
                return pass;
            }
            Fetched::NotEntitled => {
                refused = true;
                Err((
                    "team_record_stale",
                    "log in to Rubato: the owner's team record is over 7 days old".to_owned(),
                ))
            }
            Fetched::Failed(e) => Err(("http", e)),
        };
        let mut state = graph.0.state.lock().unwrap();
        match outcome {
            Ok(()) => {
                sync.record("rubato.outcome", "applied");
                count(&a.source, "applied", "");
                state.errors.remove(&a.source);
                state.not_refreshed = false;
                pass.applied.push(a.source.clone());
            }
            Err((reason, error)) => {
                sync.record("rubato.outcome", "kept");
                sync.record("rubato.reason", reason);
                count(&a.source, "kept", reason);
                if reason == "team_record_stale" {
                    state.not_refreshed = true;
                }
                // A source the server replaced meanwhile is not a fault to tell anyone about.
                if reason != "superseded" && state.errors.get(&a.source) != Some(&error) {
                    tracing::warn!(
                        rubato.graph.source = %a.source,
                        rubato.reason = reason,
                        error = %error,
                        "context graph copy not applied"
                    );
                }
                state.errors.insert(a.source.clone(), error);
                pass.kept.push((a.source.clone(), reason));
            }
        }
    }
    // An empty list is a server that has built nothing yet (a new graph file): not a reason to
    // empty the copy. A list with sources is the truth about which ones there are.
    if !announced.is_empty() {
        for row in &local {
            if announced.iter().any(|a| a.source == row.source) {
                continue;
            }
            if let Ok(true) = store.remove_source(&row.source).await {
                graph.0.state.lock().unwrap().errors.remove(&row.source);
                pass.removed.push(row.source.clone());
            }
        }
    }
    let after = store.sources().await.unwrap_or_default();
    let in_step = announced.iter().all(|a| {
        after
            .iter()
            .any(|r| r.source == a.source && r.content_hash.as_ref() == Some(&a.content_hash))
    });
    if in_step {
        graph.0.state.lock().unwrap().behind_since = None;
    }
    restrict(&graph.0.path);
    store.close().await;
    pass
}

/// The sync task: a pass on every announce, and one every five minutes for what failed.
pub async fn run(
    graph: Graph,
    companion_id: uuid::Uuid,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    observe_behind(&graph, companion_id);
    loop {
        tokio::select! {
            _ = graph.0.wake.notified() => {}
            _ = tokio::time::sleep(RETRY) => {}
            _ = shutdown.changed() => return Ok(()),
        }
        pass(&graph, Utc::now()).await;
    }
}
