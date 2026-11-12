//! Table `plan_approvals`.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{ApproveState, IssueRef};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct Intent {
    pub login: String,
    pub issue: IssueRef,
    pub sha: String,
    pub branch: String,
    pub state: ApproveState,
    pub comment_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn state_of(text: &str) -> ApproveState {
    if text == "posted" {
        ApproveState::Posted
    } else {
        ApproveState::Opening
    }
}

fn intent(r: &sqlx::sqlite::SqliteRow) -> Intent {
    Intent {
        login: r.get("login"),
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        sha: r.get("sha"),
        branch: r.get("branch"),
        state: state_of(&r.get::<String, _>("state")),
        comment_url: r.get("comment_url"),
        created_at: parse_ts(&r.get::<String, _>("created_at")),
    }
}

pub async fn find(
    db: &SqlitePool,
    login: &str,
    issue: &IssueRef,
    sha: &str,
) -> Result<Option<Intent>> {
    Ok(sqlx::query(
        "SELECT * FROM plan_approvals WHERE login = ? AND repo = ? AND number = ? AND sha = ?",
    )
    .bind(login)
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(sha)
    .fetch_optional(db)
    .await?
    .map(|r| intent(&r)))
}

/// Insert an `opening` row. False when one exists already: another request got there first.
pub async fn open(
    db: &SqlitePool,
    login: &str,
    issue: &IssueRef,
    sha: &str,
    branch: &str,
) -> Result<bool> {
    Ok(sqlx::query(
        "INSERT OR IGNORE INTO plan_approvals (login, repo, number, sha, branch, state, created_at) VALUES (?, ?, ?, ?, ?, 'opening', ?)",
    )
    .bind(login)
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(sha)
    .bind(branch)
    .bind(ts(Utc::now()))
    .execute(db)
    .await?
    .rows_affected()
        == 1)
}

pub async fn posted(db: &SqlitePool, i: &Intent, comment_id: u64, comment_url: &str) -> Result<()> {
    sqlx::query("UPDATE plan_approvals SET state = 'posted', comment_id = ?, comment_url = ? WHERE login = ? AND repo = ? AND number = ? AND sha = ?")
        .bind(comment_id as i64)
        .bind(comment_url)
        .bind(&i.login)
        .bind(&i.issue.repo)
        .bind(i.issue.number as i64)
        .bind(&i.sha)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn forget(db: &SqlitePool, login: &str, issue: &IssueRef, sha: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM plan_approvals WHERE login = ? AND repo = ? AND number = ? AND sha = ?",
    )
    .bind(login)
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(sha)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn opening(db: &SqlitePool) -> Result<Vec<Intent>> {
    Ok(
        sqlx::query("SELECT * FROM plan_approvals WHERE state = 'opening'")
            .fetch_all(db)
            .await?
            .iter()
            .map(intent)
            .collect(),
    )
}
