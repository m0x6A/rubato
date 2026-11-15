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
    /// What the last handoff report said, once one came (ADR 0055).
    pub handoff: Option<HandoffFacts>,
}

/// A handoff report's facts: never a state, so whether the approval is current stays derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffFacts {
    /// `HandoffOutcome::name()`.
    pub outcome: String,
    pub at: DateTime<Utc>,
    pub commit: Option<String>,
    /// ADRs the handoff could not accept in this repo (finding `AdrsNotAccepted`).
    pub adrs_left: Vec<String>,
}

const COLUMNS: &str = "id, repo, number, branch, draft_id, rev, plan_sha, bundle_hash, approved_by, approved_at, self, summary, handoff_outcome, handoff_at, handoff_commit, handoff_adrs_left";

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
        handoff: match (
            r.get::<Option<String>, _>("handoff_outcome"),
            r.get::<Option<String>, _>("handoff_at"),
        ) {
            (Some(outcome), Some(at)) => Some(HandoffFacts {
                outcome,
                at: parse_ts(&at),
                commit: r.get("handoff_commit"),
                adrs_left: r
                    .get::<Option<String>, _>("handoff_adrs_left")
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default(),
            }),
            _ => None,
        },
    })
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<RubatoApproval>> {
    let r = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM rubato_approvals WHERE id = ?"
    ))
    .bind(id.to_string())
    .fetch_optional(db)
    .await?;
    r.as_ref().map(row).transpose()
}

/// Record a handoff report on the approval `id`, only when it is an approval of `issue`. True
/// when a row was written; a later report replaces an earlier one.
pub async fn record_handoff(
    db: &SqlitePool,
    id: Uuid,
    issue: &IssueRef,
    facts: &HandoffFacts,
) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE rubato_approvals SET handoff_outcome = ?, handoff_at = ?, handoff_commit = ?, handoff_adrs_left = ? \
         WHERE id = ? AND repo = ? AND number = ?",
    )
    .bind(&facts.outcome)
    .bind(ts(facts.at))
    .bind(&facts.commit)
    .bind(serde_json::to_string(&facts.adrs_left)?)
    .bind(id.to_string())
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .execute(db)
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// The approval of this version, if one is recorded. A version is the plan and its whole bundle:
/// a revision that changes only the design note or an ADR is a new version to approve.
pub async fn find(
    db: &SqlitePool,
    issue: &IssueRef,
    branch: &str,
    plan_sha: &str,
    bundle_hash: &str,
) -> Result<Option<RubatoApproval>> {
    let r = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM rubato_approvals WHERE repo = ? AND number = ? AND branch = ? AND plan_sha = ? AND bundle_hash = ?"
    ))
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(branch)
    .bind(plan_sha)
    .bind(bundle_hash)
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
