//! Periodic reads of GitHub: the map (ADR 0010), plan files on issue branches (ADR 0005), PRs
//! and checks, `Decision:` comments (ADR 0014). Everything lands as rows; progress is derived.

use crate::db::{self, IssueMeta};
use crate::plan_parse::parse_plan;
use crate::progress;
use crate::state::{App, LoadedMap};
use anyhow::{Context, Result};
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{ChecksStatus, DecisionRecord, IssueRef, PullRequestState, SystemMap};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::sync::watch;

/// Fetch, validate and cache the map. Falls back to the cached copy when GitHub fails.
pub async fn reload_map(app: &App) -> Result<LoadedMap> {
    let cfg = &app.config;
    if let Some(file) = &cfg.map_file {
        return load_map_from_file(app, file).await;
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
            if let Some((version, yaml, _)) = db::load_map(&app.db).await? {
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
    db::store_map(&app.db, &version, &yaml, loaded.loaded_at).await?;
    *app.map.write().await = Some(loaded.clone());
    app.metrics
        .map_loaded
        .record(1, &[KeyValue::new(attr::MAP_VERSION, version.clone())]);
    tracing::info!(version = %version, nodes = loaded.map.nodes.len(), seams = loaded.map.seams.len(), "map loaded");
    Ok(loaded)
}

/// Local-file mode (`RUBATO_MAP_FILE`): the version is a content hash, since there may be no git.
async fn load_map_from_file(app: &App, file: &std::path::Path) -> Result<LoadedMap> {
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
    db::store_map(&app.db, &version, &yaml, loaded.loaded_at).await?;
    *app.map.write().await = Some(loaded.clone());
    app.metrics
        .map_loaded
        .record(1, &[KeyValue::new(attr::MAP_VERSION, version.clone())]);
    tracing::info!(version = %version, file = %file.display(), "map loaded from file");
    Ok(loaded)
}

/// Load the map from the database at startup so the server answers before the first fetch.
pub async fn warm_map(app: &App) -> Result<()> {
    if let Some((version, yaml, loaded_at)) = db::load_map(&app.db).await? {
        let map: SystemMap = serde_yaml_ng::from_str(&yaml)?;
        *app.map.write().await = Some(LoadedMap {
            map,
            version,
            loaded_at,
        });
    }
    Ok(())
}

/// Slug from the branch `issue/<n>-<slug>`.
fn slug_of(branch: &str, number: u64) -> String {
    branch
        .strip_prefix(&format!("issue/{number}-"))
        .unwrap_or("")
        .to_owned()
}

/// One pass over every issue of interest.
pub async fn poll_once(app: &App) -> Result<()> {
    let Some(map) = app.map.read().await.clone() else {
        tracing::warn!("no map loaded; skipping poll");
        return Ok(());
    };
    let mut issues: BTreeSet<IssueRef> = db::list_pins(&app.db)
        .await?
        .into_iter()
        .map(|p| p.issue)
        .collect();
    issues.extend(db::launched_issues(&app.db).await?);
    issues.extend(
        db::all_sessions(&app.db)
            .await?
            .into_iter()
            .filter_map(|s| s.issue),
    );
    issues.extend(db::all_plans(&app.db).await?.into_iter().map(|p| p.issue));

    for issue in &issues {
        if let Err(e) = poll_issue(app, &map, issue).await {
            tracing::warn!(issue = %issue, error = %e, "poll failed for issue");
        }
    }
    match app.github.rate_limit().await {
        Ok((remaining, limit)) => {
            app.metrics.rate_limit_remaining.record(remaining, &[]);
            app.metrics.rate_limit_limit.record(limit, &[]);
            if limit > 0 && remaining * 10 < limit {
                tracing::warn!(remaining, limit, "GitHub rate limit under 10%");
            }
        }
        Err(e) => tracing::debug!(error = %e, "rate limit read failed"),
    }
    // Gauges from the derived picture.
    let built = crate::api::build_overview_inputs(app).await?;
    let built = progress::build(&built, Some(&map), &app.config.limits, Utc::now());
    let (by_state, by_seam) = progress::counts(&built);
    for (state, n) in by_state {
        app.metrics.issues_by_state.record(
            n as u64,
            &[KeyValue::new("state", format!("{state:?}").to_lowercase())],
        );
    }
    for (seam, n) in by_seam {
        app.metrics
            .seam_open_plans
            .record(n as u64, &[KeyValue::new(attr::SEAM, seam)]);
    }
    let unknown: usize = built
        .findings
        .iter()
        .map(|f| {
            if let rubato_protocol::Finding::UnknownIds { ids, .. } = f {
                ids.len()
            } else {
                0
            }
        })
        .sum();
    app.metrics
        .plan_unknown_id_count
        .record(unknown as u64, &[]);
    app.notify("poll");
    Ok(())
}

async fn poll_issue(app: &App, map: &LoadedMap, issue: &IssueRef) -> Result<()> {
    if let Some(info) = app.github.issue(&issue.repo, issue.number).await? {
        db::upsert_issue_meta(
            &app.db,
            issue,
            &IssueMeta {
                title: Some(info.title),
                url: Some(info.url),
                github_state: Some(info.state),
            },
        )
        .await?;
    }
    let branches = app
        .github
        .branches_with_prefix(&issue.repo, &format!("issue/{}-", issue.number))
        .await?;
    let branch = branches
        .iter()
        .find(|b| IssueRef::number_from_branch(b) == Some(issue.number))
        .cloned();
    if let Some(branch) = &branch {
        let path = map
            .map
            .plan_path_for(&issue.repo, issue.number, &slug_of(branch, issue.number));
        match app.github.file(&issue.repo, &path, branch).await? {
            Some(text) => match parse_plan(&text, &path) {
                Ok(mut plan) => {
                    plan.git_ref = Some(branch.clone());
                    db::upsert_plan(&app.db, issue, branch, &path, Some(&plan), None).await?;
                }
                Err(e) => {
                    app.metrics.plan_parse_failures.add(
                        1,
                        &[
                            KeyValue::new(attr::PLAN_PATH, path.clone()),
                            KeyValue::new(attr::REPO, issue.repo.clone()),
                        ],
                    );
                    db::upsert_plan(&app.db, issue, branch, &path, None, Some(&e.to_string()))
                        .await?;
                }
            },
            None => db::delete_plan(&app.db, issue).await?,
        }
        if let Some(pr) = app.github.pull_for_branch(&issue.repo, branch).await? {
            let checks = if pr.merged_at.is_some() {
                ChecksStatus::Green
            } else {
                app.github.checks(&issue.repo, &pr.head_sha).await?
            };
            db::upsert_pr(
                &app.db,
                issue,
                &PullRequestState {
                    repo: issue.repo.clone(),
                    number: pr.number,
                    branch: branch.clone(),
                    open: pr.open,
                    draft: pr.draft,
                    checks,
                    merged_at: pr.merged_at,
                    updated_at: pr.updated_at,
                    url: pr.url,
                },
            )
            .await?;
        }
    }
    for c in app.github.issue_comments(&issue.repo, issue.number).await? {
        if let Some(body) = DecisionRecord::parse_body(&c.body) {
            db::upsert_decision(
                &app.db,
                &DecisionRecord {
                    issue: issue.clone(),
                    comment_id: c.id,
                    author: c.author,
                    title: body.title,
                    context: body.context,
                    options: body.options,
                    chosen: body.chosen,
                    why: body.why,
                    created_at: c.created_at,
                    url: c.url,
                },
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn run(app: App, mut shutdown: watch::Receiver<bool>) {
    let mut map_age = 0u64;
    loop {
        if map_age == 0
            && let Err(e) = reload_map(&app).await
        {
            tracing::error!(error = %e, "map reload failed");
        }
        if let Err(e) = poll_once(&app).await {
            tracing::error!(error = %e, "poll failed");
        }
        let secs = app.config.limits.github_poll_interval_secs.max(5);
        map_age = (map_age + secs) % 300;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
            _ = shutdown.changed() => return,
        }
    }
}
