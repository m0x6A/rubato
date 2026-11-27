//! The copy (#330): what a companion does with a snapshot before it believes it, what it keeps
//! when a fetch fails, and when the copy goes.

use super::{COPY_FILE, Fetch, Fetched, Graph, http, pass};
use chrono::{DateTime, Utc};
use rubato_graph::{Build, Store};
use rubato_protocol::graph::{GraphNode, GraphSourceInfo, GraphSources, id, kind, source_key};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const REPO: &str = "m0x6A/rubato";

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn file(path: &str) -> GraphNode {
    GraphNode {
        id: id::file(REPO, path),
        kind: kind::node::FILE.into(),
        name: path.into(),
        repo: Some(REPO.into()),
        path: Some(path.into()),
        line: None,
        end_line: None,
        summary: None,
        detail: Default::default(),
    }
}

/// A stand-in for the server: its own graph file, what it would announce, what it would serve.
struct Server {
    store: Store,
    _dir: tempfile::TempDir,
}

impl Server {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_path(&dir.path().join("server.db"))
            .await
            .unwrap();
        Self { store, _dir: dir }
    }

    /// Build `source` from these files at `head`; what the server announces for it.
    async fn build(&self, source: &str, head: &str, paths: &[&str]) -> GraphSourceInfo {
        let source_kind = source_key::kind_of(source).unwrap();
        let build = Build {
            source: source.into(),
            kind: source_kind.into(),
            head_sha: Some(head.into()),
            map_version: None,
            generator: 1,
            generated_at: at("2026-11-26T17:00:04Z"),
            truncated: false,
            nodes: paths.iter().map(|p| file(p)).collect(),
            edges: vec![],
        };
        let g = self.store.begin_build(source, source_kind).await.unwrap();
        self.store
            .replace_source(Some(g), build, at("2026-11-26T17:00:05Z"))
            .await
            .unwrap();
        let row = self.store.source(source).await.unwrap().unwrap();
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
        }
    }

    async fn body(&self, source: &str) -> Vec<u8> {
        self.store.export(source).await.unwrap().unwrap()
    }
}

/// What the fake fetch answers per source, and every source it was asked for.
#[derive(Default)]
struct Answers {
    by_source: Mutex<HashMap<String, Fetched>>,
    asked: Mutex<Vec<String>>,
}

impl Answers {
    fn set(&self, source: &str, answer: Fetched) {
        self.by_source.lock().unwrap().insert(source.into(), answer);
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
    fn fetch(self: &Arc<Self>) -> Fetch {
        let me = self.clone();
        Arc::new(move |source: String| {
            let me = me.clone();
            Box::pin(async move {
                me.asked.lock().unwrap().push(source.clone());
                me.by_source
                    .lock()
                    .unwrap()
                    .get(&source)
                    .cloned()
                    .unwrap_or_else(|| Fetched::Failed("HTTP 404".into()))
            })
        })
    }
}

fn companion(answers: &Arc<Answers>) -> (Graph, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Graph::new(dir.path(), answers.fetch()), dir)
}

fn announce(graph: &Graph, sources: &[&GraphSourceInfo]) {
    graph.on_sources(GraphSources {
        sources: sources.iter().map(|s| (*s).clone()).collect(),
    });
}

async fn has(graph: &Graph, path: &str) -> bool {
    let Some(store) = graph.reader().await else {
        return false;
    };
    let found = store.node(&id::file(REPO, path)).await.unwrap().is_some();
    store.close().await;
    found
}

const NOW: &str = "2026-11-26T17:01:00Z";

