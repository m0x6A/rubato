use crate::{Build, Direction, Error, Replaced, Snapshot, Store, USER_VERSION};
use chrono::{DateTime, Utc};
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind};
use serde_json::{Map, Value, json};

const REPO: &str = "m0x6A/rubato";

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn detail(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn file(path: &str) -> GraphNode {
    GraphNode {
        id: id::file(REPO, path),
        kind: kind::node::FILE.into(),
        name: path.rsplit('/').next().unwrap().into(),
        repo: Some(REPO.into()),
        path: Some(path.into()),
        line: None,
        end_line: None,
        summary: None,
        detail: detail(&[("language", json!("rust"))]),
    }
}

fn symbol(path: &str, name: &str, line: u32, signature: &str) -> GraphNode {
    GraphNode {
        id: id::symbol(REPO, path, name),
        kind: kind::node::SYMBOL.into(),
        name: name.into(),
        repo: Some(REPO.into()),
        path: Some(path.into()),
        line: Some(line),
        end_line: Some(line + 9),
        summary: Some(signature.into()),
        detail: detail(&[("symbol", json!("function"))]),
    }
}

fn edge(from: &str, kind: &str, to: &str) -> GraphEdge {
    GraphEdge {
        from: from.into(),
        kind: kind.into(),
        to: to.into(),
        detail: Map::new(),
    }
}

/// A small repo source: two files, two symbols, an import and a reference.
fn small() -> Build {
    let (store, snap) = (
        "crates/rubato-graph/src/store.rs",
        "crates/rubato-graph/src/snapshot.rs",
    );
    Build {
        source: rubato_protocol::graph::source_key::repo(REPO),
        kind: kind::source::REPO.into(),
        head_sha: Some("b35788b".into()),
        map_version: None,
        generator: 1,
        generated_at: at("2026-11-26T17:00:04Z"),
        truncated: false,
        nodes: vec![
            symbol(
                store,
                "replace_source",
                240,
                "pub async fn replace_source(&self, …)",
            ),
            file(snap),
            file(store),
            symbol(
                snap,
                "node_line",
                44,
                "pub fn node_line(node: &GraphNode) -> String",
            ),
        ],
        edges: vec![
            edge(
                &id::file(REPO, store),
                kind::edge::IMPORTS,
                &id::file(REPO, snap),
            ),
            edge(
                &id::file(REPO, store),
                kind::edge::DEFINES,
                &id::symbol(REPO, store, "replace_source"),
            ),
            edge(
                &id::file(REPO, snap),
                kind::edge::DEFINES,
                &id::symbol(REPO, snap, "node_line"),
            ),
            edge(
                &id::file(REPO, store),
                kind::edge::REFERENCES,
                &id::symbol(REPO, snap, "node_line"),
            ),
        ],
    }
}

async fn file_store(dir: &tempfile::TempDir) -> (Store, String) {
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("graph.db").display()
    );
    (Store::open(&url).await.unwrap(), url)
}

fn written(r: Replaced) -> String {
    match r {
        Replaced::Written { content_hash, .. } => content_hash,
        other => panic!("expected a write, got {other:?}"),
    }
}

