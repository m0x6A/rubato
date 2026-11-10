//! Per-issue reads of GitHub: issue metadata, the issue branch, its PR and checks, `Decision:`
//! comments (ADR 0014); and the rate limit. `tasks::poll_once` calls these in order (design §9.1).

use super::store::{self, IssueMeta};
use crate::kernel::state::App;
use anyhow::Result;
use rubato_protocol::{ChecksStatus, DecisionRecord, IssueRef, PullRequestState};

pub async fn upsert_meta(app: &App, issue: &IssueRef) -> Result<()> {
    if let Some(info) = app.github.issue(&issue.repo, issue.number).await? {
        store::upsert_issue_meta(
            &app.db,
            issue,
            &IssueMeta {
                title: Some(info.title),
                url: Some(info.url),
                github_state: Some(info.state),
            },
        )
        .await?;
    }
    Ok(())
}

/// The branch `issue/<n>-<slug>` for this issue, if one exists.
pub async fn find_branch(app: &App, issue: &IssueRef) -> Result<Option<String>> {
    let branches = app
        .github
        .branches_with_prefix(&issue.repo, &format!("issue/{}-", issue.number))
        .await?;
    Ok(branches
        .iter()
        .find(|b| IssueRef::number_from_branch(b) == Some(issue.number))
        .cloned())
}

/// The newest PR for the branch; checks only while it is not merged.
pub async fn poll_pr(app: &App, issue: &IssueRef, branch: &str) -> Result<()> {
    if let Some(pr) = app.github.pull_for_branch(&issue.repo, branch).await? {
        let checks = if pr.merged_at.is_some() {
            ChecksStatus::Green
        } else {
            app.github.checks(&issue.repo, &pr.head_sha).await?
        };
        store::upsert_pr(
            &app.db,
            issue,
            &PullRequestState {
                repo: issue.repo.clone(),
                number: pr.number,
                branch: branch.to_owned(),
                open: pr.open,
                draft: pr.draft,
                checks,
                merged_at: pr.merged_at,
                updated_at: pr.updated_at,
                url: pr.url,
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn poll_decisions(app: &App, issue: &IssueRef) -> Result<()> {
    for c in app.github.issue_comments(&issue.repo, issue.number).await? {
        if let Some(body) = DecisionRecord::parse_body(&c.body) {
            store::upsert_decision(
                &app.db,
                &DecisionRecord {
                    issue: issue.clone(),
                    comment_id: c.id,
                    author: c.author,
                    title: body.title,
                    context: body.context,
                    options: body.options,
                    chosen: body.chosen,
                    why: body.why,
                    created_at: c.created_at,
                    url: c.url,
                },
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn record_rate_limit(app: &App) {
    match app.github.rate_limit().await {
        Ok((remaining, limit)) => {
            app.metrics.rate_limit_remaining.record(remaining, &[]);
            app.metrics.rate_limit_limit.record(limit, &[]);
            if limit > 0 && remaining * 10 < limit {
                tracing::warn!(remaining, limit, "GitHub rate limit under 10%");
            }
        }
        Err(e) => tracing::debug!(error = %e, "rate limit read failed"),
    }
}
