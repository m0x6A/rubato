//! Plan 189 PR 1: the seed runs once, in one transaction, copies the environment into every
//! workspace, refuses a header it cannot seal, and later starts warn while the inputs are set.

use super::start;
use crate::kernel::config::{Config, SeedEnv, TokenKey, TrafficEnv};
use crate::kernel::github::FakeGitHub;
use crate::kernel::state::{App, AppState};
use crate::kernel::telemetry::capture;
use crate::kernel::workspaces::{Source, WorkspaceEntry};
use std::path::PathBuf;
use std::sync::Arc;

const HEADER: &str = "Authorization=Bearer s3cret-token";

fn entry(id: &str, path: &str, map_ref: &str) -> WorkspaceEntry {
    WorkspaceEntry {
        id: id.into(),
        name: Some(format!("{id} app")),
        map_repo: "m0x6A/rubato".into(),
        map_path: path.into(),
        map_ref: map_ref.into(),
        map_file: (id == "shop").then(|| PathBuf::from("/srv/maps/shop.yml")),
        settings: Default::default(),
    }
}

/// A server on the database at `url`, as the environment would configure it: a workspaces file
/// with `default` and `shop`, a Prometheus source with a header, a spend cap.
fn config(url: &str, shop_ref: &str, cap: f64, key: bool) -> Config {
    let mut cfg = Config::for_tests(url.into(), "m0x6A");
    cfg.workspaces = vec![
        entry("default", "docs/design/rubato-map.yml", "main"),
        entry("shop", "docs/design/shop-map.yml", shop_ref),
    ];
    cfg.traffic_env = TrafficEnv::from_vars(
        [
            ("RUBATO_TRAFFIC_PROMETHEUS_URL", "http://prom.lan:9090"),
            ("RUBATO_TRAFFIC_PROMETHEUS_HEADERS", HEADER),
            ("RUBATO_TRAFFIC_WINDOW_SECS", "1800"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned())),
    );
    cfg.seed = SeedEnv {
        default_spend_cap_usd: Some(cap),
        ..Default::default()
    };
    cfg.token_key = key.then_some(TokenKey([9; 32]));
    cfg
}

async fn server(cfg: Config) -> App {
    let pool = crate::kernel::db::connect(&cfg.database_url).await.unwrap();
    AppState::new(pool, cfg, Arc::new(FakeGitHub::new()))
}

async fn rows(app: &App) -> Vec<(String, i64, String, String, Option<f64>)> {
    sqlx::query_as(
        "SELECT workspace_id, revision, map_ref, COALESCE(traffic_secrets_sealed, ''), default_spend_cap_usd FROM workspace_settings ORDER BY position",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn seed_from_file_and_env_once() {
    let (events, _guard) = capture::start();
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());

    // First start: both workspaces, each with the traffic source and the cap (Decision 7).
    let app = server(config(&url, "main", 5.0, true)).await;
    let started = start(&app).await.unwrap();
    assert_eq!(started.seeded, 2);
    let (entries, source) = app.workspace_entries();
    assert_eq!(source, Source::Database);
    let ids: Vec<&str> = entries.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["default", "shop"], "file order");
    for w in &entries {
        assert_eq!(w.settings.revision, 1);
        let t = w.settings.traffic.as_ref().expect("traffic copied");
        assert_eq!(t.source, "prometheus");
        assert_eq!(t.fields["url"], "http://prom.lan:9090");
        assert_eq!(t.fields["window_secs"], "1800");
        assert!(!t.fields.contains_key("headers"), "a secret is not a field");
        assert_eq!(w.settings.default_spend_cap_usd, Some(5.0));
    }
    assert_eq!(
        entries[1].map_file.as_deref(),
        Some(std::path::Path::new("/srv/maps/shop.yml")),
        "map_file comes from the file, never the table"
    );
    let first = rows(&app).await;
    for (_, _, _, sealed, _) in &first {
        assert!(!sealed.is_empty() && !sealed.contains("s3cret"), "sealed");
    }
    let opened = crate::kernel::secrets::open(&TokenKey([9; 32]), &first[0].3).unwrap();
    assert!(opened.contains(HEADER));
    assert!(
        !format!("{entries:?}").contains("s3cret") && !format!("{entries:?}").contains("prom.lan"),
        "Debug prints no value"
    );
    // The first start's own inputs are still set, and equal: warned, not differing.
    assert!(
        started.drift.iter().all(|d| !d.differs),
        "{:?}",
        started.drift
    );
    crate::kernel::workspaces::report(&app).await.unwrap();
    let logged = |msg: &str, field: &str, value: &str| {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.message == msg && e.fields.get(field).map(String::as_str) == Some(value))
    };
    assert!(logged("workspace config", "source", "database"), "N9");
    drop(app);

    // A later start with a changed file and cap: nothing written, one warning per input.
    let app = server(config(&url, "release", 7.0, true)).await;
    let again = start(&app).await.unwrap();
    assert_eq!(again.seeded, 0);
    assert_eq!(rows(&app).await, first, "the database owns the settings");
    assert_eq!(app.workspace_entries().0[1].map_ref, "main");
    let drift: Vec<(&str, bool)> = again
        .drift
        .iter()
        .map(|d| (d.setting.as_str(), d.differs))
        .collect();
    assert_eq!(
        drift,
        [
            ("RUBATO_WORKSPACES_FILE entry default", false),
            ("RUBATO_WORKSPACES_FILE entry shop", true),
            ("RUBATO_TRAFFIC_PROMETHEUS_HEADERS", false),
            ("RUBATO_TRAFFIC_PROMETHEUS_URL", false),
            ("RUBATO_TRAFFIC_WINDOW_SECS", false),
            ("RUBATO_DEFAULT_SPEND_CAP_USD", true),
        ]
    );
    assert!(logged(
        "workspace setting still set outside the database; the database holds it (ADR 0075)",
        "setting",
        "RUBATO_WORKSPACES_FILE entry shop"
    ));
    println!(
        "checked {} rows, {} warnings, and the start-up source",
        first.len(),
        drift.len()
    );
}

