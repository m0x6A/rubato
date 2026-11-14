//! Table `issue_creations`. Every state change is `… WHERE state = 'opening'`, so the handler
//! and the listing that finds a timed-out create can both finish a row and only one wins.

use crate::kernel::db::{parse_ts, ts};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{CreationState, IssueRef, MapRef};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creation {
    pub request_id: String,
    pub repo: String,
    pub target: MapRef,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub state: CreationState,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub error: Option<String>,
}

impl Creation {
    pub fn issue(&self) -> Option<IssueRef> {
        self.number.map(|n| IssueRef::new(self.repo.clone(), n))
    }
}

fn state_name(s: CreationState) -> &'static str {
    match s {
        CreationState::Opening => "opening",
        CreationState::Created => "created",
        CreationState::Failed => "failed",
    }
}

fn row(r: &sqlx::sqlite::SqliteRow) -> Creation {
    let kind: String = r.get("kind");
    let target: String = r.get("target");
    Creation {
        request_id: r.get("request_id"),
        repo: r.get("repo"),
        target: if kind == "seam" {
            MapRef::seam(target)
        } else {
            MapRef::node(target)
        },
        created_by: r.get("created_by"),
        created_at: parse_ts(&r.get::<String, _>("created_at")),
        state: match r.get::<String, _>("state").as_str() {
            "created" => CreationState::Created,
            "failed" => CreationState::Failed,
            _ => CreationState::Opening,
        },
        number: r.get::<Option<i64>, _>("number").map(|n| n as u64),
        url: r.get("url"),
        error: r.get("error"),
    }
}

/// Insert a request as `opening`. False when a row with this id exists already.
#[allow(clippy::too_many_arguments)]
pub async fn begin(
    db: &SqlitePool,
    ws: &str,
    request_id: &str,
    repo: &str,
    target: &MapRef,
    title: &str,
    by: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let (kind, t) = match target {
        MapRef::Node { id } => ("node", id),
        MapRef::Seam { id } => ("seam", id),
    };
    let r = sqlx::query("INSERT OR IGNORE INTO issue_creations (workspace_id, request_id, repo, kind, target, title, created_by, created_at, state, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'opening', ?)")
        .bind(ws).bind(request_id).bind(repo).bind(kind).bind(t).bind(title).bind(by).bind(ts(now)).bind(ts(now)).execute(db).await?;
    Ok(r.rows_affected() == 1)
}

pub async fn get(db: &SqlitePool, request_id: &str) -> Result<Option<Creation>> {
    Ok(
        sqlx::query("SELECT * FROM issue_creations WHERE request_id = ?")
            .bind(request_id)
            .fetch_optional(db)
            .await?
            .as_ref()
            .map(row),
    )
}

pub async fn opening(db: &SqlitePool) -> Result<Vec<Creation>> {
    Ok(
        sqlx::query("SELECT * FROM issue_creations WHERE state = 'opening' ORDER BY created_at")
            .fetch_all(db)
            .await?
            .iter()
            .map(row)
            .collect(),
    )
}

/// `opening` → `created`. False when the row was no longer opening (the other side won).
pub async fn created(db: &SqlitePool, request_id: &str, number: u64, url: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE issue_creations SET state = 'created', number = ?, url = ?, error = NULL, updated_at = ? WHERE request_id = ? AND state = 'opening'")
        .bind(number as i64).bind(url).bind(ts(Utc::now())).bind(request_id).execute(db).await?;
    Ok(r.rows_affected() == 1)
}

/// `opening` → `failed`, with why.
pub async fn failed(db: &SqlitePool, request_id: &str, error: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE issue_creations SET state = ?, error = ?, updated_at = ? WHERE request_id = ? AND state = 'opening'")
        .bind(state_name(CreationState::Failed)).bind(error).bind(ts(Utc::now())).bind(request_id).execute(db).await?;
    Ok(r.rows_affected() == 1)
}
