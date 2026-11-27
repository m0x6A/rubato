use super::{Extracted, Extractor};
use rubato_protocol::graph::{id, kind};

const REPO: &str = "m0x6A/rubato";

pub(crate) const ADR_ACCEPTED: &str =
    "# 0071. Agents report on purpose through one MCP server in their own companion

- **Status:** accepted; superseded in part by 0072 and 0086
- **Date:** 2026-11-15
- **Decided by:** Mattias Jonsson
- **Seam:** companion-mcp, companion-claude-code
- **Issue:** m0x6A/rubato#197

## Context

An agent tells Rubato about itself only through side channels.

## Decision

Each companion serves MCP to the sessions it launched.

Three tools, and no others without a new ADR.

## Alternatives considered

- **MCP on the server**: needs a token per session.
";

const ADR_SUPERSEDED: &str = "# 0027. Review and approval are comments

- **Status:** superseded by 0054
- **Seam:** none (a way of working)

## Decision

Comments count.
";

pub(crate) const FEATURE: &str = "Feature: Connected repositories
  The repositories behind the map's parts.

  @repos.lists-map-repo
  Scenario: The Repos tab lists the map's repository as connected
    When a person opens Repos

  @selftest
  Scenario: must fail: a scenario nobody covers
    When nothing happens

  # a comment between a tag and its scenario is fine
  @repos.what-each-provides @tooling
  Scenario Outline: The Repos tab says which parts each repository provides
    When a person opens <tab>
";

pub(crate) const AGENTS: &str = "# AGENTS.md

<!-- slices: rubato-server -->
```text
auth            GitHub login, team record, CurrentUser, tokens  users, web_sessions, user_tokens
web_link        websocket fan-out to browsers                   (none)
```
<!-- /slices -->

```text
not_a_slice     a fenced block outside any slices comment
```

<!-- slices: web -->
```text
live        the overview and every event, over /ws/web           GET /api/overview
```
<!-- /slices -->
";

/// The fixture repository: what each test extracts from.
pub(crate) fn fixture_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("AGENTS.md", AGENTS.into()),
        ("README.md", b"# rubato\n\nA lab.\n".to_vec()),
        (
            "crates/rubato-server/src/lib.rs",
            b"pub mod app;\npub mod kernel;\n".to_vec(),
        ),
        (
            "docs/decisions/0000-template.md",
            b"# NNNN. <Decision>\n".to_vec(),
        ),
        (
            "docs/decisions/0027-review-and-approval-are-comments.md",
            ADR_SUPERSEDED.into(),
        ),
        (
            "docs/decisions/0071-agent-mcp-in-the-companion.md",
            ADR_ACCEPTED.into(),
        ),
        (
            "docs/decisions/0099-no-title.md",
            b"Just prose, no title line.\n".to_vec(),
        ),
        ("features/repos.feature", FEATURE.into()),
        (
            "web/public/logo.png",
            vec![0x89, b'P', b'N', b'G', 0xff, 0xfe],
        ),
    ]
}

fn extract(files: &[(&str, Vec<u8>)]) -> Extracted {
    let mut x = Extractor::new(REPO);
    for (path, bytes) in files {
        x.file(path, bytes.len() as u64, Some(bytes));
    }
    x.finish()
}

/// The ids of `want` the extraction does not hold.
fn missing<'a>(got: &Extracted, want: &'a [String]) -> Vec<&'a String> {
    want.iter()
        .filter(|id| !got.nodes.iter().any(|n| &n.id == *id))
        .collect()
}

