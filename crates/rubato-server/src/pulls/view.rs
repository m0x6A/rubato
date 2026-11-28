//! `GET /api/pulls/{owner}/{repo}/{number}` (map-scoped, ADR 0049): a pull request's view with
//! what *Merge* would do (design 82 §2). Any pull request of a repo the workspace's map names and
//! the installation covers, open or not; any other repo is 404 `unknown_repo`, so the route
//! cannot read arbitrary repos through the installation.

use super::cache::Outcome;
use super::facts::{Gathered, gather, state_of, team_reviews};
use crate::auth::CurrentUser;
use crate::github_poll;
use crate::kernel::http::Ws;
use crate::kernel::state::{App, LoadedMap};
use crate::kernel::tenancy::TenantId;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::pulls::{FILES_PER_PAGE_HINT, FilesSummary, MergeOffer, PullDetail, reason};
use serde_json::{Value, json};

type Refused = (StatusCode, Json<Value>);

#[derive(serde::Deserialize)]
pub struct PullPath {
    pub(super) owner: String,
    pub(super) repo: String,
    pub(super) number: u64,
}

fn count(outcome: &'static str) {
    use opentelemetry::KeyValue;
    use std::sync::OnceLock;
    static C: OnceLock<opentelemetry::metrics::Counter<u64>> = OnceLock::new();
    C.get_or_init(|| {
        opentelemetry::global::meter("rubato-server")
            .u64_counter(metric::PULLS_VIEW_FETCHES)
            .build()
    })
    .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
}

/// The repo as the workspace's map spells it, when the map names it and the installation covers
/// it.
pub async fn known_repo(
    app: &App,
    t: &TenantId,
    map: &LoadedMap,
    repo: &str,
) -> anyhow::Result<Option<String>> {
    let Some(named) = map
        .map
        .repos_named()
        .into_iter()
        .find(|r| r.eq_ignore_ascii_case(repo))
        .map(str::to_owned)
    else {
        return Ok(None);
    };
    if app.github.is_installation()
        && github_poll::uncovered(&app.db, t)
            .await?
            .iter()
            .any(|u| u.eq_ignore_ascii_case(&named))
    {
        return Ok(None);
    }
    Ok(Some(named))
}

pub async fn pull_view(
    State(app): State<App>,
    ws: Ws,
    CurrentUser(login): CurrentUser,
    Path(p): Path<PullPath>,
) -> Result<Json<PullDetail>, Refused> {
    let fail = |status: StatusCode, why: &str| (status, Json(json!({ "reason": why })));
    let asked = format!("{}/{}", p.owner, p.repo);
    let Some(map) = app.map_of(ws.key()).await else {
        return Err(fail(StatusCode::SERVICE_UNAVAILABLE, "no_map"));
    };
    let failed = |e: anyhow::Error| {
        count("failed");
        tracing::warn!(rubato.repo = %asked, rubato.pr = p.number, error = %format!("{e:#}"), "pull view failed");
        fail(StatusCode::BAD_GATEWAY, "github_failed")
    };
    let repo = match known_repo(&app, ws.key().tenant(), &map, &asked).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(fail(StatusCode::NOT_FOUND, reason::UNKNOWN_REPO)),
        Err(e) => return Err(failed(e)),
    };
    let facts = match gather(&app, ws.key(), &map, &repo, p.number, &login, false).await {
        Ok(Gathered::Found(f)) => f,
        Ok(Gathered::NotFound) => return Err(fail(StatusCode::NOT_FOUND, "no_pull")),
        Err(e) => return Err(failed(e)),
    };
    count(match facts.outcome {
        Outcome::Cached => "cached",
        Outcome::Fetched => "fetched",
    });
    let f = facts.fetched();
    let pull = facts.pull_facts();
    let (reviews, outside_reviews) = team_reviews(&f.reviews);
    Ok(Json(PullDetail {
        repo: repo.clone(),
        number: f.pull.info.number,
        title: f.pull.info.title.clone(),
        author: f.pull.info.author.clone(),
        body: f.pull.body.clone(),
        state: state_of(&f.pull),
        draft: f.pull.info.draft,
        fork: f.pull.fork,
        branch: f.pull.branch.clone(),
        base: f.pull.info.base.clone().unwrap_or_default(),
        head_sha: f.pull.info.head_sha.clone(),
        mergeable: f.pull.mergeable,
        checks: facts.checks(),
        check_runs: f.runs.clone(),
        reviews,
        outside_reviews,
        files: FilesSummary::of(f.files.clone(), f.files.len() >= FILES_PER_PAGE_HINT),
        url: f.pull.info.url.clone(),
        issue: facts.issue.clone(),
        merged_at: f.pull.info.merged_at,
        merge: MergeOffer::new(
            &pull,
            facts.person.as_ref(),
            facts.gate.clone(),
            facts.stacked_above,
        ),
        read_at: f.read_at,
    }))
}
