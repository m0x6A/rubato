//! Load the system map (ADR 0010): from GitHub, or a local file, falling back to the cached copy.

use super::store;
use crate::kernel::state::{App, LoadedMap};
use anyhow::{Context, Result};
use chrono::Utc;
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
    let (version, yaml) = match fetched {
        Ok(v) => v,
        Err(e) => {
            app.metrics.map_load_failures.add(1, &[]);
            if let Some(m) = app.map.read().await.clone() {
                tracing::warn!(error = %e, "map fetch failed; keeping cached version {}", m.version);
                return Ok(m);
            }
            if let Some((version, yaml, _)) = store::load_map(&app.db).await? {
                tracing::warn!(error = %e, "map fetch failed; using the database copy {version}");
                (version, yaml)
            } else {
                return Err(e);
            }
        }
    };
    let map: SystemMap = serde_yaml_ng::from_str(&yaml).context("system-map.yml does not parse")?;
    if let Err(errors) = map.validate() {
        app.metrics.map_load_failures.add(1, &[]);
        anyhow::bail!(
            "system map invalid: {}",
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let loaded = LoadedMap {
        map,
        version: version.clone(),
        loaded_at: Utc::now(),
    };
    store::store_map(&app.db, &version, &yaml, loaded.loaded_at).await?;
    *app.map.write().await = Some(loaded.clone());
    app.metrics
        .map_loaded
        .record(1, &[KeyValue::new(attr::MAP_VERSION, version.clone())]);
    tracing::info!(version = %version, nodes = loaded.map.nodes.len(), seams = loaded.map.seams.len(), "map loaded");
    Ok(loaded)
}

/// Local-file mode (`RUBATO_MAP_FILE`): the version is a content hash, since there may be no git.
async fn load_from_file(app: &App, file: &std::path::Path) -> Result<LoadedMap> {
    let yaml = tokio::fs::read_to_string(file)
        .await
        .with_context(|| format!("reading {}", file.display()))?;
    let map: SystemMap = serde_yaml_ng::from_str(&yaml).context("system-map.yml does not parse")?;
    if let Err(errors) = map.validate() {
        app.metrics.map_load_failures.add(1, &[]);
        anyhow::bail!(
            "system map invalid: {}",
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
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
    };
    store::store_map(&app.db, &version, &yaml, loaded.loaded_at).await?;
    *app.map.write().await = Some(loaded.clone());
    app.metrics
        .map_loaded
        .record(1, &[KeyValue::new(attr::MAP_VERSION, version.clone())]);
    tracing::info!(version = %version, file = %file.display(), "map loaded from file");
    Ok(loaded)
}

/// Load the map from the database at startup so the server answers before the first fetch.
pub async fn warm(app: &App) -> Result<()> {
    if let Some((version, yaml, loaded_at)) = store::load_map(&app.db).await? {
        let map: SystemMap = serde_yaml_ng::from_str(&yaml)?;
        *app.map.write().await = Some(LoadedMap {
            map,
            version,
            loaded_at,
        });
    }
    Ok(())
}
