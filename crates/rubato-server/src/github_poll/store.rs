//! Tables `pull_requests`, `decisions`, `issue_meta`, `open_issues`, `backlog_repos`,
//! `uncovered_repos` and `plan_signals`.

use super::suggest::Placement;
use crate::kernel::db::{parse_ts, ts};
use crate::kernel::github::OpenIssues;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{
    ApprovalRecord, BacklogRepo, DecisionRecord, IssueRef, OpenIssue, PullRequestState,
    ReviewRecord,
};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

pub async fn upsert_pr(db: &SqlitePool, issue: &IssueRef, pr: &PullRequestState) -> Result<()> {
    sqlx::query("INSERT INTO pull_requests (repo, number, pr, fetched_at) VALUES (?, ?, ?, ?) ON CONFLICT(repo, number) DO UPDATE SET pr = excluded.pr, fetched_at = excluded.fetched_at")
        .bind(&issue.repo).bind(issue.number as i64).bind(serde_json::to_string(pr)?).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn all_prs(db: &SqlitePool) -> Result<HashMap<IssueRef, PullRequestState>> {
    let rows = sqlx::query("SELECT repo, number, pr FROM pull_requests")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some((
                IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                serde_json::from_str(&r.get::<String, _>("pr")).ok()?,
            ))
        })
        .collect())
}

/// The stored pull request of one issue.
pub async fn pr_for(db: &SqlitePool, issue: &IssueRef) -> Result<Option<PullRequestState>> {
    let row = sqlx::query("SELECT pr FROM pull_requests WHERE repo = ? AND number = ?")
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .fetch_optional(db)
        .await?;
    Ok(row.and_then(|r| serde_json::from_str(&r.get::<String, _>("pr")).ok()))
}

pub async fn upsert_decision(db: &SqlitePool, rec: &DecisionRecord) -> Result<()> {
    sqlx::query("INSERT INTO decisions (repo, number, comment_id, record) VALUES (?, ?, ?, ?) ON CONFLICT(repo, number, comment_id) DO UPDATE SET record = excluded.record")
        .bind(&rec.issue.repo).bind(rec.issue.number as i64).bind(rec.comment_id as i64).bind(serde_json::to_string(rec)?).execute(db).await?;
    Ok(())
}

pub async fn all_decisions(db: &SqlitePool) -> Result<Vec<DecisionRecord>> {
    let rows = sqlx::query("SELECT record FROM decisions ORDER BY comment_id")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|r| serde_json::from_str(&r.get::<String, _>("record")).ok())
        .collect())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueMeta {
    pub title: Option<String>,
    pub url: Option<String>,
    pub github_state: Option<String>,
}

pub async fn upsert_issue_meta(db: &SqlitePool, issue: &IssueRef, meta: &IssueMeta) -> Result<()> {
    sqlx::query("INSERT INTO issue_meta (repo, number, title, github_state, url, fetched_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(repo, number) DO UPDATE SET title = excluded.title, github_state = excluded.github_state, url = excluded.url, fetched_at = excluded.fetched_at")
        .bind(&issue.repo).bind(issue.number as i64).bind(&meta.title).bind(&meta.github_state).bind(&meta.url).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn all_issue_meta(db: &SqlitePool) -> Result<HashMap<IssueRef, IssueMeta>> {
    let rows = sqlx::query("SELECT repo, number, title, github_state, url FROM issue_meta")
        .fetch_all(db)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
                IssueMeta {
                    title: r.get("title"),
                    url: r.get("url"),
                    github_state: r.get("github_state"),
                },
            )
        })
        .collect())
}

pub async fn issue_meta(db: &SqlitePool, issue: &IssueRef) -> Result<Option<IssueMeta>> {
    let row = sqlx::query(
        "SELECT title, github_state, url FROM issue_meta WHERE repo = ? AND number = ?",
    )
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| IssueMeta {
        title: r.get("title"),
        url: r.get("url"),
        github_state: r.get("github_state"),
    }))
}

