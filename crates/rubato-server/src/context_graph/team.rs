//! The team's source (ADR 0084): the facts the server keeps per repository and shares across
//! workspaces (ADR 0047): repositories, pinned issues, plans and open pull requests. Pure: what
//! the slice read through the other slices' roots in, a graph out.
//!
//! Text here was written outside the server, and a launched agent reads it without a person in
//! between. So only pinned issues are nodes, and an issue's title is kept only when its author
//! is on the team (ADR 0026's rule); a plan and a pull request are on a branch of the
//! repository itself, which takes write access (fork pull requests are never listed, ADR 0034).

use super::extract::object;
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind};
use rubato_protocol::pulls::OpenPull;
use rubato_protocol::{IssueRef, Plan};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One repository the maps name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepoFact {
    pub name: String,
    /// The workspaces whose map names it, in order.
    pub workspaces: Vec<String>,
}

/// What GitHub last said about an issue the server tracks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct IssueFact {
    pub title: Option<String>,
    pub state: Option<String>,
    pub url: Option<String>,
}

/// One plan file as the server read it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlanFact {
    pub issue: IssueRef,
    pub branch: String,
    pub path: String,
    pub plan: Plan,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Input {
    pub repos: Vec<RepoFact>,
    /// Every issue pinned in any workspace.
    pub pinned: BTreeSet<(String, u64)>,
    pub issues: HashMap<IssueRef, IssueFact>,
    /// The author's login and GitHub's association, for issues in the last open listing.
    pub authors: HashMap<IssueRef, (String, String)>,
    /// The App's own login: an issue it created was written in Rubato by a team member.
    pub app_login: Option<String>,
    pub plans: Vec<PlanFact>,
    pub pulls: Vec<OpenPull>,
}

fn edge(from: String, kind: &str, to: String, detail: serde_json::Map<String, Value>) -> GraphEdge {
    GraphEdge {
        from,
        kind: kind.to_owned(),
        to,
        detail,
    }
}

/// Whether this issue's title may reach an agent: its author is known and on the team.
fn title_trusted(input: &Input, issue: &IssueRef) -> bool {
    let Some((author, association)) = input.authors.get(issue) else {
        // Closed, or never listed: nobody has said who wrote it.
        return false;
    };
    crate::github_poll::trusted(association)
        || input
            .app_login
            .as_deref()
            .is_some_and(|app| !app.is_empty() && app == author)
}

pub(super) fn build(input: &Input) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for r in &input.repos {
        nodes.push(GraphNode {
            id: id::repo(&r.name),
            kind: kind::node::REPO.into(),
            name: r.name.clone(),
            repo: Some(r.name.clone()),
            path: None,
            line: None,
            end_line: None,
            summary: None,
            detail: object([("workspaces", json!(r.workspaces))]),
        });
    }
    for (repo, number) in &input.pinned {
        let issue = IssueRef::new(repo.clone(), *number);
        let fact = input.issues.get(&issue).cloned().unwrap_or_default();
        let trusted = title_trusted(input, &issue);
        nodes.push(GraphNode {
            id: id::issue(repo, *number),
            kind: kind::node::ISSUE.into(),
            name: format!("{repo}#{number}"),
            repo: Some(repo.clone()),
            path: None,
            line: None,
            end_line: None,
            summary: fact.title.filter(|_| trusted),
            detail: object([
                ("number", json!(number)),
                ("github_state", fact.state.map_or(Value::Null, |s| json!(s))),
                ("url", fact.url.map_or(Value::Null, |u| json!(u))),
                // Said, so a reader knows a title exists and why it is not here.
                ("untrusted", if trusted { Value::Null } else { json!(true) }),
            ]),
        });
    }
    for p in &input.plans {
        let (repo, number) = (&p.issue.repo, p.issue.number);
        let me = id::plan(repo, number);
        nodes.push(GraphNode {
            id: me.clone(),
            kind: kind::node::PLAN.into(),
            name: format!("Plan {repo}#{number}"),
            repo: Some(repo.clone()),
            path: Some(p.path.clone()),
            line: None,
            end_line: None,
            summary: p.plan.title.clone(),
            detail: object([
                ("branch", json!(p.branch)),
                (
                    "approved",
                    if p.plan.approval.is_some() {
                        json!(true)
                    } else {
                        Value::Null
                    },
                ),
            ]),
        });
        edges.push(edge(
            me.clone(),
            kind::edge::PLANS,
            id::issue(repo, number),
            Default::default(),
        ));
        let mut seams: BTreeMap<&str, ()> = BTreeMap::new();
        for part in &p.plan.affected_parts {
            edges.push(edge(
                me.clone(),
                kind::edge::AFFECTS,
                id::map_ref(&part.node),
                object([
                    (
                        "status",
                        serde_json::to_value(part.status).unwrap_or(Value::Null),
                    ),
                    ("change", json!(part.change)),
                ]),
            ));
            for s in &part.seams {
                seams.insert(s.as_str(), ());
            }
        }
        for seam in seams.keys() {
            edges.push(edge(
                me.clone(),
                kind::edge::TOUCHES,
                id::map_ref(seam),
                Default::default(),
            ));
        }
    }
    for pull in &input.pulls {
        let me = id::pull(&pull.repo, pull.number);
        nodes.push(GraphNode {
            id: me.clone(),
            kind: kind::node::PULL.into(),
            name: format!("{}#{}", pull.repo, pull.number),
            repo: Some(pull.repo.clone()),
            path: None,
            line: None,
            end_line: None,
            summary: Some(pull.title.clone()),
            detail: object([
                ("number", json!(pull.number)),
                ("branch", json!(pull.branch)),
                ("base", json!(pull.base)),
                ("author", json!(pull.author)),
                ("url", json!(pull.url)),
                ("draft", if pull.draft { json!(true) } else { Value::Null }),
            ]),
        });
        if let Some(number) = IssueRef::number_from_branch(&pull.branch) {
            edges.push(edge(
                me,
                kind::edge::FOR,
                id::issue(&pull.repo, number),
                Default::default(),
            ));
        }
    }
    (nodes, edges)
}
