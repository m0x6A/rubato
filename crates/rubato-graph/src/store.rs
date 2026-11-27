//! The graph file: opening it, and replacing or removing one source at a time.

use crate::snapshot::{Hasher, canonical, detail_text, edge_line, node_line};
use crate::{Result, Snapshot, schema};
use chrono::{DateTime, Utc};
use rubato_protocol::graph::{GraphEdge, GraphNode};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteTransaction,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use std::str::FromStr;
use std::time::Duration;

/// Rows per `INSERT`: under SQLite's bound-parameter limit with ten columns a row.
const CHUNK: usize = 2_000;

/// The graph file, open. Cloning shares the pool.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

/// One row of `sources`: what a source was built from and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    pub source: String,
    pub kind: String,
    /// Bumped by every build start; a replace that names an older one writes nothing.
    pub generation: i64,
    pub head_sha: Option<String>,
    pub map_version: Option<String>,
    pub generator: u32,
    /// `None` until the first build.
    pub content_hash: Option<String>,
    pub generated_at: Option<DateTime<Utc>>,
    /// When a build last found the stored rows current.
    pub checked_at: Option<DateTime<Utc>>,
    /// The newest build's time and error, when the newest build failed.
    pub attempted_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub nodes: u64,
    pub edges: u64,
    pub truncated: bool,
}

/// One source, built: everything `replace_source` stores.
#[derive(Debug, Clone, PartialEq)]
pub struct Build {
    pub source: String,
    pub kind: String,
    pub head_sha: Option<String>,
    pub map_version: Option<String>,
    pub generator: u32,
    pub generated_at: DateTime<Utc>,
    pub truncated: bool,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl From<Snapshot> for Build {
    fn from(s: Snapshot) -> Self {
        Build {
            source: s.header.source,
            kind: s.header.kind,
            head_sha: s.header.head_sha,
            map_version: s.header.map_version,
            generator: s.header.generator,
            generated_at: s.header.generated_at,
            truncated: s.header.truncated,
            nodes: s.nodes,
            edges: s.edges,
        }
    }
}

/// What a replace did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replaced {
    /// The source's rows are the build's now.
    Written {
        content_hash: String,
        nodes: u64,
        edges: u64,
        /// Nodes or edges the build named twice; the first of each was kept.
        duplicates: u64,
    },
    /// The build hashes to what is stored: only the source's row was touched.
    Unchanged { content_hash: String },
    /// A newer build started, or the source was removed, after this one began. Nothing was
    /// written.
    Lost,
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

fn parse_ts(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn source_from_row(r: &SqliteRow) -> SourceRow {
    SourceRow {
        source: r.get("source"),
        kind: r.get("kind"),
        generation: r.get("generation"),
        head_sha: r.get("head_sha"),
        map_version: r.get("map_version"),
        generator: r.get::<i64, _>("generator") as u32,
        content_hash: r.get("content_hash"),
        generated_at: parse_ts(r.get("generated_at")),
        checked_at: parse_ts(r.get("checked_at")),
        attempted_at: parse_ts(r.get("attempted_at")),
        error: r.get("error"),
        nodes: r.get::<i64, _>("nodes") as u64,
        edges: r.get::<i64, _>("edges") as u64,
        truncated: r.get::<i64, _>("truncated") != 0,
    }
}

pub(crate) fn node_from_row(r: &SqliteRow) -> Result<GraphNode> {
    Ok(GraphNode {
        id: r.get("id"),
        kind: r.get("kind"),
        name: r.get("name"),
        repo: r.get("repo"),
        path: r.get("path"),
        line: r.get::<Option<i64>, _>("line").map(|n| n as u32),
        end_line: r.get::<Option<i64>, _>("end_line").map(|n| n as u32),
        summary: r.get("summary"),
        detail: serde_json::from_str(r.get::<&str, _>("detail"))?,
    })
}

pub(crate) fn edge_from_row(r: &SqliteRow) -> Result<GraphEdge> {
    Ok(GraphEdge {
        from: r.get("from_id"),
        kind: r.get("kind"),
        to: r.get("to_id"),
        detail: serde_json::from_str(r.get::<&str, _>("detail"))?,
    })
}

pub(crate) async fn source_in(
    tx: &mut SqliteTransaction<'_>,
    source: &str,
) -> Result<Option<SourceRow>> {
    Ok(sqlx::query("SELECT * FROM sources WHERE source = ?")
        .bind(source)
        .fetch_optional(&mut **tx)
        .await?
        .as_ref()
        .map(source_from_row))
}

impl Store {
    /// Open the file at a `sqlite:` URL, creating it when missing. A file of another schema
    /// version is emptied and created again: the graph is derived, never migrated.
    pub async fn open(url: &str) -> Result<Store> {
        let memory = url.contains(":memory:");
        let mut opts = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(10));
        if !memory {
            opts = opts.journal_mode(SqliteJournalMode::Wal);
        }
        let pool = SqlitePoolOptions::new()
            // One connection in memory: each connection there is its own database.
            .max_connections(if memory { 1 } else { 4 })
            .connect_with(opts)
            .await?;
        schema::ensure(&pool).await?;
        Ok(Store { pool })
    }

    /// Open the file at a path, creating it when missing: for a companion's copy, whose
    /// state directory may hold characters a URL would not.
    pub async fn open_path(path: &std::path::Path) -> Result<Store> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await?;
        schema::ensure(&pool).await?;
        Ok(Store { pool })
    }

