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

/// What `context` is asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum About<'a> {
    /// A path inside a repository: a file, or a directory.
    Path {
        repo: &'a str,
        path: &'a str,
    },
    /// A map id (`rubato-server`), or a part's or seam's node id.
    Part(&'a str),
    Issue {
        repo: &'a str,
        number: u64,
    },
}

/// A seam of a part asked about: what crosses it and who is on the other side.
#[derive(Debug, Clone, PartialEq)]
pub struct SeamContext {
    pub seam: GraphNode,
    /// The names of the parts it joins.
    pub joins: Vec<String>,
    /// Its declared flows, as `flow` edges between those parts.
    pub flows: Vec<GraphEdge>,
}

/// What to know before touching something: where it sits on the map and what else is there.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Context {
    /// The file or the issue asked about, when the graph has it.
    pub subject: Option<GraphNode>,
    /// The parts that own the path, are named, or the issue is pinned to.
    pub parts: Vec<GraphNode>,
    pub seams: Vec<SeamContext>,
    /// Decision records whose `Seam:` field names one of those parts or seams.
    pub adrs: Vec<GraphNode>,
    /// For a path: the slice and the scenarios of the capability its folders name.
    pub slices: Vec<GraphNode>,
    pub scenarios: Vec<GraphNode>,
    /// Issues pinned to those parts or seams.
    pub issues: Vec<GraphNode>,
    /// Plans that list one of those parts as affected, or touch one of those seams.
    pub plans: Vec<GraphNode>,
    /// Open pull requests for those issues.
    pub pulls: Vec<GraphNode>,
    /// For a file: what it defines, what it imports, what imports it.
    pub symbols: Vec<GraphNode>,
    pub imports: Vec<GraphNode>,
    pub imported_by: Vec<GraphNode>,
    /// A list was cut at the limit.
    pub truncated: bool,
}

impl Context {
    /// The id of every node in the answer.
    pub fn node_ids(&self) -> Vec<String> {
        self.subject
            .iter()
            .chain(&self.parts)
            .chain(self.seams.iter().map(|s| &s.seam))
            .chain(&self.adrs)
            .chain(&self.slices)
            .chain(&self.scenarios)
            .chain(&self.issues)
            .chain(&self.plans)
            .chain(&self.pulls)
            .chain(&self.symbols)
            .chain(&self.imports)
            .chain(&self.imported_by)
            .map(|n| n.id.clone())
            .collect()
    }
}

