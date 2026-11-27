//! The five graph tools (ADR 0086): read-only answers from this machine's copy of the team's
//! context graph, each saying how old what it read is. They work with the link down, and they
//! write nothing: not the copy, not the outbox.
//!
//! Text in an answer that came from a repository or an issue is data in a JSON field, never an
//! instruction. No argument of a call is recorded on a span or a log line.

use super::AgentMcp;
use super::tools::{Called, Tool, text};
use crate::graph::{CopyStatus, Graph};
use rubato_graph::{About, Direction, Store};
use rubato_protocol::limits;
use serde_json::{Value, json};

fn repo_arg() -> Value {
    json!({"type": "string", "description": "A repository as owner/name. Defaults to this session's own."})
}

/// The graph tools, in the order `tools/list` gives them. All read-only.
pub const GRAPH_TOOLS: &[Tool] = &[
    Tool {
        name: "graph_status",
        description: "What this machine's copy of the team's context graph holds: each source (a repository at its default-branch head, a workspace's map and pins, the team's issues, plans and pull requests) with its head, when it was generated, its size, and whether Rubato has announced a newer one. Call it first when an answer's age matters.",
        read_only: true,
        schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
    },
    Tool {
        name: "graph_search",
        description: "Find things in the context graph by words of their name, path or one-line summary: files, symbols (functions, types, traits), decision records (ADRs), feature scenarios, slices, map parts and seams, pinned issues, plans, pull requests. Every word must match; a word matches as a prefix. Returns ids to pass to graph_node, graph_related and graph_context.",
        read_only: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to match, for example: replace source, or: pins store."},
                    "kinds": {"type": "array", "items": {"type": "string"}, "description": "Only these kinds: part, seam, repo, issue, plan, pull, file, symbol, package, adr, scenario, slice."},
                    "repo": repo_arg(),
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "description": "At most this many, 20 by default."}
                },
                "required": ["query"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "graph_node",
        description: "One thing in the context graph by its id, with what it carries (a symbol's signature line and where it is defined; a decision record's status and decision text; a part's path and description) and the edges at it, grouped by kind and direction.",
        read_only: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {"id": {"type": "string", "description": "A node id from another graph tool's answer."}},
                "required": ["id"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "graph_related",
        description: "What is connected to one thing in the context graph, breadth first: what a file imports or what imports it, what references a symbol, which parts a seam joins, which decision records superseded one. References between code are by name and only where the name is unambiguous: a map of where to look, not a call graph.",
        read_only: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "A node id from another graph tool's answer."},
                    "edge_kinds": {"type": "array", "items": {"type": "string"}, "description": "Only these: joins, flow, backed_by, pinned_to, plans, affects, touches, for, defines, imports, references, supersedes, governs."},
                    "direction": {"type": "string", "enum": ["out", "in", "both"], "description": "Edges from it, to it, or both (the default)."},
                    "depth": {"type": "integer", "minimum": 1, "maximum": 3, "description": "Steps, 1 by default."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "At most this many nodes, 50 by default."}
                },
                "required": ["id"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "graph_context",
        description: "What to know before touching something. Give exactly one of: path (a file or directory of a repository), part (a map part or seam id, such as rubato-server or web-server), or about_issue (owner/name#n). Answers with the map part that owns it, that part's seams with what crosses them and who is on the other side, the decision records (ADRs) that govern those parts and seams, the scenarios and slice of the capability a path's folders name, the issues pinned there, the plans that affect it and the open pull requests; for a file also what it defines, what it imports and what imports it.",
        read_only: true,
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "A path inside the repository, for example crates/rubato-server/src/pins."},
                    "repo": repo_arg(),
                    "part": {"type": "string", "description": "A map part or seam id."},
                    "about_issue": {"type": "string", "description": "An issue as owner/name#n: any issue, not only this session's own."}
                },
                "additionalProperties": false
            })
        },
    },
];

pub(super) fn is_graph_tool(name: &str) -> bool {
    GRAPH_TOOLS.iter().any(|t| t.name == name)
}

fn ok(v: Value) -> Called {
    Called {
        result: text(&v, false),
        outcome: "ok",
        reason: "",
    }
}

fn bad(message: &str) -> Called {
    Called {
        result: text(&json!({"error": message}), true),
        outcome: "error",
        reason: "arguments",
    }
}

