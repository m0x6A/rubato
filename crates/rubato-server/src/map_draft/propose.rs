//! Propose the draft as a pull request on the map repository, withdraw it, and show the file
//! it would commit (ADR 0018).
//!
//! The proposal row is committed before the first write to GitHub and updated after the last,
//! so no lock is held across a network call. Each write is one named step; when one fails the
//! draft stays frozen in `opening`, Propose again resumes and Withdraw gives up.

use super::handlers::{Refusal, SLICE, fault};
use super::store::{self, ProposalRow, state_name};
use super::yaml;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::{App, LoadedMap};
use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use opentelemetry::KeyValue;
use rubato_protocol::map_draft::{DraftOp, ProposalState, ProposeResponse, routes};
use rubato_protocol::otel::attr;
use sha2::Digest;

/// The reason values of a refused proposal, as counted in `rubato.web.rejected`.
pub mod reason {
    pub const CANNOT_WRITE: &str = "cannot_write";
    pub const LOCAL_MAP_FILE: &str = "local_map_file";
    pub const NOTHING_TO_PROPOSE: &str = "nothing_to_propose";
    pub const STALE_OPERATION: &str = "stale_operation";
    pub const MAP_CHANGED: &str = "map_changed";
    pub const UNKNOWN_KEYS: &str = "unknown_keys";
    pub const ALREADY_MERGED: &str = "already_merged";
    pub const NO_PROPOSAL: &str = "no_proposal";
    pub const GITHUB_STEP_FAILED: &str = "github_step_failed";
}

pub(super) fn count(app: &App, outcome: &'static str) {
    app.metrics
        .map_proposals
        .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
}

/// Every state change of the proposal, with what on-call needs to follow it.
pub(super) fn log_state(from: &str, to: &str, row: &ProposalRow, pr: Option<u64>) {
    tracing::info!(
        from,
        to,
        pr = pr.or(row.pr_number),
        branch = %row.branch,
        base_sha = %row.base_sha,
        "map proposal state"
    );
}

async fn loaded(app: &App) -> Result<LoadedMap, Refusal> {
    app.map
        .read()
        .await
        .clone()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))
}

