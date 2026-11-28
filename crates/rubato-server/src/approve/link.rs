//! After the handoff (ADR 0055, plan 122 §D7): the plan read at the branch head is compared with
//! each Rubato approval on every tick.
//!
//! | The plan at the head | Outcome |
//! | --- | --- |
//! | keys equal to the record, hash equal to its plan sha | the link comment, once, as the approver |
//! | keys equal, hash changed (edited after the handoff) | shown as stale, nothing posted |
//! | keys that are no record of that branch | `ApprovalRecordMismatch`, nothing posted |
//!
//! A link comment whose outcome is unknown stays `opening` until the comments pass finds its
//! `Rubato-Approval:` line from the approver; it is never posted again on its own. Everything
//! here is derived on each read; nothing but `plan_links` is stored, and all of it is off while
//! no cutover is configured.

use super::links::{self, LinkRow, LinkState};
use super::records::{self, RubatoApproval};
use crate::auth::{self, TokenError};
use crate::kernel::github::PostError;
use crate::kernel::state::App;
use crate::kernel::tenancy::TenantId;
use crate::plans::PlanRow;
use crate::{github_poll, plan_drafts, plans, session_messages};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rubato_protocol::otel::span;
use rubato_protocol::{Finding, HandoffOutcome, PlanApprovalRecord};
use std::collections::{HashMap, HashSet};
use tracing::Instrument;
use uuid::Uuid;

/// A delivered approval with no handoff after this long is overdue (alert `ApprovalNoHandoff`).
pub const HANDOFF_OVERDUE_SECS: i64 = 1800;
/// A link comment still `opening` after this long is `LinkCommentWaiting`.
pub const LINK_WAITING_SECS: i64 = 1800;

/// Why the plan on a branch disagrees with Rubato (`ApprovalRecordMismatch.reason`).
pub mod mismatch {
    /// The keys name no approval Rubato recorded for that issue and branch.
    pub const NO_RUBATO_RECORD: &str = "no_rubato_record";
    /// Rubato has approvals of that branch, and the keys equal none of them.
    pub const RECORD_DIFFERS: &str = "record_differs";
}

/// Everything the assessment reads.
pub struct LinkFacts<'a> {
    pub approvals: &'a [RubatoApproval],
    /// Each draft's newest revision's plan sha, by draft id.
    pub newest: &'a HashMap<Uuid, String>,
    pub plans: &'a [PlanRow],
    pub links: &'a HashMap<Uuid, LinkRow>,
    /// When each approval's `plan_approved` was typed into its session.
    pub delivered: &'a HashMap<Uuid, DateTime<Utc>>,
    /// Approvers Rubato can act as (lower case).
    pub can_act: &'a HashSet<String>,
    pub now: DateTime<Utc>,
}

/// A link comment to post now: the approval, the commit the plan was read at, the plan's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub approval: RubatoApproval,
    pub commit: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Assessment {
    pub findings: Vec<Finding>,
    /// `rubato.plans.handoff_overdue`.
    pub overdue: u64,
    /// `rubato.plans.link_comment_missing`.
    pub link_missing: u64,
    /// `rubato.plans.approval_record_mismatch`.
    pub mismatch: u64,
    pub due: Vec<Due>,
}

/// The plan's keys are this approval's record: same approver (case aside), same second, same
/// plan sha.
pub fn keys_match(keys: &PlanApprovalRecord, a: &RubatoApproval) -> bool {
    let at = keys
        .approved_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t.trim()).ok());
    keys.approved_by
        .as_deref()
        .is_some_and(|b| b.trim().eq_ignore_ascii_case(&a.approved_by))
        && at.is_some_and(|t| t.timestamp() == a.approved_at.timestamp())
        && keys
            .approved_sha
            .as_deref()
            .is_some_and(|s| s.trim().eq_ignore_ascii_case(&a.plan_sha))
}

