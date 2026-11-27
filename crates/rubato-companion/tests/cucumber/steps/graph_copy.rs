//! The companion's copy of the context graph (ADR 0086, `features/graph-copy.feature`).

use crate::world::RubatoWorld;
use cucumber::{given, then, when};
use rubato_graph::{Build, Store};
use rubato_protocol::graph::{GraphNode, GraphSourceInfo, GraphSources, id, kind, source_key};
use std::time::Duration;

/// Build `source` in a server-side store of its own: what the server would announce, and the
/// snapshot body it would serve.
async fn built(source: &str, head: &str, path: &str) -> (GraphSourceInfo, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_path(&dir.path().join("server.db"))
        .await
        .unwrap();
    let repo = source.strip_prefix("repo/").expect("a repo source");
    let build = Build {
        source: source.to_owned(),
        kind: source_key::kind_of(source).unwrap().to_owned(),
        head_sha: Some(head.to_owned()),
        map_version: None,
        generator: 1,
        generated_at: chrono::Utc::now(),
        truncated: false,
        nodes: vec![GraphNode {
            id: id::file(repo, path),
            kind: kind::node::FILE.into(),
            name: path.to_owned(),
            repo: Some(repo.to_owned()),
            path: Some(path.to_owned()),
            line: None,
            end_line: None,
            summary: None,
            detail: Default::default(),
        }],
        edges: vec![],
    };
    let generation = store.begin_build(source, &build.kind).await.unwrap();
    store
        .replace_source(Some(generation), build, chrono::Utc::now())
        .await
        .unwrap();
    let row = store.source(source).await.unwrap().unwrap();
    let body = store.export(source).await.unwrap().unwrap();
    store.close().await;
    (
        GraphSourceInfo {
            source: row.source,
            kind: row.kind,
            head_sha: row.head_sha,
            map_version: row.map_version,
            content_hash: row.content_hash.unwrap(),
            generated_at: row.generated_at.unwrap(),
            nodes: row.nodes,
            edges: row.edges,
            truncated: row.truncated,
        },
        body,
    )
}

#[given(
    expr = "the test server holds the context graph source {string} at head {string} with the file {string}"
)]
async fn given_source(w: &mut RubatoWorld, source: String, head: String, path: String) {
    let (info, body) = built(&source, &head, &path).await;
    w.server().snapshots.lock().unwrap().insert(source, body);
    w.graph_announce.push(info);
}

#[given(expr = "the test server serves another body for {string} than it announces")]
async fn given_other_body(w: &mut RubatoWorld, source: String) {
    // An honest snapshot of the same source at another head: it hashes to its own header, and
    // not to what the server announces.
    let (_, other) = built(&source, "ffff000", "src/other.rs").await;
    w.server().snapshots.lock().unwrap().insert(source, other);
}

#[when("the server announces its context graph sources")]
async fn when_announce(w: &mut RubatoWorld) {
    w.server()
        .push(rubato_protocol::ServerMessage::GraphSources(GraphSources {
            sources: w.graph_announce.clone(),
        }));
}

async fn copy(w: &RubatoWorld) -> Option<Store> {
    Store::open_read_only(&w.state_dir.join(rubato_companion::graph::COPY_FILE))
        .await
        .unwrap()
}

#[then(expr = "the companion's copy holds {string} at head {string} with the file {string}")]
async fn then_copy_holds(w: &mut RubatoWorld, source: String, head: String, path: String) {
    let repo = source.strip_prefix("repo/").unwrap().to_owned();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(store) = copy(w).await {
            let row = store.source(&source).await.unwrap();
            let node = store.node(&id::file(&repo, &path)).await.unwrap();
            store.close().await;
            if let (Some(row), Some(_)) = (row, node) {
                assert_eq!(row.head_sha.as_deref(), Some(head.as_str()));
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the copy never held {source}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn asked(w: &RubatoWorld, source: &str) -> Vec<(String, bool)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let asks = w.server().snapshot_asks.lock().unwrap().clone();
        if asks.iter().any(|(s, _)| s == source) {
            return asks;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the snapshot of {source} was never asked for"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[then("every snapshot was asked for with the pairing token")]
async fn then_with_token(w: &mut RubatoWorld) {
    let asks = w.server().snapshot_asks.lock().unwrap().clone();
    assert!(!asks.is_empty(), "no snapshot was asked for");
    assert!(asks.iter().all(|(_, with_token)| *with_token), "{asks:?}");
}

#[then(expr = "the snapshot of {string} was asked for")]
async fn then_asked(w: &mut RubatoWorld, source: String) {
    asked(w, &source).await;
}

#[then("the companion's copy holds no source")]
async fn then_copy_empty(w: &mut RubatoWorld) {
    // The fetch has been answered; give the apply the time it would have taken.
    tokio::time::sleep(Duration::from_millis(500)).await;
    if let Some(store) = copy(w).await {
        let rows = store.sources().await.unwrap();
        store.close().await;
        assert!(
            rows.is_empty(),
            "a snapshot that was not announced was applied: {rows:?}"
        );
    }
}

/// Run `rubato-companion mcp` as Claude Code would: messages on stdin, one a line, answers on
/// stdout. `launched` sets `RUBATO_LAUNCH_ID`, as a session Rubato started has it.
fn bridge(
    w: &RubatoWorld,
    launched: bool,
    messages: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubato-companion"));
    cmd.arg("--state-dir")
        .arg(&w.state_dir)
        .arg("mcp")
        .current_dir(&w.root)
        .env_remove("RUBATO_LAUNCH_ID")
        .env_remove("RUBATO_FORK_TURN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if launched {
        cmd.env("RUBATO_LAUNCH_ID", "9b2e4d1a-3c5f-4e6a-9b7c-8d9e0f1a2b3c");
    }
    let mut child = cmd.spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for m in messages {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "the bridge failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Stdout is the protocol: every line of it is one JSON-RPC message.
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not a message: {l:?}: {e}")))
        .collect()
}

fn handshake() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    ]
}

#[then(
    expr = "a process speaking MCP over stdio to the companion's bridge lists {int} read-only tools"
)]
async fn then_bridge_lists(w: &mut RubatoWorld, n: usize) {
    let answers = bridge(w, false, &handshake());
    // Two answers for three messages: the notification has none.
    assert_eq!(answers.len(), 2, "{answers:?}");
    assert_eq!(answers[0]["result"]["serverInfo"]["name"], "rubato-graph");
    let tools = answers[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), n, "{tools:?}");
    assert!(
        tools
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true)
    );
}

#[then(expr = "asked through the bridge, graph_status names {string} at head {string}")]
async fn then_bridge_status(w: &mut RubatoWorld, source: String, head: String) {
    let mut messages = handshake();
    messages.push(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "graph_status", "arguments": {}}}));
    let answers = bridge(w, false, &messages);
    let result = &answers.last().unwrap()["result"];
    assert_eq!(result["isError"], false, "{result}");
    let status: serde_json::Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    let named = status["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["source"] == source.as_str())
        .unwrap_or_else(|| panic!("{source} is not in {status}"));
    assert_eq!(named["head_sha"], head.as_str());
    // The server announced exactly this one, so the copy is known to be current.
    assert_eq!(named["behind"], false);
}

#[then("in a session Rubato launched the bridge lists no tools")]
async fn then_bridge_stands_down(w: &mut RubatoWorld) {
    let answers = bridge(w, true, &handshake());
    assert_eq!(answers[1]["result"]["tools"], serde_json::json!([]));
}
