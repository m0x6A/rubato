//! Load the system map (ADR 0010): from GitHub, or a local file, falling back to the cached copy.

use super::store;
use crate::kernel::state::{App, LoadedMap};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::SystemMap;
use rubato_protocol::otel::attr;

/// Fetch, validate and cache the map. Falls back to the cached copy when GitHub fails.
pub async fn reload(app: &App) -> Result<LoadedMap> {
    let cfg = &app.config;
    if let Some(file) = &cfg.map_file {
        return load_from_file(app, file).await;
    }
    let fetched = async {
        let sha = app
            .github
            .branch_sha(&cfg.map_repo, &cfg.map_ref)
            .await?
            .context("map ref not found")?;
        let yaml = app
            .github
            .file(&cfg.map_repo, &cfg.map_path, &cfg.map_ref)
            .await?
            .context("map file not found")?;
        anyhow::Ok((sha, yaml))
    }
    .await;
    // `fallback` is set when the map comes from the database copy: that copy keeps the time it
    // was fetched, and this reload has already counted its one failure (the fetch), whose error
    // is kept so a broken copy does not hide it.
    let (version, yaml, fallback) = match fetched {
        Ok((version, yaml)) => (version, yaml, None),
        Err(e) => {
            app.metrics.map_load_failures.add(1, &[]);
            if let Some(m) = app.map.read().await.clone() {
                tracing::warn!(error = %e, "map fetch failed; keeping cached version {}", m.version);
                return Ok(m);
            }
            if let Some((version, yaml, loaded_at)) = store::load_map(&app.db).await? {
                tracing::warn!(error = %e, %loaded_at, "map fetch failed; using the database copy {version}");
                (version, yaml, Some((loaded_at, e)))
            } else {
                return Err(e);
            }
        }
    };
    let map = match &fallback {
        None => parse(app, &yaml, true)?,
        Some((_, fetch_error)) => parse(app, &yaml, false).with_context(|| {
            format!("map fetch failed: {fetch_error:#}; the database copy {version} is unusable")
        })?,
    };
    let stored_at = fallback.map(|(at, _)| at);
    let loaded = LoadedMap {
        map,
        version: version.clone(),
        loaded_at: stored_at.unwrap_or_else(Utc::now),
        yaml: yaml.clone(),
    };
    if stored_at.is_none() {
        store::store_map(&app.db, &version, &yaml, loaded.loaded_at).await?;
    }
    *app.map.write().await = Some(loaded.clone());
    app.metrics
        .map_loaded
        .record(1, &[KeyValue::new(attr::MAP_VERSION, version.clone())]);
    record_loaded_at(app, loaded.loaded_at);
    tracing::info!(version = %version, nodes = loaded.map.nodes.len(), seams = loaded.map.seams.len(), "map loaded");
    Ok(loaded)
}

