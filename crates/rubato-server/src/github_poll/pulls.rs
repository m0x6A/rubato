//! Open pull requests of every map repo (issue #82, ADR 0034): one listing per repo per tick,
//! shared with the per-issue pass through [`RepoView`], kept in `repo_pulls`. Checks of pull
//! requests no tracked issue owns are read on the full lane while they are due
//! ([`OpenPull::checks_due`]); a tracked issue's pull request keeps its checks in
//! `pull_requests` (ADR 0023) and the overview takes them from there, so nothing is copied.

use super::poll::RepoView;
use crate::kernel::db::{parse_ts, ts};
use crate::kernel::github::OpenPulls;
use crate::kernel::state::App;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{ChecksStatus, IssueRef, OpenPull, RepoPulls, SystemMap};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What one pass did, for the poll tick's log line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PullsPass {
    pub listed: usize,
    pub failed: usize,
    pub checks_read: usize,
}

/// List the open pull requests of every repo the map names (shared with the per-issue pass,
/// so a repo is listed once per tick), replace each repo's rows, and on the full lane read the
/// checks that are due for pull requests outside `tracked` (repo, pull request number).
pub async fn list_repo_pulls(
    app: &App,
    map: &SystemMap,
    view: &mut RepoView,
    tracked: &HashSet<(String, u64)>,
    full: bool,
) -> PullsPass {
    let mut pass = PullsPass::default();
    let named: BTreeSet<&str> = map.repos_named();
    for repo in &named {
        let listing = view.pull_listing(app, repo).await.clone();
        let result = match listing {
            Ok(l) => {
                pass.listed += 1;
                replace(app, repo, &l, tracked, full, &mut pass).await
            }
            Err(e) => {
                pass.failed += 1;
                tracing::warn!(rubato.repo = %repo, error = %e, "pulls listing failed");
                record_error(&app.db, repo, &e).await
            }
        };
        if let Err(e) = result {
            tracing::error!(rubato.repo = %repo, error = %format!("{e:#}"), "storing pull listing failed");
        }
    }
    if let Err(e) = forget_others(&app.db, &named).await {
        tracing::error!(error = %format!("{e:#}"), "dropping pull listings of repos no longer in the map failed");
    }
    pass
}

struct Stored {
    checks: Option<ChecksStatus>,
    checks_sha: Option<String>,
}

