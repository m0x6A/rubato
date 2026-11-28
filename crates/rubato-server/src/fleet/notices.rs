//! What wakes a lead (plan 134 design §2): when one of its workers ends, becomes blocked, or goes
//! idle after working, the lead gets a short chat from `rubato`, at most one per lead every
//! `NOTICE_EVERY_SECS`, listing every change since the last. Through the same queue as any chat
//! (ADR 0051), so it is typed only when the lead waits for input and counts on the lead's cap.
//! Nothing at all while the orchestrator flag is off.

use crate::kernel::state::App;
use crate::kernel::tenancy::WsKey;
use crate::{launch, session_messages};
use chrono::{DateTime, Duration, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::fleet::limits;
use rubato_protocol::otel::attr;
use rubato_protocol::{LaunchRole, LeadView, SessionMessageKind, SessionState, SessionStatus};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// The author of a fleet notice.
pub const AUTHOR: &str = "rubato";

#[derive(Default)]
struct Inner {
    /// Each launch's status in the last snapshot it was in.
    last: HashMap<Uuid, SessionStatus>,
    /// Lines waiting per lead.
    pending: HashMap<Uuid, Vec<String>>,
    sent: HashMap<Uuid, DateTime<Utc>>,
}

/// In memory, per server: gone on restart, when the next change starts over.
#[derive(Default)]
pub struct Notices(Mutex<Inner>);

/// What a worker's status change says to its lead, if anything.
fn change(before: Option<SessionStatus>, now: SessionStatus) -> Option<&'static str> {
    match (before, now) {
        (Some(b), n) if b == n => None,
        (_, SessionStatus::Ended) => Some("ended"),
        (_, SessionStatus::Blocked) => Some("is blocked"),
        (Some(SessionStatus::Working), SessionStatus::Idle) => Some("is idle"),
        _ => None,
    }
}

/// A companion's new snapshot: note each worker's change, then send what is due.
pub async fn on_sessions(app: &App, sessions: &[SessionState]) {
    if !app.config.orchestrator {
        return;
    }
    let notices = app.slots.get::<Notices>();
    let mut changes: Vec<(Uuid, SessionStatus, Option<SessionStatus>)> = Vec::new();
    {
        let mut g = notices.0.lock().unwrap();
        for s in sessions {
            let Some(l) = s.launch_id else { continue };
            let before = g.last.insert(l, s.status);
            if change(before, s.status).is_some() {
                changes.push((l, s.status, before));
            }
        }
    }
    let mut lines: Vec<(Uuid, String)> = Vec::new();
    for (l, status, before) in changes {
        let Ok(Some(t)) = launch::launch_target(&app.db, l).await else {
            continue;
        };
        let Some(lead) = t.lead_launch_id else {
            continue;
        };
        let what = change(before, status).unwrap_or("changed");
        lines.push((
            lead,
            format!(
                "worker {} on {} {what}",
                &l.simple().to_string()[..8],
                t.issue
            ),
        ));
    }
    flush(app, &notices, lines, app.clock.now()).await;
}

async fn flush(app: &App, notices: &Notices, lines: Vec<(Uuid, String)>, now: DateTime<Utc>) {
    let due: Vec<(Uuid, Vec<String>)> = {
        let mut g = notices.0.lock().unwrap();
        for (lead, line) in lines {
            g.pending.entry(lead).or_default().push(line);
        }
        let every = Duration::seconds(limits::NOTICE_EVERY_SECS);
        let ready: Vec<Uuid> = g
            .pending
            .keys()
            .filter(|lead| g.sent.get(lead).is_none_or(|at| now - *at >= every))
            .copied()
            .collect();
        ready
            .into_iter()
            .filter_map(|lead| {
                g.sent.insert(lead, now);
                g.pending.remove(&lead).map(|v| (lead, v))
            })
            .collect()
    };
    for (lead, items) in due {
        let text = format!("Fleet: {}.", items.join("; "));
        let kind = SessionMessageKind::Chat {
            text,
            author: AUTHOR.to_owned(),
        };
        match session_messages::enqueue(app, lead, kind, AUTHOR, session_messages::CHAT_PRIORITY)
            .await
        {
            Ok(_) => {
                app.metrics
                    .fleet_notices
                    .add(1, &[KeyValue::new(attr::OUTCOME, "queued")]);
                if items.len() > 1 {
                    app.metrics.fleet_notices.add(
                        items.len() as u64 - 1,
                        &[KeyValue::new(attr::OUTCOME, "coalesced")],
                    );
                }
            }
            Err(e) => {
                tracing::error!(slice = "fleet", rubato.launch_id = %lead, error = %format!("{e:#}"), "fleet notice not queued")
            }
        }
    }
}

/// Each live lead in workspace `ws` and its live workers, from the latest snapshots. Empty while
/// the orchestrator flag is off.
pub async fn leads(app: &App, ws: &WsKey) -> Vec<LeadView> {
    if !app.config.orchestrator {
        return vec![];
    }
    let live: HashMap<Uuid, SessionStatus> = app
        .latest_sessions()
        .into_iter()
        .filter_map(|s| s.launch_id.map(|l| (l, s.status)))
        .filter(|(_, s)| *s != SessionStatus::Ended)
        .collect();
    let mut out = Vec::new();
    for l in live.keys() {
        let Ok(Some(t)) = launch::launch_target(&app.db, *l).await else {
            continue;
        };
        if t.role != LaunchRole::Lead || launch::ws_key(app, &t).as_ref() != Some(ws) {
            continue;
        }
        let workers = launch::workers_of(&app.db, &t.tenant, *l)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.launch_id)
            .filter(|w| live.contains_key(w))
            .collect();
        out.push(LeadView {
            launch_id: *l,
            owner: t.user,
            workers,
        });
    }
    out.sort_by_key(|v| v.launch_id);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_end_a_block_or_working_then_idle_tells_the_lead() {
        use SessionStatus::*;
        let cases = [
            (None, Working, None),
            (Some(Working), Working, None),
            (Some(Working), Idle, Some("is idle")),
            (None, Idle, None),
            (Some(Idle), Blocked, Some("is blocked")),
            (Some(Working), Ended, Some("ended")),
            (None, Ended, Some("ended")),
            (Some(Blocked), Working, None),
            (Some(Idle), Unknown, None),
        ];
        for (b, n, want) in cases {
            assert_eq!(change(b, n), want, "{b:?} -> {n:?}");
        }
        println!("checked {} status changes", cases.len());
    }
}
