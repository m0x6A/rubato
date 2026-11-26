//! Building sources (ADR 0084). A repository source: one call for the default-branch head, and
//! only when the head or the generator moved, the tarball, the extraction and a whole-source
//! replace. Repositories are built one at a time across the server, so a burst is one tarball
//! and one parse.

use super::extract::{Extracted, Extractor};
use super::metrics;
use crate::kernel::github::Tarball;
use crate::kernel::state::App;
use anyhow::{Context, Result};
use rubato_graph::{Build, Replaced, Store};
use rubato_protocol::graph::{kind, source_key};
use rubato_protocol::limits;
use rubato_protocol::otel::span;
use std::collections::BTreeSet;
use std::io::Read;
use std::time::Duration;
use tracing::Instrument;

/// The extraction's version. A bump rebuilds every repository source once, at its next tick.
pub const GENERATOR: u32 = 1;

/// The whole of one tarball download. The next tick is the retry.
const TARBALL_DEADLINE: Duration = Duration::from_secs(120);

/// What one build of one source came to (`rubato.outcome` on `rubato.graph.build`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The source's rows are new.
    Built,
    /// The stored rows are still what the repository or the tables say.
    Unchanged,
    /// The build failed for this reason; the previous rows stay.
    Failed(&'static str),
    /// A newer build or a removal overtook this one; it wrote nothing.
    Lost,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Built => "built",
            Outcome::Unchanged => "unchanged",
            Outcome::Failed(_) => "failed",
            Outcome::Lost => "lost",
        }
    }
    pub fn reason(&self) -> &'static str {
        match self {
            Outcome::Failed(r) => r,
            _ => "",
        }
    }
}

/// What one tick did, for its tests and its log line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tick {
    pub built: Vec<String>,
    pub unchanged: Vec<String>,
    pub failed: Vec<(String, &'static str)>,
    pub removed: Vec<String>,
}

impl Tick {
    /// Whether what the overview shows changed.
    fn changed(&self) -> bool {
        !(self.built.is_empty() && self.failed.is_empty() && self.removed.is_empty())
    }
}

/// Read a tarball as GitHub serves it (gzip, one top-level directory) as a stream, entry by
/// entry: the repository is never whole in memory. Stops at the caps and says which one.
pub(super) fn read_tarball(gz: &[u8], repo: &str) -> Result<Extracted> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(gz));
    let mut extractor = Extractor::new(repo);
    let mut read: u64 = 0;
    let mut buf = Vec::new();
    for entry in archive.entries().context("reading the tarball")? {
        let mut entry = entry.context("reading a tarball entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let size = entry.size();
        read += size;
        if read > limits::GRAPH_TARBALL_READ_MAX_BYTES {
            extractor.cut("tarball_read");
            break;
        }
        let path = entry.path().context("a tarball path")?;
        // Drop the top-level directory GitHub adds (`<owner>-<repo>-<sha>/`).
        let Some(path) = path
            .to_str()
            .and_then(|p| p.split_once('/'))
            .map(|(_, rest)| rest.to_owned())
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        if size as usize <= limits::GRAPH_PARSE_FILE_MAX_BYTES {
            buf.clear();
            entry
                .read_to_end(&mut buf)
                .with_context(|| format!("reading {path}"))?;
            extractor.file(&path, size, Some(&buf));
        } else {
            extractor.file(&path, size, None);
        }
        if extractor.is_full() {
            break;
        }
    }
    Ok(extractor.finish())
}

/// Build one repository's source if its head or the generator moved.
pub(super) async fn repo(app: &App, store: &Store, repo: &str) -> Outcome {
    let key = source_key::repo(repo);
    let span = tracing::info_span!(
        span::GRAPH_BUILD,
        rubato.graph.source = %key,
        rubato.repo = repo,
        rubato.outcome = tracing::field::Empty,
        rubato.reason = tracing::field::Empty,
        nodes = tracing::field::Empty,
        edges = tracing::field::Empty,
    );
    let outcome = async {
        let was_failing = matches!(store.source(&key).await, Ok(Some(row)) if row.error.is_some());
        match build_repo(app, store, repo, &key).await {
            Ok(outcome) => outcome,
            Err((reason, e)) => {
                if let Err(e) = store.record_error(&key, app.clock.now(), &format!("{e:#}")).await {
                    tracing::error!(rubato.graph.source = %key, error = %e, "context graph error not recorded");
                }
                // Once on the way into failing, not on every tick of an outage.
                if !was_failing {
                    tracing::warn!(
                        rubato.graph.source = %key,
                        rubato.reason = reason,
                        error = %format!("{e:#}"),
                        "context graph build failed"
                    );
                }
                Outcome::Failed(reason)
            }
        }
    }
    .instrument(span.clone())
    .await;
    span.record("rubato.outcome", outcome.as_str());
    span.record("rubato.reason", outcome.reason());
    metrics::build(&key, &outcome);
    outcome
}

