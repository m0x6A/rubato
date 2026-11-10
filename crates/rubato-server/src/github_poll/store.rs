//! Tables `pull_requests`, `decisions` and `issue_meta`.

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::Utc;
use rubato_protocol::{DecisionRecord, IssueRef, PullRequestState};
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

#[derive(Debug, Clone, Default)]
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
    Ok(all_issue_meta(db).await?.remove(issue))
}
