//! Plan 89 PR 3, the golden test: a single-map server answers as it did before workspaces.
//!
//! A database is built at migration 0012 and seeded with the rows most likely to drift: a pinned
//! issue in a repo no map names whose approved plan does not fit (it must stay *does not fit*), a
//! launch against an older map version (`MapVersionMismatch`), a saved position, and a draft with
//! two operations and a proposal. The server then starts on it, runs every later migration, and its
//! overview, pins and draft must equal `fixtures/golden/overview.json`, recorded before the change.
//! Run with `UPDATE_GOLDEN=1` to record it again.

use rubato_protocol::progress::routes as overview_routes;
use rubato_server::kernel::config::Config;
use rubato_server::kernel::db;
use rubato_server::kernel::github::FakeGitHub;
use rubato_server::kernel::state::AppState;
use rubato_server::{app, map};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

const T: &str = "2026-11-14T08:00:00+00:00";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every key that holds the time of a run, not of the seed, becomes `"<time>"`.
fn mask(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if matches!(
                    k.as_str(),
                    "generated_at" | "loaded_at" | "last_seen" | "state_since"
                ) {
                    *x = json!("<time>");
                } else {
                    mask(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(mask),
        _ => {}
    }
}

async fn seed_at_0012(url: &str) -> usize {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(
            url.parse::<sqlx::sqlite::SqliteConnectOptions>()
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let mut m = sqlx::migrate!("./migrations");
    m.migrations = Cow::Owned(
        m.migrations
            .iter()
            .filter(|x| x.version <= 12)
            .cloned()
            .collect(),
    );
    m.run(&pool).await.unwrap();
    let plan = json!({
        "issue": { "repo": "acme/loose", "number": 5 },
        "path": "docs/plans/5.md",
        "affected_parts": [{ "node": "billing", "status": "affected", "change": "x" }],
        "o_rows": []
    });
    let approval = json!({
        "issue": { "repo": "acme/loose", "number": 5 }, "comment_id": 1, "author": "m0x6A",
        "sha": "5555555555555555555555555555555555555555", "created_at": T, "url": "https://github.com/acme/loose/issues/5#issuecomment-1"
    });
    let launch = json!({
        "id": "00000000-0000-0000-0000-000000000001", "user": "m0x6A",
        "issue": { "repo": "m0x6A/rubato", "number": 10 }, "repo_url": "https://github.com/m0x6A/rubato.git",
        "branch": "issue/10-x", "base_ref": "origin/main", "node_ids": ["rubato-server"],
        "spend_cap_usd": 5.0, "attempt_limit": 3, "attempt": 1,
        "session_id": "00000000-0000-0000-0000-000000000002", "prompt": "p", "map_version": "oldsha"
    });
    let add = json!({ "op": "add_node", "node": { "id": "billing", "kind": "context" } });
    let seam = json!({ "op": "add_seam", "seam": { "id": "billing-server", "from": "billing", "to": "rubato-server", "owner": "rubato-server" } });
    let rows: Vec<(&str, Vec<String>)> = vec![
        (
            "INSERT INTO pins VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                "acme/loose".into(),
                "5".into(),
                "node".into(),
                "rubato-server".into(),
                "m0x6A".into(),
                T.into(),
            ],
        ),
        (
            "INSERT INTO pins VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                "m0x6A/rubato".into(),
                "10".into(),
                "seam".into(),
                "web-server".into(),
                "m0x6A".into(),
                T.into(),
            ],
        ),
        (
            "INSERT INTO plans (repo, number, branch, path, plan, parse_error, fetched_at, approval_sha) VALUES (?, ?, ?, ?, ?, NULL, ?, ?)",
            vec![
                "acme/loose".into(),
                "5".into(),
                "issue/5-x".into(),
                "docs/plans/5.md".into(),
                plan.to_string(),
                T.into(),
                "5555555555555555555555555555555555555555".into(),
            ],
        ),
        (
            "INSERT INTO plan_signals VALUES (?, ?, ?, ?, ?, NULL)",
            vec![
                "acme/loose".into(),
                "5".into(),
                "1".into(),
                "approval".into(),
                approval.to_string(),
            ],
        ),
        (
            "INSERT INTO launches VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                "00000000-0000-0000-0000-000000000001".into(),
                "00000000-0000-0000-0000-0000000000aa".into(),
                "m0x6A".into(),
                "m0x6A/rubato".into(),
                "10".into(),
                launch.to_string(),
                r#"{"outcome":"started"}"#.into(),
                T.into(),
                T.into(),
            ],
        ),
        (
            "INSERT INTO map_layout VALUES (?, ?, ?, ?, ?)",
            vec![
                "rubato-server".into(),
                "100".into(),
                "200".into(),
                "m0x6A".into(),
                T.into(),
            ],
        ),
        (
            "INSERT INTO map_draft_ops (op_id, op, author, created_at) VALUES (?, ?, ?, ?)",
            vec!["op-1".into(), add.to_string(), "m0x6A".into(), T.into()],
        ),
        (
            "INSERT INTO map_draft_ops (op_id, op, author, created_at) VALUES (?, ?, ?, ?)",
            vec!["op-2".into(), seam.to_string(), "m0x6A".into(), T.into()],
        ),
        (
            "INSERT INTO map_proposal VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                "map/draft-20261114-080000".into(),
                "mapsha1".into(),
                "hash".into(),
                "1".into(),
                "proposed".into(),
                T.into(),
                "77".into(),
                "https://github.com/m0x6A/rubato/pull/77".into(),
                "m0x6A".into(),
            ],
        ),
    ];
    for (sql, binds) in &rows {
        let mut q = sqlx::query(sql);
        for b in binds {
            q = q.bind(b);
        }
        q.execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    pool.close().await;
    rows.len()
}

#[tokio::test]
async fn a_single_map_server_answers_as_before_workspaces() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/golden.db?mode=rwc", tmp.path().display());
    let seeded = seed_at_0012(&url).await;
    let pool = db::connect(&url).await.unwrap();

    let github = Arc::new(FakeGitHub::new());
    let yaml = std::fs::read_to_string(root().join("docs/design/rubato-map.yml")).unwrap();
    github.with(|g| {
        g.files.insert(
            (
                "m0x6A/rubato".into(),
                "docs/design/rubato-map.yml".into(),
                "main".into(),
            ),
            yaml,
        );
        g.branches.insert(
            "m0x6A/rubato".into(),
            vec![("main".into(), "mapsha1".into())],
        );
    });
    let app = AppState::new(pool, Config::for_tests(url, "m0x6A"), github);
    map::reload(&app).await.unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let router = app::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mut got: Value = reqwest::Client::new()
        .get(format!("http://{addr}{}", overview_routes::OVERVIEW))
        .header("x-rubato-dev-user", "m0x6A")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    mask(&mut got);
    // The map is the repo's own file, which other changes edit: the golden holds its version only.
    got["map"]["map"] = json!("<docs/design/rubato-map.yml>");

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/overview.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
    }
    let want: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("record with UPDATE_GOLDEN=1"))
            .unwrap();
    println!("golden: {seeded} seeded rows, overview compared field by field");
    assert_eq!(got, want, "the overview of a single-map server changed");

    // The cases the seed is for.
    let loose = got["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["issue"]["number"] == 5)
        .expect("pinned issue in a repo no map names");
    assert_eq!(loose["approval"]["version"], "does_not_fit", "{loose:#}");
    assert!(
        got["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "map_version_mismatch")
    );
    assert_eq!(got["draft"]["ops"].as_array().map(Vec::len), Some(2));
}
