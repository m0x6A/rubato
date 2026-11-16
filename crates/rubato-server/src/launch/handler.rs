//! `POST /api/launch`: start a session on an issue through one of the caller's own companions.

use super::prompt::{
    LEAD_PROMPT_TEMPLATE, PROMPT_TEMPLATE, RUBATO_PROMPT_TEMPLATE, is_slug, render_prompt, slugify,
};
use super::store;
use crate::auth::CurrentUser;
use crate::kernel::http::internal;
use crate::kernel::state::App;
use crate::{github_poll, pairing, pins, plan_drafts, plans};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use rubato_protocol::fleet::reason as fleet_reason;
use rubato_protocol::launch::reason as launch_reason;
use rubato_protocol::launch::routes;
use rubato_protocol::otel::{attr, event, span};
use rubato_protocol::{
    CapHit, CapKind, IssueRef, LaunchCommand, LaunchRequest, LaunchResponse, LaunchRole,
    PlanningMode, ServerMessage, capability, skill_has_handoff,
};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use uuid::Uuid;

const SLICE: &str = "launch";

/// A launch past the attempt limit is an event, not only a refusal: log `rubato.cap.hit` before
/// the write, record the cap hit, and answer 409 whether or not the write worked.
async fn refuse_past_attempt_limit(
    db: &SqlitePool,
    issue: &IssueRef,
    attempt: u32,
    attempt_limit: u32,
) -> (StatusCode, String) {
    let hit = CapHit {
        issue: issue.clone(),
        session_id: None,
        kind: CapKind::Attempts,
        value: f64::from(attempt),
        limit: f64::from(attempt_limit),
        at: Utc::now(),
    };
    tracing::warn!(rubato.issue = %hit.issue, kind = ?hit.kind, value = hit.value, limit = hit.limit, "{}", event::CAP_HIT);
    if let Err(e) = store::store_cap_hit(db, &hit, hit.at).await {
        tracing::error!(slice = SLICE, rubato.issue = %hit.issue, error = %e, "storing cap hit");
    }
    (
        StatusCode::CONFLICT,
        format!("attempt limit {attempt_limit} reached for {issue}"),
    )
}

