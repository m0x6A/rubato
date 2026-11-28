//! `POST /api/plans/drafts/approve`: a person approves one revision of a draft, as themselves
//! (ADR 0054, plan 122 §D6). Nothing is posted to GitHub; the approval is a `rubato_approvals`
//! row. Refused, with a reason counted on `rubato.web.rejected`, while no cutover is configured,
//! when the version is not the newest, while the owner's session is working, when a bundle file
//! is missing, when an ADR the plan proposes is not pinned at the mirrored file's blob, and when
//! the plan fits none of the issue's workspaces. A second press of the same version answers the
//! first approval.

use super::handlers::fits_another;
use super::records::{self, RubatoApproval};
use crate::auth::CurrentUser;
use crate::kernel::http::Ws;
use crate::kernel::state::App;
use crate::plan_drafts::{self, DraftRow, RevisionRow};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, SubsecRound, Utc};
use rubato_protocol::approve::{reason, routes};
use rubato_protocol::{
    AdrPin, DraftApproveRequest, DraftApproveResponse, IssueRef, SessionState, SessionStatus,
    adr_pins, git_blob_sha, plan_fit,
};
use serde_json::{Value, json};
use uuid::Uuid;

type Refused = (StatusCode, Json<Value>);
type Answer = (StatusCode, Json<DraftApproveResponse>);

fn answer(a: &RubatoApproval, created: bool) -> Answer {
    (
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(DraftApproveResponse {
            approval_id: a.id,
            approved_by: a.approved_by.clone(),
            approved_at: a.approved_at,
            self_approved: a.self_approved,
            plan_sha: a.plan_sha.clone(),
            bundle_hash: a.bundle_hash.clone(),
            created,
        }),
    )
}

/// Whether the owner's session on the draft's branch is in the middle of a turn: working, or
/// blocked mid-turn (a permission prompt, a failed turn), heard from within the stale window. Its
/// files may change under the approval. A session gone quiet reads as unknown, not working.
pub(super) fn owner_mid_turn(
    sessions: &[SessionState],
    issue: &IssueRef,
    draft: &DraftRow,
    now: DateTime<Utc>,
    stale_secs: u64,
) -> bool {
    sessions.iter().any(|s| {
        let fresh = now.signed_duration_since(s.last_seen).num_seconds() <= stale_secs as i64;
        let mid_turn = match s.effective_status(now, stale_secs) {
            SessionStatus::Working => true,
            SessionStatus::Blocked => fresh && s.blocked_by.is_none_or(|c| c.is_mid_turn()),
            _ => false,
        };
        mid_turn
            && s.user.eq_ignore_ascii_case(&draft.owner)
            && s.branch.as_deref() == Some(draft.branch.as_str())
            && s.repo
                .as_deref()
                .is_none_or(|r| r.eq_ignore_ascii_case(&issue.repo))
            && s.issue.as_ref().is_none_or(|i| i == issue)
    })
}

/// The rows of `## ADRs proposed` with no Blob, or whose Blob is not the git blob sha of that
/// file as the revision mirrored it. The handoff would refuse `adr_changed` on any of them.
pub(super) fn unpinned_adrs(rev: &RevisionRow) -> Vec<AdrPin> {
    let Some(plan) = rev.files.first() else {
        return vec![];
    };
    adr_pins(&plan.text)
        .into_iter()
        .filter(|p| {
            let file = p.file.trim_start_matches("./");
            match &p.blob {
                None => true,
                Some(blob) => rev
                    .files
                    .iter()
                    .find(|f| f.path == file)
                    .is_none_or(|f| git_blob_sha(f.text.as_bytes()) != *blob),
            }
        })
        .collect()
}

