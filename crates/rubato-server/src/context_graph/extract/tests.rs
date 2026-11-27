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

const LIB_RS: &str =
    "pub mod app;\n\npub fn start() {\n    let key = \"only-in-a-function-body\";\n}\n";

/// The fixture repository: what each test extracts from.
pub(crate) fn fixture_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("AGENTS.md", AGENTS.into()),
        ("README.md", b"# rubato\n\nA lab.\n".to_vec()),
        ("crates/rubato-server/src/lib.rs", LIB_RS.into()),
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
        serde_json::json!({ "language": "rust", "bytes": LIB_RS.len(), "lines": 5 })
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
    // No node carries a file's contents: of code, one signature line and no body.
    let all = serde_json::to_string(&got.nodes).unwrap();
    assert!(
        all.contains("pub fn start() {"),
        "the signature line is what a symbol says"
    );
    assert!(
        !all.contains("only-in-a-function-body"),
        "a function body reached the graph"
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
        .filter(|e| e.kind == kind::edge::GOVERNS || e.kind == kind::edge::SUPERSEDES)
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

/// The fixture repository in four languages, read from `tests/fixtures/graph-code`.
fn code_fixture() -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph-code");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

fn extract_code() -> Extracted {
    let mut files = code_fixture();
    // Tracked, and not the repository's own code: nodes, never parsed.
    for path in [
        "web/node_modules/y/index.js",
        "web/public/app.min.js",
        "web/src/env.d.ts",
    ] {
        files.push((path.to_owned(), b"function shipped() {}\n".to_vec()));
    }
    let long = format!("pub fn long({}) {{}}\n", "a: u8, ".repeat(60));
    files.push(("crates/app/src/long.rs".to_owned(), long.into_bytes()));
    files.push((
        "crates/app/src/latin1.rs".to_owned(),
        vec![b'f', b'n', b' ', 0xe5, b'(', b')'],
    ));
    let mut x = Extractor::new(REPO);
    for (path, bytes) in &files {
        x.file(path, bytes.len() as u64, Some(bytes));
    }
    x.finish()
}

#[test]
fn every_grammars_queries_compile_against_its_grammar() {
    let got = extract(&[]);
    assert_eq!(got.counts.code.broken_grammars, Vec::<String>::new());
    assert_eq!(super::code::Lang::ALL.len(), 4);
}

#[test]
fn source_files_give_their_symbols_one_fixture_per_language() {
    let got = extract_code();
    let symbols: Vec<(String, String, u32, String)> = got
        .nodes
        .iter()
        .filter(|n| n.kind == kind::node::SYMBOL)
        .map(|n| {
            (
                n.id.trim_start_matches("sym:m0x6A/rubato:").to_owned(),
                n.detail["symbol"].as_str().unwrap().to_owned(),
                n.line.unwrap(),
                n.summary.clone().unwrap(),
            )
        })
        .collect();
    // (id after the repository, what the grammar calls it, line, signature line)
    let want = [
        // Rust
        ("crates/app/src/lib.rs#store", "module", 2, "pub mod store;"),
        (
            "crates/app/src/lib.rs#open",
            "function",
            11,
            "pub fn open(url: &str) -> Store {",
        ),
        (
            "crates/app/src/lib.rs#Config",
            "class",
            18,
            "pub struct Config {",
        ),
        (
            "crates/app/src/lib.rs#Named",
            "interface",
            22,
            "pub trait Named {",
        ),
        (
            "crates/app/src/lib.rs#name",
            "method",
            27,
            "fn name(&self) -> String {",
        ),
        (
            "crates/app/src/lib.rs#twice",
            "macro",
            32,
            "macro_rules! twice {",
        ),
        (
            "crates/app/src/store.rs#connect",
            "method",
            4,
            "pub fn connect(_url: &str) -> Store {",
        ),
        // Two `new`s in one file are told apart by their line.
        (
            "crates/app/src/store.rs#new@L8",
            "method",
            8,
            "pub fn new() -> Store {",
        ),
        (
            "crates/app/src/store.rs#new@L16",
            "method",
            16,
            "pub fn new() -> Cache {",
        ),
        // TypeScript
        (
            "web/src/api.ts#request",
            "function",
            8,
            "async function request(path: string): Promise<Me> {",
        ),
        (
            "web/src/api.ts#ApiError",
            "interface",
            12,
            "interface ApiError {",
        ),
        ("web/src/api.ts#Client", "class", 16, "class Client {"),
        ("web/src/api.ts#get", "method", 17, "get(path: string) {"),
        ("web/protocol.ts#Me", "interface", 1, "interface Me {"),
        // TSX
        ("web/src/App.tsx#App", "function", 3, "function App() {"),
        // JavaScript
        (
            "web/scripts/build.js#build",
            "function",
            3,
            "function build() {",
        ),
        ("web/scripts/build.js#Runner", "class", 7, "class Runner {"),
        ("web/scripts/build.js#run", "method", 8, "run() {"),
    ];
    for (id, symbol, line, signature) in want {
        assert!(
            symbols.contains(&(id.to_owned(), symbol.to_owned(), line, signature.to_owned())),
            "no symbol {id} ({symbol}, line {line}, {signature:?}) in {symbols:#?}"
        );
    }
    println!(
        "checked {} of {} symbols; {:?}",
        want.len(),
        symbols.len(),
        got.counts.code
    );
    assert_eq!(got.counts.code.symbols, symbols.len());
    // The fixture that makes this check fail on purpose: what is not the repository's own code
    // gives no symbol, and neither does a name nobody defined.
    for absent in ["#shipped", "#vendored", "#nobody_defined_this"] {
        assert!(
            symbols.iter().all(|(id, ..)| !id.ends_with(absent)),
            "{absent} is a symbol"
        );
    }
    for path in [
        "web/vendor/x/index.js",
        "web/node_modules/y/index.js",
        "web/src/env.d.ts",
    ] {
        assert!(
            got.nodes.iter().any(|n| n.id == id::file(REPO, path)),
            "{path} is a file"
        );
    }

    let doc = |suffix: &str| {
        got.nodes
            .iter()
            .find(|n| n.id.ends_with(suffix))
            .and_then(|n| n.detail.get("doc"))
            .and_then(|d| d.as_str())
            .map(str::to_owned)
    };
    assert_eq!(
        doc("lib.rs#open").as_deref(),
        Some("Opens the store at `url`.")
    );
    assert_eq!(doc("api.ts#request").as_deref(), Some("Requests a path."));
    assert_eq!(
        doc("lib.rs#Config"),
        None,
        "an attribute is not a doc comment"
    );
    // A signature is one line, and capped.
    let long = got
        .nodes
        .iter()
        .find(|n| n.id.ends_with("long.rs#long"))
        .unwrap();
    assert_eq!(long.summary.as_ref().unwrap().chars().count(), 200);
}

#[test]
fn imports_resolve_to_a_file_of_the_repository_or_to_a_package() {
    let got = extract_code();
    let short = |s: &str| s.replace("m0x6A/rubato:", "");
    let mut imports: Vec<(String, String)> = got
        .edges
        .iter()
        .filter(|e| e.kind == kind::edge::IMPORTS)
        .map(|e| (short(&e.from), short(&e.to)))
        .collect();
    imports.sort();
    let want = [
        // `mod store;` and `use crate::store::Store;`: one edge.
        ("file:crates/app/src/lib.rs", "file:crates/app/src/store.rs"),
        ("file:crates/app/src/lib.rs", "file:crates/app/src/util.rs"),
        // Another crate of the repository, by its directory name.
        (
            "file:crates/app/src/lib.rs",
            "file:crates/other-crate/src/lib.rs",
        ),
        ("file:crates/app/src/lib.rs", "pkg:cargo/serde"),
        // `../src/format.js` is the TypeScript source it compiles from.
        ("file:web/scripts/build.js", "file:web/src/format.ts"),
        ("file:web/src/App.tsx", "file:web/src/api.ts"),
        ("file:web/src/api.ts", "file:web/protocol.ts"),
        ("file:web/src/api.ts", "file:web/src/format.ts"),
        ("file:web/src/api.ts", "pkg:npm/@scope/pkg"),
        ("file:web/src/api.ts", "pkg:npm/react"),
    ];
    assert_eq!(
        imports,
        want.map(|(a, b)| (a.to_owned(), b.to_owned())),
        "std and node:fs are the runtime, not a package"
    );
    let packages: Vec<&str> = got
        .nodes
        .iter()
        .filter(|n| n.kind == kind::node::PACKAGE)
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(packages, ["serde", "@scope/pkg", "react"]);
}

#[test]
fn a_reference_is_an_edge_only_when_its_name_is_exactly_one_definition() {
    let got = extract_code();
    let short = |s: &str| s.replace("m0x6A/rubato:", "");
    let mut references: Vec<(String, String)> = got
        .edges
        .iter()
        .filter(|e| e.kind == kind::edge::REFERENCES)
        .map(|e| (short(&e.from), short(&e.to)))
        .collect();
    references.sort();
    assert_eq!(
        references,
        [
            (
                "file:crates/app/src/lib.rs",
                "sym:crates/other-crate/src/lib.rs#helper"
            ),
            ("file:web/scripts/build.js", "sym:web/src/format.ts#format"),
            ("file:web/src/App.tsx", "sym:web/src/api.ts#request"),
            ("file:web/src/api.ts", "sym:web/src/format.ts#format"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    // `Store::new()` inside store.rs names two definitions: counted, never guessed.
    assert!(
        references.iter().all(|(_, to)| !to.contains("#new")),
        "an ambiguous name became an edge"
    );
    assert!(got.counts.code.unresolved > 0);
    assert_eq!(got.counts.code.references, 4);
    println!(
        "{} references resolved, {} counted and not stored",
        got.counts.code.references, got.counts.code.unresolved
    );
}

#[test]
fn a_file_that_does_not_parse_is_still_a_node_and_is_counted() {
    let got = extract_code();
    // Syntax errors: the file is a node, what parsed around the error is kept, and it counts.
    assert!(
        got.nodes
            .iter()
            .any(|n| n.id == id::file(REPO, "crates/app/src/broken.rs"))
    );
    assert!(
        got.nodes
            .iter()
            .any(|n| n.id == id::symbol(REPO, "crates/app/src/broken.rs", "fine"))
    );
    assert_eq!(got.counts.code.with_errors, 1);
    // Not UTF-8: a node, and nothing read from it.
    let latin1 = got
        .nodes
        .iter()
        .find(|n| n.id == id::file(REPO, "crates/app/src/latin1.rs"))
        .unwrap();
    assert!(latin1.detail.get("lines").is_none());
    assert!(got.nodes.iter().all(|n| !n.id.contains("latin1.rs#")));
    assert_eq!(got.truncated, None);
}