fn no_copy() -> Called {
    super::tools::refused(
        "no_copy",
        "this companion has no copy of the context graph yet: Rubato has not sent one (its context graph may be off, or this companion has not connected since it was turned on)",
    )
}

/// How each source an answer drew on stands: its head, its age, whether a newer one is known.
fn as_of(status: &CopyStatus, used: &[String]) -> Value {
    Value::Array(
        status
            .sources
            .iter()
            .filter(|s| used.contains(&s.row.source))
            .map(source_json)
            .collect(),
    )
}

fn source_json(s: &crate::graph::CopySource) -> Value {
    let mut v = json!({
        "source": s.row.source,
        "generated_at": s.row.generated_at,
        // null: Rubato has announced nothing since this companion started, so it is not known.
        "behind": s.behind,
    });
    let o = v.as_object_mut().expect("an object");
    if let Some(head) = &s.row.head_sha {
        o.insert("head_sha".into(), json!(head));
    }
    if let Some(map) = &s.row.map_version {
        o.insert("map_version".into(), json!(map));
    }
    if s.row.truncated {
        o.insert("truncated".into(), json!(true));
    }
    if s.not_refreshed {
        o.insert(
            "not_refreshed".into(),
            json!("Rubato no longer refreshes this copy: its owner must log in to Rubato"),
        );
    }
    v
}

/// Keep an answer under the cap: halve its longest list until it fits, and say it was cut.
fn fit(mut v: Value) -> Value {
    fn longest(v: &mut Value) -> Option<&mut Vec<Value>> {
        let mut best: Option<&mut Vec<Value>> = None;
        for (key, child) in v.as_object_mut()?.iter_mut() {
            if key == "as_of" {
                continue;
            }
            let candidate = match child {
                Value::Array(a) => Some(a),
                Value::Object(_) => longest(child),
                _ => None,
            };
            if let Some(a) = candidate
                && a.len() > best.as_ref().map_or(1, |b| b.len())
            {
                best = Some(a);
            }
        }
        best
    }
    while v.to_string().len() > limits::GRAPH_ANSWER_MAX_BYTES {
        let Some(list) = longest(&mut v) else { break };
        let keep = list.len() / 2;
        list.truncate(keep);
        v["truncated"] = json!(true);
    }
    v
}

