//! One list of approvals (ADR 0054, plan 122 §D6): `Approved:` comments, filtered by the
//! cutover, and approvals given in Rubato. The overview's approval state, `ApprovalStale`, the
//! `rubato.plans.approval_stale` gauge and the poll's `BranchPrefs` all read it. The newest
//! *counted* approval decides, so a comment that no longer counts never hides a current one.

use super::records::{self, RubatoApproval};
use crate::github_poll;
use crate::kernel::state::App;
use crate::plan_drafts;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{ApprovalRecord, ApprovalSource};
use std::collections::HashMap;
use uuid::Uuid;

/// The merged list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Approvals {
    /// Counted approvals, each with the plan hash to compare it with when the plan is not read
    /// from the branch it names: for a comment, the hash `github_poll` read on that branch; for
    /// a Rubato approval, the newest revision of its draft, so that before handoff it is current
    /// while its plan sha is the newest revision's.
    pub counted: Vec<(ApprovalRecord, Option<String>)>,
    /// `Approved:` comments posted at or after the cutover: not counted, shown as the finding
    /// `CommentApprovalIgnored`.
    pub ignored: Vec<ApprovalRecord>,
}

/// A Rubato approval as an entry of the list.
pub fn record_of(a: &RubatoApproval) -> ApprovalRecord {
    ApprovalRecord {
        issue: a.issue.clone(),
        source: ApprovalSource::Rubato,
        comment_id: None,
        approval_id: Some(a.id),
        author: a.approved_by.clone(),
        sha: Some(a.plan_sha.clone()),
        branch: Some(a.branch.clone()),
        note: None,
        created_at: a.approved_at,
        url: String::new(),
    }
}

/// Merge comment approvals (with their stored target hash) and Rubato approvals (with the plan
/// hash of their draft's newest revision) under `cutover`. Pure.
pub fn merge(
    comments: Vec<(ApprovalRecord, Option<String>)>,
    rubato: Vec<(RubatoApproval, Option<String>)>,
    cutover: Option<DateTime<Utc>>,
) -> Approvals {
    let mut out = Approvals::default();
    for (a, target) in comments {
        if a.counts(cutover) {
            out.counted.push((a, target));
        } else {
            out.ignored.push(a);
        }
    }
    out.counted
        .extend(rubato.iter().map(|(a, t)| (record_of(a), t.clone())));
    out
}

/// The merged list as stored now.
pub async fn approvals(app: &App) -> Result<Approvals> {
    let comments = github_poll::all_approvals(&app.db).await?;
    let newest: HashMap<Uuid, String> = plan_drafts::latest_drafts(app)
        .await?
        .into_iter()
        .map(|d| (d.summary.draft_id, d.summary.plan_sha))
        .collect();
    let rubato = records::all(&app.db)
        .await?
        .into_iter()
        .map(|a| {
            let target = newest.get(&a.draft_id).cloned();
            (a, target)
        })
        .collect();
    Ok(merge(comments, rubato, app.config.comment_approvals_until))
}