/// Each workspace has its own traffic source (issue #281): when `shop`'s has moved since the
/// seed, the variables differ, although `default`'s row still equals them. Fails on purpose when
/// only `default`'s row is compared.
#[tokio::test(flavor = "current_thread")]
async fn drift_compares_every_workspaces_traffic_source() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let app = server(config(&url, "main", 5.0, true)).await;
    assert_eq!(start(&app).await.unwrap().seeded, 2);
    sqlx::query(
        "UPDATE workspace_settings SET traffic_fields = ?, default_spend_cap_usd = 9 WHERE workspace_id = 'shop'",
    )
    .bind(r#"{"url":"http://mimir.shop.lan:9009","window_secs":"1800"}"#)
    .execute(&app.db)
    .await
    .unwrap();
    drop(app);

    let app = server(config(&url, "main", 5.0, true)).await;
    let again = start(&app).await.unwrap();
    let drift: Vec<(&str, bool)> = again
        .drift
        .iter()
        .map(|d| (d.setting.as_str(), d.differs))
        .collect();
    assert_eq!(
        drift,
        [
            ("RUBATO_WORKSPACES_FILE entry default", false),
            ("RUBATO_WORKSPACES_FILE entry shop", false),
            ("RUBATO_TRAFFIC_PROMETHEUS_HEADERS", false),
            ("RUBATO_TRAFFIC_PROMETHEUS_URL", true),
            ("RUBATO_TRAFFIC_WINDOW_SECS", false),
            ("RUBATO_DEFAULT_SPEND_CAP_USD", true),
        ]
    );
    println!(
        "checked {} settings against 2 workspaces' rows",
        drift.len()
    );
}

/// Must fail on purpose: a header with no key to seal it refuses the start, names both
/// variables, and leaves the table empty (one transaction).
#[tokio::test(flavor = "current_thread")]
async fn a_header_without_a_token_key_refuses_the_start() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let app = server(config(&url, "main", 5.0, false)).await;
    let e = start(&app).await.unwrap_err().to_string();
    assert!(e.contains("RUBATO_TRAFFIC_PROMETHEUS_HEADERS"), "{e}");
    assert!(e.contains("RUBATO_TOKEN_KEY"), "{e}");
    assert!(rows(&app).await.is_empty(), "nothing written");
    assert!(!e.contains("s3cret"), "the value is never in the error");
}

/// A deploy from the legacy variables: one `default` row with no traffic and no caps.
#[tokio::test(flavor = "current_thread")]
async fn a_legacy_deploy_seeds_default_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let app = server(Config::for_tests(url, "m0x6A")).await;
    let started = start(&app).await.unwrap();
    assert_eq!(started.seeded, 1);
    let (entries, _) = app.workspace_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "default");
    assert_eq!(entries[0].settings.traffic, None);
    assert_eq!(entries[0].settings.default_spend_cap_usd, None);
    assert!(started.drift.is_empty(), "nothing set: {:?}", started.drift);
}
