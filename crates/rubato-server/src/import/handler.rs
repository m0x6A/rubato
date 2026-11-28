//! `POST /api/plans/import` (ADR 0058), map-scoped for the fit check.
//!
//! - The plan on the branch parses and fits this workspace: a draft owned by the importer, its
//!   files at the head as revision 1, the last committer of the plan file kept as its author.
//! - Otherwise: a session of the importer's, on that branch, rewrites the plan into Rubato's form.
//!
//! Refused, counted on `rubato.web.rejected`, when the branch has a draft, when an approval
//! counts for the issue's plan, and when the branch or the plan is not there. Nothing is sent to
//! the original author's session.

use crate::auth::CurrentUser;
use crate::kernel::http::Ws;
use crate::kernel::state::App;
use crate::{launch, plan_drafts, plans};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rubato_protocol::plan_draft::{import_reason as reason, routes};
use rubato_protocol::{
    DraftFile, ImportRequest, ImportResponse, IssueRef, LaunchRequest, bundle_paths, plan_fit,
};
use serde_json::{Value, json};
use tracing::Instrument as _;

type Refused = (StatusCode, Json<Value>);

/// What a rewrite session is told on top of the launch prompt.
fn rewrite_note(branch: &str, path: &str, reasons: &[String]) -> String {
    format!(
        "Rubato: the plan at {path} on {branch} was written outside Rubato and cannot be approved as it stands ({}). Rewrite it into Rubato's form with the rubato skill, keeping its decisions and its intent; the original is the file on the branch.",
        if reasons.is_empty() {
            "it does not fit the map".to_owned()
        } else {
            reasons.join("; ")
        }
    )
}

pub async fn import(
    State(app): State<App>,
    ws: Ws,
    CurrentUser(login): CurrentUser,
    Json(req): Json<ImportRequest>,
) -> Result<(StatusCode, Json<ImportResponse>), Refused> {
    let refuse = |status: StatusCode, why: &str, extra: Value| {
        app.reject(routes::PLAN_IMPORT, why, &login);
        let mut v = json!({ "reason": why });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        (status, Json(v))
    };
    let internal = |e: anyhow::Error| {
        tracing::error!(error = %format!("{e:#}"), "import failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "reason": "internal" })),
        )
    };
    let issue = IssueRef::new(req.repo.clone(), req.number);
    let prefix = format!("issue/{}-", issue.number);
    if req
        .branch
        .strip_prefix(&prefix)
        .is_none_or(|s| s.is_empty())
    {
        return Err(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::BAD_BRANCH,
            json!({}),
        ));
    }
    let Some(map) = app.map_of(ws.key()).await else {
        return Err(refuse(StatusCode::SERVICE_UNAVAILABLE, "no_map", json!({})));
    };
    if plan_drafts::latest_drafts(&app, ws.key().tenant())
        .await
        .map_err(internal)?
        .iter()
        .any(|d| d.issue == issue && d.summary.branch == req.branch)
    {
        return Err(refuse(
            StatusCode::CONFLICT,
            reason::ALREADY_IN_RUBATO,
            json!({}),
        ));
    }
    // Counted approvals, both sources: a plan approved by comment before the cutover, or in
    // Rubato, is not outside Rubato.
    let approvals = crate::approve::approvals(&app, ws.key().tenant())
        .await
        .map_err(internal)?;
    if approvals.counted.iter().any(|(a, _)| a.issue == issue) {
        return Err(refuse(StatusCode::CONFLICT, reason::APPROVED, json!({})));
    }
    let Some(head) = app
        .github
        .unscoped(&issue.repo)
        .branch_sha(&issue.repo, &req.branch)
        .await
        .map_err(internal)?
    else {
        return Err(refuse(StatusCode::NOT_FOUND, reason::NO_BRANCH, json!({})));
    };
    let path = plans::plan_path(&map, &issue, &req.branch);
    let Some(text) = app
        .github
        .unscoped(&issue.repo)
        .file(&issue.repo, &path, &head)
        .instrument(crate::kernel::github::part_read(
            crate::kernel::github::READS_PLAN,
        ))
        .await
        .map_err(internal)?
    else {
        return Err(refuse(
            StatusCode::NOT_FOUND,
            reason::NO_PLAN,
            json!({ "path": path }),
        ));
    };
    let parsed = plans::parse_plan(&text, &path);
    let fit = plan_fit(
        parsed.as_ref().ok(),
        parsed.as_ref().err().map(|e| e.to_string()).as_deref(),
        &map.map,
    );
    if fit.as_ref().is_some_and(|f| f.fits) {
        // The bundle as it is at the head: the plan, then each path it names that exists.
        let mut files = vec![DraftFile {
            path: path.clone(),
            text: text.clone(),
        }];
        let mut missing = Vec::new();
        for p in bundle_paths(&text, &path).into_iter().skip(1) {
            match app
                .github
                .unscoped(&issue.repo)
                .file(&issue.repo, &p, &head)
                .await
                .map_err(internal)?
            {
                Some(t) => files.push(DraftFile { path: p, text: t }),
                None => missing.push(p),
            }
        }
        let original_author = match app
            .github
            .unscoped(&issue.repo)
            .last_commit_author(&issue.repo, &path, &head)
            .await
        {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(rubato.issue = %issue, error = %format!("{e:#}"), "import: the plan's last commit could not be read");
                None
            }
        };
        let made = plan_drafts::import(
            &app,
            ws.key().tenant(),
            plan_drafts::ImportedBundle {
                issue: issue.clone(),
                branch: req.branch.clone(),
                owner: login.clone(),
                original_author: original_author.clone(),
                files,
                missing,
            },
        )
        .await
        .map_err(internal)?;
        let Ok((draft_id, rev)) = made else {
            return Err(refuse(
                StatusCode::CONFLICT,
                reason::ALREADY_IN_RUBATO,
                json!({}),
            ));
        };
        tracing::info!(rubato.issue = %issue, rubato.branch = %req.branch, rubato.user = %login, original_author = ?original_author, "plan imported");
        return Ok((
            StatusCode::CREATED,
            Json(ImportResponse::Imported {
                draft_id,
                rev,
                original_author,
            }),
        ));
    }
    let reasons: Vec<String> = match (&parsed, &fit) {
        (Err(e), _) => vec![e.to_string()],
        (_, Some(f)) => f.reasons.iter().map(|r| format!("{r:?}")).collect(),
        _ => vec![],
    };
    let launched = launch::start(
        &app,
        ws.key(),
        &login,
        LaunchRequest {
            issue: issue.clone(),
            companion_id: None,
            slug: None,
            branch: Some(req.branch.clone()),
            spend_cap_usd: None,
            attempt_limit: None,
            prompt: None,
            role: Default::default(),
        },
        Some(&rewrite_note(&req.branch, &path, &reasons)),
        None,
    )
    .await
    .map_err(|(status, detail)| {
        refuse(status, reason::LAUNCH_REFUSED, json!({ "detail": detail }))
    })?;
    tracing::info!(rubato.issue = %issue, rubato.branch = %req.branch, rubato.user = %login, launch_id = %launched.command.id, "plan import: rewrite launched");
    Ok((
        StatusCode::ACCEPTED,
        Json(ImportResponse::Rewriting {
            launch_id: launched.command.id,
            branch: launched.command.branch,
        }),
    ))
}