    /// Open an existing file for reading only. `None` when there is no file yet, or when it is
    /// of another schema version: both mean "no copy to answer from".
    pub async fn open_read_only(path: &std::path::Path) -> Result<Option<Store>> {
        if !path.exists() {
            return Ok(None);
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        if schema::user_version(&pool).await? != schema::USER_VERSION {
            return Ok(None);
        }
        Ok(Some(Store { pool }))
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Every source, by key.
    pub async fn sources(&self) -> Result<Vec<SourceRow>> {
        Ok(sqlx::query("SELECT * FROM sources ORDER BY source")
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(source_from_row)
            .collect())
    }

    pub async fn source(&self, source: &str) -> Result<Option<SourceRow>> {
        Ok(sqlx::query("SELECT * FROM sources WHERE source = ?")
            .bind(source)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(source_from_row))
    }

    /// Start a build of `source`: its generation, which the build passes to `replace_source`.
    /// Every start bumps it, so of two builds of one source only the one started last writes.
    pub async fn begin_build(&self, source: &str, kind: &str) -> Result<i64> {
        let row = sqlx::query(
            "INSERT INTO sources (source, kind, generation) VALUES (?, ?, 1) \
             ON CONFLICT(source) DO UPDATE SET generation = generation + 1 \
             RETURNING generation",
        )
        .bind(source)
        .bind(kind)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("generation"))
    }

    /// The stored rows of `source` are still what its repository or tables say: note the time,
    /// and clear a failure the last attempt left.
    pub async fn mark_checked(&self, source: &str, at: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE sources SET checked_at = ?, attempted_at = NULL, error = NULL WHERE source = ?",
        )
        .bind(ts(at))
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The same graph stands at another head: a companion whose copy has the announced hash
    /// records where the source is now, without fetching anything.
    pub async fn set_head(
        &self,
        source: &str,
        head_sha: Option<&str>,
        map_version: Option<&str>,
    ) -> Result<()> {
        sqlx::query("UPDATE sources SET head_sha = ?, map_version = ? WHERE source = ?")
            .bind(head_sha)
            .bind(map_version)
            .bind(source)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Close every connection, so the file can be removed.
    pub async fn close(self) {
        self.pool.close().await;
    }

    /// A build of `source` failed. Its previous rows stay; the error shows next to them.
    pub async fn record_error(&self, source: &str, at: DateTime<Utc>, error: &str) -> Result<()> {
        sqlx::query("UPDATE sources SET attempted_at = ?, error = ? WHERE source = ?")
            .bind(ts(at))
            .bind(error)
            .bind(source)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Replace every row of one source with `build`, in one transaction: readers see the old
    /// source or the new one, never half.
    ///
    /// `generation` is what `begin_build` answered. When the source has moved on from it, or is
    /// gone, nothing is written and the answer is `Lost`. `None` skips the check: a companion
    /// applying a snapshot has one writer.
    ///
    /// A build that hashes to what is stored, from the same generator, writes no rows.
    pub async fn replace_source(
        &self,
        generation: Option<i64>,
        mut build: Build,
        now: DateTime<Utc>,
    ) -> Result<Replaced> {
        // Canonical order, each thing once. The order is the snapshot's and the hash's.
        for n in &mut build.nodes {
            n.detail = canonical(&n.detail);
        }
        build.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut duplicates = build.nodes.len();
        build.nodes.dedup_by(|later, first| later.id == first.id);
        duplicates -= build.nodes.len();
        let mut edges: Vec<(GraphEdge, String)> = build
            .edges
            .drain(..)
            .map(|mut e| {
                e.detail = canonical(&e.detail);
                let text = detail_text(&e.detail);
                (e, text)
            })
            .collect();
        edges.sort_by(|(a, at), (b, bt)| {
            (&a.from, &a.kind, &a.to, at).cmp(&(&b.from, &b.kind, &b.to, bt))
        });
        let before = edges.len();
        edges.dedup_by(|(l, lt), (f, ft)| {
            l.from == f.from && l.kind == f.kind && l.to == f.to && lt == ft
        });
        duplicates += before - edges.len();

        let mut hasher = Hasher::new();
        for n in &build.nodes {
            hasher.line(&node_line(n));
        }
        for (e, _) in &edges {
            hasher.line(&edge_line(e));
        }
        let content_hash = hasher.finish();

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let stored = source_in(&mut tx, &build.source).await?;
        if let Some(g) = generation
            && stored.as_ref().map(|s| s.generation) != Some(g)
        {
            return Ok(Replaced::Lost);
        }
        if let Some(s) = &stored
            && s.content_hash.as_deref() == Some(content_hash.as_str())
            && s.generator == build.generator
        {
            // The same graph, perhaps at a newer head: the row says where it stands now.
            sqlx::query(
                "UPDATE sources SET head_sha = ?, map_version = ?, checked_at = ?, \
                 attempted_at = NULL, error = NULL WHERE source = ?",
            )
            .bind(&build.head_sha)
            .bind(&build.map_version)
            .bind(ts(now))
            .bind(&build.source)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(Replaced::Unchanged { content_hash });
        }

        delete_rows(&mut tx, &build.source).await?;
        for chunk in build.nodes.chunks(CHUNK) {
            let mut q = QueryBuilder::<Sqlite>::new(
                "INSERT INTO nodes (source, id, kind, name, repo, path, line, end_line, summary, detail) ",
            );
            q.push_values(chunk, |mut b, n| {
                b.push_bind(&build.source)
                    .push_bind(&n.id)
                    .push_bind(&n.kind)
                    .push_bind(&n.name)
                    .push_bind(&n.repo)
                    .push_bind(&n.path)
                    .push_bind(n.line.map(i64::from))
                    .push_bind(n.end_line.map(i64::from))
                    .push_bind(&n.summary)
                    .push_bind(detail_text(&n.detail));
            });
            q.build().execute(&mut *tx).await?;
            let mut q = QueryBuilder::<Sqlite>::new(
                "INSERT INTO nodes_fts (name, path, summary, source, id) ",
            );
            q.push_values(chunk, |mut b, n| {
                b.push_bind(&n.name)
                    .push_bind(&n.path)
                    .push_bind(&n.summary)
                    .push_bind(&build.source)
                    .push_bind(&n.id);
            });
            q.build().execute(&mut *tx).await?;
        }
        for chunk in edges.chunks(CHUNK) {
            let mut q = QueryBuilder::<Sqlite>::new(
                "INSERT INTO edges (source, from_id, kind, to_id, detail) ",
            );
            q.push_values(chunk, |mut b, (e, text)| {
                b.push_bind(&build.source)
                    .push_bind(&e.from)
                    .push_bind(&e.kind)
                    .push_bind(&e.to)
                    .push_bind(text);
            });
            q.build().execute(&mut *tx).await?;
        }
        let (nodes, edge_count) = (build.nodes.len() as i64, edges.len() as i64);
        sqlx::query(
            "INSERT INTO sources (source, kind, generation, head_sha, map_version, generator, \
             content_hash, generated_at, checked_at, attempted_at, error, nodes, edges, truncated) \
             VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?) \
             ON CONFLICT(source) DO UPDATE SET kind = excluded.kind, head_sha = excluded.head_sha, \
             map_version = excluded.map_version, generator = excluded.generator, \
             content_hash = excluded.content_hash, generated_at = excluded.generated_at, \
             checked_at = excluded.checked_at, attempted_at = NULL, error = NULL, \
             nodes = excluded.nodes, edges = excluded.edges, truncated = excluded.truncated",
        )
        .bind(&build.source)
        .bind(&build.kind)
        .bind(&build.head_sha)
        .bind(&build.map_version)
        .bind(i64::from(build.generator))
        .bind(&content_hash)
        .bind(ts(build.generated_at))
        .bind(ts(now))
        .bind(nodes)
        .bind(edge_count)
        .bind(i64::from(build.truncated))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Replaced::Written {
            content_hash,
            nodes: nodes as u64,
            edges: edge_count as u64,
            duplicates: duplicates as u64,
        })
    }

    /// Apply a snapshot that was read and checked. One writer, so no generation.
    pub async fn apply(&self, snapshot: Snapshot, now: DateTime<Utc>) -> Result<Replaced> {
        self.replace_source(None, snapshot.into(), now).await
    }

    /// Remove a source and its rows. A build of it still running then finds no row and writes
    /// nothing. Returns whether there was one.
    pub async fn remove_source(&self, source: &str) -> Result<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        delete_rows(&mut tx, source).await?;
        let gone = sqlx::query("DELETE FROM sources WHERE source = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(gone > 0)
    }

    /// Every column of every table, for the check that no progress state is stored.
    pub async fn all_columns(&self) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for table in ["sources", "nodes", "edges"] {
            for c in sqlx::query(&format!("PRAGMA table_info({table})"))
                .fetch_all(&self.pool)
                .await?
            {
                out.push((table.to_owned(), c.get::<String, _>("name")));
            }
        }
        Ok(out)
    }
}

async fn delete_rows(tx: &mut SqliteTransaction<'_>, source: &str) -> Result<()> {
    for sql in [
        "DELETE FROM nodes WHERE source = ?",
        "DELETE FROM edges WHERE source = ?",
        "DELETE FROM nodes_fts WHERE source = ?",
    ] {
        sqlx::query(sql).bind(source).execute(&mut **tx).await?;
    }
    Ok(())
}
