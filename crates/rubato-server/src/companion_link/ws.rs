//! The companion websocket (ADR 0012): bearer token, envelopes with sequence numbers, acks,
//! launch commands out. A span per inbound event with the session and issue as attributes.

use super::store;
use crate::kernel::state::{App, CompanionConn};
use crate::kernel::telemetry::Metrics;
use crate::{launch, pairing, queue};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{
    CompanionEnvelope, CompanionMessage, PROTOCOL_VERSION, ServerEnvelope, ServerMessage,
    SessionState, SessionStatus,
};
use tokio::sync::mpsc;
use tracing::Instrument;
use uuid::Uuid;

pub async fn upgrade(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    let pairing = match pairing::authenticate(&app.db, token).await {
        Ok(Some(p)) if p.revoked_at.is_none() => p,
        Ok(_) => {
            app.metrics
                .events_dropped
                .add(1, &[KeyValue::new(attr::DROP_REASON, "auth")]);
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "pairing lookup");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    ws.on_upgrade(move |socket| handle(app, pairing, socket))
}

/// A bookkeeping write on the companion socket that failed. The envelope is still answered,
/// so without this line a lost sequence or last-seen write would be silent (issue #28).
fn observe(write: &'static str, companion_id: Uuid, seq: u64, result: anyhow::Result<()>) {
    if let Err(e) = result {
        tracing::error!(slice = "companion_link", write, rubato.companion_id = %companion_id, seq, error = %e, "companion bookkeeping write failed");
    }
}

async fn send_env(
    app: &App,
    companion_id: Uuid,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerMessage,
) -> bool {
    let seq = pairing::next_server_seq(&app.db, companion_id)
        .await
        .unwrap_or(0);
    let env = ServerEnvelope {
        seq,
        sent_at: Utc::now(),
        message,
    };
    match serde_json::to_string(&env) {
        Ok(text) => sink.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn handle(app: App, pairing: pairing::PairingRow, socket: WebSocket) {
    let companion_id = pairing.companion_id;
    let user = pairing.user.clone();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    app.companions
        .lock()
        .unwrap()
        .insert(companion_id, CompanionConn { tx });
    app.metrics.ws_clients.record(
        app.companions.lock().unwrap().len() as u64,
        &[KeyValue::new("kind", "companion")],
    );
    tracing::info!(%companion_id, %user, "companion connected");
    app.notify("companion connected");

    loop {
        tokio::select! {
            Some(msg) = rx.recv() => {
                if !send_env(&app, companion_id, &mut sink, msg).await {
                    break;
                }
            }
            inbound = stream.next() => {
                let Some(Ok(frame)) = inbound else { break };
                match frame {
                    Message::Text(text) => {
                        let env: CompanionEnvelope = match serde_json::from_str(&text) {
                            Ok(e) => e,
                            Err(e) => {
                                app.metrics.events_dropped.add(1, &[KeyValue::new(attr::DROP_REASON, "schema")]);
                                tracing::warn!(%companion_id, error = %e, "undecodable companion envelope dropped");
                                continue;
                            }
                        };
                        if let Some(reply) = on_envelope(&app, &pairing, env).await
                            && !send_env(&app, companion_id, &mut sink, reply).await {
                            break;
                        }
                    }
                    Message::Ping(p) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    app.companions.lock().unwrap().remove(&companion_id);
    app.metrics.ws_clients.record(
        app.companions.lock().unwrap().len() as u64,
        &[KeyValue::new("kind", "companion")],
    );
    tracing::info!(%companion_id, "companion disconnected");
    app.notify("companion disconnected");
}

/// Why an already recorded envelope counts as a dropped event, if it does. A companion replays
/// its outbox after a reconnect, and that is quiet. A hello is never in an outbox, so a hello at
/// a recorded sequence number means the companion is reusing numbers and everything it sends
/// below the record is being discarded (plan 29 S5).
fn replay_drop_reason(message: &CompanionMessage) -> Option<&'static str> {
    matches!(message, CompanionMessage::Hello { .. }).then_some("replay")
}

fn on_replay(metrics: &Metrics, companion_id: Uuid, env: &CompanionEnvelope) {
    match replay_drop_reason(&env.message) {
        Some(reason) => {
            tracing::warn!(rubato.companion_id = %companion_id, seq = env.seq, "hello replayed: sequence collision");
            metrics
                .events_dropped
                .add(1, &[KeyValue::new(attr::DROP_REASON, reason)]);
        }
        None => tracing::debug!(seq = env.seq, "replayed envelope already recorded"),
    }
}

/// Record the envelope, update rows, and answer. Returns a reply when one is due.
async fn on_envelope(
    app: &App,
    pairing: &pairing::PairingRow,
    env: CompanionEnvelope,
) -> Option<ServerMessage> {
    let companion_id = pairing.companion_id;
    let kind = match &env.message {
        CompanionMessage::Hello { .. } => "hello",
        CompanionMessage::Heartbeat(_) => "heartbeat",
        CompanionMessage::Hook(_) => "hook",
        CompanionMessage::Sessions { .. } => "sessions",
        CompanionMessage::Launch(_) => "launch",
        CompanionMessage::CapHit(_) => "cap_hit",
        CompanionMessage::Stale(_) => "stale",
        CompanionMessage::Ack { .. } => "ack",
    };
    let span = tracing::info_span!(span::INBOUND_EVENT, rubato.companion_id = %companion_id, rubato.user = %pairing.user, kind, seq = env.seq);
    async move {
        let now = Utc::now();
        if !matches!(env.message, CompanionMessage::Ack { .. }) {
            let payload = serde_json::to_value(&env.message).unwrap_or_default();
            match store::record_event(&app.db, companion_id, env.seq, kind, &payload, now).await {
                Ok(false) => {
                    on_replay(&app.metrics, companion_id, &env);
                    return Some(ServerMessage::Ack { seq: env.seq });
                }
                Ok(true) => {}
                Err(e) => tracing::error!(error = %e, "recording event"),
            }
            observe(
                "set_last_companion_seq",
                companion_id,
                env.seq,
                pairing::set_last_companion_seq(&app.db, companion_id, env.seq).await,
            );
        }
        observe(
            "touch",
            companion_id,
            env.seq,
            pairing::touch(&app.db, companion_id, now).await,
        );
        match env.message {
            CompanionMessage::Hello {
                companion_id: claimed,
                protocol_version,
                user,
                ..
            } => {
                if claimed != companion_id || user != pairing.user {
                    app.metrics
                        .events_dropped
                        .add(1, &[KeyValue::new(attr::DROP_REASON, "auth")]);
                    return Some(ServerMessage::Error {
                        code: "auth".into(),
                        message: "hello does not match the pairing".into(),
                    });
                }
                if protocol_version != PROTOCOL_VERSION {
                    return Some(ServerMessage::Error {
                        code: "protocol".into(),
                        message: format!(
                            "server speaks protocol {PROTOCOL_VERSION}, companion {protocol_version}"
                        ),
                    });
                }
                Some(ServerMessage::Welcome {
                    server_version: crate::kernel::VERSION.to_owned(),
                    protocol_version: PROTOCOL_VERSION,
                    last_seen_companion_seq: pairing.last_companion_seq,
                    limits: app.config.limits,
                })
            }
            CompanionMessage::Heartbeat(h) => {
                app.metrics.companion_heartbeat.record(
                    h.at.timestamp() as u64,
                    &[
                        KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
                        KeyValue::new(attr::USER, pairing.user.clone()),
                    ],
                );
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Hook(h) => {
                tracing::info!(rubato.session_id = %h.session_id, rubato.hook_event = ?h.hook_event_name, "hook");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Sessions { mut sessions } => {
                if let Some(map) = app.map.read().await.as_ref() {
                    let named: Vec<&str> = map.map.repos_named().into_iter().collect();
                    canonicalise_repos(&mut sessions, &named);
                }
                if let Err(e) = store::store_snapshot(&app.db, companion_id, &sessions, now).await {
                    tracing::error!(error = %e, "storing snapshot");
                }
                record_sessions_by_state(&app.metrics, companion_id, &sessions);
                app.notify("sessions");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Launch(result) => {
                match launch::record_result(&app.db, &result).await {
                    Ok(Some(issue)) => {
                        tracing::info!(rubato.issue = %issue, outcome = ?std::mem::discriminant(&result), "launch result");
                        settle_queue(app, companion_id, &issue, &result).await;
                    }
                    Ok(None) => tracing::warn!("launch result for an unknown command"),
                    Err(e) => tracing::error!(error = %e, "storing launch result"),
                }
                app.notify("launch");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::CapHit(hit) => {
                tracing::warn!(rubato.issue = %hit.issue, rubato.session_id = ?hit.session_id, kind = ?hit.kind, value = hit.value, limit = hit.limit, "{}", rubato_protocol::otel::event::CAP_HIT);
                if let Err(e) = launch::record_cap_hit(&app.db, &hit, now).await {
                    tracing::error!(error = %e, "storing cap hit");
                }
                app.notify("cap hit");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Stale(s) => {
                tracing::warn!(rubato.session_id = ?s.session_id, silent_for_secs = s.silent_for_secs, "{}", rubato_protocol::otel::event::SESSION_STALE);
                observe(
                    "store_stale",
                    companion_id,
                    env.seq,
                    store::store_stale(&app.db, &s).await,
                );
                app.notify("stale");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Ack { .. } => None,
        }
    }
    .instrument(span)
    .await
}

/// `rubato.sessions.by_state` for one companion's snapshot: every status, zero included. A
/// gauge keeps its last value, so without the zero a session that stops working would leave
/// `working` at 1 and `SessionWorkingNoHooks` would page on every finished turn.
/// A remote may spell a repo differently from the map (`M0x6A/Rubato`). Each session's repo,
/// and the repo of its issue, take the map's spelling when they match it without case, so the
/// session joins the pinned issue and no stored row changes spelling (design 80 §5).
pub fn canonicalise_repos(sessions: &mut [SessionState], named: &[&str]) {
    for s in sessions {
        let Some(repo) = s.repo.clone() else { continue };
        let Some(canonical) = named
            .iter()
            .find(|n| n.eq_ignore_ascii_case(&repo) && **n != repo)
        else {
            continue;
        };
        tracing::info!(from = %repo, to = %canonical, "repo canonicalised");
        s.repo = Some((*canonical).to_owned());
        if let Some(issue) = &mut s.issue
            && issue.repo.eq_ignore_ascii_case(canonical)
        {
            issue.repo = (*canonical).to_owned();
        }
    }
}

fn record_sessions_by_state(metrics: &Metrics, companion_id: Uuid, sessions: &[SessionState]) {
    for status in SessionStatus::ALL {
        let n = sessions.iter().filter(|s| s.status == status).count();
        metrics.sessions_by_state.record(
            n as u64,
            &[
                KeyValue::new(attr::SESSION_STATE, status.name()),
                KeyValue::new(attr::COMPANION_ID, companion_id.to_string()),
            ],
        );
    }
}

/// A launch result settles the issue's queue entry on this companion, if it has one: started
/// takes it off the lane; refused or failed keeps it there with the reason (ADR 0020).
async fn settle_queue(
    app: &App,
    companion_id: Uuid,
    issue: &rubato_protocol::IssueRef,
    result: &rubato_protocol::LaunchResult,
) {
    use rubato_protocol::LaunchResult;
    let why = match result {
        LaunchResult::Started { .. } => None,
        LaunchResult::Refused { reason, detail, .. } => {
            let reason = serde_json::to_value(reason)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            Some(format!("{reason}: {detail}"))
        }
        LaunchResult::Failed { error, .. } => Some(error.clone()),
    };
    let done = match &why {
        None => queue::launch_started(&app.db, companion_id, issue).await,
        Some(w) => queue::launch_not_started(&app.db, companion_id, issue, w).await,
    };
    match (done, why) {
        (Ok(true), Some(why)) => tracing::info!(
            rubato.companion_id = %companion_id,
            rubato.issue = %issue,
            reason = %why,
            "queue entry kept, launch not started"
        ),
        (Ok(_), _) => {}
        (Err(e), _) => {
            tracing::error!(rubato.companion_id = %companion_id, rubato.issue = %issue, error = %e, "settling the queue entry failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::{capture, metric_reader};
    use rubato_protocol::otel::metric;
    use rubato_protocol::{IssueRef, StateSource};

    #[test]
    fn a_remote_spelled_differently_takes_the_maps_spelling() {
        let mut s = session(Uuid::nil(), SessionStatus::Working);
        s.repo = Some("M0x6A/Rubato".into());
        s.issue = Some(IssueRef::new("M0x6A/Rubato", 5));
        let mut other = session(Uuid::nil(), SessionStatus::Working);
        other.repo = Some("someone/else".into());
        other.issue = None;
        let mut all = vec![s, other];
        canonicalise_repos(&mut all, &["m0x6A/rubato"]);
        assert_eq!(all[0].repo.as_deref(), Some("m0x6A/rubato"));
        assert_eq!(all[0].issue, Some(IssueRef::new("m0x6A/rubato", 5)));
        assert_eq!(
            all[1].repo.as_deref(),
            Some("someone/else"),
            "not in the map: unchanged"
        );
    }

    fn session(companion_id: Uuid, status: SessionStatus) -> SessionState {
        SessionState {
            session_id: Some("s1".into()),
            companion_id,
            user: "m0x6A".into(),
            tmux: None,
            cwd: "/x".into(),
            worktree: None,
            branch: None,
            repo: None,
            issue: None,
            status,
            source: StateSource::Hook,
            last_seen: Utc::now(),
            spend_usd: None,
            branch_drift: None,
            blocked_by: None,
        }
    }

    #[test]
    fn a_snapshot_without_a_working_session_records_working_zero() {
        let id = Uuid::new_v4();
        let (reader, metrics) = metric_reader::start();
        let working = |points: &[metric_reader::Point]| {
            points
                .iter()
                .find(|p| p.attrs[attr::SESSION_STATE] == "working")
                .map(|p| p.value)
        };

        record_sessions_by_state(&metrics, id, &[session(id, SessionStatus::Working)]);
        assert_eq!(working(&reader.points(metric::SESSIONS_BY_STATE)), Some(1));

        // The turn ends. A gauge keeps its last value, so the 0 has to be recorded.
        record_sessions_by_state(&metrics, id, &[session(id, SessionStatus::Idle)]);
        let points = reader.points(metric::SESSIONS_BY_STATE);
        assert_eq!(working(&points), Some(0));
        assert_eq!(points.len(), 5, "one point per status: {points:?}");
        for p in &points {
            assert_eq!(p.attrs[attr::COMPANION_ID], id.to_string());
        }

        // An empty snapshot is all zeros.
        record_sessions_by_state(&metrics, id, &[]);
        let points = reader.points(metric::SESSIONS_BY_STATE);
        assert_eq!(points.len(), 5);
        assert!(points.iter().all(|p| p.value == 0), "{points:?}");
    }

    fn fixture(name: &str) -> CompanionEnvelope {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rubato-protocol/fixtures")
            .join(name);
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn a_replayed_hello_is_a_warning_and_a_counted_drop_and_a_heartbeat_is_quiet() {
        let hello = fixture("companion_hello.json");
        let beat = fixture("companion_heartbeat.json");
        assert_eq!(replay_drop_reason(&hello.message), Some("replay"));
        assert_eq!(replay_drop_reason(&beat.message), None);

        let (events, _g) = capture::start();
        let (reader, metrics) = metric_reader::start();
        let dropped = || reader.points(metric::EVENTS_DROPPED);
        let companion_id = Uuid::new_v4();
        on_replay(&metrics, companion_id, &beat);
        assert_eq!(dropped(), [], "an outbox replay is not a drop");
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .all(|e| e.level > tracing::Level::INFO),
            "an outbox replay stays below info"
        );
        on_replay(&metrics, companion_id, &hello);
        let events = events.lock().unwrap();
        let warned: Vec<_> = events
            .iter()
            .filter(|e| e.level == tracing::Level::WARN)
            .collect();
        assert_eq!(warned.len(), 1, "{events:?}");
        assert_eq!(warned[0].message, "hello replayed: sequence collision");
        assert_eq!(
            warned[0].fields.get("rubato.companion_id"),
            Some(&companion_id.to_string())
        );
        assert_eq!(warned[0].fields.get("seq").map(String::as_str), Some("1"));

        // The drop is counted once, under reason `replay` (#41).
        let points = dropped();
        assert_eq!(points.len(), 1, "{points:?}");
        assert_eq!(points[0].value, 1);
        assert_eq!(points[0].attrs[attr::DROP_REASON], "replay");
    }

    #[test]
    fn a_failed_bookkeeping_write_is_logged_with_companion_and_seq() {
        let (events, _guard) = capture::start();
        let id = Uuid::new_v4();
        observe("touch", id, 7, Ok(()));
        assert!(events.lock().unwrap().is_empty(), "success is quiet");
        observe("touch", id, 7, Err(anyhow::anyhow!("database is locked")));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let e = &events[0];
        assert_eq!(e.level, tracing::Level::ERROR);
        assert_eq!(e.message, "companion bookkeeping write failed");
        assert_eq!(e.fields.get("write").map(String::as_str), Some("touch"));
        assert_eq!(e.fields.get("seq").map(String::as_str), Some("7"));
        assert_eq!(
            e.fields.get("rubato.companion_id").cloned(),
            Some(id.to_string())
        );
        assert_eq!(
            e.fields.get("error").map(String::as_str),
            Some("database is locked")
        );
    }
}