pub async fn approve_draft(
    State(app): State<App>,
    ws: Ws,
    CurrentUser(login): CurrentUser,
    Json(req): Json<DraftApproveRequest>,
) -> Result<Answer, Refused> {
    let t = ws.key().tenant();
    let refuse = |status: StatusCode, why: &str, extra: Value| {
        app.reject(routes::PLAN_DRAFT_APPROVE, why, &login);
        let mut v = json!({ "reason": why });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        (status, Json(v))
    };
    let internal = |e: anyhow::Error| {
        tracing::error!(slice = "approve", error = %format!("{e:#}"), "approval in Rubato failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "reason": "internal" })),
        )
    };
    if app.config.comment_approvals_until.is_none() {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::CUTOVER_UNSET,
            json!({}),
        ));
    }
    let Some(map) = app.map_of(ws.key()).await else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "no_map", json!({})));
    };
    let issue = IssueRef::new(req.repo.clone(), req.number);
    let found = plan_drafts::on_branch(&app, &issue, &req.branch)
        .await
        .map_err(internal)?;
    let Some((draft, Some(rev))) = found else {
        return Err(refuse(StatusCode::NOT_FOUND, reason::NO_DRAFT, json!({})));
    };
    if !rev.info.plan_sha.eq_ignore_ascii_case(&req.plan_sha)
        || req.rev.is_some_and(|r| r != rev.info.rev)
    {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::NOT_LATEST,
            json!({ "current_sha": rev.info.plan_sha, "current_rev": rev.info.rev }),
        ));
    }
    // This version approved already: the first approval stands, whatever the session does now.
    if let Some(existing) = records::find(
        &app.db,
        t,
        &issue,
        &draft.branch,
        &rev.info.plan_sha,
        &rev.info.bundle_hash,
    )
    .await
    .map_err(internal)?
    {
        return Ok(answer(&existing, false));
    }
    let now = app.clock.now();
    if owner_mid_turn(
        &app.latest_sessions(),
        &issue,
        &draft,
        now,
        app.config.limits.session_stale_secs,
    ) {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::NOT_SETTLED,
            json!({ "owner": draft.owner }),
        ));
    }
    if !rev.info.missing.is_empty() {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::BUNDLE_INCOMPLETE,
            json!({ "missing": rev.info.missing }),
        ));
    }
    let unpinned = unpinned_adrs(&rev);
    if !unpinned.is_empty() {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::ADRS_UNPINNED,
            json!({ "adrs": unpinned }),
        ));
    }
    let parse_error = rev.info.parse_error.as_deref();
    let fit = plan_fit(rev.plan.as_ref(), parse_error, &map.map).unwrap_or_else(|| {
        // Neither a plan nor a parse error: nothing to approve.
        rubato_protocol::PlanFit {
            fits: false,
            reasons: vec![rubato_protocol::FitReason::ParseError {
                message: "the revision holds no plan".into(),
            }],
        }
    });
    if !fit.fits
        && !fits_another(&app, ws.as_str(), &issue, rev.plan.as_ref(), parse_error)
            .await
            .map_err(internal)?
    {
        return Err(refuse(
            StatusCode::CONFLICT,
            rubato_protocol::approve::reason::PLAN_DOES_NOT_FIT,
            json!({ "reasons": fit.reasons }),
        ));
    }
    let approval = RubatoApproval {
        id: Uuid::new_v4(),
        issue: issue.clone(),
        branch: draft.branch.clone(),
        draft_id: draft.id,
        rev: rev.info.rev,
        plan_sha: rev.info.plan_sha.clone(),
        bundle_hash: rev.info.bundle_hash.clone(),
        approved_by: login.clone(),
        // Whole seconds: the time the handoff writes into the plan is the same value.
        approved_at: now.trunc_subsecs(0),
        self_approved: login.eq_ignore_ascii_case(&draft.owner),
        summary: req.summary.clone(),
        handoff: None,
    };
    let created = records::insert(&app.db, t, &approval)
        .await
        .map_err(internal)?;
    let stored = records::find(
        &app.db,
        t,
        &issue,
        &draft.branch,
        &rev.info.plan_sha,
        &rev.info.bundle_hash,
    )
    .await
    .map_err(internal)?
    .ok_or_else(|| internal(anyhow::anyhow!("approval row vanished")))?;
    if created {
        tracing::info!(
            rubato.issue = %issue,
            rubato.branch = %stored.branch,
            rubato.approval_id = %stored.id,
            rubato.draft.rev = stored.rev,
            rubato.user = %login,
            self_approved = stored.self_approved,
            "plan approved in Rubato"
        );
        let plan_path = rev.files.first().map(|f| f.path.as_str()).unwrap_or("");
        super::deliver::on_recorded(&app, &stored, &draft.owner, plan_path).await;
        app.notify("approve");
    }
    Ok(answer(&stored, created))
}