/// Start a session on an issue through one of the caller's own companions (ADR 0004).
pub async fn launch(
    State(app): State<App>,
    ws: crate::kernel::http::Ws,
    CurrentUser(user): CurrentUser,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, (StatusCode, String)> {
    start(&app, ws.as_str(), &user, req, None, None)
        .await
        .map(Json)
}

/// A launch for `user` in workspace `ws`, as the route makes it; `note` is added to the
/// rendered prompt (Import's rewrite, ADR 0058).
pub async fn start(
    app: &App,
    ws: &str,
    user: &str,
    req: LaunchRequest,
    note: Option<&str>,
    // The owner's lead agent that asked for this worker (ADR 0072); `None` from the web.
    lead_launch_id: Option<Uuid>,
) -> Result<LaunchResponse, (StatusCode, String)> {
    // A lead only while the orchestrator flag is on (ADR 0072); a lead never starts a lead.
    if req.role == LaunchRole::Lead && (!app.config.orchestrator || lead_launch_id.is_some()) {
        app.reject(routes::LAUNCH, fleet_reason::ORCHESTRATOR_OFF, user);
        return Err((
            StatusCode::CONFLICT,
            format!(
                "{}: a lead agent needs RUBATO_ORCHESTRATOR=1",
                fleet_reason::ORCHESTRATOR_OFF
            ),
        ));
    }
    let map = app
        .map_of(ws)
        .await
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "no map loaded".into()))?;
    // A pin row from before the repo check, or of a repo the map has since dropped, does not
    // start a session that reads it (issue #163).
    if let Err((reason, msg)) = pins::check_repo(&map.map, &map.version, &req.issue.repo) {
        app.reject(routes::LAUNCH, reason, user);
        return Err((StatusCode::UNPROCESSABLE_ENTITY, msg));
    }
    let mine: Vec<pairing::PairingRow> = pairing::list_pairings(&app.db, Some(user))
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .filter(|p| p.revoked_at.is_none())
        .collect();
    let connected = app.connected_companions();
    let candidates: Vec<&pairing::PairingRow> = mine
        .iter()
        .filter(|p| connected.contains(&p.companion_id))
        .collect();
    let companion = match req.companion_id {
        Some(id) => candidates
            .iter()
            .find(|p| p.companion_id == id)
            .copied()
            .ok_or((
                StatusCode::FORBIDDEN,
                format!("companion {id} is not one of your connected companions"),
            ))?,
        None => match candidates.as_slice() {
            [one] => *one,
            [] => {
                return Err((
                    StatusCode::CONFLICT,
                    "none of your companions is connected".into(),
                ));
            }
            _ => {
                return Err((
                    StatusCode::CONFLICT,
                    "several companions connected; pass companion_id".into(),
                ));
            }
        },
    };
    // A lead goes only to a companion whose own orchestrator flag is on (ADR 0072).
    if req.role == LaunchRole::Lead && !app.announced(companion.companion_id, capability::FLEET) {
        app.reject(routes::LAUNCH, fleet_reason::COMPANION_CANNOT_LEAD, user);
        return Err((
            StatusCode::CONFLICT,
            format!(
                "{}: that companion does not run lead agents",
                fleet_reason::COMPANION_CANNOT_LEAD
            ),
        ));
    }
    let pins: Vec<String> = pins::list(&app.db, ws)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .filter(|p| p.issue == req.issue)
        .map(|p| p.target.id().to_owned())
        .collect();
    let meta = github_poll::issue_meta(&app.db, &req.issue)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .unwrap_or_default();
    let plan_row = plans::all(&app.db)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .find(|p| p.issue == req.issue);
    // A draft of the caller's on this issue decides the branch (ADR 0053, plan 122 §D5), so
    // editing the title never orphans it. A teammate's draft stays theirs: taking it over is Import.
    let mine: Vec<plan_drafts::LatestDraft> = plan_drafts::latest_drafts(app)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        .into_iter()
        .filter(|d| d.issue == req.issue && d.summary.owner.eq_ignore_ascii_case(user))
        .collect();
    let branch = choose_branch(
        &req,
        meta.title.as_deref().unwrap_or(""),
        &mine
            .iter()
            .map(|d| d.summary.branch.as_str())
            .collect::<Vec<_>>(),
    )?;
    let slug = branch[branch_prefix(&req.issue).len()..].to_owned();
    let draft = mine.into_iter().find(|d| d.summary.branch == branch);
    // The plan on this branch, when there is one. With none, the draft's newest revision sets
    // the caps (A14); with no draft either, the issue's plan as before.
    let branch_plan = plan_row
        .as_ref()
        .filter(|p| p.branch == branch)
        .and_then(|p| p.plan.clone());
    let plan = match &draft {
        Some(d) if branch_plan.is_none() => d.plan.clone(),
        _ => plan_row.and_then(|p| p.plan),
    };
    // The draft comes back only into a worktree this launch creates; the companion decides that.
    let restore = match &draft {
        Some(d)
            if branch_plan.is_none()
                && app.announced(companion.companion_id, capability::DRAFT_RESTORE) =>
        {
            plan_drafts::restore_of(app, d.summary.draft_id)
                .await
                .map_err(|e| (internal(SLICE, e), String::new()))?
        }
        _ => None,
    };
    // Pinned, or with a plan that fits this workspace (ADR 0058): an imported plan places the
    // issue by its own affected parts.
    let pins = if pins.is_empty() {
        match plan
            .as_ref()
            .filter(|p| rubato_protocol::plan_fit(Some(p), None, &map.map).is_some_and(|f| f.fits))
        {
            Some(p) => p.affected_parts.iter().map(|a| a.node.clone()).collect(),
            None => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "pin the issue to the map first".into(),
                ));
            }
        }
    } else {
        pins
    };
    let attempt = store::launch_count(&app.db, &req.issue)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?
        + 1;
    let spend_cap = req
        .spend_cap_usd
        .or(plan.as_ref().and_then(|p| p.spend_cap_usd))
        .unwrap_or(app.config.default_spend_cap_usd);
    let attempt_limit = req
        .attempt_limit
        .or(plan.as_ref().and_then(|p| p.attempt_limit))
        .unwrap_or(app.config.default_attempt_limit);
    if attempt > attempt_limit {
        let refusal = refuse_past_attempt_limit(&app.db, &req.issue, attempt, attempt_limit).await;
        app.notify("cap hit");
        return Err(refusal);
    }
    let plan_path = map
        .map
        .plan_path_for(&req.issue.repo, req.issue.number, &slug);
    let mut otel = BTreeMap::new();
    otel.insert(attr::REPO.to_owned(), req.issue.repo.clone());
    otel.insert(attr::ISSUE.to_owned(), req.issue.to_string());
    otel.insert(attr::BRANCH.to_owned(), branch.clone());
    otel.insert(attr::NODE.to_owned(), pins.join("+"));
    let vars: BTreeMap<&str, String> = [
        ("issue", req.issue.to_string()),
        ("title", meta.title.clone().unwrap_or_default()),
        ("worktree", "(created by the companion)".into()),
        ("branch", branch.clone()),
        ("nodes", pins.join(", ")),
        ("map_version", map.version.clone()),
        ("plan_path", plan_path.clone()),
        ("spend_cap_usd", format!("{spend_cap}")),
        ("attempt", attempt.to_string()),
        ("attempt_limit", attempt_limit.to_string()),
    ]
    .into_iter()
    .collect();
    let planning = planning_mode(
        app.config.comment_approvals_until.is_some(),
        &app.capabilities(companion.companion_id),
    );
    let template = match (req.role, planning) {
        (LaunchRole::Lead, _) => LEAD_PROMPT_TEMPLATE,
        (_, PlanningMode::Rubato) => RUBATO_PROMPT_TEMPLATE,
        (_, PlanningMode::Git) => PROMPT_TEMPLATE,
    };
    let command = LaunchCommand {
        id: Uuid::new_v4(),
        user: user.to_owned(),
        issue: req.issue.clone(),
        repo_url: format!("https://github.com/{}.git", req.issue.repo),
        branch,
        base_ref: "origin/main".into(),
        node_ids: pins,
        spend_cap_usd: spend_cap,
        attempt_limit,
        attempt,
        session_id: Uuid::new_v4(),
        prompt: {
            let base = req
                .prompt
                .clone()
                .unwrap_or_else(|| render_prompt(template, &vars));
            match note {
                Some(n) => format!("{base}\n\n{n}"),
                None => base,
            }
        },
        otel_resource: otel,
        map_version: Some(map.version.clone()),
        plan_path: Some(plan_path),
        draft: restore,
        planning,
        role: req.role,
        lead_launch_id,
    };
    store::store_launch(&app.db, ws, companion.companion_id, &command)
        .await
        .map_err(|e| (internal(SLICE, e), String::new()))?;
    if !app.send_to_companion(
        companion.companion_id,
        ServerMessage::Launch(command.clone()),
    ) {
        return Err((
            StatusCode::CONFLICT,
            "companion disconnected while launching".into(),
        ));
    }
    tracing::info!(launch_id = %command.id, rubato.issue = %command.issue, rubato.user = %user, companion = %companion.companion_id, planning = command.planning.name(), "{}", span::LAUNCH);
    app.notify("launch");
    Ok(LaunchResponse {
        command,
        companion_id: companion.companion_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;

    #[tokio::test]
    async fn a_failed_cap_hit_write_is_logged_and_still_answers_409() {
        let (events, _guard) = capture::start();
        // A database with no tables: the insert into `cap_hits` fails.
        let db = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let issue = IssueRef::new("m0x6A/rubato", 30);

        let (status, body) = refuse_past_attempt_limit(&db, &issue, 4, 3).await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body, "attempt limit 3 reached for m0x6A/rubato#30");
        let events = events.lock().unwrap();
        let hit = events
            .iter()
            .position(|e| e.message == event::CAP_HIT)
            .expect("the rubato.cap.hit event");
        assert_eq!(events[hit].level, tracing::Level::WARN);
        assert_eq!(
            events[hit].fields.get("rubato.issue").map(String::as_str),
            Some("m0x6A/rubato#30")
        );
        assert_eq!(
            events[hit].fields.get("kind").map(String::as_str),
            Some("Attempts")
        );
        let failed = events
            .iter()
            .position(|e| e.message == "storing cap hit")
            .expect("the storing cap hit line");
        assert!(hit < failed, "the event is logged before the write");
        let e = &events[failed];
        assert_eq!(e.level, tracing::Level::ERROR);
        assert_eq!(e.fields.get("slice").map(String::as_str), Some("launch"));
        assert_eq!(
            e.fields.get("rubato.issue").map(String::as_str),
            Some("m0x6A/rubato#30")
        );
        assert!(e.fields.contains_key("error"));
    }
}

