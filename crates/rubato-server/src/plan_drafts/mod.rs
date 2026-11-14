//! Slice `plan_drafts`: plan drafts that live in Rubato until approval, mirrored from launched
//! sessions' worktrees at their settle points, one revision per changed bundle (ADR 0053, plan
//! 122 design §D4). Owns tables `plan_drafts` and `plan_draft_revisions`. Names only `plans`
//! (the parser), so `launch` and `import` can name it.

mod handlers;
mod intake;
mod store;

#[cfg(test)]
mod tests;

use crate::kernel::state::App;
use anyhow::Result;
use chrono::{DateTime, Utc};
use rubato_protocol::{DraftRestore, DraftSummary, IssueRef, Plan};
use uuid::Uuid;

pub use handlers::{draft, revision};
pub use intake::on_draft;
pub use store::{DraftRow, RevisionRow};

/// The draft on `branch` of `issue`, with its newest revision (files included) when it has one:
/// the version an approval names (ADR 0054).
pub async fn on_branch(
    app: &App,
    issue: &IssueRef,
    branch: &str,
) -> Result<Option<(DraftRow, Option<RevisionRow>)>> {
    let Some(d) = store::find(&app.db, issue, branch).await? else {
        return Ok(None);
    };
    let latest = store::latest(&app.db, d.id).await?;
    Ok(Some((d, latest)))
}

/// One draft and its newest settled revision, for the overview.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestDraft {
    pub issue: IssueRef,
    /// `live` is false here; the overview derives it from the sessions.
    pub summary: DraftSummary,
    /// The revision's plan, when it parses: what counts as a written plan and on seams.
    pub plan: Option<Plan>,
    /// The last bundle refused because someone else owns the draft, and when.
    pub conflict: Option<(String, DateTime<Utc>)>,
}

/// A draft's newest settled revision as a launch carries it (ADR 0053, plan 122 §D5).
pub async fn restore_of(app: &App, draft_id: Uuid) -> Result<Option<DraftRestore>> {
    Ok(store::latest(&app.db, draft_id)
        .await?
        .map(|r| DraftRestore {
            draft_id,
            rev: r.info.rev,
            files: r.files,
        }))
}

/// Every draft with at least one revision.
pub async fn latest_drafts(app: &App) -> Result<Vec<LatestDraft>> {
    let mut out = Vec::new();
    for d in store::all(&app.db).await? {
        let Some(r) = store::latest(&app.db, d.id).await? else {
            continue;
        };
        out.push(LatestDraft {
            issue: d.issue.clone(),
            summary: DraftSummary {
                draft_id: d.id,
                branch: d.branch.clone(),
                owner: d.owner.clone(),
                rev: r.info.rev,
                at: r.info.at,
                plan_sha: r.info.plan_sha.clone(),
                bundle_hash: r.info.bundle_hash.clone(),
                missing: r.info.missing.clone(),
                parse_error: r.info.parse_error.clone(),
                live: false,
            },
            plan: r.plan,
            conflict: d.conflict,
        });
    }
    Ok(out)
}
