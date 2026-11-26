//! Repository sources on a timer (#326): what a tick asks GitHub for, what it stores, what it
//! keeps when a build fails, and what it never removes.

use super::build::{read_tarball, run, tick};
use super::extract::tests::fixture_files;
use super::metrics::Observed;
use super::{overview, start, store};
use crate::kernel::config::{Config, ContextGraphConfig};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState, LoadedMap};
use chrono::{DateTime, Utc};
use rubato_protocol::SystemMap;
use rubato_protocol::graph::{id, source_key};
use std::sync::Arc;
use std::time::Duration;

const REPO: &str = "m0x6A/rubato";

/// A tarball as GitHub serves it: gzip, a pax header, one top-level directory.
fn tarball(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut b = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut global = tar::Header::new_ustar();
    global.set_entry_type(tar::EntryType::XGlobalHeader);
    global.set_size(0);
    global.set_cksum();
    b.append_data(&mut global, "pax_global_header", &[][..])
        .unwrap();
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_size(0);
    dir.set_mode(0o755);
    dir.set_cksum();
    b.append_data(&mut dir, "m0x6A-rubato-abc1234/", &[][..])
        .unwrap();
    for (path, bytes) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, format!("m0x6A-rubato-abc1234/{path}"), &bytes[..])
            .unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

fn map_naming(repos: &[&str]) -> SystemMap {
    let nodes: String = repos
        .iter()
        .enumerate()
        .map(|(i, r)| format!("  - {{ id: part-{i}, kind: context, repo: {r}, path: src }}\n"))
        .collect();
    serde_yaml_ng::from_str(&format!("version: 1\nname: t\nnodes:\n{nodes}")).unwrap()
}

async fn set_map(app: &App, repos: &[&str]) {
    app.set_map(
        rubato_protocol::DEFAULT_WORKSPACE,
        LoadedMap {
            map: map_naming(repos),
            version: "v".into(),
            loaded_at: Utc::now(),
            yaml: String::new(),
        },
    )
    .await;
}

/// A server with the flag on, a map naming `REPO`, and a GitHub whose `REPO` is at `sha1`.
async fn serve() -> (App, Arc<FakeGitHub>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_tests(
        format!("sqlite://{}/t.db?mode=rwc", tmp.path().display()),
        "m0x6A",
    );
    cfg.context_graph = Some(ContextGraphConfig {
        regen_secs: 900,
        database_url: format!("sqlite://{}/t-graph.db?mode=rwc", tmp.path().display()),
    });
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    let github = Arc::new(FakeGitHub::new());
    github.with(|g| {
        g.heads.insert(REPO.into(), "sha1".into());
        g.tarballs
            .insert((REPO.into(), "sha1".into()), tarball(&fixture_files()));
    });
    let app = AppState::new(pool, cfg, github.clone());
    start(&app).await.unwrap();
    set_map(&app, &[REPO]).await;
    (app, github, tmp)
}

fn calls(github: &FakeGitHub, name: &str) -> usize {
    github.calls().iter().filter(|(n, _)| *n == name).count()
}

#[tokio::test]
async fn a_tick_builds_a_repository_once_and_then_only_asks_for_its_head() {
    let (app, github, _tmp) = serve().await;
    let key = source_key::repo(REPO);

    let first = tick(&app).await;
    assert_eq!(first.built, std::slice::from_ref(&key));
    assert_eq!(
        (calls(&github, "default_head"), calls(&github, "tarball")),
        (1, 1)
    );
    let store = store(&app).unwrap();
    let row = store.source(&key).await.unwrap().unwrap();
    assert_eq!(row.head_sha.as_deref(), Some("sha1"));
    assert!(row.nodes >= 16, "{} nodes", row.nodes);
    assert!(store.node(&id::adr(REPO, "0071")).await.unwrap().is_some());
    let built_at = row.generated_at;

    // Nothing moved: one call, no tarball, no new rows.
    let second = tick(&app).await;
    assert_eq!(
        (second.built.len(), second.unchanged),
        (0, vec![key.clone()])
    );
    assert_eq!(
        (calls(&github, "default_head"), calls(&github, "tarball")),
        (2, 1)
    );
    let row = store.source(&key).await.unwrap().unwrap();
    assert_eq!(row.generated_at, built_at);

    // The head moved: the tarball at the new head, and the new file is there.
    let mut files = fixture_files();
    files.push((
        "docs/decisions/0084-a-context-graph.md",
        b"# 0084. A context graph\n\n- **Status:** proposed\n".to_vec(),
    ));
    github.with(|g| {
        g.heads.insert(REPO.into(), "sha2".into());
        g.tarballs
            .insert((REPO.into(), "sha2".into()), tarball(&files));
    });
    let third = tick(&app).await;
    assert_eq!(third.built, std::slice::from_ref(&key));
    assert_eq!(
        (calls(&github, "default_head"), calls(&github, "tarball")),
        (3, 2)
    );
    assert_eq!(
        store
            .source(&key)
            .await
            .unwrap()
            .unwrap()
            .head_sha
            .as_deref(),
        Some("sha2")
    );
    assert!(store.node(&id::adr(REPO, "0084")).await.unwrap().is_some());
    println!(
        "three ticks: {} head calls, {} tarballs",
        calls(&github, "default_head"),
        calls(&github, "tarball")
    );
}