#[test]
fn a_repository_gives_its_files_decisions_scenarios_and_slices() {
    let got = extract(&fixture_files());
    println!(
        "found {} files, {} ADRs, {} scenarios, {} slices, {} unread documents",
        got.counts.files,
        got.counts.adrs,
        got.counts.scenarios,
        got.counts.slices,
        got.counts.unread_documents
    );
    assert_eq!(
        (
            got.counts.files,
            got.counts.adrs,
            got.counts.scenarios,
            got.counts.slices
        ),
        (9, 2, 2, 3)
    );
    assert_eq!(got.counts.unread_documents, 1, "0099 has no title line");
    assert_eq!(got.truncated, None);
    let want = [
        id::file(REPO, "crates/rubato-server/src/lib.rs"),
        id::adr(REPO, "0071"),
        id::adr(REPO, "0027"),
        id::scenario(REPO, "@repos.lists-map-repo"),
        id::scenario(REPO, "@repos.what-each-provides"),
        id::slice(REPO, "rubato-server", "auth"),
        id::slice(REPO, "web", "live"),
    ];
    assert_eq!(missing(&got, &want), Vec::<&String>::new());

    // The fixture that makes this check fail on purpose: an ADR the repository does not hold,
    // the template, the tag that is not a scenario id, and the block outside a slices comment.
    let absent = [
        id::adr(REPO, "0084"),
        id::adr(REPO, "0000"),
        id::scenario(REPO, "@selftest"),
        id::slice(REPO, "rubato-server", "not_a_slice"),
    ];
    assert_eq!(missing(&got, &absent).len(), absent.len());
}

#[test]
fn a_file_is_a_node_with_what_can_be_said_without_its_contents() {
    let got = extract(&fixture_files());
    let node = |path: &str| {
        got.nodes
            .iter()
            .find(|n| n.id == id::file(REPO, path))
            .unwrap_or_else(|| panic!("{path}"))
    };
    let lib = node("crates/rubato-server/src/lib.rs");
    assert_eq!(lib.kind, kind::node::FILE);
    assert_eq!(lib.name, "lib.rs");
    assert_eq!(lib.path.as_deref(), Some("crates/rubato-server/src/lib.rs"));
    assert_eq!(
        serde_json::Value::Object(lib.detail.clone()),
        serde_json::json!({ "language": "rust", "bytes": 29, "lines": 2 })
    );
    // Not text: a node, and nothing read from it.
    let png = node("web/public/logo.png");
    assert_eq!(
        serde_json::Value::Object(png.detail.clone()),
        serde_json::json!({ "bytes": 6 })
    );
    // Too large to read: still a node.
    let mut x = Extractor::new(REPO);
    x.file("big.sql", 9_000_000, None);
    let big = x.finish();
    assert_eq!(big.nodes[0].detail["language"], "sql");
    assert!(big.nodes[0].detail.get("lines").is_none());
    // No node carries a file's contents.
    let all = serde_json::to_string(&got.nodes).unwrap();
    assert!(
        !all.contains("pub mod kernel"),
        "source text reached the graph"
    );
    assert!(!all.contains("A lab."), "a README's text reached the graph");
}

#[test]
fn a_decision_record_says_its_status_what_it_governs_and_what_superseded_it() {
    let got = extract(&fixture_files());
    let adr = got
        .nodes
        .iter()
        .find(|n| n.id == id::adr(REPO, "0071"))
        .unwrap();
    assert_eq!(
        adr.name,
        "0071. Agents report on purpose through one MCP server in their own companion"
    );
    assert_eq!(adr.line, Some(1));
    assert_eq!(
        adr.summary.as_deref(),
        Some("accepted; superseded in part by 0072 and 0086")
    );
    assert_eq!(adr.detail["status"], "accepted");
    assert_eq!(adr.detail["date"], "2026-11-15");
    assert_eq!(
        adr.detail["seams"],
        serde_json::json!(["companion-mcp", "companion-claude-code"])
    );
    assert_eq!(
        adr.detail["decision"],
        "Each companion serves MCP to the sessions it launched.\n\nThree tools, and no others without a new ADR."
    );
    let edges: Vec<(String, String, String, bool)> = got
        .edges
        .iter()
        .map(|e| {
            (
                e.from.clone(),
                e.kind.clone(),
                e.to.clone(),
                e.detail.contains_key("in_part"),
            )
        })
        .collect();
    let a = |n: &str| id::adr(REPO, n);
    let want = [
        (a("0054"), "supersedes", a("0027"), false),
        (a("0071"), "governs", id::map_ref("companion-mcp"), false),
        (
            a("0071"),
            "governs",
            id::map_ref("companion-claude-code"),
            false,
        ),
        (a("0072"), "supersedes", a("0071"), true),
        (a("0086"), "supersedes", a("0071"), true),
    ];
    for (from, kind, to, in_part) in &want {
        assert!(
            edges.contains(&(from.clone(), (*kind).to_owned(), to.clone(), *in_part)),
            "no edge {from} {kind} {to} (in part: {in_part}) in {edges:#?}"
        );
    }
    assert_eq!(
        edges.len(),
        want.len(),
        "\"none (a way of working)\" governs nothing"
    );

    let old = got
        .nodes
        .iter()
        .find(|n| n.id == id::adr(REPO, "0027"))
        .unwrap();
    assert_eq!(old.detail["status"], "superseded");
    assert!(old.detail.get("seams").is_none());
}

