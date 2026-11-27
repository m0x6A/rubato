//! Slice `context_graph`: the context graph's sources (ADR 0084). A source is one repository at
//! its default-branch head, one workspace's map and pins, or the team's shared facts; each is
//! built on a timer and replaced whole in the slice's own SQLite file, through
//! `rubato_graph::Store`. The slice owns that file and no table of the main database, so a
//! replace of a hundred thousand rows never holds the writer that hooks, the poll and web
//! writes wait on.
//!
//! Behind `RUBATO_CONTEXT_GRAPH`, off by default: then there is no file, no task, and the
//! overview carries no `graph`. The graph is never an input to progress, pins, plans or
//! approvals.
//!
//! Built so far: the store and each source's status (#325), and repository sources on a timer
//! with their files and decision documents (#326). The workspace and team sources arrive with
//! #327, symbols with #328, the snapshot route with #329.

mod build;
mod extract;
mod metrics;

pub use build::{GENERATOR, Outcome, Tick, run, tick};

use crate::kernel::state::App;
use anyhow::{Context, Result};
use rubato_graph::{SourceRow, Store};
use rubato_protocol::graph::{GraphOverview, GraphSourceStatus};

const SLICE: &str = "context_graph";

/// The open graph file, set once at start. Empty while the flag is off.
#[derive(Default)]
struct Holder(std::sync::OnceLock<Store>);

/// Open the graph file when the flag is on. The composition root calls it once, before the
/// router serves: a file that cannot be opened ends the start with its name.
pub async fn start(app: &App) -> Result<()> {
    let Some(cfg) = &app.config.context_graph else {
        return Ok(());
    };
    let store = Store::open(&cfg.database_url)
        .await
        .with_context(|| format!("opening the context graph file {}", cfg.database_url))?;
    let _ = app.slots.get::<Holder>().0.set(store);
    Ok(())
}

/// The graph file; `None` while the flag is off.
pub fn store(app: &App) -> Option<Store> {
    app.slots.get::<Holder>().0.get().cloned()
}

/// `Overview.graph`: the interval and each source with its times, or `None` while the flag is
/// off. A graph file that cannot be read is logged and shows as no sources; it never fails the
/// overview, which is how people see everything else.
pub async fn overview(app: &App) -> Option<GraphOverview> {
    let cfg = app.config.context_graph.as_ref()?;
    let sources = match store(app) {
        None => Vec::new(),
        Some(store) => match store.sources().await {
            Ok(rows) => rows.iter().map(status).collect(),
            Err(e) => {
                tracing::error!(
                    slice = SLICE,
                    error = %e,
                    "context graph sources not read"
                );
                Vec::new()
            }
        },
    };
    Some(GraphOverview {
        regen_secs: cfg.regen_secs,
        sources,
    })
}

fn status(r: &SourceRow) -> GraphSourceStatus {
    GraphSourceStatus {
        source: r.source.clone(),
        kind: r.kind.clone(),
        head_sha: r.head_sha.clone(),
        map_version: r.map_version.clone(),
        generated_at: r.generated_at,
        checked_at: r.checked_at,
        error: r.error.clone(),
        nodes: r.nodes,
        edges: r.edges,
        truncated: r.truncated,
    }
}

#[cfg(test)]
mod build_tests;
#[cfg(test)]
mod tests;
