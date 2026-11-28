//! Tables `pull_requests`, `decisions`, `issue_meta`, `open_issues`, `backlog_repos`,
//! `uncovered_repos` and `plan_signals`.

use super::suggest::Placement;
use crate::kernel::db::{parse_ts, ts};
use crate::kernel::github::OpenIssues;
use crate::kernel::tenancy::TenantId;
use crate::kernel::tenancy::WsKey;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{
    ApprovalRecord, BacklogRepo, DecisionRecord, IssueRef, OpenIssue, PullRequestState,
    ReviewRecord,
};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

pub async fn upsert_pr(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
    pr: &PullRequestState,
) -> Result<()> {
    sqlx::query("INSERT INTO pull_requests (tenant_id, repo, number, pr, fetched_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(tenant_id, repo, number) DO UPDATE SET pr = excluded.pr, fetched_at = excluded.fetched_at")
        .bind(t.as_str()).bind(&issue.repo).bind(issue.number as i64).bind(serde_json::to_string(pr)?).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn all_prs(db: &SqlitePool, t: &TenantId) -> Result<HashMap<IssueRef, PullRequestState>> {
    let rows = sqlx::query("SELECT repo, number, pr FROM pull_requests WHERE tenant_id = ?")
        .bind(t.as_str())
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
pub async fn pr_for(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
) -> Result<Option<PullRequestState>> {
    let row =
        sqlx::query("SELECT pr FROM pull_requests WHERE tenant_id = ? AND repo = ? AND number = ?")
            .bind(t.as_str())
            .bind(&issue.repo)
            .bind(issue.number as i64)
            .fetch_optional(db)
            .await?;
    Ok(row.and_then(|r| serde_json::from_str(&r.get::<String, _>("pr")).ok()))
}

pub async fn upsert_decision(db: &SqlitePool, t: &TenantId, rec: &DecisionRecord) -> Result<()> {
    sqlx::query("INSERT INTO decisions (tenant_id, repo, number, comment_id, record) VALUES (?, ?, ?, ?, ?) ON CONFLICT(tenant_id, repo, number, comment_id) DO UPDATE SET record = excluded.record")
        .bind(t.as_str()).bind(&rec.issue.repo).bind(rec.issue.number as i64).bind(rec.comment_id as i64).bind(serde_json::to_string(rec)?).execute(db).await?;
    Ok(())
}

/// Forget the decision of one comment, if any: its author is not trusted (ADR 0065).
pub async fn delete_decision(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
    comment_id: u64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM decisions WHERE tenant_id = ? AND repo = ? AND number = ? AND comment_id = ?",
    )
    .bind(t.as_str())
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(comment_id as i64)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn all_decisions(db: &SqlitePool, t: &TenantId) -> Result<Vec<DecisionRecord>> {
    let rows = sqlx::query("SELECT record FROM decisions WHERE tenant_id = ? ORDER BY comment_id")
        .bind(t.as_str())
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

pub async fn upsert_issue_meta(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
    meta: &IssueMeta,
) -> Result<()> {
    sqlx::query("INSERT INTO issue_meta (tenant_id, repo, number, title, github_state, url, fetched_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(tenant_id, repo, number) DO UPDATE SET title = excluded.title, github_state = excluded.github_state, url = excluded.url, fetched_at = excluded.fetched_at")
        .bind(t.as_str()).bind(&issue.repo).bind(issue.number as i64).bind(&meta.title).bind(&meta.github_state).bind(&meta.url).bind(ts(Utc::now())).execute(db).await?;
    Ok(())
}

pub async fn all_issue_meta(db: &SqlitePool, t: &TenantId) -> Result<HashMap<IssueRef, IssueMeta>> {
    let rows = sqlx::query(
        "SELECT repo, number, title, github_state, url FROM issue_meta WHERE tenant_id = ?",
    )
    .bind(t.as_str())
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

pub async fn issue_meta(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
) -> Result<Option<IssueMeta>> {
    let row = sqlx::query(
        "SELECT title, github_state, url FROM issue_meta WHERE tenant_id = ? AND repo = ? AND number = ?",
    )
    .bind(t.as_str()).bind(&issue.repo)
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
/// transaction: running it twice gives the same rows. Suggestions are scored against one
/// workspace's map and kept for `ws` (table `issue_suggestions`, ADR 0047).
pub async fn replace_open_issues(
    db: &SqlitePool,
    t: &TenantId,
    ws: &WsKey,
    repo: &str,
    listing: &OpenIssues,
    placements: &HashMap<u64, Placement>,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM open_issues WHERE tenant_id = ? AND repo = ?")
        .bind(t.as_str())
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM issue_suggestions WHERE tenant_id = ? AND workspace_id = ? AND repo = ?",
    )
    .bind(ws.tenant.as_str())
    .bind(ws.ws())
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
        sqlx::query("INSERT OR REPLACE INTO open_issues (tenant_id, repo, number, title, url, labels, updated_at, map_line, author, author_association) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(t.as_str()).bind(repo).bind(i.number as i64).bind(&i.title).bind(&i.url).bind(serde_json::to_string(&i.labels)?).bind(ts(i.updated_at)).bind(line).bind(&i.author).bind(&i.author_association).execute(&mut *tx).await?;
        if let Some(suggestion) = suggestion {
            sqlx::query("INSERT OR REPLACE INTO issue_suggestions (tenant_id, workspace_id, repo, number, suggestion) VALUES (?, ?, ?, ?, ?)")
                .bind(ws.tenant.as_str())
        .bind(ws.ws()).bind(repo).bind(i.number as i64).bind(suggestion).execute(&mut *tx).await?;
        }
    }
    sqlx::query("INSERT INTO backlog_repos (tenant_id, repo, listed_at, truncated, error) VALUES (?, ?, ?, ?, NULL) ON CONFLICT(tenant_id, repo) DO UPDATE SET listed_at = excluded.listed_at, truncated = excluded.truncated, error = NULL")
        .bind(t.as_str()).bind(repo).bind(ts(now)).bind(listing.truncated).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Who wrote each open issue: its author's login and GitHub's association, as the last listing
/// said. An issue that is closed, or was never listed, is not here.
pub async fn open_issue_authors(
    db: &SqlitePool,
    t: &TenantId,
) -> Result<HashMap<IssueRef, (String, String)>> {
    Ok(sqlx::query(
        "SELECT repo, number, author, author_association FROM open_issues WHERE tenant_id = ?",
    )
    .bind(t.as_str())
    .fetch_all(db)
    .await?
    .iter()
    .map(|r| {
        (
            IssueRef::new(r.get::<String, _>("repo"), r.get::<i64, _>("number") as u64),
            (r.get("author"), r.get("author_association")),
        )
    })
    .collect())
}

/// A listing failed: keep the rows and the last good time, record why.
pub async fn record_listing_error(
    db: &SqlitePool,
    t: &TenantId,
    repo: &str,
    error: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO backlog_repos (tenant_id, repo, listed_at, truncated, error) VALUES (?, ?, NULL, 0, ?) ON CONFLICT(tenant_id, repo) DO UPDATE SET error = excluded.error")
        .bind(t.as_str()).bind(repo).bind(error).execute(db).await?;
    Ok(())
}

/// A repo the map no longer names: its issues and its listing row go.
pub async fn forget_repo(db: &SqlitePool, t: &TenantId, repo: &str) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM open_issues WHERE tenant_id = ? AND repo = ?")
        .bind(t.as_str())
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM backlog_repos WHERE tenant_id = ? AND repo = ?")
        .bind(t.as_str())
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM issue_suggestions WHERE tenant_id = ? AND repo = ?")
        .bind(t.as_str())
        .bind(repo)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Every listed open issue, with `ws`'s suggestion, and every repo's listing state.
pub async fn backlog(
    db: &SqlitePool,
    t: &TenantId,
    ws: &WsKey,
) -> Result<(Vec<OpenIssue>, Vec<BacklogRepo>)> {
    let issues = sqlx::query(
        "SELECT o.repo, o.number, o.title, o.url, o.labels, o.updated_at, o.map_line, s.suggestion FROM open_issues o LEFT JOIN issue_suggestions s ON s.tenant_id = ? AND s.workspace_id = ? AND s.repo = o.repo AND s.number = o.number WHERE o.tenant_id = ? ORDER BY o.repo, o.number",
    )
    .bind(ws.tenant.as_str())
    .bind(ws.ws())
    .bind(t.as_str())
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
        sqlx::query("SELECT repo, listed_at, truncated, error FROM backlog_repos WHERE tenant_id = ? ORDER BY repo")
            .bind(t.as_str()).fetch_all(db)
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
    t: &TenantId,
    repos: &[String],
    now: DateTime<Utc>,
) -> Result<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM uncovered_repos WHERE tenant_id = ?")
        .bind(t.as_str())
        .execute(&mut *tx)
        .await?;
    for repo in repos {
        sqlx::query("INSERT INTO uncovered_repos (tenant_id, repo, seen_at) VALUES (?, ?, ?)")
            .bind(t.as_str())
            .bind(repo)
            .bind(ts(now))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn uncovered(db: &SqlitePool, t: &TenantId) -> Result<Vec<String>> {
    Ok(
        sqlx::query("SELECT repo FROM uncovered_repos WHERE tenant_id = ? ORDER BY repo")
            .bind(t.as_str())
            .fetch_all(db)
            .await?
            .iter()
            .map(|r| r.get("repo"))
            .collect(),
    )
}

// Table `plan_signals` (ADR 0027): `Review:` and `Approved:` comments from members.

/// Store a review or approval signal. True when this comment was not stored before.
pub async fn upsert_signal(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
    comment_id: u64,
    kind: &str,
    record: &str,
) -> Result<bool> {
    let known =
        sqlx::query("SELECT 1 FROM plan_signals WHERE tenant_id = ? AND repo = ? AND number = ? AND comment_id = ?")
            .bind(t.as_str()).bind(&issue.repo)
            .bind(issue.number as i64)
            .bind(comment_id as i64)
            .fetch_optional(db)
            .await?
            .is_some();
    sqlx::query("INSERT INTO plan_signals (tenant_id, repo, number, comment_id, kind, record) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(tenant_id, repo, number, comment_id) DO UPDATE SET kind = excluded.kind, record = excluded.record")
        .bind(t.as_str()).bind(&issue.repo).bind(issue.number as i64).bind(comment_id as i64).bind(kind).bind(record)
        .execute(db).await?;
    Ok(!known)
}

/// Drop the issue's signals whose comment was not seen in this read (deleted, or edited into
/// something else).
pub async fn drop_signals_except(
    db: &SqlitePool,
    t: &TenantId,
    issue: &IssueRef,
    kept: &[u64],
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT comment_id FROM plan_signals WHERE tenant_id = ? AND repo = ? AND number = ?",
    )
    .bind(t.as_str())
    .bind(&issue.repo)
    .bind(issue.number as i64)
    .fetch_all(db)
    .await?;
    for r in rows {
        let id = r.get::<i64, _>("comment_id");
        if !kept.contains(&(id as u64)) {
            sqlx::query(
                "DELETE FROM plan_signals WHERE tenant_id = ? AND repo = ? AND number = ? AND comment_id = ?",
            )
            .bind(t.as_str()).bind(&issue.repo)
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
    t: &TenantId,
    issue: &IssueRef,
    comment_id: u64,
    sha: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE plan_signals SET target_sha = ? WHERE tenant_id = ? AND repo = ? AND number = ? AND comment_id = ?",
    )
    .bind(sha)
    .bind(t.as_str()).bind(&issue.repo)
    .bind(issue.number as i64)
    .bind(comment_id as i64)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn all_reviews(db: &SqlitePool, t: &TenantId) -> Result<Vec<ReviewRecord>> {
    let rows =
        sqlx::query("SELECT record FROM plan_signals WHERE tenant_id = ? AND kind = 'review' ORDER BY comment_id")
            .bind(t.as_str()).fetch_all(db)
            .await?;
    Ok(rows
        .iter()
        .filter_map(|r| serde_json::from_str(&r.get::<String, _>("record")).ok())
        .collect())
}

/// The branch to prefer for each issue, from the merged list of counted approvals the
/// composition root builds (ADR 0032, ADR 0054): the branch of the newest approval given in
/// Rubato, so the handoff commit is read even when the issue has two `issue/` refs; else the
/// branch the newest counted comment names, for issues whose newest one names one. Pure: this
/// slice never names `approve`, which builds the list.
pub fn approved_branches(
    approvals: &[(ApprovalRecord, Option<String>)],
) -> HashMap<IssueRef, String> {
    let key = |a: &ApprovalRecord| (a.created_at, a.comment_id, a.approval_id);
    let mut rubato: HashMap<&IssueRef, &ApprovalRecord> = HashMap::new();
    let mut comment: HashMap<&IssueRef, &ApprovalRecord> = HashMap::new();
    for (a, _) in approvals {
        let newest = if a.source.is_comment() {
            &mut comment
        } else {
            &mut rubato
        };
        match newest.get(&a.issue) {
            Some(n) if key(n) >= key(a) => {}
            _ => {
                newest.insert(&a.issue, a);
            }
        }
    }
    let mut out: HashMap<IssueRef, String> = comment
        .into_iter()
        .filter_map(|(i, a)| a.branch.clone().map(|b| (i.clone(), b)))
        .collect();
    for (i, a) in rubato {
        if let Some(b) = &a.branch {
            out.insert(i.clone(), b.clone());
        }
    }
    out
}

/// Every comment carrying a `Rubato-Approval:` line (ADR 0055).
pub async fn link_comments(
    db: &SqlitePool,
    t: &TenantId,
) -> Result<Vec<super::links::LinkComment>> {
    let rows = sqlx::query(
        "SELECT record FROM plan_signals WHERE tenant_id = ? AND kind = 'link' ORDER BY comment_id",
    )
    .bind(t.as_str())
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| serde_json::from_str(&r.get::<String, _>("record")).ok())
        .collect())
}

/// Every approval, with the plan hash of the branch it names when it names another one.
pub async fn all_approvals(
    db: &SqlitePool,
    t: &TenantId,
) -> Result<Vec<(ApprovalRecord, Option<String>)>> {
    let rows = sqlx::query(
        "SELECT record, target_sha FROM plan_signals WHERE tenant_id = ? AND kind = 'approval' ORDER BY comment_id",
    )
    .bind(t.as_str()).fetch_all(db)
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