/// Replace a repo's open issues with a fresh listing and mark the listing good, in one
/// transaction: running it twice gives the same rows.
pub async fn replace_open_issues(
    db: &SqlitePool,
    repo: &str,
    listing: &OpenIssues,
    placements: &HashMap<u64, Placement>,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM open_issues WHERE repo = ?")
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    for i in &listing.issues {
        let p = placements.get(&i.number);
        let line = p
            .and_then(|p| p.untrusted_line.as_ref())
            .map(serde_json::to_string)
            .transpose()?;
        let suggestion = p
            .and_then(|p| p.suggestion.as_ref())
            .map(serde_json::to_string)
            .transpose()?;
        sqlx::query("INSERT OR REPLACE INTO open_issues (repo, number, title, url, labels, updated_at, map_line, suggestion) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(repo).bind(i.number as i64).bind(&i.title).bind(&i.url).bind(serde_json::to_string(&i.labels)?).bind(ts(i.updated_at)).bind(line).bind(suggestion).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO backlog_repos (repo, listed_at, truncated, error) VALUES (?, ?, ?, NULL) ON CONFLICT(repo) DO UPDATE SET listed_at = excluded.listed_at, truncated = excluded.truncated, error = NULL")
        .bind(repo).bind(ts(now)).bind(listing.truncated).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// A listing failed: keep the rows and the last good time, record why.
pub async fn record_listing_error(db: &SqlitePool, repo: &str, error: &str) -> Result<()> {
    sqlx::query("INSERT INTO backlog_repos (repo, listed_at, truncated, error) VALUES (?, NULL, 0, ?) ON CONFLICT(repo) DO UPDATE SET error = excluded.error")
        .bind(repo).bind(error).execute(db).await?;
    Ok(())
}

/// A repo the map no longer names: its issues and its listing row go.
pub async fn forget_repo(db: &SqlitePool, repo: &str) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM open_issues WHERE repo = ?")
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM backlog_repos WHERE repo = ?")
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Every listed open issue and every repo's listing state.
pub async fn backlog(db: &SqlitePool) -> Result<(Vec<OpenIssue>, Vec<BacklogRepo>)> {
    let issues = sqlx::query(
        "SELECT repo, number, title, url, labels, updated_at, map_line, suggestion FROM open_issues ORDER BY repo, number",
    )
    .fetch_all(db)
    .await?
    .iter()
    .map(|r| OpenIssue {
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        title: r.get("title"),
        url: r.get("url"),
        labels: serde_json::from_str(&r.get::<String, _>("labels")).unwrap_or_default(),
        updated_at: parse_ts(&r.get::<String, _>("updated_at")),
        map_line: r
            .get::<Option<String>, _>("map_line")
            .and_then(|j| serde_json::from_str(&j).ok()),
        suggestion: r
            .get::<Option<String>, _>("suggestion")
            .and_then(|j| serde_json::from_str(&j).ok()),
    })
    .collect();
    let repos =
        sqlx::query("SELECT repo, listed_at, truncated, error FROM backlog_repos ORDER BY repo")
            .fetch_all(db)
            .await?
            .iter()
            .map(|r| BacklogRepo {
                repo: r.get("repo"),
                listed_at: r
                    .get::<Option<String>, _>("listed_at")
                    .map(|t| parse_ts(&t)),
                truncated: r.get::<bool, _>("truncated"),
                error: r.get("error"),
            })
            .collect();
    Ok((issues, repos))
}

/// Replace the set of repos the installation does not cover.
pub async fn replace_uncovered(
    db: &SqlitePool,
    repos: &[String],
    now: DateTime<Utc>,
) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM uncovered_repos")
        .execute(&mut *tx)
        .await?;
    for repo in repos {
        sqlx::query("INSERT INTO uncovered_repos (repo, seen_at) VALUES (?, ?)")
            .bind(repo)
            .bind(ts(now))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn uncovered(db: &SqlitePool) -> Result<Vec<String>> {
    Ok(
        sqlx::query("SELECT repo FROM uncovered_repos ORDER BY repo")
            .fetch_all(db)
            .await?
            .iter()
            .map(|r| r.get("repo"))
            .collect(),
    )
}

// Table `plan_signals` (ADR 0027): `Review:` and `Approved:` comments from members.

pub async fn upsert_signal(
    db: &SqlitePool,
    issue: &IssueRef,
    comment_id: u64,
    kind: &str,
    record: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO plan_signals (repo, number, comment_id, kind, record) VALUES (?, ?, ?, ?, ?) ON CONFLICT(repo, number, comment_id) DO UPDATE SET kind = excluded.kind, record = excluded.record")
        .bind(&issue.repo).bind(issue.number as i64).bind(comment_id as i64).bind(kind).bind(record)
        .execute(db).await?;
    Ok(())
}

/// Drop the issue's signals whose comment was not seen in this read (deleted, or edited into
/// something else).
pub async fn drop_signals_except(db: &SqlitePool, issue: &IssueRef, kept: &[u64]) -> Result<()> {
    let rows = sqlx::query("SELECT comment_id FROM plan_signals WHERE repo = ? AND number = ?")
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .fetch_all(db)
        .await?;
    for r in rows {
        let id = r.get::<i64, _>("comment_id");
        if !kept.contains(&(id as u64)) {
            sqlx::query(
                "DELETE FROM plan_signals WHERE repo = ? AND number = ? AND comment_id = ?",
            )
            .bind(&issue.repo)
            .bind(issue.number as i64)
            .bind(id)
            .execute(db)
            .await?;
        }
    }
    Ok(())
}

/// The plan hash on the branch an approval names, when that is not the branch the plan is read
/// from. `None` when that branch has no plan file.
pub async fn set_target_sha(
    db: &SqlitePool,
    issue: &IssueRef,
    comment_id: u64,
    sha: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE plan_signals SET target_sha = ? WHERE repo = ? AND number = ? AND comment_id = ?",
    )
    .bind(sha)
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(comment_id as i64)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn all_reviews(db: &SqlitePool) -> Result<Vec<ReviewRecord>> {
    let rows =
        sqlx::query("SELECT record FROM plan_signals WHERE kind = 'review' ORDER BY comment_id")
            .fetch_all(db)
            .await?;
    Ok(rows
        .iter()
        .filter_map(|r| serde_json::from_str(&r.get::<String, _>("record")).ok())
        .collect())
}

/// The branch the newest approval of each issue names, for issues whose newest approval names
/// one (ADR 0032: the plan is read from that branch).
pub async fn approved_branches(db: &SqlitePool) -> Result<HashMap<IssueRef, String>> {
    let mut newest: HashMap<IssueRef, ApprovalRecord> = HashMap::new();
    for (a, _) in all_approvals(db).await? {
        match newest.get(&a.issue) {
            Some(n) if (n.created_at, n.comment_id) >= (a.created_at, a.comment_id) => {}
            _ => {
                newest.insert(a.issue.clone(), a);
            }
        }
    }
    Ok(newest
        .into_iter()
        .filter_map(|(i, a)| a.branch.map(|b| (i, b)))
        .collect())
}

/// Every approval, with the plan hash of the branch it names when it names another one.
pub async fn all_approvals(db: &SqlitePool) -> Result<Vec<(ApprovalRecord, Option<String>)>> {
    let rows = sqlx::query(
        "SELECT record, target_sha FROM plan_signals WHERE kind = 'approval' ORDER BY comment_id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some((
                serde_json::from_str(&r.get::<String, _>("record")).ok()?,
                r.get::<Option<String>, _>("target_sha"),
            ))
        })
        .collect())
}
