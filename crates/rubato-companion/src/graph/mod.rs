//! Slice `graph`: this machine's copy of the team's context graph (ADR 0086). The server
//! announces its sources on the websocket; the slice fetches each one that differs, checks it
//! against its own hash and the announced one, and applies it whole to `graph.sqlite` in the
//! state directory. The graph tools answer from that file, with or without the server.
//!
//! Nothing leaves the machine through this slice: the copy only flows in. The slice names no
//! other slice; the runtime hands it each `GraphSources` and what the server's `Welcome` said.

mod fetch;
mod sync;

pub use fetch::{Fetch, Fetched, http};
pub use sync::{Pass, pass, run};

use chrono::{DateTime, Utc};
use rubato_graph::{SourceRow, Store};
use rubato_protocol::graph::{GraphSourceInfo, GraphSources};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The copy's file name in the state directory.
pub const COPY_FILE: &str = "graph.sqlite";

/// One source of the copy, as a tool or `status` says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopySource {
    pub row: SourceRow,
    /// Whether the server has announced a newer one than the copy holds. `None` when no
    /// announce has arrived since this companion started: not known, never "current".
    pub behind: Option<bool>,
    /// The server refused to refresh it: the owner has no fresh team record (log in to Rubato).
    pub not_refreshed: bool,
    /// Why the last fetch of it did not end in an apply.
    pub error: Option<String>,
}

/// The copy as a whole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopyStatus {
    pub sources: Vec<CopySource>,
    /// Whether the server's last `Welcome` offered the graph. `None` before the first one.
    pub offered: Option<bool>,
}

#[derive(Debug, Default)]
struct State {
    /// The newest list the server sent. `None` until one arrives.
    announced: Option<Vec<GraphSourceInfo>>,
    offered: Option<bool>,
    not_refreshed: bool,
    errors: BTreeMap<String, String>,
    /// Since when the copy has not matched the announced list; `None` while it does.
    behind_since: Option<DateTime<Utc>>,
}

struct Inner {
    path: PathBuf,
    fetch: Fetch,
    state: Mutex<State>,
    wake: tokio::sync::Notify,
}

/// The slice's handle: the runtime feeds it, the sync task works through it, the tools read
/// through it. Cloning shares it.
#[derive(Clone)]
pub struct Graph(Arc<Inner>);

impl Graph {
    pub fn new(state_dir: &Path, fetch: Fetch) -> Self {
        Self(Arc::new(Inner {
            path: state_dir.join(COPY_FILE),
            fetch,
            state: Mutex::new(State::default()),
            wake: tokio::sync::Notify::new(),
        }))
    }

    /// Where the copy is.
    pub fn path(&self) -> &Path {
        &self.0.path
    }

    /// The server sent its list: the truth about what this copy should hold.
    pub fn on_sources(&self, sources: GraphSources) {
        if let Err(n) = sources.check() {
            // A server never sends this; a list we cannot trust changes nothing.
            tracing::warn!(
                sources = n,
                "context graph: announced list over the cap, ignored"
            );
            return;
        }
        self.0.state.lock().unwrap().announced = Some(sources.sources);
        self.0.wake.notify_one();
    }

    /// What the server's `Welcome` said about the graph. Not offered changes nothing in the
    /// copy: it keeps answering, with its age.
    pub fn on_welcome(&self, offered: bool) {
        self.0.state.lock().unwrap().offered = Some(offered);
    }

    /// The copy for reading, or `None` when there is none yet (or it is of another version).
    pub async fn reader(&self) -> Option<Store> {
        Store::open_read_only(&self.0.path).await.ok().flatten()
    }

    /// Every source of the copy with how it stands against what the server announced.
    pub async fn status(&self) -> CopyStatus {
        let rows = match self.reader().await {
            Some(store) => {
                let rows = store.sources().await.unwrap_or_default();
                store.close().await;
                rows
            }
            None => Vec::new(),
        };
        let state = self.0.state.lock().unwrap();
        CopyStatus {
            sources: rows
                .into_iter()
                .map(|row| {
                    let behind = state.announced.as_ref().map(|list| {
                        list.iter()
                            .find(|a| a.source == row.source)
                            .is_some_and(|a| Some(&a.content_hash) != row.content_hash.as_ref())
                    });
                    CopySource {
                        error: state.errors.get(&row.source).cloned(),
                        not_refreshed: state.not_refreshed,
                        behind,
                        row,
                    }
                })
                .collect(),
            offered: state.offered,
        }
    }

    /// Seconds the copy has been behind the announced list; 0 while it matches.
    pub fn behind_secs(&self, now: DateTime<Utc>) -> u64 {
        self.0
            .state
            .lock()
            .unwrap()
            .behind_since
            .map_or(0, |since| (now - since).num_seconds().max(0) as u64)
    }

    /// The pairing is gone: so is the copy.
    pub async fn forget(&self) {
        sync::delete_copy(&self.0.path).await;
        let mut state = self.0.state.lock().unwrap();
        state.announced = None;
        state.errors.clear();
        state.behind_since = None;
    }
}

/// What `rubato-companion status` says about the copy: one line per source, or that there is
/// none. Read from the file alone, so it answers with no companion running.
pub async fn report(state_dir: &Path) -> Vec<String> {
    let Ok(Some(store)) = Store::open_read_only(&state_dir.join(COPY_FILE)).await else {
        return vec!["context graph: no copy on this machine".to_owned()];
    };
    let rows = store.sources().await.unwrap_or_default();
    store.close().await;
    let mut out = vec![format!("context graph: a copy of {} source(s)", rows.len())];
    for r in rows {
        let at = match (&r.head_sha, &r.map_version) {
            (Some(head), _) => format!(" at {}", &head[..head.len().min(7)]),
            (None, Some(map)) => format!(" of map {}", &map[..map.len().min(7)]),
            (None, None) => String::new(),
        };
        let generated = r.generated_at.map_or("never built".to_owned(), |t| {
            format!("generated {}", t.to_rfc3339())
        });
        out.push(format!(
            "  {}{at}: {generated}, {} nodes, {} edges{}",
            r.source,
            r.nodes,
            r.edges,
            if r.truncated { ", cut by a cap" } else { "" }
        ));
    }
    out
}

#[cfg(test)]
mod tests;