fn hash(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn body(ops: &[DraftOp], by: &str) -> String {
    // Logins are in backticks on purpose: a bare @name would notify people from a server.
    let mut out = format!(
        "Opened by Rubato from the team's shared map draft, proposed by `{by}`.\n\n| # | Change | By |\n| --- | --- | --- |\n"
    );
    for (i, o) in ops.iter().enumerate() {
        out.push_str(&format!(
            "| {} | {} | `{}` |\n",
            i + 1,
            o.op.summary(),
            o.author
        ));
    }
    out.push_str("\nThe file is written from Rubato's map model, so hand-written comments in it are not kept. When this is merged and Rubato has loaded the map again, the draft empties. Closing it without merging makes the draft editable again, with every change kept.\n");
    out
}

/// One write to GitHub failed: said to the user with the step, logged with what on-call needs,
/// counted. The proposal row, if there is one, stays in `opening`.
fn step_failed(
    app: &App,
    user: &str,
    step: &'static str,
    branch: &str,
    base_sha: &str,
    frozen: bool,
    e: &anyhow::Error,
) -> Refusal {
    tracing::error!(
        step,
        rubato.repo = %app.config.map_repo,
        branch,
        base_sha,
        rubato.user = user,
        error = %format!("{e:#}"),
        "map proposal failed"
    );
    count(app, "failed");
    app.reject(routes::DRAFT_PROPOSE, reason::GITHUB_STEP_FAILED, user);
    let what = match step {
        "head" => "read the head of the map branch",
        "file" => "read the map file on the map branch",
        "ref" => "create the proposal's branch",
        "contents" => "write the map file to the proposal's branch",
        "close" => "close the pull request",
        _ => "open the pull request",
    };
    let after = if frozen {
        " The draft stays frozen: Propose again resumes, Withdraw gives up."
    } else {
        " Nothing was written and the draft is unchanged."
    };
    (
        StatusCode::BAD_GATEWAY,
        format!("GitHub did not let Rubato {what} (step {step}): {e:#}.{after}"),
    )
}

pub async fn propose(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<ProposeResponse>, Refusal> {
    let map = loaded(&app).await?;
    let cfg = &app.config;
    let refuse = |why: &'static str, text: String| -> Refusal {
        app.reject(routes::DRAFT_PROPOSE, why, &user);
        (StatusCode::CONFLICT, text)
    };
    let existing = store::proposal(&app.db).await.map_err(fault)?;
    match existing.as_ref().map(|p| p.state) {
        // Asked twice: the answer is the proposal there is.
        Some(ProposalState::Proposed) => {
            return Ok(Json(ProposeResponse {
                proposal: existing.expect("checked").to_wire(),
            }));
        }
        Some(ProposalState::Merged) => {
            return Err(refuse(
                reason::ALREADY_MERGED,
                "The proposal was merged; the draft empties when the map has been loaded again."
                    .into(),
            ));
        }
        _ => {}
    }
    if !app.github.can_write() {
        return Err(refuse(
            reason::CANNOT_WRITE,
            "This Rubato cannot write to GitHub: it runs on a personal token or none, and only a GitHub App installation can open pull requests. Copy YAML gives the full proposed file.".into(),
        ));
    }
    if cfg.map_file.is_some() {
        return Err(refuse(
            reason::LOCAL_MAP_FILE,
            "The map is read from a local file, so there is no repository to propose to. Copy YAML gives the full proposed file.".into(),
        ));
    }
    let (ops, preview) = store::ops(&app.db, &map.map).await.map_err(fault)?;
    if ops.is_empty() {
        return Err(refuse(
            reason::NOTHING_TO_PROPOSE,
            "The draft is empty: there is nothing to propose.".into(),
        ));
    }
    if let Some(stale) = ops.iter().find(|o| o.stale.is_some()) {
        return Err(refuse(
            reason::STALE_OPERATION,
            format!(
                "\"{}\" by {} no longer applies ({}). Remove it from the draft, then propose.",
                stale.op.summary(),
                stale.author,
                stale.stale.as_deref().unwrap_or_default()
            ),
        ));
    }
    let frozen = existing.is_some();
    let (branch, base) = existing
        .as_ref()
        .map(|p| (p.branch.clone(), p.base_sha.clone()))
        .unwrap_or_default();

    // The proposal starts at the map branch's head, and rewrites the file that is loaded.
    // Other commits on that branch do not matter; a different map file does.
    let head = match app.github.branch_sha(&cfg.map_repo, &cfg.map_ref).await {
        Ok(Some(sha)) => sha,
        Ok(None) => {
            let e = anyhow::anyhow!("branch {} not found in {}", cfg.map_ref, cfg.map_repo);
            return Err(step_failed(&app, &user, "head", &branch, &base, frozen, &e));
        }
        Err(e) => return Err(step_failed(&app, &user, "head", &branch, &base, frozen, &e)),
    };
    let on_branch = match app
        .github
        .file(&cfg.map_repo, &cfg.map_path, &cfg.map_ref)
        .await
    {
        Ok(f) => f,
        Err(e) => return Err(step_failed(&app, &user, "file", &branch, &base, frozen, &e)),
    };
    if on_branch.as_deref() != Some(map.yaml.as_str()) {
        return Err(refuse(
            reason::MAP_CHANGED,
            format!(
                "The map file on {} is not the one Rubato has loaded (version {}). Reload the map first, then propose.",
                cfg.map_ref, map.version
            ),
        ));
    }
    let unknown = yaml::unknown_keys(&map.yaml).map_err(fault)?;
    if !unknown.is_empty() {
        return Err(refuse(
            reason::UNKNOWN_KEYS,
            format!(
                "The map file has keys Rubato does not know, which a rewrite would drop: {}. Remove them from the file, or change the map by hand.",
                unknown.join(", ")
            ),
        ));
    }
    let content = yaml::render(&preview).map_err(fault)?;

    // The one row there can be: committed here, before anything is written to GitHub. From now
    // on the draft takes no operations.
    let row = match existing {
        Some(row) => row,
        None => {
            let branch = format!("map/draft-{}", Utc::now().format("%Y%m%d-%H%M%S"));
            match store::start_proposal(
                &app.db,
                &branch,
                &head,
                &hash(&map.yaml),
                &user,
                Utc::now(),
            )
            .await
            .map_err(fault)?
            {
                Ok(row) => {
                    log_state("drafting", "opening", &row, None);
                    row
                }
                // Someone else pressed Propose at the same moment: theirs is the proposal.
                Err(theirs) => theirs,
            }
        }
    };
    app.notify("draft");
    let fail = |step: &'static str, e: anyhow::Error| {
        step_failed(&app, &user, step, &row.branch, &row.base_sha, true, &e)
    };

    app.github
        .create_ref(&cfg.map_repo, &row.branch, &row.base_sha)
        .await
        .map_err(|e| fail("ref", e))?;
    // A resumed proposal may have written the file already.
    let written = app
        .github
        .file(&cfg.map_repo, &cfg.map_path, &row.branch)
        .await
        .map_err(|e| fail("contents", e))?;
    if written.as_deref() != Some(content.as_str()) {
        let message = format!(
            "Map: {} change{} from Rubato's draft",
            ops.len(),
            if ops.len() == 1 { "" } else { "s" }
        );
        app.github
            .put_file(
                &cfg.map_repo,
                &row.branch,
                &cfg.map_path,
                &content,
                &message,
            )
            .await
            .map_err(|e| fail("contents", e))?;
    }
    // A pull request opened by a call that then timed out is found, not opened twice.
    let pr = match app
        .github
        .pull_for_branch(&cfg.map_repo, &row.branch)
        .await
        .map_err(|e| fail("pull", e))?
    {
        Some(p) => (p.number, p.url),
        None => app
            .github
            .create_pull(
                &cfg.map_repo,
                &row.branch,
                &cfg.map_ref,
                &format!(
                    "Map: {} change{} proposed from Rubato",
                    ops.len(),
                    if ops.len() == 1 { "" } else { "s" }
                ),
                &body(&ops, &row.proposed_by),
            )
            .await
            .map_err(|e| fail("pull", e))?,
    };
    store::set_state(
        &app.db,
        ProposalState::Proposed,
        Utc::now(),
        Some((pr.0, &pr.1)),
    )
    .await
    .map_err(fault)?;
    log_state("opening", "proposed", &row, Some(pr.0));
    count(&app, "opened");
    app.notify("draft");
    let proposal = store::proposal(&app.db).await.map_err(fault)?.ok_or((
        internal(SLICE, anyhow::anyhow!("the proposal row is gone")),
        String::new(),
    ))?;
    Ok(Json(ProposeResponse {
        proposal: proposal.to_wire(),
    }))
}

