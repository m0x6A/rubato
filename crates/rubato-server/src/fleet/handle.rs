//! One lead request, from claim to answer (plan 134 design §3). The server decides who the lead
//! is and whose agents those are, from the pairing and the stored launches; the agent's word is
//! never taken for either. `other_owner` and `identity` mean the line between people was
//! tested: they are counted with the target's owner and page (`FleetCrossOwner`).

use super::store::{self, Claim};
use crate::kernel::state::App;
use crate::{launch, plan_drafts, session_log, session_messages};
use chrono::{Duration, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::fleet::{limits, reason};
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{
    FleetAction, FleetEntry, FleetOutcome, FleetRequest, FleetResult, LaunchRequest, LaunchRole,
    SessionStatus, capability,
};
use std::collections::{HashMap, HashSet};
use tracing::Instrument;
use uuid::Uuid;

/// What one request came to, before it is written into a `FleetResult`.
struct Answer {
    outcome: FleetOutcome,
    reason: Option<&'static str>,
    message: Option<String>,
    fleet: Option<Vec<FleetEntry>>,
    launch_id: Option<Uuid>,
    message_id: Option<Uuid>,
    target_owner: Option<String>,
}

impl Answer {
    fn ok() -> Self {
        Self {
            outcome: FleetOutcome::Ok,
            reason: None,
            message: None,
            fleet: None,
            launch_id: None,
            message_id: None,
            target_owner: None,
        }
    }

    fn refused(why: &'static str, message: impl Into<String>) -> Self {
        Self {
            outcome: FleetOutcome::Refused,
            reason: Some(why),
            message: Some(message.into()),
            ..Self::ok()
        }
    }

    fn result(self, req: &FleetRequest) -> FleetResult {
        FleetResult {
            request_id: req.request_id,
            lead_launch_id: req.lead_launch_id,
            outcome: self.outcome,
            reason: self.reason.map(str::to_owned),
            message: self.message,
            fleet: self.fleet,
            launch_id: self.launch_id,
            message_id: self.message_id,
        }
    }
}

fn count(app: &App, action: &str, outcome: &str, why: &str) {
    app.metrics.fleet_requests.add(
        1,
        &[
            KeyValue::new(attr::FLEET_ACTION, action.to_owned()),
            KeyValue::new(attr::OUTCOME, outcome.to_owned()),
            KeyValue::new(attr::DROP_REASON, why.to_owned()),
        ],
    );
}

/// Handle a `FleetRequest` from `companion_id`, paired to `user`. `None` when the companion did
/// not announce `fleet`: it is sent nothing it did not ask to take (ADR 0072).
pub async fn on_request(
    app: &App,
    companion_id: Uuid,
    user: &str,
    req: &FleetRequest,
) -> Option<FleetResult> {
    let action = req.action.name();
    let s = tracing::info_span!(
        span::FLEET_REQUEST,
        rubato.launch_id = %req.lead_launch_id,
        rubato.fleet.action = action,
        rubato.user = %user,
        rubato.target_launch_id = tracing::field::Empty,
        rubato.target_owner = tracing::field::Empty,
        rubato.outcome = tracing::field::Empty,
        rubato.reason = tracing::field::Empty,
    );
    async move {
        let span = tracing::Span::current();
        if let Some(t) = req.action.target() {
            span.record("rubato.target_launch_id", t.to_string());
        }
        let announced = app.announced(companion_id, capability::FLEET);
        // Step 1: the flag. Nothing else happens with it off.
        if !app.config.orchestrator || !announced {
            span.record("rubato.outcome", "refused");
            span.record("rubato.reason", reason::ORCHESTRATOR_OFF);
            count(app, action, "refused", reason::ORCHESTRATOR_OFF);
            tracing::warn!(rubato.launch_id = %req.lead_launch_id, rubato.reason = reason::ORCHESTRATOR_OFF, "fleet request refused");
            return announced.then(|| {
                Answer::refused(reason::ORCHESTRATOR_OFF, "the orchestrator flag is off on this server").result(req)
            });
        }
        let now = app.clock.now();
        // Step 2: the claim, before any check, so a repeat never acts twice.
        let answer = match store::claim(&app.db, companion_id, user, req, now).await {
            Err(e) => {
                tracing::error!(slice = "fleet", error = %format!("{e:#}"), "fleet claim failed");
                return Some(Answer::refused("internal", "the request could not be recorded").result(req));
            }
            Ok(Claim::Done(r)) => {
                span.record("rubato.outcome", "repeat");
                count(app, action, "repeat", "");
                return Some(*r);
            }
            Ok(Claim::Running(since)) => {
                let late = now - since > Duration::seconds(limits::IN_PROGRESS_SECS);
                let outcome = if late { FleetOutcome::Unknown } else { FleetOutcome::InProgress };
                span.record("rubato.outcome", outcome.name());
                count(app, action, outcome.name(), "");
                return Some(Answer { outcome, message: Some(if late {
                    "this request started and never finished; check fleet_list before you ask again".into()
                } else {
                    "still being acted on".into()
                }), ..Answer::ok() }.result(req));
            }
            // The row is the first request's: answer, but record nothing over it.
            Ok(Claim::Reused) => {
                span.record("rubato.outcome", "refused");
                span.record("rubato.reason", reason::REQUEST_ID_REUSED);
                count(app, action, "refused", reason::REQUEST_ID_REUSED);
                return Some(Answer::refused(reason::REQUEST_ID_REUSED, "that request_id was used for another action").result(req));
            }
            Ok(Claim::New) => act(app, companion_id, user, req).await,
        };
        let why = answer.reason.unwrap_or("");
        if let Some(o) = &answer.target_owner {
            span.record("rubato.target_owner", o.as_str());
        }
        span.record("rubato.outcome", answer.outcome.name());
        if !why.is_empty() {
            span.record("rubato.reason", why);
            tracing::warn!(rubato.launch_id = %req.lead_launch_id, rubato.target_launch_id = ?req.action.target(), rubato.target_owner = ?answer.target_owner, rubato.reason = why, "fleet request refused");
        }
        count(app, action, answer.outcome.name(), why);
        let result = answer.result(req);
        if let Err(e) = store::finish(&app.db, companion_id, &result, now).await {
            tracing::error!(slice = "fleet", error = %format!("{e:#}"), "fleet result not recorded");
        }
        Some(result)
    }
    .instrument(s)
    .await
}

/// A replayed envelope's request: its stored answer, never a second action.
pub async fn on_replay(app: &App, companion_id: Uuid, req: &FleetRequest) -> Option<FleetResult> {
    if !app.announced(companion_id, capability::FLEET) {
        return None;
    }
    store::stored(&app.db, companion_id, req)
        .await
        .ok()
        .flatten()
}

/// Steps 3 to 8 for a newly claimed request.
async fn act(app: &App, companion_id: Uuid, user: &str, req: &FleetRequest) -> Answer {
    match act_inner(app, companion_id, user, req).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(slice = "fleet", error = %format!("{e:#}"), "fleet request failed");
            Answer::refused("internal", "the server failed while acting")
        }
    }
}

