//! Table `session_log_entries`. One row per (session, prompt, role); a replayed hook writes
//! nothing new.

use crate::kernel::db::{parse_ts, ts};
use crate::kernel::tenancy::TenantId;
use anyhow::Result;
use rubato_protocol::{IssueRef, LogKind, LogRole, SessionLogEntry};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Insert one entry; returns false when it was already there.
pub async fn insert(
    db: &SqlitePool,
    t: &TenantId,
    launch_id: Uuid,
    issue: &IssueRef,
    e: &SessionLogEntry,
) -> Result<bool> {
    let n = sqlx::query("INSERT OR IGNORE INTO session_log_entries (tenant_id, session_id, prompt_id, role, launch_id, repo, number, kind, text, truncated, message_id, mixed, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(t.as_str()).bind(&e.session_id)
        .bind(&e.prompt_id)
        .bind(e.role.name())
        .bind(launch_id.to_string())
        .bind(&issue.repo)
        .bind(issue.number as i64)
        .bind(e.kind.name())
        .bind(&e.text)
        .bind(e.truncated)
        .bind(e.message_id.map(|m| m.to_string()))
        .bind(e.mixed)
        .bind(ts(e.at))
        .execute(db)
        .await?
        .rows_affected();
    Ok(n == 1)
}

fn role(s: &str) -> LogRole {
    if s == "assistant" {
        LogRole::Assistant
    } else {
        LogRole::User
    }
}

fn kind(s: &str) -> LogKind {
    match s {
        "reply" => LogKind::Reply,
        "review" => LogKind::Review,
        "summary" => LogKind::Summary,
        _ => LogKind::Prompt,
    }
}

/// The session message a prompt carried (its `UserPromptSubmit` entry's `message_id`).
pub async fn prompt_message(
    db: &SqlitePool,
    t: &TenantId,
    launch_id: Uuid,
    session_id: &str,
    prompt_id: &str,
) -> Result<Option<Uuid>> {
    let r = sqlx::query("SELECT message_id FROM session_log_entries WHERE tenant_id = ? AND launch_id = ? AND session_id = ? AND prompt_id = ? AND role = 'user'")
        .bind(t.as_str()).bind(launch_id.to_string())
        .bind(session_id)
        .bind(prompt_id)
        .fetch_optional(db)
        .await?;
    Ok(r.and_then(|r| r.get::<Option<String>, _>("message_id"))
        .map(|m| m.parse())
        .transpose()?)
}

/// A launch's entries, oldest first; within one turn the prompt before the reply.
pub async fn for_launch(
    db: &SqlitePool,
    t: &TenantId,
    launch_id: Uuid,
) -> Result<Vec<SessionLogEntry>> {
    let rows = sqlx::query("SELECT session_id, prompt_id, role, kind, text, truncated, message_id, mixed, at FROM session_log_entries WHERE tenant_id = ? AND launch_id = ? ORDER BY at, CASE role WHEN 'user' THEN 0 ELSE 1 END")
        .bind(t.as_str()).bind(launch_id.to_string())
        .fetch_all(db)
        .await?;
    rows.iter()
        .map(|r| {
            Ok(SessionLogEntry {
                session_id: r.get("session_id"),
                prompt_id: r.get("prompt_id"),
                role: role(&r.get::<String, _>("role")),
                kind: kind(&r.get::<String, _>("kind")),
                text: r.get("text"),
                truncated: r.get("truncated"),
                message_id: r
                    .get::<Option<String>, _>("message_id")
                    .map(|m| m.parse())
                    .transpose()?,
                mixed: r.get("mixed"),
                at: parse_ts(&r.get::<String, _>("at")),
            })
        })
        .collect()
}

/// The newest assistant entry's text per launch, for these launches only.
pub async fn last_replies(
    db: &SqlitePool,
    launch_ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, String>> {
    let mut out = std::collections::HashMap::new();
    if launch_ids.is_empty() {
        return Ok(out);
    }
    let marks = vec!["?"; launch_ids.len()].join(", ");
    let sql = format!(
        "SELECT launch_id, text FROM session_log_entries e WHERE role = 'assistant' AND launch_id IN ({marks}) \
         AND at = (SELECT MAX(at) FROM session_log_entries x WHERE x.launch_id = e.launch_id AND x.role = 'assistant')"
    );
    let mut q = sqlx::query(&sql);
    for id in launch_ids {
        q = q.bind(id.to_string());
    }
    for r in q.fetch_all(db).await? {
        if let Ok(id) = r.get::<String, _>("launch_id").parse() {
            out.insert(id, r.get("text"));
        }
    }
    Ok(out)
}
