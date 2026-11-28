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
use crate::kernel::tenancy::TenantId;
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
    let t = &app.repo_tenant(&issue.repo)?;
    let Some(d) = store::find(&app.db, t, issue, branch).await? else {
        return Ok(None);
    };
    let latest = store::latest(&app.db, t, d.id).await?;
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
pub async fn restore_of(app: &App, t: &TenantId, draft_id: Uuid) -> Result<Option<DraftRestore>> {
    Ok(store::latest(&app.db, t, draft_id)
        .await?
        .map(|r| DraftRestore {
            draft_id,
            rev: r.info.rev,
            files: r.files,
        }))
}

/// The plan bundle of a branch as Import read it at the head (ADR 0058).
pub struct ImportedBundle {
    pub issue: IssueRef,
    pub branch: String,
    pub owner: String,
    pub original_author: Option<String>,
    /// The plan first, then the bundle paths found at the head.
    pub files: Vec<rubato_protocol::DraftFile>,
    pub missing: Vec<String>,
}

/// Why an import made no draft.
#[derive(Debug, PartialEq, Eq)]
pub enum ImportRefused {
    /// The branch has a draft already.
    AlreadyInRubato,
}

/// Make the imported bundle a new draft owned by the importer, its files revision 1 (ADR 0058).
pub async fn import(
    app: &App,
    t: &TenantId,
    b: ImportedBundle,
) -> Result<std::result::Result<(Uuid, u32), ImportRefused>> {
    let now = app.clock.now();
    let Some(draft_id) = store::create_imported(
        &app.db,
        t,
        &b.issue,
        &b.branch,
        &b.owner,
        b.original_author.as_deref(),
        now,
    )
    .await?
    else {
        return Ok(Err(ImportRefused::AlreadyInRubato));
    };
    let plan_file = b
        .files
        .first()
        .cloned()
        .unwrap_or(rubato_protocol::DraftFile {
            path: String::new(),
            text: String::new(),
        });
    let (plan, parse_error) = match crate::plans::parse_plan(&plan_file.text, &plan_file.path) {
        Ok(p) => (Some(p), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let texts: Vec<(String, String)> = b
        .files
        .iter()
        .map(|f| (f.path.clone(), f.text.clone()))
        .collect();
    let rev = store::RevisionRow {
        info: rubato_protocol::DraftRevisionInfo {
            rev: 0,
            at: now,
            plan_sha: rubato_protocol::approval_hash(&plan_file.text),
            bundle_hash: rubato_protocol::bundle_hash(&plan_file.path, &texts),
            session_id: None,
            missing: b.missing,
            parse_error,
        },
        launch_id: None,
        files: b.files,
        plan,
    };
    let n = store::insert_revision(&app.db, t, draft_id, &rev)
        .await?
        .unwrap_or(1);
    app.notify_in(t, "draft");
    Ok(Ok((draft_id, n)))
}

/// Every draft with at least one revision.
pub async fn latest_drafts(app: &App, t: &TenantId) -> Result<Vec<LatestDraft>> {
    let mut out = Vec::new();
    for d in store::all(&app.db, t).await? {
        let Some(r) = store::latest(&app.db, t, d.id).await? else {
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
                imported_from: d.imported_from.clone(),
                original_author: d.original_author.clone(),
                // Derived on each overview build, against that workspace's map.
                fits: None,
            },
            plan: r.plan,
            conflict: d.conflict,
        });
    }
    Ok(out)
}
