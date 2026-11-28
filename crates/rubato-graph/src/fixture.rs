//! A small graph of three sources, for the tests of this crate and of the companion's tools:
//! the same picture on both sides. Not used outside tests.

use crate::{Build, Store};
use chrono::{DateTime, Utc};
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind, source_key};
use serde_json::{Map, Value, json};

pub const REPO: &str = "m0x6A/rubato";
pub const WORKSPACE: &str = "default";
pub const HEAD: &str = "b35788be4eed6ce6c4656291ac1409d79c1e42f8";
pub const GENERATED_AT: &str = "2026-11-26T17:00:04Z";

fn detail(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn node(
    id: String,
    kind: &str,
    name: &str,
    path: Option<&str>,
    summary: Option<&str>,
    d: &[(&str, Value)],
) -> GraphNode {
    GraphNode {
        id,
        kind: kind.to_owned(),
        name: name.to_owned(),
        repo: Some(REPO.to_owned()),
        path: path.map(str::to_owned),
        line: None,
        end_line: None,
        summary: summary.map(str::to_owned),
        detail: detail(d),
    }
}

fn edge(from: &str, kind: &str, to: &str, d: &[(&str, Value)]) -> GraphEdge {
    GraphEdge {
        from: from.to_owned(),
        kind: kind.to_owned(),
        to: to.to_owned(),
        detail: detail(d),
    }
}

fn build(
    source: String,
    source_kind: &str,
    head: Option<&str>,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
) -> Build {
    Build {
        source,
        kind: source_kind.to_owned(),
        head_sha: head.map(str::to_owned),
        map_version: (source_kind == kind::source::WORKSPACE).then(|| "b35788b".to_owned()),
        generator: 2,
        generated_at: GENERATED_AT.parse::<DateTime<Utc>>().unwrap(),
        truncated: false,
        nodes,
        edges,
    }
}

/// The three sources: one workspace's map and pins, the team's facts, one repository.
pub fn sources() -> Vec<Build> {
    let part = |name: &str| id::part(WORKSPACE, name);
    let seam = |name: &str| id::seam(WORKSPACE, name);
    let ws = |part_kind: &str| {
        vec![
            ("workspace", json!(WORKSPACE)),
            ("part_kind", json!(part_kind)),
        ]
    };
    let a_part = |name: &str, path: Option<&str>, says: &str, part_kind: &str| {
        let mut n = node(
            part(name),
            kind::node::PART,
            name,
            path,
            Some(says),
            &ws(part_kind),
        );
        if path.is_none() {
            n.repo = None;
        }
        n
    };
    let workspace = build(
        source_key::workspace("default", WORKSPACE),
        kind::source::WORKSPACE,
        None,
        vec![
            a_part(
                "rubato-server",
                Some("crates/rubato-server"),
                "One per team.",
                "context",
            ),
            a_part(
                "rubato-companion",
                Some("crates/rubato-companion"),
                "One per developer machine.",
                "context",
            ),
            a_part(
                "web",
                Some("web"),
                "React map view served by the server.",
                "context",
            ),
            a_part("claude-code", None, "The only agent in v0.", "external"),
            {
                let mut n = node(
                    seam("companion-mcp"),
                    kind::node::SEAM,
                    "companion-mcp",
                    None,
                    Some("A launched session asks its own companion."),
                    &[
                        ("workspace", json!(WORKSPACE)),
                        ("owner", json!("rubato-companion")),
                    ],
                );
                n.repo = None;
                n
            },
            {
                let mut n = node(
                    seam("web-server"),
                    kind::node::SEAM,
                    "web-server",
                    None,
                    Some("The browser reads the whole picture in one payload."),
                    &[
                        ("workspace", json!(WORKSPACE)),
                        ("owner", json!("rubato-server")),
                    ],
                );
                n.repo = None;
                n
            },
        ],
        vec![
            edge(
                &seam("companion-mcp"),
                kind::edge::JOINS,
                &part("rubato-companion"),
                &[("role", json!("from")), ("owner", json!(true))],
            ),
            edge(
                &seam("companion-mcp"),
                kind::edge::JOINS,
                &part("claude-code"),
                &[("role", json!("to"))],
            ),
            edge(
                &seam("web-server"),
                kind::edge::JOINS,
                &part("web"),
                &[("role", json!("from"))],
            ),
            edge(
                &seam("web-server"),
                kind::edge::JOINS,
                &part("rubato-server"),
                &[("role", json!("to")), ("owner", json!(true))],
            ),
            edge(
                &part("claude-code"),
                kind::edge::FLOW,
                &part("rubato-companion"),
                &[
                    ("seam", json!("companion-mcp")),
                    ("flow", json!("data")),
                    (
                        "carries",
                        json!("report_status: one line of what the agent is doing"),
                    ),
                ],
            ),
            edge(
                &part("rubato-server"),
                kind::edge::FLOW,
                &part("web"),
                &[
                    ("seam", json!("web-server")),
                    ("flow", json!("data")),
                    ("carries", json!("Overview")),
                ],
            ),
            edge(
                &part("rubato-server"),
                kind::edge::BACKED_BY,
                &id::repo(REPO),
                &[("path", json!("crates/rubato-server"))],
            ),
            edge(
                &id::issue(REPO, 322),
                kind::edge::PINNED_TO,
                &part("rubato-server"),
                &[],
            ),
            edge(
                &id::issue(REPO, 331),
                kind::edge::PINNED_TO,
                &seam("companion-mcp"),
                &[],
            ),
        ],
    );
    let team = build(
        source_key::team("default"),
        kind::source::TEAM,
        None,
        vec![
            node(
                id::repo(REPO),
                kind::node::REPO,
                REPO,
                None,
                None,
                &[("workspaces", json!([WORKSPACE]))],
            ),
            node(
                id::issue(REPO, 322),
                kind::node::ISSUE,
                "m0x6A/rubato#322",
                None,
                Some("Expose a repository context graph"),
                &[("number", json!(322)), ("github_state", json!("open"))],
            ),
            node(
                id::issue(REPO, 331),
                kind::node::ISSUE,
                "m0x6A/rubato#331",
                None,
                None,
                &[("number", json!(331)), ("untrusted", json!(true))],
            ),
            node(
                id::plan(REPO, 322),
                kind::node::PLAN,
                "Plan m0x6A/rubato#322",
                Some("docs/plans/322.md"),
                Some("A repository context graph"),
                &[("branch", json!("issue/322-context-graph"))],
            ),
            node(
                id::pull(REPO, 323),
                kind::node::PULL,
                "m0x6A/rubato#323",
                None,
                Some("Plan #322"),
                &[
                    ("number", json!(323)),
                    ("branch", json!("issue/322-context-graph")),
                ],
            ),
        ],
        vec![
            edge(
                &id::plan(REPO, 322),
                kind::edge::PLANS,
                &id::issue(REPO, 322),
                &[],
            ),
            edge(
                &id::plan(REPO, 322),
                kind::edge::AFFECTS,
                &id::map_ref("rubato-server"),
                &[("status", json!("affected"))],
            ),
            edge(
                &id::plan(REPO, 322),
                kind::edge::AFFECTS,
                &id::map_ref("web"),
                &[("status", json!("affected"))],
            ),
            edge(
                &id::plan(REPO, 322),
                kind::edge::TOUCHES,
                &id::map_ref("web-server"),
                &[],
            ),
            edge(
                &id::pull(REPO, 323),
                kind::edge::FOR,
                &id::issue(REPO, 322),
                &[],
            ),
        ],
    );
    let (store, pins_mod, tools, web_pins) = (
        "crates/rubato-server/src/pins/store.rs",
        "crates/rubato-server/src/pins/mod.rs",
        "crates/rubato-companion/src/agent_mcp/tools.rs",
        "web/src/features/pins/index.ts",
    );
    let file = |path: &str, language: &str| {
        node(
            id::file(REPO, path),
            kind::node::FILE,
            path.rsplit('/').next().unwrap(),
            Some(path),
            None,
            &[("language", json!(language))],
        )
    };
    let mut add_pin = node(
        id::symbol(REPO, store, "add_pin"),
        kind::node::SYMBOL,
        "add_pin",
        Some(store),
        Some("pub async fn add_pin(db: &SqlitePool, ws: &str, pin: &IssuePin) -> Result<()> {"),
        &[("symbol", json!("function"))],
    );
    add_pin.line = Some(15);
    add_pin.end_line = Some(20);
    let repo = build(
        source_key::repo(REPO),
        kind::source::REPO,
        Some(HEAD),
        vec![
            file(store, "rust"),
            file(pins_mod, "rust"),
            file(tools, "rust"),
            file(web_pins, "typescript"),
            add_pin,
            node(
                id::package(REPO, "cargo", "sqlx"),
                kind::node::PACKAGE,
                "sqlx",
                None,
                None,
                &[("ecosystem", json!("cargo"))],
            ),
            node(
                id::adr(REPO, "0071"),
                kind::node::ADR,
                "0071. Agents report on purpose through one MCP server in their own companion",
                Some("docs/decisions/0071-agent-mcp-in-the-companion.md"),
                Some("accepted; superseded in part by 0072"),
                &[
                    ("status", json!("accepted")),
                    ("seams", json!(["companion-mcp"])),
                    (
                        "decision",
                        json!("Each companion serves MCP to the sessions it launched."),
                    ),
                ],
            ),
            node(
                id::adr(REPO, "0016"),
                kind::node::ADR,
                "0016. Keep one shared map layout on the server",
                Some("docs/decisions/0016-shared-map-layout.md"),
                Some("accepted"),
                &[
                    ("status", json!("accepted")),
                    ("seams", json!(["web-server"])),
                ],
            ),
            node(
                id::scenario(REPO, "@pins.pin-to-a-node"),
                kind::node::SCENARIO,
                "@pins.pin-to-a-node",
                Some("features/pins.feature"),
                Some("A person pins an issue to a part"),
                &[("capability", json!("pins"))],
            ),
            node(
                id::scenario(REPO, "@repos.lists-map-repo"),
                kind::node::SCENARIO,
                "@repos.lists-map-repo",
                Some("features/repos.feature"),
                Some("The Repos tab lists the map's repository"),
                &[("capability", json!("repos"))],
            ),
            node(
                id::slice(REPO, "rubato-server", "pins"),
                kind::node::SLICE,
                "pins",
                Some("AGENTS.md"),
                Some("pin an issue to a node or seam, Map: lines"),
                &[
                    ("deployable", json!("rubato-server")),
                    ("owns", json!("pins, map_line_pins")),
                ],
            ),
            node(
                id::slice(REPO, "web", "pins"),
                kind::node::SLICE,
                "pins",
                Some("AGENTS.md"),
                Some("the \"Pin to…\" menu"),
                &[("deployable", json!("web"))],
            ),
        ],
        vec![
            edge(
                &id::file(REPO, store),
                kind::edge::DEFINES,
                &id::symbol(REPO, store, "add_pin"),
                &[],
            ),
            edge(
                &id::file(REPO, pins_mod),
                kind::edge::IMPORTS,
                &id::file(REPO, store),
                &[],
            ),
            edge(
                &id::file(REPO, store),
                kind::edge::IMPORTS,
                &id::package(REPO, "cargo", "sqlx"),
                &[],
            ),
            edge(
                &id::adr(REPO, "0071"),
                kind::edge::GOVERNS,
                &id::map_ref("companion-mcp"),
                &[],
            ),
            edge(
                &id::adr(REPO, "0016"),
                kind::edge::GOVERNS,
                &id::map_ref("web-server"),
                &[],
            ),
        ],
    );
    vec![workspace, team, repo]
}

/// The three sources applied to a new file at `path`, as a companion's copy would hold them.
pub async fn copy_at(path: &std::path::Path) -> Store {
    let store = Store::open_path(path).await.expect("a new graph file");
    for b in sources() {
        let at = b.generated_at;
        store
            .replace_source(None, b, at)
            .await
            .expect("the fixture applies");
    }
    store
}