#[test]
fn a_long_decision_is_cut_at_its_cap_on_a_character_boundary() {
    let long = "å".repeat(5_000);
    let text = format!("# 0100. Long\n\n- **Status:** proposed\n\n## Decision\n\n{long}\n");
    let got = extract(&[("docs/decisions/0100-long.md", text.into_bytes())]);
    let decision = got.nodes[1].detail["decision"].as_str().unwrap();
    assert_eq!(
        decision.len(),
        4096,
        "two bytes a character: the cap lands on a boundary"
    );
    assert!(decision.chars().all(|c| c == 'å'));
}

#[test]
fn a_scenario_and_a_slice_say_where_they_are() {
    let got = extract(&fixture_files());
    let s = got
        .nodes
        .iter()
        .find(|n| n.id == id::scenario(REPO, "@repos.what-each-provides"))
        .unwrap();
    assert_eq!(
        s.summary.as_deref(),
        Some("The Repos tab says which parts each repository provides")
    );
    assert_eq!(
        (s.path.as_deref(), s.line),
        (Some("features/repos.feature"), Some(14))
    );
    assert_eq!(s.detail["capability"], "repos");
    assert_eq!(s.detail["feature"], "Connected repositories");

    let auth = got
        .nodes
        .iter()
        .find(|n| n.id == id::slice(REPO, "rubato-server", "auth"))
        .unwrap();
    assert_eq!(
        auth.summary.as_deref(),
        Some("GitHub login, team record, CurrentUser, tokens")
    );
    assert_eq!(auth.detail["owns"], "users, web_sessions, user_tokens");
    assert_eq!(auth.detail["deployable"], "rubato-server");
    assert_eq!(auth.line, Some(5));
}

#[test]
fn the_real_repository_reads_as_the_fixtures_do() {
    // The repository's own documents, read from disk: the rules hold for the real thing.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["docs/decisions", "features"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                files.push((format!("{dir}/{name}"), std::fs::read(&path).unwrap()));
            }
        }
    }
    files.push((
        "AGENTS.md".to_owned(),
        std::fs::read(root.join("AGENTS.md")).unwrap(),
    ));
    let mut x = Extractor::new(REPO);
    for (path, bytes) in &files {
        x.file(path, bytes.len() as u64, Some(bytes));
    }
    let got = x.finish();
    println!(
        "the repository itself: {} ADRs, {} scenarios, {} slices, {} unread",
        got.counts.adrs, got.counts.scenarios, got.counts.slices, got.counts.unread_documents
    );
    assert!(got.counts.adrs >= 80, "{} ADRs", got.counts.adrs);
    assert!(
        got.counts.scenarios >= 100,
        "{} scenarios",
        got.counts.scenarios
    );
    assert!(got.counts.slices >= 60, "{} slices", got.counts.slices);
    assert_eq!(got.counts.unread_documents, 0);
    let me = got
        .nodes
        .iter()
        .find(|n| n.id == id::slice(REPO, "rubato-server", "context_graph"))
        .expect("this slice is in AGENTS.md");
    assert!(me.summary.as_deref().unwrap().contains("context graph"));
    assert!(
        got.edges.iter().any(|e| e.from == id::adr(REPO, "0086")
            && e.kind == kind::edge::GOVERNS
            && e.to == id::map_ref("companion-mcp")),
        "ADR 0086 governs companion-mcp"
    );
}
