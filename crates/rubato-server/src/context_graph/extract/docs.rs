//! The documents with a regular structure: decision records (`0000-template.md`), feature
//! scenarios (ADR 0069) and AGENTS.md slice blocks. The ADR rules are the skill's
//! (`skills/rubato/scripts/adrs.ts`), so the graph and `adrs.ts` read a record the same way.

use super::object;
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind};
use rubato_protocol::limits;
use serde_json::{Value, json};

/// `…/decisions/NNNN-<slug>.md`, the template left out.
pub(super) fn is_adr_path(path: &str) -> bool {
    let Some((dir, name)) = path.rsplit_once('/') else {
        return false;
    };
    (dir == "decisions" || dir.ends_with("/decisions"))
        && name.ends_with(".md")
        && name.len() > 8
        && name.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && name.as_bytes()[4] == b'-'
        && !name.starts_with("0000-")
}

/// `features/…/*.feature`, at the root or under a crate.
pub(super) fn is_feature_path(path: &str) -> bool {
    path.ends_with(".feature") && (path.starts_with("features/") || path.contains("/features/"))
}

/// `- **Name:** value`, the first one.
fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("- **{name}:**");
    text.lines()
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// The text under `## <heading>` up to the next `## `.
fn section<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
    let mut start = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        match start {
            None if line.trim_end() == format!("## {heading}") => start = Some(at + line.len()),
            Some(s) if line.starts_with("## ") => return Some(text[s..at].trim()),
            _ => {}
        }
        at += line.len();
    }
    start.map(|s| text[s..].trim())
}

fn capped(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Every run of four digits in `text`: the ADR numbers a status line names.
fn numbers(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i - start == 4 {
                out.push(&text[start..i]);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn is_map_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// The map ids a `Seam:` field names. "none (dependency)" and prose name none.
fn seam_ids(field: &str) -> Vec<&str> {
    let head = field.split(['(', ';']).next().unwrap_or("");
    head.split(',')
        .map(str::trim)
        .filter(|s| is_map_id(s) && *s != "none")
        .collect()
}

/// One decision record: its node, and the edges its header states. `None` when the file has no
/// `# NNNN. Title` line.
pub(super) fn adr(repo: &str, path: &str, text: &str) -> Option<(GraphNode, Vec<GraphEdge>)> {
    let (line_no, number, title) = text.lines().enumerate().find_map(|(i, l)| {
        let rest = l.strip_prefix("# ")?;
        let (number, title) = rest.split_once('.')?;
        (number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit()))
            .then(|| (i as u32 + 1, number, title.trim()))
    })?;
    let status_line = field(text, "Status").unwrap_or("");
    let status = status_line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches([';', ',', '.']);
    let seams = seam_ids(field(text, "Seam").unwrap_or(""));
    let me = id::adr(repo, number);
    let mut edges: Vec<GraphEdge> = seams
        .iter()
        .map(|s| GraphEdge {
            from: me.clone(),
            kind: kind::edge::GOVERNS.into(),
            to: id::map_ref(s),
            detail: Default::default(),
        })
        .collect();
    // "superseded by 0054", or "accepted; superseded in part by 0071 and 0072": the newer
    // record supersedes this one.
    if let Some(at) = status_line.find("superseded") {
        let tail = &status_line[at..];
        let in_part = tail.starts_with("superseded in part");
        for newer in numbers(tail) {
            edges.push(GraphEdge {
                from: id::adr(repo, newer),
                kind: kind::edge::SUPERSEDES.into(),
                to: me.clone(),
                detail: object([("in_part", if in_part { json!(true) } else { Value::Null })]),
            });
        }
    }
    let decision = section(text, "Decision").map(|d| capped(d, limits::GRAPH_DECISION_MAX_BYTES));
    let node = GraphNode {
        id: me,
        kind: kind::node::ADR.into(),
        name: format!("{number}. {title}"),
        repo: Some(repo.to_owned()),
        path: Some(path.to_owned()),
        line: Some(line_no),
        end_line: None,
        summary: Some(status_line.to_owned()).filter(|s| !s.is_empty()),
        detail: object([
            ("number", json!(number)),
            ("status", json!(status)),
            (
                "date",
                field(text, "Date").map_or(Value::Null, |d| json!(d)),
            ),
            (
                "seams",
                if seams.is_empty() {
                    Value::Null
                } else {
                    json!(seams)
                },
            ),
            ("decision", decision.map_or(Value::Null, |d| json!(d))),
        ]),
    };
    Some((node, edges))
}

/// Every scenario with a `@<capability>.<slug>` tag (ADR 0069).
pub(super) fn scenarios(repo: &str, path: &str, text: &str) -> Vec<GraphNode> {
    let mut out = Vec::new();
    let mut feature = None;
    let mut tags: Vec<&str> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("Feature:") {
            feature = Some(name.trim());
            tags.clear();
        } else if line.starts_with('@') {
            tags.extend(line.split_whitespace().filter(|t| t.starts_with('@')));
        } else if let Some(name) = line
            .strip_prefix("Scenario Outline:")
            .or_else(|| line.strip_prefix("Scenario:"))
        {
            for tag in tags.drain(..) {
                // `@selftest` and `@tooling` mark a test, not a scenario id.
                let Some((capability, slug)) = tag[1..].split_once('.') else {
                    continue;
                };
                if capability.is_empty() || slug.is_empty() {
                    continue;
                }
                out.push(GraphNode {
                    id: id::scenario(repo, tag),
                    kind: kind::node::SCENARIO.into(),
                    name: tag.to_owned(),
                    repo: Some(repo.to_owned()),
                    path: Some(path.to_owned()),
                    line: Some(i as u32 + 1),
                    end_line: None,
                    summary: Some(name.trim().to_owned()),
                    detail: object([
                        ("capability", json!(capability)),
                        ("feature", feature.map_or(Value::Null, |f| json!(f))),
                    ]),
                });
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            tags.clear();
        }
    }
    out
}

/// The slices AGENTS.md declares in its `<!-- slices: <deployable> -->` blocks: one line each,
/// the name, what it does and what it owns, columns apart by two spaces or more.
pub(super) fn slices(repo: &str, text: &str) -> Vec<GraphNode> {
    let mut out = Vec::new();
    let mut deployable: Option<&str> = None;
    let mut fenced = false;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("<!-- slices:") {
            deployable = rest.strip_suffix("-->").map(str::trim);
            fenced = false;
        } else if trimmed == "<!-- /slices -->" {
            deployable = None;
            fenced = false;
        } else if deployable.is_some() && trimmed.starts_with("```") {
            fenced = !fenced;
        } else if let (Some(deployable), true) = (deployable, fenced) {
            let Some(name) = trimmed.split_whitespace().next() else {
                continue;
            };
            let rest = trimmed[name.len()..].trim();
            let columns: Vec<&str> = rest
                .split("  ")
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            let does = columns.first().copied();
            let owns = (columns.len() > 1).then(|| columns[columns.len() - 1]);
            out.push(GraphNode {
                id: id::slice(repo, deployable, name),
                kind: kind::node::SLICE.into(),
                name: name.to_owned(),
                repo: Some(repo.to_owned()),
                path: Some("AGENTS.md".to_owned()),
                line: Some(i as u32 + 1),
                end_line: None,
                summary: does.map(str::to_owned),
                detail: object([
                    ("deployable", json!(deployable)),
                    ("owns", owns.map_or(Value::Null, |o| json!(o))),
                ]),
            });
        }
    }
    out
}
