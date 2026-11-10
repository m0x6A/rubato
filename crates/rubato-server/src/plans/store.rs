//! Table `plans`.

use crate::kernel::db::ts;
use anyhow::Result;
use chrono::Utc;
use rubato_protocol::{IssueRef, Plan};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct PlanRow {
    pub issue: IssueRef,
    pub branch: String,
    pub path: String,
    pub plan: Option<Plan>,
    pub parse_error: Option<String>,
}

pub async fn upsert_plan(
    db: &SqlitePool,
    issue: &IssueRef,
    branch: &str,
    path: &str,
    plan: Option<&Plan>,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO plans (repo, number, branch, path, plan, parse_error, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(repo, number) DO UPDATE SET branch = excluded.branch, path = excluded.path, plan = excluded.plan, parse_error = excluded.parse_error, fetched_at = excluded.fetched_at")
        .bind(&issue.repo).bind(issue.number as i64).bind(branch).bind(path).bind(plan.map(serde_json::to_string).transpose()?).bind(error).bind(ts(Utc::now()))
        .execute(db).await?;
    Ok(())
}

pub async fn delete_plan(db: &SqlitePool, issue: &IssueRef) -> Result<()> {
    sqlx::query("DELETE FROM plans WHERE repo = ? AND number = ?")
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn all_plans(db: &SqlitePool) -> Result<Vec<PlanRow>> {
    let rows = sqlx::query("SELECT * FROM plans").fetch_all(db).await?;
    Ok(rows
        .iter()
        .map(|r| PlanRow {
            issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
            branch: r.get("branch"),
            path: r.get("path"),
            plan: r
                .get::<Option<String>, _>("plan")
                .and_then(|s| serde_json::from_str(&s).ok()),
            parse_error: r.get("parse_error"),
        })
        .collect())
}