#[tokio::test]
async fn an_announced_source_is_fetched_checked_and_applied_once() {
    let server = Server::new().await;
    let key = source_key::repo(REPO);
    let v1 = server.build(&key, "sha1", &["a.rs", "b.rs"]).await;
    let answers = Arc::new(Answers::default());
    answers.set(&key, Fetched::Body(server.body(&key).await));
    let (graph, dir) = companion(&answers);

    // Before any announce: no copy, nothing asked, nothing known.
    assert_eq!(pass(&graph, at(NOW)).await, Default::default());
    assert!(graph.reader().await.is_none());
    assert!(graph.status().await.sources.is_empty());

    announce(&graph, &[&v1]);
    let first = pass(&graph, at(NOW)).await;
    assert_eq!(first.applied, std::slice::from_ref(&key));
    assert!(has(&graph, "a.rs").await);
    let status = graph.status().await;
    assert_eq!(status.sources.len(), 1);
    let s = &status.sources[0];
    assert_eq!(s.row.head_sha.as_deref(), Some("sha1"));
    assert_eq!(
        s.row.content_hash.as_deref(),
        Some(v1.content_hash.as_str())
    );
    assert_eq!(
        (s.behind, s.not_refreshed, s.error.clone()),
        (Some(false), false, None)
    );
    assert_eq!(graph.behind_secs(at("2026-11-26T18:00:00Z")), 0);

    // The owner's alone, as the token beside it is.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(COPY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }

    // The same list again: the copy matches, nothing is fetched.
    let second = pass(&graph, at(NOW)).await;
    assert_eq!(second, Default::default());
    assert_eq!(
        answers.asked(),
        std::slice::from_ref(&key),
        "one fetch in all"
    );

    // The same graph at a newer head: where it stands is noted, nothing is fetched.
    let moved = GraphSourceInfo {
        head_sha: Some("sha2".into()),
        ..v1.clone()
    };
    announce(&graph, &[&moved]);
    pass(&graph, at(NOW)).await;
    assert_eq!(answers.asked().len(), 1);
    assert_eq!(
        graph.status().await.sources[0].row.head_sha.as_deref(),
        Some("sha2")
    );
}

#[tokio::test]
async fn a_snapshot_that_does_not_check_out_is_dropped_and_the_old_copy_answers() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::{Read, Write};

    let server = Server::new().await;
    let key = source_key::repo(REPO);
    let v1 = server.build(&key, "sha1", &["a.rs"]).await;
    let v1_body = server.body(&key).await;
    let answers = Arc::new(Answers::default());
    answers.set(&key, Fetched::Body(v1_body.clone()));
    let (graph, _dir) = companion(&answers);
    announce(&graph, &[&v1]);
    pass(&graph, at(NOW)).await;

    let v2 = server.build(&key, "sha2", &["a.rs", "new.rs"]).await;
    let v2_body = server.body(&key).await;
    announce(&graph, &[&v2]);

    // Tampered on the way: its lines do not hash to its header.
    let mut text = String::new();
    flate2::read::GzDecoder::new(&v2_body[..])
        .read_to_string(&mut text)
        .unwrap();
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(text.replace("new.rs", "evil.rs").as_bytes())
        .unwrap();
    answers.set(&key, Fetched::Body(gz.finish().unwrap()));
    let p = pass(&graph, at(NOW)).await;
    assert_eq!(p.kept, [(key.clone(), "hash_mismatch")]);
    assert!(
        has(&graph, "a.rs").await && !has(&graph, "evil.rs").await && !has(&graph, "new.rs").await
    );
    let s = graph.status().await.sources.remove(0);
    assert_eq!(s.behind, Some(true));
    assert!(
        s.error.as_deref().unwrap().contains("hash"),
        "{:?}",
        s.error
    );
    assert_eq!(
        s.row.head_sha.as_deref(),
        Some("sha1"),
        "it still says what it is"
    );
    // Behind since that pass, and rising: what the alert reads.
    assert_eq!(graph.behind_secs(at("2026-11-26T17:11:00Z")), 600);

    // An honest body that is not the announced one (the server moved on): dropped too.
    answers.set(&key, Fetched::Body(v1_body));
    assert_eq!(
        pass(&graph, at(NOW)).await.kept,
        [(key.clone(), "superseded")]
    );
    // Not gzip at all, and a server that does not answer: the copy stays.
    for (answer, reason) in [
        (Fetched::Body(b"<html>502</html>".to_vec()), "parse"),
        (
            Fetched::Failed("request: connection refused".into()),
            "http",
        ),
    ] {
        answers.set(&key, answer);
        assert_eq!(pass(&graph, at(NOW)).await.kept, [(key.clone(), reason)]);
        assert!(has(&graph, "a.rs").await);
    }

    // The real one arrives: applied, in step again.
    answers.set(&key, Fetched::Body(v2_body));
    assert_eq!(
        pass(&graph, at(NOW)).await.applied,
        std::slice::from_ref(&key)
    );
    assert!(has(&graph, "new.rs").await);
    let s = graph.status().await.sources.remove(0);
    assert_eq!((s.behind, s.error), (Some(false), None));
    assert_eq!(graph.behind_secs(at("2026-11-26T18:00:00Z")), 0);
}

