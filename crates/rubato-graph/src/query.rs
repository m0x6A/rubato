//! Reads: what the graph tools ask. Every query is bounded by a limit its caller caps.

use crate::store::{edge_from_row, node_from_row};
use crate::{Result, Store};
use rubato_protocol::graph::{GraphEdge, GraphNode};
use sqlx::{QueryBuilder, Sqlite};
use std::collections::BTreeSet;

/// Which way to follow edges from a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    Both,
}

/// What a traversal reached.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Related {
    /// The nodes reached, the start left out, in the order they were found.
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// The limit cut it.
    pub truncated: bool,
}

/// What someone typed, as an FTS5 query: its words, each a quoted prefix, all required. Nothing
/// of FTS5's own syntax survives, so no input is an error.
fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(12)
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

impl Store {
    /// The node with this id.
    pub async fn node(&self, id: &str) -> Result<Option<GraphNode>> {
        sqlx::query("SELECT * FROM nodes WHERE id = ? ORDER BY source LIMIT 1")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .as_ref()
            .map(node_from_row)
            .transpose()
    }

    /// Nodes whose name, path or summary match every word of `text`, best first.
    pub async fn search(
        &self,
        text: &str,
        kinds: &[String],
        repo: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GraphNode>> {
        let Some(fts) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let mut q = QueryBuilder::<Sqlite>::new(
            "SELECT n.* FROM nodes_fts f JOIN nodes n ON n.source = f.source AND n.id = f.id \
             WHERE nodes_fts MATCH ",
        );
        q.push_bind(fts);
        if !kinds.is_empty() {
            q.push(" AND n.kind IN (");
            let mut list = q.separated(", ");
            for k in kinds {
                list.push_bind(k);
            }
            q.push(")");
        }
        if let Some(repo) = repo {
            q.push(" AND n.repo = ").push_bind(repo);
        }
        // A match in the name counts for more than one in the path, and that more than one in
        // the summary.
        q.push(" ORDER BY bm25(nodes_fts, 8.0, 2.0, 1.0), n.id LIMIT ")
            .push_bind(limit as i64);
        q.build()
            .fetch_all(self.pool())
            .await?
            .iter()
            .map(node_from_row)
            .collect()
    }

    /// The edges at `id` in one direction, by kind when `kinds` is not empty.
    pub async fn edges(
        &self,
        id: &str,
        direction: Direction,
        kinds: &[String],
        limit: usize,
    ) -> Result<Vec<GraphEdge>> {
        let mut out = Vec::new();
        for (column, wanted) in [("from_id", Direction::Out), ("to_id", Direction::In)] {
            if direction != Direction::Both && direction != wanted {
                continue;
            }
            let mut q = QueryBuilder::<Sqlite>::new(format!(
                "SELECT from_id, kind, to_id, detail FROM edges WHERE {column} = "
            ));
            q.push_bind(id);
            if !kinds.is_empty() {
                q.push(" AND kind IN (");
                let mut list = q.separated(", ");
                for k in kinds {
                    list.push_bind(k);
                }
                q.push(")");
            }
            q.push(" ORDER BY kind, from_id, to_id LIMIT ")
                .push_bind(limit.saturating_sub(out.len()) as i64);
            for r in q.build().fetch_all(self.pool()).await? {
                out.push(edge_from_row(&r)?);
            }
        }
        Ok(out)
    }

    /// Breadth first from `id`, at most `depth` steps and `limit` nodes. An edge whose other end
    /// is not a node in this file (a `map:` reference, a source not copied yet) is returned with
    /// the edges and reaches no node.
    pub async fn related(
        &self,
        id: &str,
        kinds: &[String],
        direction: Direction,
        depth: u8,
        limit: usize,
    ) -> Result<Related> {
        let mut out = Related::default();
        let mut seen: BTreeSet<String> = BTreeSet::from([id.to_owned()]);
        let mut frontier = vec![id.to_owned()];
        'levels: for _ in 0..depth {
            let mut next = Vec::new();
            for at in &frontier {
                for e in self.edges(at, direction, kinds, limit + 1).await? {
                    let other = if &e.from == at { &e.to } else { &e.from }.clone();
                    if !out.edges.contains(&e) {
                        out.edges.push(e);
                    }
                    if !seen.insert(other.clone()) {
                        continue;
                    }
                    let Some(node) = self.node(&other).await? else {
                        continue;
                    };
                    if out.nodes.len() >= limit {
                        out.truncated = true;
                        break 'levels;
                    }
                    out.nodes.push(node);
                    next.push(other);
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::fts_query;

    #[test]
    fn what_someone_typed_never_reaches_fts5_as_syntax() {
        let cases = [
            ("replace_source", Some(r#""replace"* "source"*"#)),
            (
                "crates/rubato-server",
                Some(r#""crates"* "rubato"* "server"*"#),
            ),
            (r#"a" OR "b"#, Some(r#""a"* "OR"* "b"*"#)),
            ("NEAR(x y)", Some(r#""NEAR"* "x"* "y"*"#)),
            ("  -- * ", None),
            ("", None),
        ];
        for (typed, want) in cases {
            assert_eq!(fts_query(typed).as_deref(), want, "{typed:?}");
        }
        println!("checked {} typed queries", cases.len());
    }
}
