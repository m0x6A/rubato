//! Tables `plan_drafts` and `plan_draft_revisions`.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{DraftFile, DraftRevisionInfo, IssueRef, Plan};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct DraftRow {
    pub id: Uuid,
    pub issue: IssueRef,
    pub branch: String,
    pub owner: String,
    pub conflict: Option<(String, DateTime<Utc>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevisionRow {
    pub info: DraftRevisionInfo,
    pub launch_id: Option<Uuid>,
    pub files: Vec<DraftFile>,
    pub plan: Option<Plan>,
}

fn draft(r: &SqliteRow) -> Result<DraftRow> {
    let conflict = match (
        r.get::<Option<String>, _>("conflict_user"),
        r.get::<Option<String>, _>("conflict_at"),
    ) {
        (Some(u), Some(at)) => Some((u, parse_ts(&at))),
        _ => None,
    };
    Ok(DraftRow {
        id: r.get::<String, _>("id").parse()?,
        issue: IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
        branch: r.get("branch"),
        owner: r.get("owner"),
        conflict,
    })
}

fn revision(r: &SqliteRow) -> Result<RevisionRow> {
    Ok(RevisionRow {
        info: DraftRevisionInfo {
            rev: r.get::<i64, _>("rev") as u32,
            at: parse_ts(&r.get::<String, _>("at")),
            plan_sha: r.get("plan_sha"),
            bundle_hash: r.get("bundle_hash"),
            session_id: r.get("session_id"),
            missing: serde_json::from_str(&r.get::<String, _>("missing"))?,
            parse_error: r.get("parse_error"),
        },
        launch_id: r
            .get::<Option<String>, _>("launch_id")
            .map(|s| s.parse())
            .transpose()?,
        files: serde_json::from_str(&r.get::<String, _>("files"))?,
        plan: r
            .get::<Option<String>, _>("plan")
            .map(|p| serde_json::from_str(&p))
            .transpose()?,
    })
}

const DRAFT_COLUMNS: &str = "id, repo, number, branch, owner, conflict_user, conflict_at";
const REVISION_COLUMNS: &str =
    "rev, at, plan_sha, bundle_hash, session_id, missing, parse_error, launch_id, files, plan";

/// The draft of this branch, created with `owner` when there is none.
pub async fn get_or_create(
    db: &SqlitePool,
    issue: &IssueRef,
    branch: &str,
    owner: &str,
    now: DateTime<Utc>,
) -> Result<DraftRow> {
    sqlx::query("INSERT OR IGNORE INTO plan_drafts (id, repo, number, branch, owner, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string())
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .bind(branch)
        .bind(owner)
        .bind(ts(now))
        .execute(db)
        .await?;
    let r = sqlx::query(&format!(
        "SELECT {DRAFT_COLUMNS} FROM plan_drafts WHERE repo = ? AND number = ? AND branch = ?"
    ))
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(branch)
    .fetch_one(db)
    .await?;
    draft(&r)
}

pub async fn find(db: &SqlitePool, issue: &IssueRef, branch: &str) -> Result<Option<DraftRow>> {
    let r = sqlx::query(&format!(
        "SELECT {DRAFT_COLUMNS} FROM plan_drafts WHERE repo = ? AND number = ? AND branch = ?"
    ))
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(branch)
    .fetch_optional(db)
    .await?;
    r.as_ref().map(draft).transpose()
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<DraftRow>> {
    let r = sqlx::query(&format!(
        "SELECT {DRAFT_COLUMNS} FROM plan_drafts WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    r.as_ref().map(draft).transpose()
}

pub async fn all(db: &SqlitePool) -> Result<Vec<DraftRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {DRAFT_COLUMNS} FROM plan_drafts ORDER BY repo, number, branch"
    ))
    .fetch_all(db)
    .await?;
    rows.iter().map(draft).collect()
}

pub async fn record_conflict(
    db: &SqlitePool,
    id: Uuid,
    user: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query("UPDATE plan_drafts SET conflict_user = ?, conflict_at = ? WHERE id = ?")
        .bind(user)
        .bind(ts(at))
        .bind(id.to_string())
        .execute(db)
        .await?;
    Ok(())
}

pub async fn latest(db: &SqlitePool, draft_id: Uuid) -> Result<Option<RevisionRow>> {
    let r = sqlx::query(&format!(
        "SELECT {REVISION_COLUMNS} FROM plan_draft_revisions WHERE draft_id = ? ORDER BY rev DESC LIMIT 1"
    ))
    .bind(draft_id.to_string())
    .fetch_optional(db)
    .await?;
    r.as_ref().map(revision).transpose()
}

pub async fn revisions(db: &SqlitePool, draft_id: Uuid) -> Result<Vec<RevisionRow>> {
    let rows = sqlx::query(&format!(
        "SELECT {REVISION_COLUMNS} FROM plan_draft_revisions WHERE draft_id = ? ORDER BY rev"
    ))
    .bind(draft_id.to_string())
    .fetch_all(db)
    .await?;
    rows.iter().map(revision).collect()
}

pub async fn revision_of(db: &SqlitePool, draft_id: Uuid, rev: u32) -> Result<Option<RevisionRow>> {
    let r = sqlx::query(&format!(
        "SELECT {REVISION_COLUMNS} FROM plan_draft_revisions WHERE draft_id = ? AND rev = ?"
    ))
    .bind(draft_id.to_string())
    .bind(rev as i64)
    .fetch_optional(db)
    .await?;
    r.as_ref().map(revision).transpose()
}

/// Store the next revision, numbered after the latest, unless its bundle hash equals the latest
/// one's. One statement, so two bundles at once cannot take the same number or both be stored.
pub async fn insert_revision(
    db: &SqlitePool,
    draft_id: Uuid,
    rev: &RevisionRow,
) -> Result<Option<u32>> {
    let d = draft_id.to_string();
    let r = sqlx::query(
        "INSERT INTO plan_draft_revisions (draft_id, rev, launch_id, session_id, files, missing, plan_sha, bundle_hash, plan, parse_error, at) \
         SELECT ?, COALESCE(MAX(rev), 0) + 1, ?, ?, ?, ?, ?, ?, ?, ?, ? FROM plan_draft_revisions WHERE draft_id = ? \
         HAVING COALESCE((SELECT bundle_hash FROM plan_draft_revisions WHERE draft_id = ? ORDER BY rev DESC LIMIT 1), '') <> ? \
         RETURNING rev",
    )
    .bind(&d)
    .bind(rev.launch_id.map(|l| l.to_string()))
    .bind(&rev.info.session_id)
    .bind(serde_json::to_string(&rev.files)?)
    .bind(serde_json::to_string(&rev.info.missing)?)
    .bind(&rev.info.plan_sha)
    .bind(&rev.info.bundle_hash)
    .bind(rev.plan.as_ref().map(serde_json::to_string).transpose()?)
    .bind(&rev.info.parse_error)
    .bind(ts(rev.info.at))
    .bind(&d)
    .bind(&d)
    .bind(&rev.info.bundle_hash)
    .fetch_optional(db)
    .await?;
    Ok(r.map(|r| r.get::<i64, _>("rev") as u32))
}
