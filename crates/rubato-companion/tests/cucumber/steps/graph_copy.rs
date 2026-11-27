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