/// Give the proposal up: close the pull request, delete the branch, and make the draft editable
/// again with every operation kept.
pub async fn withdraw(
    State(app): State<App>,
    CurrentUser(user): CurrentUser,
) -> Result<StatusCode, Refusal> {
    let cfg = &app.config;
    let Some(row) = store::proposal(&app.db).await.map_err(fault)? else {
        app.reject(routes::DRAFT_WITHDRAW, reason::NO_PROPOSAL, &user);
        return Err((
            StatusCode::CONFLICT,
            "There is no proposal to withdraw.".into(),
        ));
    };
    if row.state == ProposalState::Merged {
        app.reject(routes::DRAFT_WITHDRAW, reason::ALREADY_MERGED, &user);
        return Err((
            StatusCode::CONFLICT,
            "The proposal was merged and cannot be withdrawn; the draft empties when the map has been loaded again.".into(),
        ));
    }
    let fail = |e: anyhow::Error| {
        let (_, text) = step_failed(&app, &user, "close", &row.branch, &row.base_sha, true, &e);
        (
            StatusCode::BAD_GATEWAY,
            text.replace(
                "Propose again resumes, Withdraw gives up.",
                "Withdraw again to retry.",
            ),
        )
    };
    // The pull request may exist without the row knowing its number (a call that timed out).
    let open = match row.pr_number {
        Some(n) => Some(n),
        None => app
            .github
            .pull_for_branch(&cfg.map_repo, &row.branch)
            .await
            .map_err(fail)?
            .filter(|p| p.open)
            .map(|p| p.number),
    };
    if let Some(number) = open {
        app.github
            .close_pull(&cfg.map_repo, number)
            .await
            .map_err(fail)?;
    }
    if let Err(e) = app.github.delete_ref(&cfg.map_repo, &row.branch).await {
        tracing::warn!(branch = %row.branch, error = %format!("{e:#}"), "the proposal's branch was not deleted");
    }
    store::drop_proposal(&app.db).await.map_err(fault)?;
    log_state(state_name(row.state), "drafting", &row, open);
    tracing::info!(rubato.user = %user, branch = %row.branch, "map proposal withdrawn");
    count(&app, "withdrawn");
    app.notify("draft");
    Ok(StatusCode::OK)
}

/// The file a proposal would commit, as text: also the way out when the server cannot propose.
pub async fn proposed_yaml(
    State(app): State<App>,
    _user: CurrentUser,
) -> Result<Response, Refusal> {
    let map = loaded(&app).await?;
    let (_, preview) = store::ops(&app.db, &map.map).await.map_err(fault)?;
    let text = yaml::render(&preview).map_err(fault)?;
    Ok(([(header::CONTENT_TYPE, "text/yaml; charset=utf-8")], text).into_response())
}