#[tokio::test]
async fn a_failed_build_keeps_the_last_good_rows_and_says_why() {
    let (app, github, _tmp) = serve().await;
    let key = source_key::repo(REPO);
    tick(&app).await;
    let store = store(&app).unwrap();
    let good = store.source(&key).await.unwrap().unwrap();

    // A new head whose tarball cannot be had.
    github.with(|g| {
        g.heads.insert(REPO.into(), "sha2".into());
        g.tarball_error = Some("502 from the archive host".into());
    });
    let failed = tick(&app).await;
    assert_eq!(failed.failed, [(key.clone(), "tarball")]);
    let row = store.source(&key).await.unwrap().unwrap();
    assert_eq!(
        row.head_sha, good.head_sha,
        "still the graph of the old head"
    );
    assert_eq!(
        (row.nodes, row.content_hash.clone()),
        (good.nodes, good.content_hash.clone())
    );
    assert_eq!(
        row.checked_at, good.checked_at,
        "not current, so its age keeps rising"
    );
    assert!(
        row.error.as_deref().unwrap().contains("502"),
        "{:?}",
        row.error
    );
    let shown = overview(&app).await.unwrap();
    assert!(shown.sources[0].error.is_some());

    // GitHub refusing the head call is a failure of its own kind.
    github.with(|g| g.exhausted = true);
    assert_eq!(tick(&app).await.failed, [(key.clone(), "head")]);

    // It heals: built at the new head, the error gone.
    github.with(|g| {
        g.exhausted = false;
        g.tarball_error = None;
        g.tarballs
            .insert((REPO.into(), "sha2".into()), tarball(&fixture_files()));
    });
    tick(&app).await;
    let row = store.source(&key).await.unwrap().unwrap();
    assert_eq!((row.head_sha.as_deref(), row.error), (Some("sha2"), None));
}

#[tokio::test]
async fn a_tarball_over_the_cap_is_a_cut_source_not_a_failure() {
    let (app, github, _tmp) = serve().await;
    github.with(|g| g.tarball_too_large = true);
    let t = tick(&app).await;
    assert!(t.failed.is_empty(), "{:?}", t.failed);
    let row = store(&app)
        .unwrap()
        .source(&source_key::repo(REPO))
        .await
        .unwrap()
        .unwrap();
    assert!(row.truncated);
    assert_eq!((row.nodes, row.error), (0, None));
    assert_eq!(row.head_sha.as_deref(), Some("sha1"));
}

#[tokio::test]
async fn repositories_build_one_after_the_other_and_an_uncovered_one_is_not_asked_for() {
    let (app, github, _tmp) = serve().await;
    set_map(&app, &[REPO, "m0x6A/other", "m0x6A/private"]).await;
    github.with(|g| {
        g.heads.insert("m0x6A/other".into(), "o1".into());
        g.tarballs.insert(
            ("m0x6A/other".into(), "o1".into()),
            tarball(&fixture_files()),
        );
    });
    sqlx::query("INSERT INTO uncovered_repos (repo, seen_at) VALUES ('m0x6A/private', '2026-11-26T17:00:00Z')")
        .execute(&app.db)
        .await
        .unwrap();
    let t = tick(&app).await;
    assert_eq!(t.built.len(), 2);
    let order: Vec<(&str, String)> = github
        .calls()
        .into_iter()
        .filter(|(n, _)| matches!(*n, "default_head" | "tarball"))
        .collect();
    // One repository at a time: each head call is followed by that repository's tarball before
    // the next repository is asked for. Which comes first is the map's order.
    let mut built = Vec::new();
    for pair in order.chunks(2) {
        let [(head, repo), (tarball, at)] = pair else {
            panic!("a call without its pair: {order:?}");
        };
        assert_eq!((*head, *tarball), ("default_head", "tarball"), "{order:?}");
        assert!(at.starts_with(&format!("{repo} ")), "{order:?}");
        built.push(repo.clone());
    }
    built.sort();
    assert_eq!(built, ["m0x6A/other", REPO], "and never the uncovered one");
}