/// Local-file mode (`RUBATO_MAP_FILE`): the version is a content hash, since there may be no git.
async fn load_from_file(app: &App, file: &std::path::Path) -> Result<LoadedMap> {
    let yaml = tokio::fs::read_to_string(file)
        .await
        .with_context(|| format!("reading {}", file.display()))
        .inspect_err(|_| app.metrics.map_load_failures.add(1, &[]))?;
    let map = parse(app, &yaml, true)?;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(yaml.as_bytes());
    let version = format!(
        "file-{}",
        digest
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let loaded = LoadedMap {
        map,
        version: version.clone(),
        loaded_at: Utc::now(),
        yaml: yaml.clone(),
    };
    store::store_map(&app.db, &version, &yaml, loaded.loaded_at).await?;
    *app.map.write().await = Some(loaded.clone());
    app.metrics
        .map_loaded
        .record(1, &[KeyValue::new(attr::MAP_VERSION, version.clone())]);
    record_loaded_at(app, loaded.loaded_at);
    tracing::info!(version = %version, file = %file.display(), "map loaded from file");
    Ok(loaded)
}

/// Parse and validate a map. `count` is false when the reload has already counted its failure,
/// so one reload adds at most one to `rubato.map.load_failures`.
fn parse(app: &App, yaml: &str, count: bool) -> Result<SystemMap> {
    let failed = || {
        if count {
            app.metrics.map_load_failures.add(1, &[]);
        }
    };
    let map: SystemMap = serde_yaml_ng::from_str(yaml)
        .context("system-map.yml does not parse")
        .inspect_err(|_| failed())?;
    if let Err(errors) = map.validate() {
        failed();
        anyhow::bail!(
            "system map invalid: {}",
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    Ok(map)
}

/// Load the map from the database at startup so the server answers before the first fetch.
pub async fn warm(app: &App) -> Result<()> {
    if let Some((version, yaml, loaded_at)) = store::load_map(&app.db).await? {
        let map: SystemMap = serde_yaml_ng::from_str(&yaml)?;
        *app.map.write().await = Some(LoadedMap {
            map,
            version,
            loaded_at,
            yaml,
        });
        // The stored time, so a restart during a GitHub outage shows the real age.
        record_loaded_at(app, loaded_at);
    }
    Ok(())
}

/// `rubato.map.loaded_at`: unix seconds of the load. A failed reload records nothing, so the
/// gauge keeps its value and "Map age" keeps growing.
fn record_loaded_at(app: &App, at: DateTime<Utc>) {
    app.metrics
        .map_loaded_at
        .record(u64::try_from(at.timestamp()).unwrap_or(0), &[]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::config::Config;
    use crate::kernel::db::connect;
    use crate::kernel::github::FakeGitHub;
    use crate::kernel::state::AppState;
    use crate::kernel::telemetry::metric_reader;
    use rubato_protocol::otel::metric;
    use std::sync::Arc;

    const MAP: &str = include_str!("../../../../docs/design/rubato-map.yml");

    /// An app whose instruments are read by the returned reader.
    async fn app(cfg: Config, github: Arc<FakeGitHub>) -> (App, metric_reader::Reader) {
        let pool = connect(&cfg.database_url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(pool, cfg, github);
        Arc::get_mut(&mut app).unwrap().metrics = metrics;
        (app, reader)
    }

    #[track_caller]
    fn only(reader: &metric_reader::Reader, name: &str) -> metric_reader::Point {
        let points = reader.points(name);
        assert_eq!(points.len(), 1, "{name}: {points:?}");
        points.into_iter().next().unwrap()
    }

    #[tokio::test]
    async fn invalid_flows_keep_the_previous_map() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("system-map.yml");
        let mut cfg = Config::for_tests(
            format!("sqlite://{}/t.db?mode=rwc", tmp.path().display()),
            "m0x6A",
        );
        cfg.map_file = Some(file.clone());
        let (a, reader) = app(cfg, Arc::new(FakeGitHub::new())).await;
        std::fs::write(
            &file,
            include_str!("../../../../schema/fixtures/maps/flows.yml"),
        )
        .unwrap();
        let good = reload(&a).await.unwrap();
        assert_eq!(good.map.seam("web-server").unwrap().flows.len(), 3);

        // The file gets a flow that leaves its seam: the load fails naming the seam, is
        // counted, and the map in use is still the previous one.
        std::fs::write(
            &file,
            include_str!("../../../../schema/fixtures/maps/flows-off-seam.yml"),
        )
        .unwrap();
        let e = reload(&a).await.unwrap_err();
        assert!(format!("{e:#}").contains("\"web-server\""), "{e:#}");
        assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 1);
        let kept = a.map.read().await.clone().unwrap();
        assert_eq!(kept.version, good.version);
        assert_eq!(kept.map.name, "flows-fixture");
    }

    #[tokio::test]
    async fn invalid_scenarios_keep_the_previous_map() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("system-map.yml");
        let mut cfg = Config::for_tests(
            format!("sqlite://{}/t.db?mode=rwc", tmp.path().display()),
            "m0x6A",
        );
        cfg.map_file = Some(file.clone());
        let (a, reader) = app(cfg, Arc::new(FakeGitHub::new())).await;
        std::fs::write(
            &file,
            include_str!("../../../../schema/fixtures/maps/scenarios.yml"),
        )
        .unwrap();
        let good = reload(&a).await.unwrap();
        assert_eq!(good.map.scenarios.len(), 2);

        // A scenario step with no declared flow, or one that matches two: the load fails naming
        // the scenario, is counted once, and the map in use is still the previous one.
        std::fs::write(
            &file,
            include_str!("../../../../schema/fixtures/maps/scenarios-bad.yml"),
        )
        .unwrap();
        let e = format!("{:#}", reload(&a).await.unwrap_err());
        assert!(e.contains("scenario \"off-flow\" step 1"), "{e}");
        assert!(e.contains("name one with carries"), "{e}");
        assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 1);
        let kept = a.map.read().await.clone().unwrap();
        assert_eq!(kept.map.name, "scenarios-fixture");
    }

    #[tokio::test]
    async fn loaded_at_follows_loads_and_survives_failures() {
        let tmp = tempfile::tempdir().unwrap();
        let db_url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let github = Arc::new(FakeGitHub::new());
        github.with(|g| {
            g.files.insert(
                (
                    "m0x6A/rubato".into(),
                    "docs/design/rubato-map.yml".into(),
                    "main".into(),
                ),
                MAP.into(),
            );
            g.branches
                .insert("m0x6A/rubato".into(), vec![("main".into(), "sha1".into())]);
        });
        let cfg = Config::for_tests(db_url.clone(), "m0x6A");

        // (a) A load records its time, in unix seconds, with no unit and no attributes.
        let (a, reader) = app(cfg.clone(), github.clone()).await;
        let loaded = reload(&a).await.unwrap();
        let at = u64::try_from(loaded.loaded_at.timestamp()).unwrap();
        let point = only(&reader, metric::MAP_LOADED_AT);
        assert_eq!(point.value, at);
        assert_eq!(point.unit, "");
        assert!(point.attrs.is_empty(), "{point:?}");
        assert!(reader.points(metric::MAP_LOAD_FAILURES).is_empty());

        // (b) The ref goes away: the cached map stays, the gauge keeps its value on the next
        // collection, and the failure is counted.
        github.with(|g| g.branches.clear());
        let kept = reload(&a).await.unwrap();
        assert_eq!(kept.loaded_at, loaded.loaded_at);
        assert_eq!(only(&reader, metric::MAP_LOADED_AT).value, at);
        assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 1);

        // (c) A fresh server on the same database reports the stored time after `warm`.
        let (b, reader) = app(cfg, github).await;
        assert!(reader.points(metric::MAP_LOADED_AT).is_empty());
        warm(&b).await.unwrap();
        assert_eq!(only(&reader, metric::MAP_LOADED_AT).value, at);

        // (d) A map file that cannot be read, then one that does not parse: each is counted and
        // neither sets the gauge.
        let file = tmp.path().join("system-map.yml");
        let mut cfg = Config::for_tests(
            format!("sqlite://{}/file.db?mode=rwc", tmp.path().display()),
            "m0x6A",
        );
        cfg.map_file = Some(file.clone());
        let (c, reader) = app(cfg, Arc::new(FakeGitHub::new())).await;
        assert!(reload(&c).await.is_err());
        assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 1);
        std::fs::write(&file, "nodes: [unclosed").unwrap();
        assert!(reload(&c).await.is_err());
        assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 2);
        assert!(reader.points(metric::MAP_LOADED_AT).is_empty());

        // The file is fixed: the load records its time.
        std::fs::write(&file, MAP).unwrap();
        let loaded = reload(&c).await.unwrap();
        assert_eq!(
            only(&reader, metric::MAP_LOADED_AT).value,
            u64::try_from(loaded.loaded_at.timestamp()).unwrap()
        );
    }

    /// A server with no map in memory over `db`, and a GitHub with no branches, so every fetch
    /// fails and `reload` falls back to the database copy.
    async fn failing_github_over(db: &std::path::Path) -> (App, metric_reader::Reader) {
        let cfg = Config::for_tests(format!("sqlite://{}?mode=rwc", db.display()), "m0x6A");
        app(cfg, Arc::new(FakeGitHub::new())).await
    }

    #[tokio::test]
    async fn the_database_fallback_keeps_the_stored_time() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, reader) = failing_github_over(&tmp.path().join("t.db")).await;
        let stored_at = Utc::now() - chrono::Duration::hours(3);
        store::store_map(&a.db, "sha-old", MAP, stored_at)
            .await
            .unwrap();

        // The map in use and the gauge carry the stored time, not the time of this reload, and
        // the failed fetch is counted once.
        let loaded = reload(&a).await.unwrap();
        assert_eq!(loaded.version, "sha-old");
        assert_eq!(loaded.loaded_at.timestamp(), stored_at.timestamp());
        let at = u64::try_from(stored_at.timestamp()).unwrap();
        assert_eq!(only(&reader, metric::MAP_LOADED_AT).value, at);
        assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 1);
        // The stored row keeps its time.
        let (_, _, row_at) = store::load_map(&a.db).await.unwrap().unwrap();
        assert_eq!(row_at.timestamp(), stored_at.timestamp());
    }

    const OFF_SEAM: &str = include_str!("../../../../schema/fixtures/maps/flows-off-seam.yml");

    #[tokio::test]
    async fn a_broken_database_copy_counts_one_failure_per_reload() {
        // A copy that does not parse, then one that parses but does not validate.
        for (n, (yaml, names)) in [
            ("nodes: [unclosed", "does not parse"),
            (OFF_SEAM, "\"web-server\""),
        ]
        .into_iter()
        .enumerate()
        {
            let tmp = tempfile::tempdir().unwrap();
            let (a, reader) = failing_github_over(&tmp.path().join(format!("{n}.db"))).await;
            store::store_map(&a.db, "sha-bad", yaml, Utc::now())
                .await
                .unwrap();

            // One reload, one failure: the fetch is counted, the broken copy behind it is not
            // counted again, and the error still names the failed fetch.
            let e = format!("{:#}", reload(&a).await.unwrap_err());
            assert!(e.contains(names), "{e}");
            assert!(e.contains("map fetch failed: map ref not found"), "{e}");
            assert!(e.contains("the database copy sha-bad is unusable"), "{e}");
            assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, 1, "{yaml}");
            assert!(reader.points(metric::MAP_LOADED_AT).is_empty());
            assert!(a.map.read().await.is_none());
        }
    }

    #[tokio::test]
    async fn a_fetched_map_that_is_broken_is_counted_once_and_not_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let github = Arc::new(FakeGitHub::new());
        let serve = |yaml: &str| {
            github.with(|g| {
                g.files.insert(
                    (
                        "m0x6A/rubato".into(),
                        "docs/design/rubato-map.yml".into(),
                        "main".into(),
                    ),
                    yaml.into(),
                );
                g.branches
                    .insert("m0x6A/rubato".into(), vec![("main".into(), "sha1".into())]);
            })
        };
        let cfg = Config::for_tests(
            format!("sqlite://{}/t.db?mode=rwc", tmp.path().display()),
            "m0x6A",
        );
        let (a, reader) = app(cfg, github.clone()).await;

        // Each broken fetch adds one failure, sets no time and stores nothing.
        for (n, (yaml, names)) in [
            ("nodes: [unclosed", "does not parse"),
            (OFF_SEAM, "\"web-server\""),
        ]
        .into_iter()
        .enumerate()
        {
            serve(yaml);
            let e = format!("{:#}", reload(&a).await.unwrap_err());
            assert!(e.contains(names), "{e}");
            assert_eq!(only(&reader, metric::MAP_LOAD_FAILURES).value, n as u64 + 1);
            assert!(reader.points(metric::MAP_LOADED_AT).is_empty());
            assert!(store::load_map(&a.db).await.unwrap().is_none());
            assert!(a.map.read().await.is_none());
        }
    }
}