async fn act_inner(
    app: &App,
    companion_id: Uuid,
    user: &str,
    req: &FleetRequest,
) -> anyhow::Result<Answer> {
    // Step 3: the lead is a launch of this pairing's companion.
    let Some(lead) = launch::launch_target(&app.db, req.lead_launch_id).await? else {
        return Ok(Answer::refused(
            reason::IDENTITY,
            "no such launch on this companion",
        ));
    };
    if lead.companion_id != companion_id || !lead.user.eq_ignore_ascii_case(user) {
        return Ok(Answer::refused(
            reason::IDENTITY,
            "that launch is not one of this companion's",
        ));
    }
    // Step 4: a lead.
    if lead.role != LaunchRole::Lead {
        return Ok(Answer::refused(
            reason::NOT_A_LEAD,
            "only a lead launch has the fleet tools",
        ));
    }
    // Step 5: the owner is the pairing's user.
    let owner = user;
    match &req.action {
        FleetAction::List => Ok(Answer {
            fleet: Some(list(app, owner, &lead.workspace).await?),
            ..Answer::ok()
        }),
        FleetAction::Message { launch_id, text } => {
            if let Some(refused) = target_check(app, owner, req.lead_launch_id, *launch_id).await? {
                return Ok(refused);
            }
            // The lead's own text must say something; the line naming it is added after.
            if text.trim().is_empty() {
                return Ok(Answer::refused(reason::EMPTY, "the message is empty"));
            }
            let framed = format!(
                "From {owner}'s lead agent (launch {}):\n{text}",
                short(req.lead_launch_id)
            );
            match session_messages::enqueue_chat_via(
                app,
                *launch_id,
                &framed,
                owner,
                req.lead_launch_id,
            )
            .await?
            {
                Ok(id) => Ok(Answer {
                    message_id: Some(id),
                    ..Answer::ok()
                }),
                Err(session_messages::ChatRefused::Empty) => {
                    Ok(Answer::refused(reason::EMPTY, "the message is empty"))
                }
                Err(session_messages::ChatRefused::TooLong) => Ok(Answer::refused(
                    reason::TOO_LONG,
                    "the message is longer than a session takes, with the line naming you",
                )),
                Err(session_messages::ChatRefused::UnknownLaunch) => {
                    Ok(Answer::refused(reason::UNKNOWN_LAUNCH, "no such launch"))
                }
            }
        }
        FleetAction::Stop { launch_id } => {
            if let Some(refused) = target_check(app, owner, req.lead_launch_id, *launch_id).await? {
                return Ok(refused);
            }
            match session_messages::stop_as(app, owner, *launch_id, Some(req.lead_launch_id)).await
            {
                Ok(v) => Ok(Answer {
                    message_id: Some(v.id),
                    ..Answer::ok()
                }),
                Err(session_messages::StopRefused::No(_, why)) => Ok(Answer::refused(
                    reason::LAUNCH_REFUSED,
                    format!("the stop was not sent: {why}"),
                )),
                Err(session_messages::StopRefused::Internal(e)) => Err(e),
            }
        }
        FleetAction::Launch {
            issue,
            companion_id: target_companion,
            slug,
        } => {
            // Step 8: the fleet size, then someone else's issue, then the ordinary launch.
            let live = live_workers(app, req.lead_launch_id).await?;
            if live >= app.config.fleet_max as usize {
                return Ok(Answer::refused(
                    reason::FLEET_FULL,
                    format!("{live} live workers; the most is {}", app.config.fleet_max),
                ));
            }
            let since = Utc::now() - Duration::days(limits::SOMEONE_ELSES_DAYS);
            let drafted_by_another = plan_drafts::latest_drafts(app)
                .await?
                .iter()
                .any(|d| d.issue == *issue && !d.summary.owner.eq_ignore_ascii_case(owner));
            if drafted_by_another
                || launch::launched_by_another(&app.db, issue, owner, since).await?
            {
                return Ok(Answer::refused(
                    reason::SOMEONE_ELSES_ISSUE,
                    "another person launched or drafted that issue",
                ));
            }
            let note = format!(
                "This session was started by {owner}'s lead agent (launch {}). It may message or stop you; your plan is still approved by a person in Rubato.",
                short(req.lead_launch_id)
            );
            let r = launch::start(
                app,
                &lead.workspace,
                owner,
                LaunchRequest {
                    issue: issue.clone(),
                    companion_id: *target_companion,
                    slug: slug.clone(),
                    branch: None,
                    spend_cap_usd: None,
                    attempt_limit: None,
                    prompt: None,
                    role: LaunchRole::Worker,
                },
                Some(&note),
                Some(req.lead_launch_id),
            )
            .await;
            match r {
                Ok(resp) => Ok(Answer {
                    launch_id: Some(resp.command.id),
                    ..Answer::ok()
                }),
                Err((status, why)) => Ok(Answer::refused(
                    reason::LAUNCH_REFUSED,
                    format!("{status}: {why}"),
                )),
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: skip step 6, so the must-fail test shows the boundary test bites.
    pub static SKIP_TARGET_CHECK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Step 6 for a message or a stop: `None` when the lead may act on `target`.
async fn target_check(
    app: &App,
    owner: &str,
    lead: Uuid,
    target: Uuid,
) -> anyhow::Result<Option<Answer>> {
    #[cfg(test)]
    if SKIP_TARGET_CHECK.with(|s| s.get()) {
        return Ok(None);
    }
    let Some(t) = launch::launch_target(&app.db, target).await? else {
        return Ok(Some(Answer::refused(
            reason::UNKNOWN_LAUNCH,
            "no such launch",
        )));
    };
    if !t.user.eq_ignore_ascii_case(owner) {
        return Ok(Some(Answer {
            target_owner: Some(t.user.clone()),
            ..Answer::refused(reason::OTHER_OWNER, "that is another person's session")
        }));
    }
    if target == lead || t.lead_launch_id != Some(lead) {
        return Ok(Some(Answer {
            target_owner: Some(t.user.clone()),
            ..Answer::refused(reason::NOT_LED, "you may act only on workers you launched")
        }));
    }
    Ok(None)
}

fn short(id: Uuid) -> String {
    id.simple().to_string()[..8].to_owned()
}

/// Sessions of the latest snapshots by launch id.
fn sessions_by_launch(app: &App) -> HashMap<Uuid, SessionStatus> {
    app.latest_sessions()
        .into_iter()
        .filter_map(|s| s.launch_id.map(|l| (l, s.status)))
        .collect()
}

/// A worker is live while its companion has not answered the launch, or it started and its
/// session is in a snapshot and has not ended.
fn is_live(row: &launch::FleetLaunch, live: &HashMap<Uuid, SessionStatus>) -> bool {
    match row.outcome.as_deref() {
        None => true,
        Some("started") => live
            .get(&row.launch_id)
            .is_some_and(|s| *s != SessionStatus::Ended),
        _ => false,
    }
}

/// How many of `lead`'s workers are live.
pub async fn live_workers(app: &App, lead: Uuid) -> anyhow::Result<usize> {
    let live = sessions_by_launch(app);
    Ok(launch::workers_of(&app.db, lead)
        .await?
        .iter()
        .filter(|r| is_live(r, &live))
        .count())
}

/// `fleet_list`: the owner's launches in `ws` live or started in the last 24 hours, newest first,
/// at most 50. Other people's launches are never in it.
async fn list(app: &App, owner: &str, ws: &str) -> anyhow::Result<Vec<FleetEntry>> {
    let live = sessions_by_launch(app);
    let since = Utc::now() - Duration::hours(limits::LIST_HOURS);
    let mut rows = launch::launches_of(&app.db, owner, ws, since, limits::LIST_MAX).await?;
    let have: HashSet<Uuid> = rows.iter().map(|r| r.launch_id).collect();
    let older: Vec<Uuid> = live
        .keys()
        .filter(|id| !have.contains(id))
        .copied()
        .collect();
    rows.extend(launch::launches_by_id(&app.db, owner, &older).await?);
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    rows.truncate(limits::LIST_MAX as usize);
    let ids: Vec<Uuid> = rows.iter().map(|r| r.launch_id).collect();
    let waiting = session_messages::waiting_counts(app, &ids).await?;
    let replies = session_log::last_replies(&app.db, &ids).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(FleetEntry {
            launch_id: r.launch_id,
            issue: r.issue.clone(),
            branch: r.branch.clone(),
            companion_id: r.companion_id,
            client: None,
            state: live.get(&r.launch_id).copied(),
            progress: None,
            role: r.role,
            started_by: r
                .lead_launch_id
                .map_or_else(|| "person".to_owned(), |l| format!("lead:{l}")),
            queued_messages: waiting.get(&r.launch_id).copied().unwrap_or(0),
            last_reply: replies
                .get(&r.launch_id)
                .map(|t| first_line(t, limits::LAST_REPLY_CHARS)),
        });
    }
    Ok(out)
}

fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() <= max {
        line.to_owned()
    } else {
        let mut cut: String = line.chars().take(max - 1).collect();
        cut.push('…');
        cut
    }
}