async fn replace(
    app: &App,
    repo: &str,
    listing: &OpenPulls,
    tracked: &HashSet<(String, u64)>,
    full: bool,
    pass: &mut PullsPass,
) -> Result<()> {
    let before: HashMap<u64, Stored> =
        sqlx::query("SELECT number, checks, checks_sha FROM repo_pulls WHERE repo = ?")
            .bind(repo)
            .fetch_all(&app.db)
            .await?
            .iter()
            .map(|r| {
                (
                    r.get::<i64, _>("number") as u64,
                    Stored {
                        checks: r
                            .get::<Option<String>, _>("checks")
                            .and_then(|c| serde_json::from_value(c.into()).ok()),
                        checks_sha: r.get("checks_sha"),
                    },
                )
            })
            .collect();
    let mut rows = Vec::with_capacity(listing.pulls.len());
    for (branch, pr) in &listing.pulls {
        let old = before.get(&pr.number);
        let (mut checks, mut checks_sha) = (
            old.and_then(|o| o.checks),
            old.and_then(|o| o.checks_sha.clone()),
        );
        let owned = tracked.contains(&(repo.to_owned(), pr.number));
        if full && !owned && OpenPull::checks_due(checks, checks_sha.as_deref(), &pr.head_sha) {
            match app.github.checks(repo, &pr.head_sha).await {
                Ok(c) => {
                    pass.checks_read += 1;
                    checks = Some(c);
                    checks_sha = Some(pr.head_sha.clone());
                }
                Err(e) => {
                    tracing::warn!(rubato.repo = %repo, number = pr.number, error = %format!("{e:#}"), "reading checks of a pull request failed")
                }
            }
        }
        rows.push((branch.clone(), pr.clone(), checks, checks_sha));
    }
    let mut tx = app.db.begin().await?;
    sqlx::query("DELETE FROM repo_pulls WHERE repo = ?")
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    for (branch, pr, checks, checks_sha) in rows {
        let checks = checks.map(|c| {
            serde_json::to_value(c)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        });
        sqlx::query("INSERT INTO repo_pulls (repo, number, branch, base, title, author, draft, head_sha, checks, checks_sha, updated_at, url) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(repo).bind(pr.number as i64).bind(&branch).bind(pr.base.as_deref().unwrap_or_default())
            .bind(&pr.title).bind(&pr.author).bind(pr.draft).bind(&pr.head_sha).bind(checks).bind(checks_sha)
            .bind(ts(pr.updated_at)).bind(&pr.url).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO repo_pulls_status (repo, listed_at, truncated, error) VALUES (?, ?, ?, NULL) ON CONFLICT(repo) DO UPDATE SET listed_at = excluded.listed_at, truncated = excluded.truncated, error = NULL")
        .bind(repo).bind(ts(Utc::now())).bind(listing.truncated).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// A listing failed: keep the rows and the last good time, record why.
async fn record_error(db: &SqlitePool, repo: &str, error: &str) -> Result<()> {
    sqlx::query("INSERT INTO repo_pulls_status (repo, listed_at, truncated, error) VALUES (?, NULL, 0, ?) ON CONFLICT(repo) DO UPDATE SET error = excluded.error")
        .bind(repo).bind(error).execute(db).await?;
    Ok(())
}

async fn forget_others(db: &SqlitePool, named: &BTreeSet<&str>) -> Result<()> {
    let stored: Vec<String> = sqlx::query("SELECT repo FROM repo_pulls_status")
        .fetch_all(db)
        .await?
        .iter()
        .map(|r| r.get("repo"))
        .collect();
    for repo in stored.iter().filter(|r| !named.contains(r.as_str())) {
        let mut tx = db.begin().await?;
        sqlx::query("DELETE FROM repo_pulls WHERE repo = ?")
            .bind(repo)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM repo_pulls_status WHERE repo = ?")
            .bind(repo)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// Every stored listing, one entry per repo that has been listed or has failed, in repo order.
/// `issue` comes from the branch name (ADR 0006); checks of a tracked pull request are left for
/// the caller to take from `pull_requests`.
pub async fn repo_pulls(db: &SqlitePool) -> Result<Vec<RepoPulls>> {
    let mut by_repo: Vec<RepoPulls> = sqlx::query(
        "SELECT repo, listed_at, truncated, error FROM repo_pulls_status ORDER BY repo",
    )
    .fetch_all(db)
    .await?
    .iter()
    .map(|r| RepoPulls {
        repo: r.get("repo"),
        pulls: Vec::new(),
        listed_at: r
            .get::<Option<String>, _>("listed_at")
            .map(|t| parse_ts(&t)),
        truncated: r.get::<bool, _>("truncated"),
        error: r.get("error"),
    })
    .collect();
    let rows = sqlx::query("SELECT repo, number, branch, base, title, author, draft, head_sha, checks, updated_at, url FROM repo_pulls ORDER BY repo, number")
        .fetch_all(db)
        .await?;
    for r in &rows {
        let repo: String = r.get("repo");
        let branch: String = r.get("branch");
        let Some(entry) = by_repo.iter_mut().find(|e| e.repo == repo) else {
            continue;
        };
        let updated: DateTime<Utc> = parse_ts(&r.get::<String, _>("updated_at"));
        entry.pulls.push(OpenPull {
            issue: IssueRef::number_from_branch(&branch).map(|n| IssueRef::new(repo.clone(), n)),
            repo: repo.clone(),
            number: r.get::<i64, _>("number") as u64,
            title: r.get("title"),
            author: r.get("author"),
            base: r.get("base"),
            draft: r.get::<bool, _>("draft"),
            head_sha: r.get("head_sha"),
            checks: r
                .get::<Option<String>, _>("checks")
                .and_then(|c| serde_json::from_value(c.into()).ok()),
            updated_at: updated,
            url: r.get("url"),
            branch,
        });
    }
    Ok(by_repo)
}