fn branch_prefix(issue: &IssueRef) -> String {
    format!("issue/{}-", issue.number)
}

/// The branch a launch goes on: the one named, else one from the slug named, else the caller's
/// only draft of the issue, else a slug of the title. Several drafts and none named is 409
/// `pass_branch` (ADR 0053): guessing would resume the wrong one.
fn choose_branch(
    req: &LaunchRequest,
    title: &str,
    drafts: &[&str],
) -> Result<String, (StatusCode, String)> {
    let prefix = branch_prefix(&req.issue);
    match (&req.branch, &req.slug, drafts) {
        (Some(b), _, _) => match b.strip_prefix(&prefix) {
            Some(s) if is_branch_rest(s) => Ok(b.clone()),
            _ => Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("branch must be {prefix}<slug>"),
            )),
        },
        (None, Some(slug), _) if !is_slug(slug) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "slug: letters, digits and single hyphens only".into(),
        )),
        (None, Some(slug), _) => Ok(format!("{prefix}{slug}")),
        (None, None, [one]) => Ok((*one).to_owned()),
        (None, None, []) => Ok(format!("{prefix}{}", slugify(title))),
        (None, None, several) => Err((
            StatusCode::CONFLICT,
            format!(
                "{}: you own drafts of {} on {}; pass branch",
                launch_reason::PASS_BRANCH,
                req.issue,
                several.join(", ")
            ),
        )),
    }
}

