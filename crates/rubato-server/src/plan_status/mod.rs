//! Slice `plan_status`: where the plan of each launched session stands, pushed to the companion
//! that runs it (ADR 0071, plan 226 §4), so its agent's `plan_status` tool can answer. Derived
//! from the overview's `IssueSummary` (the draft with its own fit, the review, the counted
//! approval with its version, the plan on the branch); never stored, never an input to progress.
//!
//! Driven by the composition root only: on every poll tick with the overviews `refresh` built,
//! and right after a companion is welcomed (the kernel's broadcast), from the last tick's
//! statuses. Names `launch` (which companion a launch went to, and its workspace). Named by
//! nobody but the composition root: `overview` names `companion_link`, so `companion_link` may
//! not name this slice. Owns no table; its state is in memory (`App::slots`).

use crate::kernel::state::App;
use crate::launch;
use chrono::{DateTime, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    BranchStatus, DraftStatus, IssueSummary, PlanStatus, ServerMessage, SessionStatus, capability,
};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use uuid::Uuid;

/// The statuses last derived, by launch (with the companion it went to), and the ones last sent
/// on each companion socket.
#[derive(Default)]
struct Pushes {
    derived: Mutex<HashMap<Uuid, (Uuid, PlanStatus)>>,
    sent: Mutex<HashMap<(Uuid, Uuid), HashMap<Uuid, PlanStatus>>>,
}

/// Where launch `launch_id`'s plan stands, from its issue's summary. `branch` is the session's
/// branch: a draft on another branch is not this launch's.
pub fn derive(
    summary: &IssueSummary,
    launch_id: Uuid,
    branch: Option<&str>,
    now: DateTime<Utc>,
) -> PlanStatus {
    let draft = summary
        .draft
        .as_ref()
        .filter(|d| branch.is_none_or(|b| b == d.branch))
        .map(|d| DraftStatus {
            rev: d.rev,
            at: d.at,
            plan_sha: d.plan_sha.clone(),
            fits: d.fits == Some(true),
        });
    let branch_plan = summary.plan.as_ref().map(|p| BranchStatus {
        sha: summary.plan_sha.clone().unwrap_or_default(),
        carries_approval: p.approval.is_some(),
    });
    PlanStatus {
        launch_id,
        as_of: now,
        draft,
        review: summary.review.clone(),
        approval: summary.approval.clone(),
        branch: branch_plan,
    }
}

/// The same status, whenever it was derived.
fn same(a: &PlanStatus, b: &PlanStatus) -> bool {
    PlanStatus {
        as_of: b.as_of,
        ..a.clone()
    } == *b
}

fn count(app: &App, reason: &'static str, outcome: &'static str) {
    app.metrics.plan_status_sent.add(
        1,
        &[
            KeyValue::new(attr::DROP_REASON, reason),
            KeyValue::new(attr::OUTCOME, outcome),
        ],
    );
}

fn send(app: &App, companion_id: Uuid, s: PlanStatus, reason: &'static str) -> bool {
    let ok = app.send_to_companion(companion_id, ServerMessage::PlanStatus(Box::new(s)));
    count(app, reason, if ok { "sent" } else { "offline" });
    ok
}

/// Connected companions that take `PlanStatus`, with their socket ids. A companion's
/// capabilities are kept after it disconnects, so only connected ones are asked.
fn takers(app: &App) -> Vec<(Uuid, Uuid)> {
    app.connected_companions()
        .into_iter()
        .filter(|c| app.announced(*c, capability::AGENT_MCP))
        .filter_map(|c| Some((c, app.companion_conn(c)?)))
        .collect()
}

/// Every poll tick: derive each live launch's status from its own workspace's overview and send
/// what changed since the last one sent on that companion's socket (`welcome` for a socket not
/// sent on yet, `change` otherwise).
pub async fn on_tick(app: &App, built: &[(crate::kernel::tenancy::WsKey, Vec<IssueSummary>)]) {
    let now = Utc::now();
    let pushes = app.slots.get::<Pushes>();
    let companions = takers(app);
    let sessions = app.latest_sessions();
    let mut derived = HashMap::new();
    for (c, conn) in &companions {
        for s in sessions
            .iter()
            .filter(|s| s.companion_id == *c && s.status != SessionStatus::Ended)
        {
            let Some(l) = s.launch_id else { continue };
            let target = match launch::launch_target(&app.db, l).await {
                Ok(Some(t)) if t.companion_id == *c => t,
                Ok(_) => continue,
                Err(e) => {
                    tracing::warn!(rubato.companion_id = %c, rubato.launch_id = %l, error = %format!("{e:#}"), "plan status not derived");
                    continue;
                }
            };
            let Some(summary) = built
                .iter()
                .find(|(ws, _)| launch::ws_key(app, &target).as_ref() == Some(ws))
                .and_then(|(_, issues)| issues.iter().find(|i| i.issue == target.issue))
            else {
                count(app, "change", "no_summary");
                continue;
            };
            let st = derive(summary, l, s.branch.as_deref(), now);
            derived.insert(l, (*c, st.clone()));
            let mut sent = pushes.sent.lock().unwrap();
            let first = !sent.contains_key(&(*c, *conn));
            let last = sent.entry((*c, *conn)).or_default();
            if last.get(&l).is_some_and(|p| same(p, &st)) {
                continue;
            }
            if send(
                app,
                *c,
                st.clone(),
                if first { "welcome" } else { "change" },
            ) {
                last.insert(l, st);
            }
        }
    }
    *pushes.derived.lock().unwrap() = derived;
    let current: HashSet<(Uuid, Uuid)> = companions.into_iter().collect();
    pushes
        .sent
        .lock()
        .unwrap()
        .retain(|k, _| current.contains(k));
}

/// Right after a companion is welcomed: its live launches' last derived statuses on its new
/// socket, without building an overview. The next tick corrects anything this missed.
pub async fn on_welcome(app: &App, companion_id: Uuid) {
    if !app.announced(companion_id, capability::AGENT_MCP) {
        return;
    }
    let Some(conn) = app.companion_conn(companion_id) else {
        return;
    };
    let live: HashSet<Uuid> = app
        .latest_sessions()
        .iter()
        .filter(|s| s.companion_id == companion_id && s.status != SessionStatus::Ended)
        .filter_map(|s| s.launch_id)
        .collect();
    let pushes = app.slots.get::<Pushes>();
    let derived: Vec<(Uuid, PlanStatus)> = pushes
        .derived
        .lock()
        .unwrap()
        .iter()
        .filter(|(l, (c, _))| *c == companion_id && live.contains(*l))
        .map(|(l, (_, s))| (*l, s.clone()))
        .collect();
    let mut sent = pushes.sent.lock().unwrap();
    let on_socket = sent.entry((companion_id, conn)).or_default();
    let mut n = 0;
    for (l, s) in derived {
        if send(app, companion_id, s.clone(), "welcome") {
            on_socket.insert(l, s);
            n += 1;
        }
    }
    tracing::info!(rubato.companion_id = %companion_id, launches = n, "plan status pushed after welcome");
}

#[cfg(test)]
mod tests;
