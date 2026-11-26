//! A workspace's source (ADR 0084): its map as nodes and edges, and which issue is pinned where.
//! Pure: the loaded map and the workspace's pins in, a graph out.

use super::extract::object;
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind};
use rubato_protocol::{IssuePin, MapRef, SystemMap};
use serde_json::{Value, json};

/// What `serde` calls an enum value: the word the map file uses.
fn word<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn text(v: &Option<String>) -> Value {
    v.as_ref().map_or(Value::Null, |s| json!(s))
}

fn edge(from: String, kind: &str, to: String, detail: serde_json::Map<String, Value>) -> GraphEdge {
    GraphEdge {
        from,
        kind: kind.to_owned(),
        to,
        detail,
    }
}

/// The parts, seams and flows of one workspace's map, and its pins.
pub(super) fn build(
    ws: &str,
    map: &SystemMap,
    pins: &[IssuePin],
) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for n in &map.nodes {
        let me = id::part(ws, &n.id);
        nodes.push(GraphNode {
            id: me.clone(),
            kind: kind::node::PART.into(),
            name: n.id.clone(),
            repo: n.repo.clone(),
            path: n.path.clone(),
            line: None,
            end_line: None,
            summary: n.description.clone(),
            detail: object([
                ("workspace", json!(ws)),
                ("part_kind", word(&n.kind)),
                ("service", n.service.as_ref().map_or(Value::Null, word)),
                ("retired", text(&n.retired)),
            ]),
        });
        if let Some(repo) = &n.repo {
            edges.push(edge(
                me,
                kind::edge::BACKED_BY,
                id::repo(repo),
                object([("path", text(&n.path))]),
            ));
        }
    }
    for s in &map.seams {
        let me = id::seam(ws, &s.id);
        nodes.push(GraphNode {
            id: me.clone(),
            kind: kind::node::SEAM.into(),
            name: s.id.clone(),
            repo: None,
            path: None,
            line: None,
            end_line: None,
            summary: s.description.clone().or_else(|| s.carries.clone()),
            detail: object([
                ("workspace", json!(ws)),
                ("from", json!(s.from)),
                ("to", json!(s.to)),
                ("owner", json!(s.owner)),
                ("carries", text(&s.carries)),
                ("guard", text(&s.guard)),
                ("retired", text(&s.retired)),
            ]),
        });
        for (role, part) in [("from", &s.from), ("to", &s.to)] {
            edges.push(edge(
                me.clone(),
                kind::edge::JOINS,
                id::part(ws, part),
                object([
                    ("role", json!(role)),
                    (
                        "owner",
                        if part == &s.owner {
                            json!(true)
                        } else {
                            Value::Null
                        },
                    ),
                ]),
            ));
        }
        for f in &s.flows {
            edges.push(edge(
                id::part(ws, &f.from),
                kind::edge::FLOW,
                id::part(ws, &f.to),
                object([
                    ("seam", json!(s.id)),
                    ("flow", word(&f.kind)),
                    ("carries", json!(f.carries)),
                ]),
            ));
        }
    }
    for p in pins {
        let target = match &p.target {
            MapRef::Node { id: part } => id::part(ws, part),
            MapRef::Seam { id: seam } => id::seam(ws, seam),
        };
        edges.push(edge(
            id::issue(&p.issue.repo, p.issue.number),
            kind::edge::PINNED_TO,
            target,
            object([("pinned_by", json!(p.pinned_by))]),
        ));
    }
    (nodes, edges)
}