type Failed = (&'static str, anyhow::Error);

async fn build_repo(app: &App, store: &Store, repo: &str, key: &str) -> Result<Outcome, Failed> {
    let fail = |reason: &'static str| move |e: anyhow::Error| (reason, e);
    let now = app.clock.now();
    let head = app
        .github
        .default_head(repo)
        .await
        .map_err(fail("head"))?
        .ok_or_else(|| ("head", anyhow::anyhow!("{repo} has no default-branch head")))?;
    let stored = store.source(key).await.map_err(|e| ("store", e.into()))?;
    if let Some(row) = &stored
        && row.content_hash.is_some()
        && row.generator == GENERATOR
        && row.head_sha.as_deref() == Some(head.as_str())
    {
        store
            .mark_checked(key, now)
            .await
            .map_err(|e| ("store", e.into()))?;
        return Ok(Outcome::Unchanged);
    }
    let generation = store
        .begin_build(key, kind::source::REPO)
        .await
        .map_err(|e| ("store", e.into()))?;
    let tarball = tokio::time::timeout(
        TARBALL_DEADLINE,
        app.github
            .tarball(repo, &head, limits::GRAPH_TARBALL_MAX_BYTES),
    )
    .await
    .map_err(|_| {
        (
            "tarball",
            anyhow::anyhow!("the tarball of {repo} took over {TARBALL_DEADLINE:?}"),
        )
    })?
    .map_err(fail("tarball"))?;
    let extracted = match tarball {
        // Over the cap is a cut source, never a failed build: it says so in every answer.
        Tarball::TooLarge => Extracted {
            truncated: Some("tarball"),
            ..Extracted::default()
        },
        Tarball::Gz(gz) => {
            let repo = repo.to_owned();
            tokio::task::spawn_blocking(move || read_tarball(&gz, &repo))
                .await
                .map_err(|e| ("extract", anyhow::anyhow!("extraction stopped: {e}")))?
                .map_err(fail("extract"))?
        }
    };
    if let Some(cap) = extracted.truncated {
        metrics::truncated(key, cap);
    }
    let counts = extracted.counts.clone();
    let build = Build {
        source: key.to_owned(),
        kind: kind::source::REPO.into(),
        head_sha: Some(head.clone()),
        map_version: None,
        generator: GENERATOR,
        generated_at: now,
        truncated: extracted.truncated.is_some(),
        nodes: extracted.nodes,
        edges: extracted.edges,
    };
    match store
        .replace_source(Some(generation), build, now)
        .await
        .map_err(|e| ("store", e.into()))?
    {
        Replaced::Lost => Ok(Outcome::Lost),
        Replaced::Unchanged { .. } => Ok(Outcome::Unchanged),
        Replaced::Written {
            nodes,
            edges,
            duplicates,
            ..
        } => {
            let span = tracing::Span::current();
            span.record("nodes", nodes);
            span.record("edges", edges);
            tracing::info!(
                rubato.graph.source = %key,
                head = %head,
                nodes,
                edges,
                duplicates,
                files = counts.files,
                adrs = counts.adrs,
                scenarios = counts.scenarios,
                slices = counts.slices,
                unread_documents = counts.unread_documents,
                "context graph source built"
            );
            Ok(Outcome::Built)
        }
    }
}

/// One pass over every repository source: each repository the loaded maps name and the
/// installation covers, one at a time; then the sources whose repository is gone.
pub async fn tick(app: &App) -> Tick {
    let mut tick = Tick::default();
    let Some(store) = super::store(app) else {
        return tick;
    };
    let named = app.union_repos().await;
    let uncovered: BTreeSet<String> = crate::github_poll::uncovered(&app.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.to_ascii_lowercase())
        .collect();
    let mut wanted = BTreeSet::new();
    for repo in &named {
        // A repository the installation does not cover is a finding in the overview already;
        // asking for it would fail every tick.
        if uncovered.contains(&repo.to_ascii_lowercase()) {
            continue;
        }
        let key = source_key::repo(repo);
        match self::repo(app, &store, repo).await {
            Outcome::Built => tick.built.push(key.clone()),
            Outcome::Unchanged | Outcome::Lost => tick.unchanged.push(key.clone()),
            Outcome::Failed(reason) => tick.failed.push((key.clone(), reason)),
        }
        wanted.insert(key);
    }
    // Never on a tick with no map loaded: the list a companion is sent is the truth, and one
    // bad tick must not empty every copy.
    if !named.is_empty() {
        for row in store.sources().await.unwrap_or_default() {
            if row.kind != kind::source::REPO || wanted.contains(&row.source) {
                continue;
            }
            // Uncovered now, covered before: keep what was built until the map stops naming it.
            let still_named = named
                .iter()
                .any(|r| source_key::repo(r).eq_ignore_ascii_case(&row.source));
            if still_named {
                continue;
            }
            match store.remove_source(&row.source).await {
                Ok(_) => {
                    tracing::info!(rubato.graph.source = %row.source, "context graph source removed");
                    tick.removed.push(row.source);
                }
                Err(e) => {
                    tracing::error!(rubato.graph.source = %row.source, error = %e, "context graph source not removed")
                }
            }
        }
    }
    metrics::observe(
        &store,
        app.config
            .context_graph
            .as_ref()
            .map_or(0, |c| c.regen_secs),
    )
    .await;
    if tick.changed() {
        app.notify("context_graph");
    }
    tick
}

/// The timer: a tick at start, then one every `every`, until shutdown. It never ends on an
/// error; a failing source is a recorded failure, not a dead task.
pub async fn run(app: App, every: Duration, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    loop {
        tick(&app).await;
        tokio::select! {
            _ = tokio::time::sleep(every) => {}
            _ = shutdown.changed() => return,
        }
    }
}
