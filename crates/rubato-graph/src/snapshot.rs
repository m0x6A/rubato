//! A snapshot: one source as gzipped NDJSON, a header line and then the nodes in id order and
//! the edges in (`from`, `kind`, `to`, `detail`) order. The functions here are the only ones
//! that turn a node or an edge into a line and the only ones that hash lines, so the hash the
//! server stores at build time is the hash of what it later serves.

use crate::store::{edge_from_row, node_from_row};
use crate::{Error, Result, Store};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use rubato_protocol::graph::{
    GraphEdge, GraphNode, GraphSnapshotHeader, GraphSnapshotLine, SNAPSHOT_FORMAT,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};

/// Rows read per step while writing a snapshot.
const PAGE: i64 = 5_000;

/// `detail` with its keys sorted at every depth: the same detail is the same text, whatever
/// order it was built in.
pub(crate) fn canonical(detail: &Map<String, Value>) -> Map<String, Value> {
    fn value(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(canonical(m)),
            Value::Array(a) => Value::Array(a.iter().map(value).collect()),
            other => other.clone(),
        }
    }
    let mut keys: Vec<&String> = detail.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|k| (k.clone(), value(&detail[k])))
        .collect()
}

/// A detail as the store keeps it: canonical JSON, `{}` when empty.
pub(crate) fn detail_text(detail: &Map<String, Value>) -> String {
    serde_json::to_string(&canonical(detail)).expect("a JSON object serialises")
}

/// One node as its snapshot line. The node's detail must already be canonical.
pub fn node_line(node: &GraphNode) -> String {
    #[derive(serde::Serialize)]
    struct Line<'a> {
        node: &'a GraphNode,
    }
    serde_json::to_string(&Line { node }).expect("a node serialises")
}

/// One edge as its snapshot line. The edge's detail must already be canonical.
pub fn edge_line(edge: &GraphEdge) -> String {
    #[derive(serde::Serialize)]
    struct Line<'a> {
        edge: &'a GraphEdge,
    }
    serde_json::to_string(&Line { edge }).expect("an edge serialises")
}

pub(crate) struct Hasher(Sha256);