#[tokio::test]
async fn a_replaced_source_reads_back_whole_after_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let (store, url) = file_store(&dir).await;
    let build = small();
    let generation = store.begin_build(&build.source, &build.kind).await.unwrap();
    let now = at("2026-11-26T17:00:05Z");
    let hash = written(
        store
            .replace_source(Some(generation), build.clone(), now)
            .await
            .unwrap(),
    );
    drop(store);

    let store = Store::open(&url).await.unwrap();
    let row = store.source(&build.source).await.unwrap().unwrap();
    assert_eq!(row.content_hash.as_deref(), Some(hash.as_str()));
    assert_eq!((row.nodes, row.edges), (4, 4));
    assert_eq!(row.head_sha.as_deref(), Some("b35788b"));
    assert_eq!(row.generated_at, Some(build.generated_at));
    assert_eq!(row.checked_at, Some(now));
    let node = store
        .node(&id::symbol(
            REPO,
            "crates/rubato-graph/src/store.rs",
            "replace_source",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(node.line, Some(240));
    assert_eq!(node.detail["symbol"], "function");

    // The read-only opener a tool uses sees the same rows and cannot write.
    let ro = Store::open_read_only(&dir.path().join("graph.db"))
        .await
        .unwrap()
        .expect("the file exists at this version");
    assert_eq!(ro.sources().await.unwrap(), vec![row]);
    assert!(matches!(
        ro.remove_source(&build.source).await,
        Err(Error::Db(_))
    ));
    assert!(
        Store::open_read_only(&dir.path().join("no-such.db"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_replace_that_lost_its_generation_writes_nothing() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let build = small();
    let now = at("2026-11-26T17:00:05Z");
    let first = store.begin_build(&build.source, &build.kind).await.unwrap();
    let second = store.begin_build(&build.source, &build.kind).await.unwrap();
    assert_eq!((first, second), (1, 2));

    // The older build finishes last: it must not write over what the newer one will store.
    let lost = store
        .replace_source(Some(first), build.clone(), now)
        .await
        .unwrap();
    assert_eq!(lost, Replaced::Lost);
    assert!(store.node(&build.nodes[0].id).await.unwrap().is_none());
    assert_eq!(store.source(&build.source).await.unwrap().unwrap().nodes, 0);

    written(
        store
            .replace_source(Some(second), build.clone(), now)
            .await
            .unwrap(),
    );

    // Removed while a build runs: that build finds no row and writes nothing.
    let third = store.begin_build(&build.source, &build.kind).await.unwrap();
    assert!(store.remove_source(&build.source).await.unwrap());
    assert_eq!(
        store
            .replace_source(Some(third), build.clone(), now)
            .await
            .unwrap(),
        Replaced::Lost
    );
    assert!(store.sources().await.unwrap().is_empty());
    assert!(store.node(&build.nodes[0].id).await.unwrap().is_none());
    assert!(
        store
            .search("replace", &[], None, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_failed_build_keeps_the_previous_rows_and_says_so() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let build = small();
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    let hash = written(
        store
            .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
            .await
            .unwrap(),
    );
    store
        .record_error(
            &build.source,
            at("2026-11-26T17:15:05Z"),
            "tarball: deadline",
        )
        .await
        .unwrap();
    let row = store.source(&build.source).await.unwrap().unwrap();
    assert_eq!(row.error.as_deref(), Some("tarball: deadline"));
    assert_eq!(row.attempted_at, Some(at("2026-11-26T17:15:05Z")));
    assert_eq!(row.content_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(
        row.checked_at,
        Some(at("2026-11-26T17:00:05Z")),
        "not current"
    );
    assert_eq!(row.nodes, 4);

    store
        .mark_checked(&build.source, at("2026-11-26T17:30:05Z"))
        .await
        .unwrap();
    let row = store.source(&build.source).await.unwrap().unwrap();
    assert_eq!((row.error, row.attempted_at), (None, None));
}

#[tokio::test]
async fn the_stored_hash_is_the_hash_of_the_served_body() {
    let dir = tempfile::tempdir().unwrap();
    let (store, url) = file_store(&dir).await;
    let build = small();
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    let hash = written(
        store
            .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
            .await
            .unwrap(),
    );
    drop(store);
    let store = Store::open(&url).await.unwrap();

    let body = store.export(&build.source).await.unwrap().unwrap();
    let snapshot = Snapshot::parse(&body, 1 << 20).unwrap();
    assert_eq!(snapshot.header.content_hash, hash);
    assert_eq!(snapshot.header.head_sha.as_deref(), Some("b35788b"));
    assert_eq!((snapshot.nodes.len(), snapshot.edges.len()), (4, 4));
    let ids: Vec<&str> = snapshot.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "nodes travel in id order");

    // A companion applies it and ends with the same source at the same hash.
    let copy = Store::open("sqlite::memory:").await.unwrap();
    let applied = copy
        .apply(snapshot, at("2026-11-26T17:01:00Z"))
        .await
        .unwrap();
    assert_eq!(written(applied), hash);
    assert_eq!(copy.export(&build.source).await.unwrap().unwrap(), body);

    // Never built, or unknown: nothing to serve.
    store.begin_build("team", kind::source::TEAM).await.unwrap();
    assert!(store.export("team").await.unwrap().is_none());
    assert!(store.export("repo/o/none").await.unwrap().is_none());
}

#[tokio::test]
async fn the_same_graph_built_in_another_order_has_the_same_hash_and_writes_no_rows() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let mut build = small();
    build.nodes[0].detail = detail(&[("b", json!({"y": 1, "x": 2})), ("a", json!(1))]);
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    let hash = written(
        store
            .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
            .await
            .unwrap(),
    );

    let mut again = build.clone();
    again.nodes.reverse();
    again.edges.reverse();
    again.nodes.push(again.nodes[0].clone());
    let last = again.nodes.len() - 2;
    again.nodes[last].detail = detail(&[("a", json!(1)), ("b", json!({"x": 2, "y": 1}))]);
    again.head_sha = Some("c0ffee0".into());
    again.generated_at = at("2026-11-26T17:15:04Z");
    let g = store.begin_build(&again.source, &again.kind).await.unwrap();
    let r = store
        .replace_source(Some(g), again, at("2026-11-26T17:15:05Z"))
        .await
        .unwrap();
    assert_eq!(r, Replaced::Unchanged { content_hash: hash });
    let row = store.source(&build.source).await.unwrap().unwrap();
    assert_eq!(
        row.generated_at,
        Some(build.generated_at),
        "the rows are the old ones"
    );
    assert_eq!(row.checked_at, Some(at("2026-11-26T17:15:05Z")));
    assert_eq!(
        row.head_sha.as_deref(),
        Some("c0ffee0"),
        "and it says where it stands"
    );

    // A new generator rewrites even an equal graph, so a fixed extractor reaches every copy.
    let mut bumped = build.clone();
    bumped.generator = 2;
    let g = store
        .begin_build(&bumped.source, &bumped.kind)
        .await
        .unwrap();
    assert!(matches!(
        store
            .replace_source(Some(g), bumped, at("2026-11-26T17:30:05Z"))
            .await
            .unwrap(),
        Replaced::Written { duplicates: 0, .. }
    ));
}

#[tokio::test]
async fn a_body_that_must_not_be_applied_is_refused() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::{Read, Write};

    let store = Store::open("sqlite::memory:").await.unwrap();
    let build = small();
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    store
        .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
        .await
        .unwrap();
    let good = store.export(&build.source).await.unwrap().unwrap();
    let mut text = String::new();
    flate2::read::GzDecoder::new(&good[..])
        .read_to_string(&mut text)
        .unwrap();
    let gz = |text: &str| {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(text.as_bytes()).unwrap();
        e.finish().unwrap()
    };
    let refused = |body: Vec<u8>, max: u64| match Snapshot::parse(&body, max) {
        Err(Error::Snapshot(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    };

    assert!(Snapshot::parse(&gz(&text), 1 << 20).is_ok(), "the control");
    let cases = [
        (
            text.replace("replace_source", "replace_sourcE"),
            1 << 20,
            "hash",
        ),
        (
            text.replace("\"format\":1", "\"format\":2"),
            1 << 20,
            "format 2",
        ),
        (
            text.lines().take(3).collect::<Vec<_>>().join("\n"),
            1 << 20,
            "hash",
        ),
        (text.clone(), 64, "over 64 bytes"),
        (String::new(), 1 << 20, "empty"),
    ];
    for (body, max, want) in &cases {
        let why = refused(gz(body), *max);
        assert!(why.contains(want), "{why:?} should name {want:?}");
    }
    println!("checked {} bodies that must be refused", cases.len());
}

#[tokio::test]
async fn a_file_of_another_schema_version_is_emptied_and_created_again() {
    let dir = tempfile::tempdir().unwrap();
    let (store, url) = file_store(&dir).await;
    let build = small();
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    store
        .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
        .await
        .unwrap();
    sqlx::query(&format!("PRAGMA user_version = {}", USER_VERSION + 1))
        .execute(store.pool())
        .await
        .unwrap();
    drop(store);

    // A reader of this version has no copy to answer from.
    assert!(
        Store::open_read_only(&dir.path().join("graph.db"))
            .await
            .unwrap()
            .is_none()
    );
    let store = Store::open(&url).await.unwrap();
    assert!(store.sources().await.unwrap().is_empty());
    assert!(store.node(&build.nodes[0].id).await.unwrap().is_none());
    assert_eq!(
        crate::schema::user_version(store.pool()).await.unwrap(),
        USER_VERSION
    );
}

#[tokio::test]
async fn search_and_traversal_answer_from_the_stored_rows() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let mut build = small();
    // A kind this version does not know is stored and found like any other.
    build.nodes.push(GraphNode {
        id: "runbook:m0x6A/rubato:replace".into(),
        kind: "runbook".into(),
        name: "How to replace a disk".into(),
        repo: Some(REPO.into()),
        path: None,
        line: None,
        end_line: None,
        summary: None,
        detail: Map::new(),
    });
    // A plan names a map id without a workspace: an edge to no node in this file.
    let store_file = id::file(REPO, "crates/rubato-graph/src/store.rs");
    build.edges.push(edge(
        &store_file,
        "documents",
        &id::map_ref("rubato-server"),
    ));
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    store
        .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
        .await
        .unwrap();

    let names = |nodes: &[GraphNode]| nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
    let found = store.search("replace", &[], None, 10).await.unwrap();
    assert_eq!(
        names(&found),
        ["How to replace a disk", "replace_source"],
        "a prefix of a word in the name"
    );
    let found = store
        .search("replace", &[kind::node::SYMBOL.into()], None, 10)
        .await
        .unwrap();
    assert_eq!(names(&found), ["replace_source"]);
    let found = store
        .search("snapshot rs", &[], Some(REPO), 10)
        .await
        .unwrap();
    assert_eq!(
        names(&found),
        ["snapshot.rs", "node_line"],
        "words of a path"
    );
    assert!(
        store
            .search("replace", &[], Some("other/repo"), 10)
            .await
            .unwrap()
            .is_empty()
    );
    for hostile in [r#"" OR 1"#, "NEAR(", "*", "a:b", "--"] {
        store.search(hostile, &[], None, 10).await.unwrap();
    }

    let one = store
        .related(&store_file, &[], Direction::Out, 1, 10)
        .await
        .unwrap();
    assert_eq!(
        names(&one.nodes),
        ["replace_source", "snapshot.rs", "node_line"]
    );
    assert_eq!(one.edges.len(), 4, "the edge to map:rubato-server is kept");
    assert!(!one.truncated);

    let imports = store
        .related(
            &store_file,
            &[kind::edge::IMPORTS.into()],
            Direction::Out,
            3,
            10,
        )
        .await
        .unwrap();
    assert_eq!(names(&imports.nodes), ["snapshot.rs"]);

    let back = store
        .related(
            &id::symbol(REPO, "crates/rubato-graph/src/snapshot.rs", "node_line"),
            &[],
            Direction::In,
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(names(&back.nodes), ["snapshot.rs", "store.rs"]);

    let cut = store
        .related(&store_file, &[], Direction::Both, 3, 2)
        .await
        .unwrap();
    assert_eq!(cut.nodes.len(), 2);
    assert!(cut.truncated);
}

#[tokio::test]
async fn no_column_holds_a_progress_state_or_a_session() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let cols = store.all_columns().await.unwrap();
    assert!(cols.len() >= 25, "parsed only {} columns", cols.len());
    let offenders: Vec<_> = cols
        .iter()
        .filter(|(_, c)| c.contains("progress") || c.contains("session") || c == "state")
        .collect();
    assert!(
        offenders.is_empty(),
        "the graph stores no progress: {offenders:?}"
    );
    println!("checked {} columns of the graph file", cols.len());
}

/// A hundred thousand nodes and as many edges, replaced in one transaction, while a reader
/// keeps reading the old source until the new one is there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_source_is_replaced_whole_while_readers_read() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = file_store(&dir).await;
    let mut build = small();
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    store
        .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
        .await
        .unwrap();

    const N: usize = 100_000;
    build.nodes = (0..N)
        .map(|i| symbol("src/big.rs", &format!("f{i:06}"), i as u32 + 1, "fn f()"))
        .collect();
    build.edges = (1..N)
        .map(|i| {
            edge(
                &id::symbol(REPO, "src/big.rs", &format!("f{:06}", i - 1)),
                kind::edge::REFERENCES,
                &id::symbol(REPO, "src/big.rs", &format!("f{i:06}")),
            )
        })
        .collect();
    let reader = {
        let store = store.clone();
        let source = build.source.clone();
        tokio::spawn(async move {
            let mut seen = std::collections::BTreeSet::new();
            loop {
                let row = store.source(&source).await.unwrap().unwrap();
                seen.insert(row.nodes);
                if row.nodes == N as u64 {
                    return seen;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        })
    };
    let started = std::time::Instant::now();
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    let r = store
        .replace_source(Some(g), build, at("2026-11-26T17:15:05Z"))
        .await
        .unwrap();
    assert!(matches!(
        r,
        Replaced::Written {
            nodes: 100_000,
            edges: 99_999,
            ..
        }
    ));
    let seen = tokio::time::timeout(std::time::Duration::from_secs(30), reader)
        .await
        .expect("the reader saw the new source")
        .unwrap();
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        [4, N as u64],
        "a reader sees the old source or the new one, never half"
    );
    println!(
        "replaced {N} nodes and {} edges in {:?}",
        N - 1,
        started.elapsed()
    );
}