/// Pure: findings, gauges and the link comments due, from the facts.
pub fn assess(f: &LinkFacts) -> Assessment {
    let mut out = Assessment::default();
    // What the plan at each branch head says: per approval, handed off (its keys are there) and
    // whether the text still hashes to it (a matching commit).
    let mut at_head: HashMap<Uuid, (bool, &PlanRow)> = HashMap::new();
    for row in f.plans {
        let Some(keys) = row.plan.as_ref().and_then(|p| p.approval.as_ref()) else {
            continue;
        };
        let of_branch: Vec<&RubatoApproval> = f
            .approvals
            .iter()
            .filter(|a| a.issue == row.issue && a.branch == row.branch)
            .collect();
        match of_branch.iter().find(|a| keys_match(keys, a)) {
            Some(a) => {
                let same = row
                    .approval_sha
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(&a.plan_sha));
                at_head.insert(a.id, (same, row));
            }
            None => {
                out.mismatch += 1;
                out.findings.push(Finding::ApprovalRecordMismatch {
                    issue: row.issue.clone(),
                    branch: row.branch.clone(),
                    reason: if of_branch.is_empty() {
                        mismatch::NO_RUBATO_RECORD
                    } else {
                        mismatch::RECORD_DIFFERS
                    }
                    .to_owned(),
                });
            }
        }
    }
    // The newest approval of each branch: only its ADRs left are still news.
    let mut newest_of_branch: HashMap<(&rubato_protocol::IssueRef, &str), &RubatoApproval> =
        HashMap::new();
    for a in f.approvals {
        let k = (&a.issue, a.branch.as_str());
        if newest_of_branch
            .get(&k)
            .is_none_or(|n| (n.approved_at, n.id) < (a.approved_at, a.id))
        {
            newest_of_branch.insert(k, a);
        }
    }
    for a in f.approvals {
        let link = f.links.get(&a.id);
        let head = at_head.get(&a.id);
        let matched = head.is_some_and(|(same, _)| *same);
        let posted = link.is_some_and(|l| l.state == LinkState::Posted);
        let opening = link.filter(|l| l.state == LinkState::Opening);
        if (matched && !posted) || opening.is_some() {
            out.link_missing += 1;
        }
        if let Some(l) = opening
            && f.now - l.created_at > Duration::seconds(LINK_WAITING_SECS)
        {
            out.findings.push(Finding::LinkCommentWaiting {
                issue: a.issue.clone(),
                approval_id: a.id,
            });
        }
        if matched && link.is_none() {
            let (_, row) = head.expect("matched has a head");
            match &row.head_sha {
                Some(commit) if f.can_act.contains(&a.approved_by.to_ascii_lowercase()) => {
                    out.due.push(Due {
                        approval: a.clone(),
                        commit: commit.clone(),
                        path: row.path.clone(),
                    });
                }
                Some(_) => out.findings.push(Finding::LinkWaitingForApprover {
                    issue: a.issue.clone(),
                    approver: a.approved_by.clone(),
                }),
                // Read before the head sha was kept: the next read of the branch fills it.
                None => {}
            }
        }
        // Overdue: typed into the session long enough ago, still the newest revision and the
        // newest approval of its branch, not on the branch, and no report that it committed or
        // that what it approved changed under it (plan, bundle or a pinned ADR: nothing was
        // written, and the approval reads as for an earlier version until a person approves
        // again). A superseded approval is never overdue.
        let current = f
            .newest
            .get(&a.draft_id)
            .is_some_and(|s| s.eq_ignore_ascii_case(&a.plan_sha))
            && newest_of_branch
                .get(&(&a.issue, a.branch.as_str()))
                .is_some_and(|n| n.id == a.id);
        let reported = a.handoff.as_ref().is_some_and(|h| {
            [
                HandoffOutcome::Committed,
                HandoffOutcome::PlanChanged,
                HandoffOutcome::BundleChanged,
                HandoffOutcome::AdrChanged,
            ]
            .iter()
            .any(|o| h.outcome == o.name())
        });
        let delivered_long_ago = f
            .delivered
            .get(&a.id)
            .is_some_and(|t| f.now - *t > Duration::seconds(HANDOFF_OVERDUE_SECS));
        if delivered_long_ago && current && head.is_none() && !reported {
            out.overdue += 1;
        }
        if let Some(h) = &a.handoff
            && !h.adrs_left.is_empty()
            && newest_of_branch
                .get(&(&a.issue, a.branch.as_str()))
                .is_some_and(|n| n.id == a.id)
        {
            out.findings.push(Finding::AdrsNotAccepted {
                issue: a.issue.clone(),
                adrs: h.adrs_left.clone(),
            });
        }
    }
    out
}

/// The facts as stored now, assessed. Empty while no cutover is configured.
pub async fn current(app: &App, t: &TenantId) -> Result<Assessment> {
    if app.config.comment_approvals_until.is_none() {
        return Ok(Assessment::default());
    }
    let approvals = records::all(&app.db, t).await?;
    let newest: HashMap<Uuid, String> = plan_drafts::latest_drafts(app, t)
        .await?
        .into_iter()
        .map(|d| (d.summary.draft_id, d.summary.plan_sha))
        .collect();
    let plans = plans::all(&app.db, t).await?;
    let links = links::all(&app.db, t).await?;
    let delivered = session_messages::plan_approved_delivered(&app.db, t).await?;
    let mut can_act = HashSet::new();
    for login in approvals
        .iter()
        .map(|a| a.approved_by.to_ascii_lowercase())
        .collect::<HashSet<_>>()
    {
        if auth::can_act(app, &login).await.is_ok() {
            can_act.insert(login);
        }
    }
    Ok(assess(&LinkFacts {
        approvals: &approvals,
        newest: &newest,
        plans: &plans,
        links: &links,
        delivered: &delivered,
        can_act: &can_act,
        now: app.clock.now(),
    }))
}

/// The findings for the overview.
pub async fn link_findings(app: &App, t: &TenantId) -> Result<Vec<Finding>> {
    Ok(current(app, t).await?.findings)
}