impl Hasher {
    pub(crate) fn new() -> Self {
        Self(Sha256::new())
    }
    pub(crate) fn line(&mut self, line: &str) {
        self.0.update(line.as_bytes());
        self.0.update(b"\n");
    }
    pub(crate) fn finish(self) -> String {
        let mut out = String::from("sha256:");
        for b in self.0.finalize() {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

/// `sha256:<hex>` over the lines, each followed by a newline.
pub fn content_hash<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut h = Hasher::new();
    for l in lines {
        h.line(l);
    }
    h.finish()
}

/// A snapshot that was read and checked: its hash is its header's.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub header: GraphSnapshotHeader,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl Snapshot {
    /// Read a gzipped body, refusing one that is over `max_uncompressed` bytes, of another
    /// format, or whose lines do not hash and count to what its header says.
    pub fn parse(gz: &[u8], max_uncompressed: u64) -> Result<Snapshot> {
        let bad = |why: String| Error::Snapshot(why);
        let mut reader = BufReader::new(GzDecoder::new(gz).take(max_uncompressed + 1));
        let mut read: u64 = 0;
        let mut line = String::new();
        let mut next = |line: &mut String| -> Result<bool> {
            line.clear();
            let n = reader.read_line(line)?;
            read += n as u64;
            if read > max_uncompressed {
                return Err(bad(format!("body is over {max_uncompressed} bytes")));
            }
            Ok(n > 0)
        };
        if !next(&mut line)? {
            return Err(bad("empty body".into()));
        }
        let header: GraphSnapshotHeader =
            serde_json::from_str(line.trim_end()).map_err(|e| bad(format!("header: {e}")))?;
        if header.format != SNAPSHOT_FORMAT {
            return Err(bad(format!(
                "format {} is not {SNAPSHOT_FORMAT}",
                header.format
            )));
        }
        let mut hasher = Hasher::new();
        let (mut nodes, mut edges) = (Vec::new(), Vec::new());
        while next(&mut line)? {
            let text = line.trim_end_matches('\n');
            if text.is_empty() {
                continue;
            }
            hasher.line(text);
            match serde_json::from_str::<GraphSnapshotLine>(text)
                .map_err(|e| bad(format!("line {}: {e}", nodes.len() + edges.len() + 2)))?
            {
                GraphSnapshotLine::Node(n) => nodes.push(n),
                GraphSnapshotLine::Edge(e) => edges.push(e),
            }
        }
        let hash = hasher.finish();
        if hash != header.content_hash {
            return Err(bad(format!(
                "hash {hash} is not the header's {}",
                header.content_hash
            )));
        }
        if nodes.len() as u64 != header.nodes || edges.len() as u64 != header.edges {
            return Err(bad(format!(
                "{} nodes and {} edges, the header says {} and {}",
                nodes.len(),
                edges.len(),
                header.nodes,
                header.edges
            )));
        }
        Ok(Snapshot {
            header,
            nodes,
            edges,
        })
    }
}

impl Store {
    /// One source as a gzipped snapshot body, read in one read transaction, or `None` when the
    /// source is unknown or was never built. Fails when the lines served do not hash to what
    /// was stored at build time: a body nobody could apply is never sent.
    pub async fn export(&self, source: &str) -> Result<Option<Vec<u8>>> {
        let mut tx = self.pool().begin().await?;
        let Some(row) = crate::store::source_in(&mut tx, source).await? else {
            return Ok(None);
        };
        let (Some(content_hash), Some(generated_at)) = (row.content_hash.clone(), row.generated_at)
        else {
            return Ok(None);
        };
        let header = GraphSnapshotHeader {
            format: SNAPSHOT_FORMAT,
            source: row.source.clone(),
            kind: row.kind.clone(),
            head_sha: row.head_sha.clone(),
            map_version: row.map_version.clone(),
            generator: row.generator,
            content_hash: content_hash.clone(),
            generated_at,
            nodes: row.nodes,
            edges: row.edges,
            truncated: row.truncated,
        };
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        writeln!(gz, "{}", serde_json::to_string(&header)?)?;
        let mut hasher = Hasher::new();
        // Rows of a source are inserted in canonical order, so rowid order is that order; the
        // hash check below is what would notice if that ever stopped being true.
        let mut after: i64 = 0;
        loop {
            let rows = sqlx::query(
                "SELECT rowid, id, kind, name, repo, path, line, end_line, summary, detail \
                 FROM nodes WHERE source = ? AND rowid > ? ORDER BY rowid LIMIT ?",
            )
            .bind(source)
            .bind(after)
            .bind(PAGE)
            .fetch_all(&mut *tx)
            .await?;
            let Some(last) = rows.last() else { break };
            after = sqlx::Row::get(last, "rowid");
            for r in &rows {
                let line = node_line(&node_from_row(r)?);
                hasher.line(&line);
                writeln!(gz, "{line}")?;
            }
        }
        let mut after: i64 = 0;
        loop {
            let rows = sqlx::query(
                "SELECT rowid, from_id, kind, to_id, detail FROM edges \
                 WHERE source = ? AND rowid > ? ORDER BY rowid LIMIT ?",
            )
            .bind(source)
            .bind(after)
            .bind(PAGE)
            .fetch_all(&mut *tx)
            .await?;
            let Some(last) = rows.last() else { break };
            after = sqlx::Row::get(last, "rowid");
            for r in &rows {
                let line = edge_line(&edge_from_row(r)?);
                hasher.line(&line);
                writeln!(gz, "{line}")?;
            }
        }
        let served = hasher.finish();
        if served != content_hash {
            return Err(Error::Snapshot(format!(
                "{source}: the stored hash {content_hash} is not the hash of its rows, {served}"
            )));
        }
        Ok(Some(gz.finish()?))
    }
}
