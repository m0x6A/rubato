//! Table `rubato_approvals`: approvals given in Rubato (ADR 0054), one per version of a draft.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::IssueRef;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// One recorded approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubatoApproval {
    pub id: Uuid,
    pub issue: IssueRef,
    pub branch: String,
    pub draft_id: Uuid,
    pub rev: u32,
    pub plan_sha: String,
    pub bundle_hash: String,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
    /// The approver owns the draft.
    pub self_approved: bool,
    pub summary: String,
}

const COLUMNS: &str = "id, repo, number, branch, draft_id, rev, plan_sha, bundle_hash, approved_by, approved_at, self, summary";

fn row(r: &SqliteRow) -> Result<RubatoApproval> {
    Ok(RubatoApproval {
        id: r.get::<String, _>("id").parse()?,
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        branch: r.get("branch"),
        draft_id: r.get::<String, _>("draft_id").parse()?,
        rev: r.get::<i64, _>("rev") as u32,
        plan_sha: r.get("plan_sha"),
        bundle_hash: r.get("bundle_hash"),
        approved_by: r.get("approved_by"),
        approved_at: parse_ts(&r.get::<String, _>("approved_at")),
        self_approved: r.get::<i64, _>("self") != 0,
        summary: r.get("summary"),
    })
}

/// The approval of this version, if one is recorded.
pub async fn find(
    db: &SqlitePool,
    issue: &IssueRef,
    branch: &str,
    plan_sha: &str,
) -> Result<Option<RubatoApproval>> {
    let r = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM rubato_approvals WHERE repo = ? AND number = ? AND branch = ? AND plan_sha = ?"
    ))
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(branch)
    .bind(plan_sha)
    .fetch_optional(db)
    .await?;
    r.as_ref().map(row).transpose()
}

/// Record `a` unless this version is approved already. True when this call inserted it; two
/// presses at once insert one row.
pub async fn insert(db: &SqlitePool, a: &RubatoApproval) -> Result<bool> {
    let at = ts(a.approved_at);
    let r = sqlx::query(
        "INSERT OR IGNORE INTO rubato_approvals (id, repo, number, branch, draft_id, rev, plan_sha, bundle_hash, approved_by, approved_at, self, summary, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(a.id.to_string())
    .bind(&a.issue.repo)
    .bind(a.issue.number as i64)
    .bind(&a.branch)
    .bind(a.draft_id.to_string())
    .bind(a.rev as i64)
    .bind(&a.plan_sha)
    .bind(&a.bundle_hash)
    .bind(&a.approved_by)
    .bind(&at)
    .bind(a.self_approved as i64)
    .bind(&a.summary)
    .bind(&at)
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Every recorded approval, oldest first.
pub async fn all(db: &SqlitePool) -> Result<Vec<RubatoApproval>> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM rubato_approvals ORDER BY approved_at, id"
    ))
    .fetch_all(db)
    .await?;
    rows.iter().map(row).collect()
}