/// Whether `path` is `dir` or under it. An empty `dir` is the repository's root.
fn under(path: &str, dir: &str) -> bool {
    let dir = dir.trim_matches('/');
    dir.is_empty()
        || path == dir
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn push(list: &mut Vec<GraphNode>, node: GraphNode, limit: usize, truncated: &mut bool) {
    if list.iter().any(|n| n.id == node.id) {
        return;
    }
    if list.len() >= limit {
        *truncated = true;
        return;
    }
    list.push(node);
}

impl Store {
    /// The sources these nodes came from, each once.
    pub async fn sources_of(&self, ids: &[String]) -> Result<Vec<String>> {
        let mut out = BTreeSet::new();
        for chunk in ids.chunks(500) {
            let mut q =
                QueryBuilder::<Sqlite>::new("SELECT DISTINCT source FROM nodes WHERE id IN (");
            let mut list = q.separated(", ");
            for id in chunk {
                list.push_bind(id);
            }
            q.push(")");
            for r in q.build().fetch_all(self.pool()).await? {
                out.insert(sqlx::Row::get::<String, _>(&r, "source"));
            }
        }
        Ok(out.into_iter().collect())
    }

    async fn nodes_where(&self, kind: &str, repo: &str) -> Result<Vec<GraphNode>> {
        sqlx::query("SELECT * FROM nodes WHERE kind = ? AND repo = ? ORDER BY id")
            .bind(kind)
            .bind(repo)
            .fetch_all(self.pool())
            .await?
            .iter()
            .map(node_from_row)
            .collect()
    }

    /// The nodes at the other end of `id`'s edges of these kinds.
    async fn across(
        &self,
        id: &str,
        direction: Direction,
        kinds: &[&str],
        limit: usize,
    ) -> Result<Vec<GraphNode>> {
        let kinds: Vec<String> = kinds.iter().map(|k| (*k).to_owned()).collect();
        let mut out = Vec::new();
        for e in self.edges(id, direction, &kinds, limit + 1).await? {
            let other = if e.from == id { &e.to } else { &e.from };
            if let Some(n) = self.node(other).await? {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// What to know before touching a path, a part or an issue. Each list holds at most
    /// `limit` nodes.
    pub async fn context(&self, about: About<'_>, limit: usize) -> Result<Context> {
        use rubato_protocol::graph::{id, kind};
        let mut c = Context::default();
        let mut cut = false;
        // 1. Where it sits: the parts and seams it is about.
        let mut seams: Vec<GraphNode> = Vec::new();
        match about {
            About::Path { repo, path } => {
                let path = path.trim_matches('/');
                c.subject = self.node(&id::file(repo, path)).await?;
                // The part whose path is the longest prefix, in each workspace that names the repo.
                let mut best: std::collections::BTreeMap<String, GraphNode> = Default::default();
                for part in self.nodes_where(kind::node::PART, repo).await? {
                    let dir = part.path.clone().unwrap_or_default();
                    if !under(path, &dir) {
                        continue;
                    }
                    let ws = part
                        .detail
                        .get("workspace")
                        .and_then(|w| w.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let longer = best
                        .get(&ws)
                        .is_none_or(|b| b.path.as_deref().unwrap_or("").len() < dir.len());
                    if longer {
                        best.insert(ws, part);
                    }
                }
                c.parts = best.into_values().collect();
                let segments: Vec<&str> = path
                    .split('/')
                    .map(|s| s.split('.').next().unwrap_or(s))
                    .collect();
                for s in self.nodes_where(kind::node::SCENARIO, repo).await? {
                    let capability = s
                        .detail
                        .get("capability")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if segments.contains(&capability) {
                        push(&mut c.scenarios, s, limit, &mut cut);
                    }
                }
                for s in self.nodes_where(kind::node::SLICE, repo).await? {
                    let deployable = s
                        .detail
                        .get("deployable")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if segments.contains(&s.name.as_str()) && path.contains(deployable) {
                        push(&mut c.slices, s, limit, &mut cut);
                    }
                }
                if let Some(file) = &c.subject {
                    for n in self
                        .across(&file.id, Direction::Out, &[kind::edge::DEFINES], limit)
                        .await?
                    {
                        push(&mut c.symbols, n, limit, &mut cut);
                    }
                    for n in self
                        .across(&file.id, Direction::Out, &[kind::edge::IMPORTS], limit)
                        .await?
                    {
                        push(&mut c.imports, n, limit, &mut cut);
                    }
                    for n in self
                        .across(&file.id, Direction::In, &[kind::edge::IMPORTS], limit)
                        .await?
                    {
                        push(&mut c.imported_by, n, limit, &mut cut);
                    }
                }
            }
            About::Part(named) => {
                let found: Vec<GraphNode> = if named.starts_with("part:")
                    || named.starts_with("seam:")
                {
                    self.node(named).await?.into_iter().collect()
                } else {
                    sqlx::query("SELECT * FROM nodes WHERE kind IN ('part', 'seam') AND name = ? ORDER BY id")
                        .bind(named)
                        .fetch_all(self.pool())
                        .await?
                        .iter()
                        .map(node_from_row)
                        .collect::<Result<_>>()?
                };
                for n in found {
                    if n.kind == kind::node::SEAM {
                        seams.push(n);
                    } else {
                        c.parts.push(n);
                    }
                }
            }
            About::Issue { repo, number } => {
                let me = id::issue(repo, number);
                c.subject = self.node(&me).await?;
                for n in self
                    .across(&me, Direction::Out, &[kind::edge::PINNED_TO], limit)
                    .await?
                {
                    if n.kind == kind::node::SEAM {
                        seams.push(n);
                    } else {
                        push(&mut c.parts, n, limit, &mut cut);
                    }
                }
                // Its own plan and pull requests, whatever it is pinned to.
                for n in self
                    .across(&me, Direction::In, &[kind::edge::PLANS], limit)
                    .await?
                {
                    push(&mut c.plans, n, limit, &mut cut);
                }
                for n in self
                    .across(&me, Direction::In, &[kind::edge::FOR], limit)
                    .await?
                {
                    push(&mut c.pulls, n, limit, &mut cut);
                }
            }
        }
        // 2. The seams of those parts.
        for part in &c.parts {
            for seam in self
                .across(&part.id, Direction::In, &[kind::edge::JOINS], limit)
                .await?
            {
                if !seams.iter().any(|s| s.id == seam.id) {
                    seams.push(seam);
                }
            }
        }
        if seams.len() > limit {
            seams.truncate(limit);
            cut = true;
        }
        for seam in seams {
            let joined = self
                .across(&seam.id, Direction::Out, &[kind::edge::JOINS], 4)
                .await?;
            let mut flows = Vec::new();
            for part in &joined {
                for e in self
                    .edges(
                        &part.id,
                        Direction::Out,
                        &[kind::edge::FLOW.to_owned()],
                        200,
                    )
                    .await?
                {
                    let on_this_seam =
                        e.detail.get("seam").and_then(|v| v.as_str()) == Some(&seam.name);
                    if on_this_seam && joined.iter().any(|p| p.id == e.to) {
                        flows.push(e);
                    }
                }
            }
            c.seams.push(SeamContext {
                joins: joined.into_iter().map(|p| p.name).collect(),
                seam,
                flows,
            });
        }
        // 3. What names those map ids: decisions and plans. They name ids without a workspace.
        let mut names: Vec<String> = c.parts.iter().map(|p| p.name.clone()).collect();
        names.extend(c.seams.iter().map(|s| s.seam.name.clone()));
        names.sort();
        names.dedup();
        for name in &names {
            let named = id::map_ref(name);
            for n in self
                .across(&named, Direction::In, &[kind::edge::GOVERNS], limit)
                .await?
            {
                push(&mut c.adrs, n, limit, &mut cut);
            }
            for n in self
                .across(
                    &named,
                    Direction::In,
                    &[kind::edge::AFFECTS, kind::edge::TOUCHES],
                    limit,
                )
                .await?
            {
                push(&mut c.plans, n, limit, &mut cut);
            }
        }
        // 4. The work on them: pinned issues, and the pull requests for those and for the plans.
        let mut pinned_at: Vec<String> = c.parts.iter().map(|p| p.id.clone()).collect();
        pinned_at.extend(c.seams.iter().map(|s| s.seam.id.clone()));
        for target in &pinned_at {
            for n in self
                .across(target, Direction::In, &[kind::edge::PINNED_TO], limit)
                .await?
            {
                push(&mut c.issues, n, limit, &mut cut);
            }
        }
        let mut issue_ids: Vec<String> = c.issues.iter().map(|i| i.id.clone()).collect();
        for plan in &c.plans {
            for e in self
                .edges(&plan.id, Direction::Out, &[kind::edge::PLANS.to_owned()], 2)
                .await?
            {
                issue_ids.push(e.to);
            }
        }
        issue_ids.sort();
        issue_ids.dedup();
        for issue in &issue_ids {
            for n in self
                .across(issue, Direction::In, &[kind::edge::FOR], limit)
                .await?
            {
                push(&mut c.pulls, n, limit, &mut cut);
            }
        }
        c.truncated = cut;
        Ok(c)
    }
}
