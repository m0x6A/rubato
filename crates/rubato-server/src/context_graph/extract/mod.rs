//! What a repository's files say, as nodes and edges (ADR 0084): every tracked file, and the
//! documents with a regular structure. Pure: paths and bytes in, a graph out, so it is tested
//! without GitHub and without SQLite. No file's contents are kept; a decision document's
//! Decision section is, capped.

mod code;
mod docs;

pub use code::CodeCounts;

use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind};
use rubato_protocol::limits;
use serde_json::{Map, Value, json};

/// How many of each thing one repository gave, for the build's log line and its tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub files: usize,
    pub adrs: usize,
    pub scenarios: usize,
    pub slices: usize,
    /// Documents of a known place that did not read as one (an ADR with no title line).
    pub unread_documents: usize,
    /// Symbols, imports and references from source files (ADR 0085).
    pub code: CodeCounts,
}

/// One repository, extracted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extracted {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub counts: Counts,
    /// The cap that cut it, if one did (`rubato.reason` on `rubato.graph.truncated`).
    pub truncated: Option<&'static str>,
}

/// Collects one repository's files in any order.
pub struct Extractor {
    repo: String,
    out: Extracted,
    code: code::Code,
}

fn language(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    Some(match ext {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "cs" => "csharp",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "rb" => "ruby",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "md" => "markdown",
        "feature" => "gherkin",
        "yml" | "yaml" => "yaml",
        "json" => "json",
        "toml" => "toml",
        "html" => "html",
        "css" => "css",
        _ => return None,
    })
}

pub(crate) fn object(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
    pairs
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
}

impl Extractor {
    pub fn new(repo: &str) -> Self {
        Self {
            repo: repo.to_owned(),
            out: Extracted::default(),
            code: code::Code::new(),
        }
    }

    /// Whether a cap has cut this repository: the caller stops feeding it.
    pub fn is_full(&self) -> bool {
        self.out.truncated.is_some()
    }

    /// Say that a cap outside the extractor cut the repository.
    pub fn cut(&mut self, cap: &'static str) {
        self.out.truncated.get_or_insert(cap);
    }

    /// One tracked file. `bytes` is its content when it was small enough to read; without it
    /// the file is still a node.
    pub fn file(&mut self, path: &str, size: u64, bytes: Option<&[u8]>) {
        if self.out.counts.files >= limits::GRAPH_FILES_MAX {
            self.cut("files");
            return;
        }
        if self.out.nodes.len() >= limits::GRAPH_NODES_MAX {
            self.cut("nodes");
            return;
        }
        let text = bytes.and_then(|b| std::str::from_utf8(b).ok());
        self.out.counts.files += 1;
        self.out.nodes.push(GraphNode {
            id: id::file(&self.repo, path),
            kind: kind::node::FILE.into(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            repo: Some(self.repo.clone()),
            path: Some(path.to_owned()),
            line: None,
            end_line: None,
            summary: None,
            detail: object([
                ("language", language(path).map_or(Value::Null, |l| json!(l))),
                ("bytes", json!(size)),
                (
                    "lines",
                    text.map_or(Value::Null, |t| json!(t.lines().count())),
                ),
            ]),
        });
        let Some(text) = text else { return };
        if code::Lang::of(path).is_some() && !code::is_vendored(path) {
            self.code.file(path, text);
            return;
        }
        if docs::is_adr_path(path) {
            match docs::adr(&self.repo, path, text) {
                Some((node, edges)) => {
                    self.out.counts.adrs += 1;
                    self.out.nodes.push(node);
                    self.out.edges.extend(edges);
                }
                None => self.out.counts.unread_documents += 1,
            }
        } else if docs::is_feature_path(path) {
            let found = docs::scenarios(&self.repo, path, text);
            self.out.counts.scenarios += found.len();
            self.out.nodes.extend(found);
        } else if path == "AGENTS.md" {
            let found = docs::slices(&self.repo, text);
            self.out.counts.slices += found.len();
            self.out.nodes.extend(found);
        }
    }

    pub fn finish(self) -> Extracted {
        let Extractor {
            repo,
            mut out,
            code,
        } = self;
        let paths: std::collections::BTreeSet<String> = out
            .nodes
            .iter()
            .filter(|n| n.kind == kind::node::FILE)
            .filter_map(|n| n.path.clone())
            .collect();
        let (nodes, edges, counts) = code.finish(&repo, &paths);
        out.nodes.extend(nodes);
        out.edges.extend(edges);
        out.counts.code = counts;
        if out.nodes.len() > limits::GRAPH_NODES_MAX {
            out.nodes.truncate(limits::GRAPH_NODES_MAX);
            out.truncated.get_or_insert("nodes");
        }
        if out.edges.len() > limits::GRAPH_EDGES_MAX {
            out.edges.truncate(limits::GRAPH_EDGES_MAX);
            out.truncated.get_or_insert("edges");
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests;