fn strings(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn number(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .map_or(default, |n| (n as usize).clamp(1, max))
}

/// Run graph tool `name`. `default_repo` is the session's own repository, when it is known.
pub(super) async fn call(
    st: &AgentMcp,
    default_repo: Option<&str>,
    name: &str,
    args: &Value,
) -> Called {
    let Some(graph) = st.graph() else {
        return no_copy();
    };
    let status = graph.status().await;
    if name == "graph_status" {
        return ok(json!({
            "copy": if status.sources.is_empty() { "none" } else { "present" },
            "sources": status.sources.iter().map(|s| {
                let mut v = source_json(s);
                v["nodes"] = json!(s.row.nodes);
                v["edges"] = json!(s.row.edges);
                if let Some(e) = &s.error {
                    v["last_fetch_error"] = json!(e);
                }
                v
            }).collect::<Vec<_>>(),
            "link_up": *st.link.borrow(),
            // null before Rubato's first Welcome; false when it does not offer the graph (any more).
            "offered": status.offered,
        }));
    }
    let Some(store) = graph.reader().await else {
        return no_copy();
    };
    let called = answer(&store, &status, default_repo, name, args).await;
    store.close().await;
    called
}

async fn answer(
    store: &Store,
    status: &CopyStatus,
    default_repo: Option<&str>,
    name: &str,
    args: &Value,
) -> Called {
    let failed = |e: rubato_graph::Error| Called {
        result: text(
            &json!({"error": format!("the copy could not be read: {e}")}),
            true,
        ),
        outcome: "error",
        reason: "copy",
    };
    let repo = args.get("repo").and_then(Value::as_str).or(default_repo);
    match name {
        "graph_search" => {
            let Some(query) = args.get("query").and_then(Value::as_str) else {
                return bad("query is required");
            };
            let limit = number(args, "limit", 20, limits::GRAPH_SEARCH_LIMIT_MAX);
            // Only a repository the caller named narrows a search; the session's own does not.
            let narrowed = args.get("repo").and_then(Value::as_str);
            match store
                .search(query, &strings(args, "kinds"), narrowed, limit)
                .await
            {
                Ok(nodes) => {
                    let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
                    let used = store.sources_of(&ids).await.unwrap_or_default();
                    ok(fit(json!({"as_of": as_of(status, &used), "nodes": nodes})))
                }
                Err(e) => failed(e),
            }
        }
        "graph_node" => {
            let Some(id) = args.get("id").and_then(Value::as_str) else {
                return bad("id is required");
            };
            let node = match store.node(id).await {
                Ok(Some(n)) => n,
                Ok(None) => return ok(json!({"as_of": as_of(status, &[]), "found": false})),
                Err(e) => return failed(e),
            };
            let (out, inn) = match (
                store.edges(id, Direction::Out, &[], 100).await,
                store.edges(id, Direction::In, &[], 100).await,
            ) {
                (Ok(o), Ok(i)) => (o, i),
                (Err(e), _) | (_, Err(e)) => return failed(e),
            };
            let used = store.sources_of(&[id.to_owned()]).await.unwrap_or_default();
            ok(fit(
                json!({"as_of": as_of(status, &used), "found": true, "node": node, "edges": {"out": out, "in": inn}}),
            ))
        }
        "graph_related" => {
            let Some(id) = args.get("id").and_then(Value::as_str) else {
                return bad("id is required");
            };
            let direction = match args.get("direction").and_then(Value::as_str) {
                None | Some("both") => Direction::Both,
                Some("out") => Direction::Out,
                Some("in") => Direction::In,
                Some(_) => return bad("direction is out, in or both"),
            };
            let depth = number(
                args,
                "depth",
                1,
                usize::from(limits::GRAPH_RELATED_DEPTH_MAX),
            ) as u8;
            let limit = number(args, "limit", 50, limits::GRAPH_RELATED_LIMIT_MAX);
            match store
                .related(id, &strings(args, "edge_kinds"), direction, depth, limit)
                .await
            {
                Ok(r) => {
                    let mut ids: Vec<String> = r.nodes.iter().map(|n| n.id.clone()).collect();
                    ids.push(id.to_owned());
                    let used = store.sources_of(&ids).await.unwrap_or_default();
                    ok(fit(
                        json!({"as_of": as_of(status, &used), "nodes": r.nodes, "edges": r.edges, "truncated": r.truncated}),
                    ))
                }
                Err(e) => failed(e),
            }
        }
        "graph_context" => {
            let path = args.get("path").and_then(Value::as_str);
            let part = args.get("part").and_then(Value::as_str);
            let issue = args.get("about_issue").and_then(Value::as_str);
            let parsed_issue = issue.map(str::parse::<rubato_protocol::IssueRef>);
            let about = match (path, part, &parsed_issue) {
                (Some(path), None, None) => match repo {
                    Some(repo) => About::Path { repo, path },
                    None => {
                        return bad(
                            "repo is required with path: this session's repository is not known",
                        );
                    }
                },
                (None, Some(part), None) => About::Part(part),
                (None, None, Some(Ok(issue))) => About::Issue {
                    repo: &issue.repo,
                    number: issue.number,
                },
                (None, None, Some(Err(_))) => return bad("about_issue is owner/name#n"),
                _ => return bad("give exactly one of path, part or about_issue"),
            };
            match store.context(about, 25).await {
                Ok(c) => {
                    let used = store.sources_of(&c.node_ids()).await.unwrap_or_default();
                    let seams: Vec<Value> = c
                        .seams
                        .iter()
                        .map(|s| json!({"seam": s.seam, "joins": s.joins, "flows": s.flows}))
                        .collect();
                    ok(fit(json!({
                        "as_of": as_of(status, &used),
                        "subject": c.subject,
                        "parts": c.parts,
                        "seams": seams,
                        "adrs": c.adrs,
                        "slices": c.slices,
                        "scenarios": c.scenarios,
                        "issues": c.issues,
                        "plans": c.plans,
                        "pulls": c.pulls,
                        "symbols": c.symbols,
                        "imports": c.imports,
                        "imported_by": c.imported_by,
                        "truncated": c.truncated,
                    })))
                }
                Err(e) => failed(e),
            }
        }
        _ => bad("unknown graph tool"),
    }
}

impl AgentMcp {
    /// The copy the graph tools answer from. The composition root sets it once, when the graph
    /// is on for this machine.
    pub fn set_graph(&self, graph: Graph) {
        let _ = self.graph.set(graph);
    }

    pub(super) fn graph(&self) -> Option<&Graph> {
        self.graph.get()
    }
}