/// What may follow `issue/<n>-` in a branch a launch names: what the companion's `check_ref`
/// accepts, so the refusal is a 422 here and not a failed launch on the developer's machine.
fn is_branch_rest(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
        && !s.starts_with(['-', '/'])
        && !s.ends_with('/')
        && !s.ends_with(".lock")
        && !s.contains("..")
        && !s.contains("//")
}

/// What a session needs from its companion to plan in Rubato (ADR 0053, plan 122 §D7): typed
/// messages, mirrored drafts, the handoff typed and reported, and a skill with `plan.ts handoff`.
const PLANNING_NEEDS: [&str; 3] = [
    capability::SESSION_INPUT,
    capability::DRAFT_MIRROR,
    capability::PLAN_HANDOFF,
];

/// How a launched session plans: `rubato` only once the cutover is set (until then plans are
/// approved by comment, so a draft in Rubato could never be approved) and only when the
/// companion carries every part of it. Anything less plans in Git as before: a session half in
/// Rubato would draft a plan nobody can hand off.
pub fn planning_mode(cutover_set: bool, caps: &[String]) -> PlanningMode {
    let has = |c: &str| caps.iter().any(|x| x == c);
    let skill = caps
        .iter()
        .filter_map(|c| c.strip_prefix(capability::SKILL_PREFIX))
        .any(skill_has_handoff);
    if cutover_set && PLANNING_NEEDS.iter().all(|c| has(c)) && skill {
        PlanningMode::Rubato
    } else {
        PlanningMode::Git
    }
}

#[cfg(test)]
mod planning_tests {
    use super::*;

    fn caps(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| (*c).to_owned()).collect()
    }

    const ALL: [&str; 4] = [
        "session_input",
        "draft_mirror",
        "plan_handoff",
        "skill:0.2.0",
    ];

    #[test]
    fn rubato_only_after_the_cutover_and_with_every_capability() {
        assert_eq!(planning_mode(true, &caps(&ALL)), PlanningMode::Rubato);
        assert_eq!(
            planning_mode(false, &caps(&ALL)),
            PlanningMode::Git,
            "no cutover: approval by comment"
        );
        for missing in ALL {
            let fewer: Vec<&str> = ALL.iter().copied().filter(|c| *c != missing).collect();
            assert_eq!(
                planning_mode(true, &caps(&fewer)),
                PlanningMode::Git,
                "without {missing}"
            );
        }
        assert_eq!(
            planning_mode(true, &[]),
            PlanningMode::Git,
            "an older companion announces nothing"
        );
    }

    #[test]
    fn a_skill_older_than_the_handoff_plans_in_git() {
        let old = [
            "session_input",
            "draft_mirror",
            "plan_handoff",
            "skill:0.1.0",
        ];
        assert_eq!(planning_mode(true, &caps(&old)), PlanningMode::Git);
        let newer = [
            "session_input",
            "draft_mirror",
            "plan_handoff",
            "skill:0.3.1",
        ];
        assert_eq!(planning_mode(true, &caps(&newer)), PlanningMode::Rubato);
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    fn req(branch: Option<&str>, slug: Option<&str>) -> LaunchRequest {
        serde_json::from_value(serde_json::json!({
            "issue": { "repo": "m0x6A/rubato", "number": 122 },
            "branch": branch,
            "slug": slug,
        }))
        .unwrap()
    }

    /// A slug or branch from the request reaches `launch.sh` on the developer's machine: only
    /// what the companion's `check_ref` accepts gets past this, as a 422 (#156).
    #[test]
    fn a_launch_names_only_a_slug_or_branch_the_companion_accepts() {
        let ok = |b: Option<&str>, s: Option<&str>| choose_branch(&req(b, s), "Title", &[]);
        assert_eq!(
            ok(None, Some("plan-in-rubato")).unwrap(),
            "issue/122-plan-in-rubato"
        );
        assert_eq!(
            ok(Some("issue/122-plan_in.rubato"), None).unwrap(),
            "issue/122-plan_in.rubato"
        );
        for bad in ["it's", "a b", "x;y", "-x", ""] {
            assert_eq!(
                ok(None, Some(bad)).unwrap_err().0,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{bad:?}"
            );
        }
        for bad in [
            "issue/122-it's",
            "issue/122-a b",
            "issue/122-a..b",
            "issue/122-",
            "issue/9-x",
            "issue/122-x.lock",
        ] {
            assert_eq!(
                ok(Some(bad), None).unwrap_err().0,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{bad:?}"
            );
        }
    }
}