/// The link comment's body (§D7). The login is written without `@`, so nobody is notified.
pub fn link_body(github_web: &str, a: &RubatoApproval, commit: &str, path: &str) -> String {
    let short = &commit[..commit.len().min(7)];
    let mut body = format!(
        "Plan approved in Rubato: [{path} at {short}]({}/{}/blob/{commit}/{path})\nApproved by {} at {}.\n\n",
        github_web.trim_end_matches('/'),
        a.issue.repo,
        a.approved_by.trim_start_matches('@'),
        a.approved_at.format("%Y-%m-%d %H:%M UTC"),
    );
    if !a.summary.trim().is_empty() {
        body.push_str(a.summary.trim());
        body.push_str("\n\n");
    }
    body.push_str(&format!("{} {}\n", github_poll::LINK_LINE, a.id));
    body
}

/// One tick: settle `opening` links the comments pass found, post the ones due, record the
/// three gauges. Every tick, so a handoff's link comment does not wait for the full lane; an
/// approver with no token is asked again each tick at the cost of one database read.
pub async fn reconcile_links(app: &App, t: &TenantId) {
    if let Err(e) = settle_found(app, t).await {
        tracing::error!(error = %format!("{e:#}"), "settling link comments failed");
    }
    let assessed = match current(app, t).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading approvals for link comments failed");
            return;
        }
    };
    let mut posted = 0u64;
    for d in &assessed.due {
        if post_link(app, t, d).await {
            posted += 1;
        }
    }
    // The gauges count what remains after this tick's posts.
    let link_missing = assessed.link_missing.saturating_sub(posted);
    app.metrics
        .plans_handoff_overdue
        .record(assessed.overdue, &[]);
    app.metrics
        .plans_link_comment_missing
        .record(link_missing, &[]);
    app.metrics
        .plans_approval_record_mismatch
        .record(assessed.mismatch, &[]);
}

/// `opening` rows whose comment the comments pass found from the approver become `posted`.
async fn settle_found(app: &App, t: &TenantId) -> Result<()> {
    let links = links::all(&app.db, t).await?;
    if !links.values().any(|l| l.state == LinkState::Opening) {
        return Ok(());
    }
    let found = github_poll::link_comments(&app.db, t).await?;
    for l in links.values().filter(|l| l.state == LinkState::Opening) {
        let Some(a) = records::get(&app.db, t, l.approval_id).await? else {
            continue;
        };
        if let Some(c) = found.iter().find(|c| {
            c.approval_id == a.id
                && c.issue == a.issue
                && c.author.eq_ignore_ascii_case(&a.approved_by)
        }) {
            links::posted(&app.db, t, a.id, c.comment_id, &c.url, app.clock.now()).await?;
            tracing::info!(rubato.issue = %a.issue, rubato.approval_id = %a.id, comment_id = c.comment_id, "link comment found in the comments");
            app.notify("link comment");
        }
    }
    Ok(())
}

/// Post one link comment as the approver. Claimed first, so it is posted at most once.
async fn post_link(app: &App, t: &TenantId, d: &Due) -> bool {
    let a = &d.approval;
    let token = match auth::access_token(app, &a.approved_by).await {
        Ok(t) => t,
        Err(TokenError::NotStored | TokenError::Rejected) => {
            tracing::debug!(rubato.approval_id = %a.id, approver = %a.approved_by, "link comment waits for the approver to log in");
            return false;
        }
        Err(TokenError::Failed(e)) => {
            tracing::warn!(rubato.approval_id = %a.id, error = %format!("{e:#}"), "link comment: the approver's token is unavailable");
            return false;
        }
    };
    let now = app.clock.now();
    match links::open(&app.db, t, a.id, &d.commit, now).await {
        Ok(true) => {}
        Ok(false) => return false,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "link comment: claiming failed");
            return false;
        }
    }
    let body = link_body(app.config.github_web(), a, &d.commit, &d.path);
    let span = tracing::info_span!(
        span::PLAN_LINK_COMMENT,
        rubato.issue = %a.issue,
        rubato.approval_id = %a.id,
        rubato.outcome = tracing::field::Empty
    );
    let posted = app
        .github
        .as_person()
        .comment_as(&token, &a.issue.repo, a.issue.number, &body)
        .instrument(span.clone())
        .await;
    match posted {
        Ok(c) => {
            span.record("rubato.outcome", "posted");
            if let Err(e) = links::posted(&app.db, t, a.id, c.id, &c.url, now).await {
                tracing::error!(error = %format!("{e:#}"), "link comment: recording failed");
            }
            tracing::info!(rubato.issue = %a.issue, rubato.approval_id = %a.id, commit = %d.commit, url = %c.url, "link comment posted");
            app.notify("link comment");
            true
        }
        Err(PostError::Refused { status }) => {
            span.record("rubato.outcome", "refused");
            if let Err(e) = links::forget(&app.db, t, a.id).await {
                tracing::error!(error = %format!("{e:#}"), "link comment: dropping the claim failed");
            }
            tracing::warn!(rubato.issue = %a.issue, rubato.approval_id = %a.id, status, "GitHub refused the link comment");
            false
        }
        Err(PostError::Unknown(e)) => {
            span.record("rubato.outcome", "unknown");
            tracing::warn!(rubato.issue = %a.issue, rubato.approval_id = %a.id, error = %e, "link comment unknown");
            false
        }
    }
}