#[tokio::test]
async fn a_403_keeps_the_copy_marked_and_a_401_deletes_it() {
    let server = Server::new().await;
    let (repo, team) = (source_key::repo(REPO), source_key::TEAM.to_owned());
    let v1 = server.build(&repo, "sha1", &["a.rs"]).await;
    let answers = Arc::new(Answers::default());
    answers.set(&repo, Fetched::Body(server.body(&repo).await));
    let (graph, dir) = companion(&answers);
    announce(&graph, &[&v1]);
    pass(&graph, at(NOW)).await;

    // The owner's team record went stale: the server refreshes nothing. The copy answers on.
    let v2 = server.build(&repo, "sha2", &["a.rs", "b.rs"]).await;
    let t1 = server.build(&team, "", &["t"]).await;
    announce(&graph, &[&v2, &t1]);
    answers.set(&repo, Fetched::NotEntitled);
    answers.set(&team, Fetched::NotEntitled);
    let before = answers.asked().len();
    let p = pass(&graph, at(NOW)).await;
    assert_eq!(
        p.kept,
        [
            (repo.clone(), "team_record_stale"),
            (team.clone(), "team_record_stale")
        ]
    );
    assert_eq!(
        answers.asked().len(),
        before + 1,
        "one refusal is enough to stop asking"
    );
    assert!(has(&graph, "a.rs").await);
    let s = graph.status().await.sources.remove(0);
    assert!(s.not_refreshed && s.behind == Some(true));

    // The owner logs in again: refreshed, the mark gone.
    answers.set(&repo, Fetched::Body(server.body(&repo).await));
    answers.set(&team, Fetched::Body(server.body(&team).await));
    assert_eq!(pass(&graph, at(NOW)).await.applied.len(), 2);
    assert!(
        graph
            .status()
            .await
            .sources
            .iter()
            .all(|s| !s.not_refreshed)
    );

    // The pairing is revoked: nothing of the team's stays on this machine.
    let v3 = server.build(&repo, "sha3", &["c.rs"]).await;
    announce(&graph, &[&v3]);
    answers.set(&repo, Fetched::Unauthorized);
    let p = pass(&graph, at(NOW)).await;
    assert!(p.deleted_copy);
    assert!(!dir.path().join(COPY_FILE).exists());
    assert!(graph.reader().await.is_none());
    assert!(graph.status().await.sources.is_empty());
}

