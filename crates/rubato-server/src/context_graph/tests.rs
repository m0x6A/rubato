//! The slice's skeleton (#325): the flag, the graph's own file, and each source's status.

use super::{overview, start, store};
use crate::kernel::config::{Config, ContextGraphConfig};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use chrono::{DateTime, Utc};
use rubato_graph::Build;
use rubato_protocol::graph::{GraphEdge, GraphNode, id, kind, source_key};
use std::sync::Arc;
use std::time::{Duration, Instant};

const REPO: &str = "m0x6A/rubato";

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

async fn app(tmp: &tempfile::TempDir, graph: bool) -> App {
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let mut cfg = Config::for_tests(url, "m0x6A");
    if graph {
        cfg.context_graph = Some(ContextGraphConfig {
            regen_secs: 900,
            database_url: format!("sqlite://{}/t-graph.db?mode=rwc", tmp.path().display()),
        });
    }
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    AppState::new(pool, cfg, Arc::new(FakeGitHub::new()))
}

fn symbols(n: usize) -> Build {
    let node = |i: usize| GraphNode {
        id: id::symbol(REPO, "src/big.rs", &format!("f{i:06}")),
        kind: kind::node::SYMBOL.into(),
        name: format!("f{i:06}"),
        repo: Some(REPO.into()),
        path: Some("src/big.rs".into()),
        line: Some(i as u32 + 1),
        end_line: None,
        summary: Some("fn f()".into()),
        detail: Default::default(),
    };
    Build {
        source: source_key::repo(REPO),
        kind: kind::source::REPO.into(),
        head_sha: Some("b35788b".into()),
        map_version: None,
        generator: 1,
        generated_at: at("2026-11-26T17:00:04Z"),
        truncated: false,
        nodes: (0..n).map(node).collect(),
        edges: (1..n)
            .map(|i| GraphEdge {
                from: id::symbol(REPO, "src/big.rs", &format!("f{:06}", i - 1)),
                kind: kind::edge::REFERENCES.into(),
                to: id::symbol(REPO, "src/big.rs", &format!("f{i:06}")),
                detail: Default::default(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn with_the_flag_off_there_is_no_file_and_no_graph_in_the_overview() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(&tmp, false).await;
    start(&app).await.unwrap();
    assert!(store(&app).is_none());
    assert_eq!(
        overview(&app, &crate::kernel::tenancy::TenantId::default_tenant()).await,
        None
    );
    assert!(!tmp.path().join("t-graph.db").exists());
}

#[tokio::test]
async fn with_the_flag_on_the_overview_says_how_each_source_stands() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(&tmp, true).await;
    // Before the file is open, and before any build: the interval and no sources.
    let before = overview(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .expect("the flag is on");
    assert_eq!((before.regen_secs, before.sources.len()), (900, 0));

    start(&app).await.unwrap();
    assert!(tmp.path().join("t-graph.db").exists());
    let store = store(&app).expect("opened at start");
    let build = symbols(3);
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    store
        .replace_source(Some(g), build.clone(), at("2026-11-26T17:00:05Z"))
        .await
        .unwrap();
    store
        .begin_build("team/default", kind::source::TEAM)
        .await
        .unwrap();
    store
        .record_error(
            "team/default",
            at("2026-11-26T17:00:06Z"),
            "plans: not read",
        )
        .await
        .unwrap();

    let o = overview(&app, &crate::kernel::tenancy::TenantId::default_tenant())
        .await
        .unwrap();
    let json = serde_json::to_value(&o).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "regen_secs": 900,
            "sources": [
                { "source": "repo/m0x6A/rubato", "kind": "repo", "head_sha": "b35788b",
                  "generated_at": "2026-11-26T17:00:04Z", "checked_at": "2026-11-26T17:00:05Z",
                  "nodes": 3, "edges": 2 },
                // Never built, and its only attempt failed: no times, the error, no rows.
                { "source": "team/default", "kind": "team", "error": "plans: not read",
                  "nodes": 0, "edges": 0 }
            ]
        })
    );
}

/// The reason the graph has its own file: a hundred-thousand-row replace must not make a web
/// write or a hook insert wait. Each write to the main database here has to finish within a
/// second while the replace runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_replace_does_not_hold_the_main_databases_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let app = app(&tmp, true).await;
    start(&app).await.unwrap();
    let store = store(&app).unwrap();
    sqlx::query("CREATE TABLE probe (n INTEGER NOT NULL)")
        .execute(&app.db)
        .await
        .unwrap();

    let build = symbols(100_000);
    let g = store.begin_build(&build.source, &build.kind).await.unwrap();
    let replace = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .replace_source(Some(g), build, at("2026-11-26T17:00:05Z"))
                .await
        }
    });
    let (mut writes, mut slowest) = (0u32, Duration::ZERO);
    while !replace.is_finished() {
        let started = Instant::now();
        let mut tx = app.db.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("INSERT INTO probe (n) VALUES (?)")
            .bind(writes)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        slowest = slowest.max(started.elapsed());
        writes += 1;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(matches!(
        replace.await.unwrap().unwrap(),
        rubato_graph::Replaced::Written { nodes: 100_000, .. }
    ));
    assert!(writes > 0, "no write overlapped the replace");
    assert!(
        slowest < Duration::from_secs(1),
        "a main-database write took {slowest:?} during the replace"
    );
    println!("{writes} main-database writes during the replace, the slowest {slowest:?}");
}
