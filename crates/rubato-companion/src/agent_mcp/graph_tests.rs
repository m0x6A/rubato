//! The five graph tools (#331), through the receiver's real router: read-only, each answer with
//! its age, and no way around `/mcp`'s identity rule.

use super::graph_tools::GRAPH_TOOLS;
use super::tests::{World, world};
use crate::graph::{Fetched, Graph};
use chrono::Utc;
use rubato_graph::fixture::{self, HEAD, REPO};
use rubato_protocol::graph::id;
use serde_json::{Value, json};
use std::sync::Arc;

/// The JSON a tool result's text item holds.
fn body(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// A world whose companion holds the fixture graph as its copy. Nothing is ever fetched.
async fn with_copy() -> (World, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    fixture::copy_at(&dir.path().join(crate::graph::COPY_FILE))
        .await
        .close()
        .await;
    let w = world(Utc::now(), false).await;
    let never: crate::graph::Fetch =
        Arc::new(|_| Box::pin(async { Fetched::Failed("the tools never fetch".into()) }));
    w.st.set_graph(Graph::new(dir.path(), never));
    (w, dir)
}

/// Every graph tool with arguments that find something in the fixture.
fn every_call() -> Vec<(&'static str, Value)> {
    let store = "crates/rubato-server/src/pins/store.rs";
    vec![
        ("graph_status", json!({})),
        ("graph_search", json!({"query": "add pin"})),
        ("graph_node", json!({"id": id::file(REPO, store)})),
        (
            "graph_related",
            json!({"id": id::file(REPO, store), "direction": "both", "depth": 2}),
        ),
        ("graph_context", json!({"path": store})),
        ("graph_context", json!({"part": "companion-mcp"})),
        ("graph_context", json!({"about_issue": "m0x6A/rubato#322"})),
    ]
}

/// What the copy holds: every source's row and its whole snapshot body. SQLite folds its
/// write-ahead log into the file when a reader closes, so the file's bytes are not the measure;
/// its rows are.
async fn contents(dir: &tempfile::TempDir) -> Vec<(rubato_graph::SourceRow, Vec<u8>)> {
    let store = rubato_graph::Store::open_read_only(&dir.path().join(crate::graph::COPY_FILE))
        .await
        .unwrap()
        .expect("a copy");
    let mut out = Vec::new();
    for row in store.sources().await.unwrap() {
        let body = store.export(&row.source).await.unwrap().unwrap();
        out.push((row, body));
    }
    store.close().await;
    out
}

#[tokio::test]
async fn every_graph_tool_answers_and_writes_nothing() {
    let (mut w, dir) = with_copy().await;
    let before = contents(&dir).await;
    let calls = every_call();
    for (tool, args) in &calls {
        let r = w.call(7, tool, args.clone()).await;
        assert_eq!(r["isError"], false, "{tool} {args}: {r}");
    }
    println!("called {} graph tools", calls.len());
    assert!(
        calls
            .iter()
            .map(|(t, _)| *t)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == GRAPH_TOOLS.len()
    );
    // Nothing left the machine, and the copy is byte for byte what it was.
    assert!(w.out.try_recv().is_err(), "a graph tool sent an envelope");
    assert!(
        contents(&dir).await == before,
        "a graph tool changed the copy"
    );

    // The fixture that makes this check fail on purpose: a call that does write is seen.
    let store = rubato_graph::Store::open_path(&dir.path().join(crate::graph::COPY_FILE))
        .await
        .unwrap();
    store.remove_source("team").await.unwrap();
    store.close().await;
    assert!(
        contents(&dir).await != before,
        "the check cannot see a write"
    );
}

#[tokio::test]
async fn every_answer_says_how_old_what_it_read_is() {
    let (w, _dir) = with_copy().await;
    let mut checked = 0;
    for (tool, args) in every_call() {
        let answer = body(&w.call(7, tool, args.clone()).await);
        let sources = if tool == "graph_status" {
            &answer["sources"]
        } else {
            &answer["as_of"]
        };
        let sources = sources
            .as_array()
            .unwrap_or_else(|| panic!("{tool}: {answer}"));
        assert!(!sources.is_empty(), "{tool} {args}: no source named");
        for s in sources {
            assert!(
                s["generated_at"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("2026-11-26T17:00:04")),
                "{tool}: {s}"
            );
            // Nothing announced since this companion started: not known, never "current".
            assert_eq!(s["behind"], Value::Null, "{tool}: {s}");
            if s["source"] == "repo/m0x6A/rubato" {
                assert_eq!(s["head_sha"], HEAD);
            }
        }
        checked += 1;
    }
    println!("checked as_of on {checked} answers");

    let status = body(&w.call(7, "graph_status", json!({})).await);
    assert_eq!(status["copy"], "present");
    assert_eq!(status["link_up"], true);
    assert_eq!(status["offered"], Value::Null);
    assert_eq!(status["sources"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn the_tools_answer_what_the_copy_holds() {
    let (w, _dir) = with_copy().await;
    let store = "crates/rubato-server/src/pins/store.rs";

    let found = body(
        &w.call(
            7,
            "graph_search",
            json!({"query": "add pin", "kinds": ["symbol"]}),
        )
        .await,
    );
    assert_eq!(found["nodes"][0]["id"], id::symbol(REPO, store, "add_pin"));
    assert!(
        found["nodes"][0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("pub async fn add_pin(")
    );
    assert_eq!(found["nodes"][0]["line"], 15);

    let node = body(
        &w.call(7, "graph_node", json!({"id": id::adr(REPO, "0071")}))
            .await,
    );
    assert_eq!(node["found"], true);
    assert_eq!(node["node"]["detail"]["status"], "accepted");
    assert_eq!(node["edges"]["out"][0]["to"], "map:companion-mcp");
    let missing = body(
        &w.call(7, "graph_node", json!({"id": "file:nobody/nothing:x"}))
            .await,
    );
    assert_eq!(missing["found"], false);

    let related = body(
        &w.call(
            7,
            "graph_related",
            json!({"id": id::file(REPO, store), "direction": "in", "edge_kinds": ["imports"]}),
        )
        .await,
    );
    assert_eq!(related["nodes"][0]["name"], "mod.rs");

    // A path defaults to this session's own repository.
    let context = body(&w.call(7, "graph_context", json!({"path": store})).await);
    assert_eq!(context["parts"][0]["name"], "rubato-server");
    assert_eq!(context["seams"][0]["seam"]["name"], "web-server");
    assert_eq!(context["adrs"][0]["detail"]["status"], "accepted");
    assert_eq!(context["issues"][0]["name"], "m0x6A/rubato#322");
    assert_eq!(context["symbols"][0]["name"], "add_pin");
    let seam = body(
        &w.call(7, "graph_context", json!({"part": "companion-mcp"}))
            .await,
    );
    assert_eq!(
        seam["seams"][0]["joins"],
        json!(["claude-code", "rubato-companion"])
    );
    // A title from outside the team is not in the copy, so it is not in an answer.
    assert_eq!(seam["issues"][0]["detail"]["untrusted"], true);
    assert!(seam["issues"][0].get("summary").is_none());

    for (args, why) in [
        (json!({}), "exactly one"),
        (json!({"path": "a", "part": "b"}), "exactly one"),
        (json!({"about_issue": "not an issue"}), "owner/name#n"),
    ] {
        let r = w.call(7, "graph_context", args).await;
        assert_eq!(r["isError"], true);
        assert!(body(&r)["error"].as_str().unwrap().contains(why), "{r}");
    }
    assert_eq!(w.calls("graph_context", "error", "arguments"), 3);
}

#[tokio::test]
async fn a_graph_tool_may_ask_about_any_issue_and_still_needs_a_known_launch() {
    let (w, _dir) = with_copy().await;
    // Launch 7 is on issue #7. Asking about #322 is a read, not an identity claim.
    let r = w
        .call(
            7,
            "graph_context",
            json!({"about_issue": "m0x6A/rubato#322"}),
        )
        .await;
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(body(&r)["subject"]["name"], "m0x6A/rubato#322");
    assert_eq!(w.calls("graph_context", "refused", "identity"), 0);
    assert_eq!(w.calls("graph_context", "ok", ""), 1);

    // `/mcp` keeps its rule: no launch, or one this companion does not know, gets nothing.
    for launch in [None, Some(99)] {
        let res = w
            .rpc(launch, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "graph_status", "arguments": {}}}))
            .await;
        let v: Value = res.json().await.unwrap();
        assert_eq!(v["result"]["isError"], true);
        assert_eq!(body(&v["result"])["refused"], "no_launch");
    }
    assert_eq!(w.calls("graph_status", "refused", "no_launch"), 2);

    // The list names them, read-only.
    let list: Value = w
        .rpc(
            Some(7),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        )
        .await
        .json()
        .await
        .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3 + 5);
    for t in tools
        .iter()
        .filter(|t| t["name"].as_str().unwrap().starts_with("graph_"))
    {
        assert_eq!(t["annotations"]["readOnlyHint"], true, "{t}");
    }
}

#[tokio::test]
async fn without_a_copy_the_tools_say_so_and_without_the_graph_they_are_not_listed() {
    // The graph is on for this machine, and nothing has arrived yet.
    let dir = tempfile::tempdir().unwrap();
    let w = world(Utc::now(), false).await;
    let never: crate::graph::Fetch = Arc::new(|_| Box::pin(async { Fetched::Failed("no".into()) }));
    w.st.set_graph(Graph::new(dir.path(), never));
    let status = body(&w.call(7, "graph_status", json!({})).await);
    assert_eq!(
        (&status["copy"], status["sources"].as_array().unwrap().len()),
        (&json!("none"), 0)
    );
    let r = w.call(7, "graph_search", json!({"query": "pins"})).await;
    assert_eq!(r["isError"], true);
    assert_eq!(body(&r)["refused"], "no_copy");
    assert_eq!(w.calls("graph_search", "refused", "no_copy"), 1);

    // local.yml keeps the graph off this machine: three tools, as before.
    let off = world(Utc::now(), false).await;
    let list: Value = off
        .rpc(
            Some(7),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 3);
    let r = off.call(7, "graph_status", json!({})).await;
    assert_eq!(body(&r)["refused"], "no_copy");
}