#[tokio::test]
async fn a_source_is_removed_only_when_a_loaded_map_no_longer_names_its_repository() {
    let (app, _github, _tmp) = serve().await;
    let key = source_key::repo(REPO);
    tick(&app).await;
    let store = store(&app).unwrap();

    // No map loaded (a failed load at start): nothing is removed, or one bad tick would empty
    // every companion's copy.
    app.maps.write().await.clear();
    let t = tick(&app).await;
    assert!(t.removed.is_empty() && t.built.is_empty());
    assert!(store.source(&key).await.unwrap().is_some());

    // A map that names another repository: the old source goes, with its rows.
    set_map(&app, &["m0x6A/other"]).await;
    let t = tick(&app).await;
    assert_eq!(t.removed, std::slice::from_ref(&key));
    assert!(store.source(&key).await.unwrap().is_none());
    assert!(store.node(&id::adr(REPO, "0071")).await.unwrap().is_none());
}

#[tokio::test]
async fn the_timer_ticks_at_start_and_then_once_an_interval() {
    let (app, github, _tmp) = serve().await;
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let every = Duration::from_millis(400);
    let task = tokio::spawn(run(app.clone(), every, shutdown));

    // At start: one tick, the build. Nothing more before the interval is over.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        (calls(&github, "default_head"), calls(&github, "tarball")),
        (1, 1)
    );
    // After one interval: the head is asked for again, and nothing is downloaded.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        (calls(&github, "default_head"), calls(&github, "tarball")),
        (2, 1)
    );
    // The head moves: the next tick rebuilds.
    github.with(|g| {
        g.heads.insert(REPO.into(), "sha2".into());
        g.tarballs
            .insert((REPO.into(), "sha2".into()), tarball(&fixture_files()));
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        (calls(&github, "default_head"), calls(&github, "tarball")),
        (3, 2)
    );

    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("the timer ends at shutdown")
        .unwrap();
}

#[test]
fn a_tarball_is_read_file_by_file_without_its_top_directory() {
    let mut files = fixture_files();
    // Larger than the parse cap: a node, never read.
    files.push(("data/big.json", vec![b'x'; 600 * 1024]));
    let got = read_tarball(&tarball(&files), REPO).unwrap();
    assert_eq!(
        got.counts.files, 10,
        "the directory and the pax header are not files"
    );
    assert_eq!(got.truncated, None);
    let big = got
        .nodes
        .iter()
        .find(|n| n.id == id::file(REPO, "data/big.json"))
        .expect("the path has no top-level directory");
    assert_eq!(big.detail["bytes"], 600 * 1024);
    assert!(big.detail.get("lines").is_none());
    assert!(read_tarball(b"not gzip at all", REPO).is_err());
}

#[test]
fn a_sources_age_rises_from_its_last_current_check_also_when_it_was_never_built() {
    let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
    let observed = Observed {
        regen_secs: 900,
        sources: [
            (
                "repo/m0x6A/rubato".to_owned(),
                (Some(at("2026-11-26T17:00:00Z")), 10, 3),
            ),
            ("team".to_owned(), (None, 0, 0)),
        ]
        .into(),
    };
    let first_seen = at("2026-11-26T16:00:00Z");
    assert_eq!(
        observed.ages(first_seen, at("2026-11-26T17:45:00Z")),
        [
            ("repo/m0x6A/rubato".to_owned(), 2700),
            ("team".to_owned(), 6300)
        ]
    );
    // No tick since: the same stored time, a larger age. That is what the alert reads.
    assert_eq!(
        observed.ages(first_seen, at("2026-11-26T18:00:00Z"))[0].1,
        3600
    );
}