#[tokio::test]
async fn a_source_the_list_no_longer_names_is_removed_but_an_empty_list_removes_nothing() {
    let server = Server::new().await;
    let (repo, other) = (source_key::repo(REPO), source_key::repo("m0x6A/other"));
    let a = server.build(&repo, "sha1", &["a.rs"]).await;
    let b = server.build(&other, "o1", &["b.rs"]).await;
    let answers = Arc::new(Answers::default());
    answers.set(&repo, Fetched::Body(server.body(&repo).await));
    answers.set(&other, Fetched::Body(server.body(&other).await));
    let (graph, _dir) = companion(&answers);
    announce(&graph, &[&a, &b]);
    assert_eq!(pass(&graph, at(NOW)).await.applied.len(), 2);

    // A server with a new, empty graph file has built nothing yet: not a reason to empty the copy.
    announce(&graph, &[]);
    assert!(pass(&graph, at(NOW)).await.removed.is_empty());
    assert!(has(&graph, "a.rs").await && has(&graph, "b.rs").await);

    // A list that names one of the two: the other goes.
    announce(&graph, &[&a]);
    assert_eq!(pass(&graph, at(NOW)).await.removed, [other]);
    assert!(has(&graph, "a.rs").await && !has(&graph, "b.rs").await);

    // A list over the cap is not a list a server sends: ignored whole.
    graph.on_sources(GraphSources {
        sources: (0..65)
            .map(|i| GraphSourceInfo {
                source: format!("repo/o/r{i}"),
                ..a.clone()
            })
            .collect(),
    });
    assert_eq!(pass(&graph, at(NOW)).await, Default::default());
    assert!(has(&graph, "a.rs").await);
}

#[tokio::test]
async fn the_copy_answers_without_the_server_and_says_what_it_does_not_know() {
    let server = Server::new().await;
    let key = source_key::repo(REPO);
    let v1 = server.build(&key, "sha1", &["a.rs"]).await;
    let answers = Arc::new(Answers::default());
    answers.set(&key, Fetched::Body(server.body(&key).await));
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::new(dir.path(), answers.fetch());
    announce(&graph, &[&v1]);
    pass(&graph, at(NOW)).await;
    graph.on_welcome(true);
    assert_eq!(graph.status().await.offered, Some(true));

    // A restart with the link down: the file is still there, and nothing has been announced.
    let restarted = Graph::new(dir.path(), answers.fetch());
    assert!(has(&restarted, "a.rs").await);
    let status = restarted.status().await;
    assert_eq!(status.offered, None);
    assert_eq!(
        status.sources[0].behind, None,
        "not known is not \"current\""
    );
    // A server that no longer offers the graph changes nothing in the copy.
    restarted.on_welcome(false);
    assert_eq!(pass(&restarted, at(NOW)).await, Default::default());
    assert!(has(&restarted, "a.rs").await);
    assert_eq!(restarted.status().await.offered, Some(false));
}

/// The real fetch against a real socket: the token as a bearer, the source as a query, and each
/// status as its own answer. And never the token in clear text off this machine.
#[tokio::test]
async fn the_real_fetch_sends_the_token_and_reads_each_answer() {
    use axum::extract::Query;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;

    async fn snapshot(
        headers: HeaderMap,
        Query(q): Query<HashMap<String, String>>,
    ) -> (StatusCode, Vec<u8>) {
        let bearer = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        match (bearer, q.get("source").map(String::as_str)) {
            ("Bearer the-token", Some("repo/m0x6A/rubato")) => (StatusCode::OK, b"a body".to_vec()),
            ("Bearer the-token", Some("team")) => (StatusCode::FORBIDDEN, vec![]),
            ("Bearer the-token", _) => (StatusCode::NOT_FOUND, vec![]),
            _ => (StatusCode::UNAUTHORIZED, vec![]),
        }
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        rubato_protocol::graph::routes::GRAPH_SNAPSHOT,
        get(snapshot),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let fetch = http(url.clone(), "the-token".into(), false);
    assert_eq!(
        fetch("repo/m0x6A/rubato".into()).await,
        Fetched::Body(b"a body".to_vec())
    );
    assert_eq!(fetch("team".into()).await, Fetched::NotEntitled);
    assert_eq!(
        fetch("workspace/gone".into()).await,
        Fetched::Failed("HTTP 404".into())
    );
    let revoked = http(url, "an-old-token".into(), false);
    assert_eq!(revoked("team".into()).await, Fetched::Unauthorized);

    // Plain http off loopback: refused before any request, unless the pairing was `--insecure`
    // (ADR 0067). 192.0.2.1 is a documentation address; nothing is sent to it.
    let lan = http("http://192.0.2.1:9".into(), "the-token".into(), false);
    match lan("team".into()).await {
        Fetched::Failed(why) => assert!(why.contains("plain text off loopback"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}
