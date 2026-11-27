//! The companion websocket (ADR 0012): bearer token, envelopes with sequence numbers, acks,
//! launch commands out. A span per inbound event with the session and issue as attributes.

use super::store;
use crate::kernel::state::{App, CompanionConn};
use crate::kernel::telemetry::Metrics;
use crate::{
    agent_reports, approve, fleet, launch, pairing, plan_drafts, queue, rooms, session_log,
    session_messages, spend_history,
};
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

/// The largest companion message the server reads (ADR 0066): twice the companion's 2 MiB hook
/// body limit, so an honest companion never meets it.
pub const COMPANION_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// The most sessions one `Sessions` snapshot may carry; a larger one is not stored (ADR 0066).
pub const MAX_SESSIONS_PER_SNAPSHOT: usize = 256;

pub async fn upgrade(State(app): State<App>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let ws = ws
        .max_message_size(COMPANION_MAX_MESSAGE_BYTES)
        .max_frame_size(COMPANION_MAX_MESSAGE_BYTES);
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let pairing = match pairing::authenticate(&app.db, &token).await {
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
    ws.on_upgrade(move |socket| handle(app, pairing, token, socket))
}

/// The pairing again, read by the socket's token before an envelope is recorded (#161). `None`
/// when it was revoked, re-paired with another token, or cannot be read: the envelope is
/// refused and the socket closed. Counted as `rubato.events.dropped{rubato.reason="revoked"}`.
async fn still_paired(
    app: &App,
    token: &str,
    companion_id: Uuid,
    seq: u64,
) -> Option<pairing::PairingRow> {
    let row = match pairing::authenticate(&app.db, token).await {
        Ok(Some(p)) if p.revoked_at.is_none() && p.companion_id == companion_id => return Some(p),
        Ok(_) => None,
        Err(e) => {
            tracing::error!(rubato.companion_id = %companion_id, error = %e, "pairing re-check failed");
            None
        }
    };
    refuse_revoked(&app.metrics, companion_id, seq);
    row
}

fn refuse_revoked(metrics: &Metrics, companion_id: Uuid, seq: u64) {
    tracing::warn!(rubato.companion_id = %companion_id, seq, "envelope from a revoked pairing dropped");
    metrics
        .events_dropped
        .add(1, &[KeyValue::new(attr::DROP_REASON, "revoked")]);
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

async fn handle(app: App, mut pairing: pairing::PairingRow, token: String, socket: WebSocket) {
    let companion_id = pairing.companion_id;
    let user = pairing.user.clone();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    let entry = CompanionConn::new(tx, app.clock.now());
    let conn = entry.conn;
    // A newer socket for the same companion replaces the entry; dropping the older sender
    // closes the older socket below.
    app.companions.lock().unwrap().insert(companion_id, entry);
    app.metrics.ws_clients.record(
        app.companions.lock().unwrap().len() as u64,
        &[KeyValue::new("kind", "companion")],
    );
    tracing::info!(%companion_id, %user, "companion connected");
    app.notify("companion connected");

    let reason = loop {
        tokio::select! {
            msg = rx.recv() => {
                // The sender is gone: the pairing was revoked (`pairing::revoke` drops it after
                // its `Error { auth }`), or a newer socket replaced this one. Close, so nothing
                // more from this companion is applied (ADR 0062, #161).
                let Some(msg) = msg else {
                    let _ = sink.close().await;
                    break "sender_dropped";
                };
                if !send_env(&app, companion_id, &mut sink, msg).await {
                    break "closed";
                }
            }
            inbound = stream.next() => {
                let frame = match inbound {
                    Some(Ok(frame)) => frame,
                    Some(Err(e)) => {
                        on_read_error(&app.metrics, companion_id, &e);
                        break "read_error";
                    }
                    None => break "closed",
                };
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
                        let Some(fresh) = still_paired(&app, &token, companion_id, env.seq).await else {
                            let _ = send_env(&app, companion_id, &mut sink, ServerMessage::Error {
                                code: "auth".into(),
                                message: "pairing revoked".into(),
                            }).await;
                            let _ = sink.send(Message::Close(None)).await;
                            break "revoked";
                        };
                        pairing = fresh;
                        if let Some(reply) = on_envelope(&app, &pairing, env).await
                            && !send_env(&app, companion_id, &mut sink, reply).await {
                            break "closed";
                        }
                    }
                    Message::Ping(p) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Message::Close(_) => break "closed",
                    _ => {}
                }
            }
        }
    };
    // #69: what the gauge says now. Read before the lock, recorded under it, so a newer socket's
    // `Hello` (which records after this socket's entry is replaced) is never overwritten.
    let cached = app.sessions_of(companion_id).unwrap_or_default();
    let sessions = {
        let mut companions = app.companions.lock().unwrap();
        let left = match companions.get(&companion_id) {
            Some(c) if c.conn != conn => Left::ToNewer,
            Some(_) => {
                companions.remove(&companion_id);
                if reason == "revoked" {
                    Left::Revoked
                } else {
                    Left::Gone
                }
            }
            // `pairing::revoke` removed the entry before the sender dropped.
            None => Left::Revoked,
        };
        match left {
            Left::Gone => record_sessions_by_state_unknown(&app.metrics, companion_id, &cached),
            Left::Revoked => record_sessions_by_state(&app.metrics, companion_id, &[]),
            Left::ToNewer => {}
        }
        left.as_str()
    };
    app.metrics.ws_clients.record(
        app.companions.lock().unwrap().len() as u64,
        &[KeyValue::new("kind", "companion")],
    );
    tracing::info!(%companion_id, reason, sessions, "companion disconnected");
    app.notify("companion disconnected");
}

/// A read that ended the socket. A message over the cap is a dropped event, counted
/// `too_large`; anything else (a reset) is logged only. The text is tungstenite's
/// `Space limit exceeded`, pinned by `app::tests::an_oversized_companion_message_ends_the_socket`.
fn on_read_error(metrics: &Metrics, companion_id: Uuid, e: &axum::Error) {
    let text = e.to_string();
    let too_large = text.contains("Space limit exceeded");
    tracing::warn!(rubato.companion_id = %companion_id, error = %text, too_large, "companion socket read failed");
    if too_large {
        metrics
            .events_dropped
            .add(1, &[KeyValue::new(attr::DROP_REASON, "too_large")]);
    }
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

/// Put the inbound span under the companion's send span when the envelope names one (ADR 0081),
/// and count what the field gave. It is used for nothing else: an envelope whose `traceparent`
/// does not parse is handled like one without (plan 287 A2).
fn link_to_sender(metrics: &Metrics, span: &tracing::Span, traceparent: Option<&str>) {
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
    };
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    let outcome = match traceparent.map(rubato_protocol::otel::TraceParent::parse) {
        None => "absent",
        Some(None) => "invalid",
        Some(Some(tp)) => {
            let remote = SpanContext::new(
                TraceId::from(tp.trace_id),
                SpanId::from(tp.span_id),
                TraceFlags::new(tp.flags),
                true,
                TraceState::default(),
            );
            // Without an OpenTelemetry layer (no OTLP endpoint) there is nothing to parent.
            let _ = span.set_parent(opentelemetry::Context::new().with_remote_span_context(remote));
            "linked"
        }
    };
    metrics
        .inbound_traceparent
        .add(1, &[KeyValue::new(attr::OUTCOME, outcome)]);
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
        CompanionMessage::SessionMessageResult(_) => "session_message_result",
        CompanionMessage::Draft(_) => "draft",
        CompanionMessage::HandoffReport(_) => "handoff_report",
        CompanionMessage::RoomReply(_) => "room_reply",
        CompanionMessage::AgentReport(_) => "agent_report",
        CompanionMessage::FleetRequest(_) => "fleet_request",
        CompanionMessage::Ack { .. } => "ack",
    };
    // Declared empty so the `agent_report` arm can record them: `tracing` drops a record() of a
    // field the span did not declare (plan 226 §1).
    let span = tracing::info_span!(span::INBOUND_EVENT, otel.kind = "server", rubato.companion_id = %companion_id, rubato.user = %pairing.user, kind, seq = env.seq, rubato.launch_id = tracing::field::Empty, rubato.issue = tracing::field::Empty, rubato.outcome = tracing::field::Empty);
    link_to_sender(&app.metrics, &span, env.traceparent.as_deref());
    async move {
        let now = app.clock.now();
        if !matches!(env.message, CompanionMessage::Ack { .. }) {
            let payload = stored_payload(&env.message);
            match store::record_event(&app.db, companion_id, env.seq, kind, &payload, now).await {
                Ok(false) => {
                    on_replay(&app.metrics, companion_id, &env);
                    // A result whose envelope was recorded but not applied (a crash in between)
                    // must still settle its message; applying it twice changes nothing.
                    if let CompanionMessage::SessionMessageResult(r) = &env.message {
                        session_messages::on_result(app, companion_id, r).await;
                    }
                    // A replayed fleet request gets its stored answer, never a second action
                    // (ADR 0072).
                    if let CompanionMessage::FleetRequest(r) = &env.message
                        && let Some(res) = fleet::on_replay(app, companion_id, r).await
                    {
                        app.send_to_companion(companion_id, ServerMessage::FleetResult(Box::new(res)));
                    }
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
        if app.touch_companion(companion_id, now)
            && let Some(cached) = app.sessions_of(companion_id)
        {
            // It had gone quiet (#69): heard from again, its last snapshot counts again.
            record_sessions_by_state(&app.metrics, companion_id, &cached);
        }
        match env.message {
            CompanionMessage::Hello {
                companion_id: claimed,
                protocol_version,
                user,
                capabilities,
                client,
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
                app.set_capabilities(companion_id, capabilities);
                observe(
                    "set_client",
                    companion_id,
                    env.seq,
                    pairing::set_client(&app.db, companion_id, client).await,
                );
                // Queued here before the Welcome is answered: the socket sends the Welcome first.
                session_messages::on_hello(app, companion_id).await;
                // #69: the companion sends `Sessions` only when they changed, so its last
                // snapshot counts again from here; nothing cached records nothing.
                if let Some(cached) = app.sessions_of(companion_id) {
                    record_sessions_by_state(&app.metrics, companion_id, &cached);
                }
                // Its launches' plan statuses follow the Welcome (ADR 0071, plan 226 §4).
                app.companion_welcomed(companion_id);
                Some(ServerMessage::Welcome {
                    server_version: crate::kernel::VERSION.to_owned(),
                    protocol_version: PROTOCOL_VERSION,
                    last_seen_companion_seq: pairing.last_companion_seq,
                    limits: app.config.limits,
                    capabilities: vec![
                        rubato_protocol::capability::SESSION_MESSAGES.to_owned(),
                        rubato_protocol::capability::DRAFTS.to_owned(),
                        rubato_protocol::capability::HANDOFF_REPORTS.to_owned(),
                        rubato_protocol::capability::ROOM_REPLIES.to_owned(),
                        // It takes agent reports and pushes plan statuses (ADR 0071).
                        rubato_protocol::capability::AGENT_MCP.to_owned(),
                    ]
                    .into_iter()
                    // A lead's fleet requests, only with the orchestrator flag on (ADR 0072).
                    .chain(app.config.orchestrator.then(|| rubato_protocol::capability::FLEET.to_owned()))
                    // It builds the context graph and serves snapshots, only with its flag on (ADR 0086).
                    .chain(app.config.context_graph.is_some().then(|| rubato_protocol::capability::CONTEXT_GRAPH.to_owned()))
                    .collect(),
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
                session_messages::on_hook(app, companion_id, &h).await;
                session_log::on_hook(app, companion_id, &h).await;
                // After the log has the Stop: a session-mode room turn's reply (ADR 0057).
                rooms::on_hook(app, companion_id, &h).await;
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Sessions { mut sessions } => {
                if sessions.len() > MAX_SESSIONS_PER_SNAPSHOT {
                    tracing::warn!(rubato.companion_id = %companion_id, sessions = sessions.len(), cap = MAX_SESSIONS_PER_SNAPSHOT, "sessions snapshot over the cap");
                    app.metrics
                        .events_dropped
                        .add(1, &[KeyValue::new(attr::DROP_REASON, "too_many_sessions")]);
                    return Some(ServerMessage::Ack { seq: env.seq });
                }
                // The pairing is the only source of a session's companion and user (#162).
                let overwritten = own_sessions(&mut sessions, companion_id, &pairing.user);
                if overwritten > 0 {
                    tracing::warn!(rubato.companion_id = %companion_id, overwritten, "session identity overwritten");
                }
                // Against every workspace's repos (ADR 0048): a session's repo is named by some map.
                let union = app.union_repos().await;
                if !union.is_empty() {
                    let named: Vec<&str> = union.iter().map(String::as_str).collect();
                    canonicalise_repos(&mut sessions, &named);
                }
                if let Err(e) = store::store_snapshot(&app.db, companion_id, &sessions, now).await {
                    tracing::error!(error = %e, "storing snapshot");
                }
                app.set_sessions(companion_id, sessions.clone());
                // Keyed by the pairing's companion id, never the one a session claims.
                observe(
                    "record_spend",
                    companion_id,
                    env.seq,
                    spend_history::record(&app.db, &app.metrics, companion_id, &sessions, now)
                        .await,
                );
                record_sessions_by_state(&app.metrics, companion_id, &sessions);
                // A launch out of this snapshot loses its agent line (ADR 0071, plan 226 §1).
                let live: Vec<Uuid> = sessions.iter().filter_map(|s| s.launch_id).collect();
                observe(
                    "prune_agent_reports",
                    companion_id,
                    env.seq,
                    agent_reports::prune(&app.db, companion_id, &live).await.map(|_| ()),
                );
                session_messages::on_sessions(app, &sessions).await;
                // A lead hears when its workers end, block or go idle (ADR 0072).
                fleet::on_sessions(app, &sessions).await;
                app.notify("sessions");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Launch(result) => {
                match launch::record_result(&app.db, companion_id, &result).await {
                    Ok(Some(issue)) => {
                        tracing::info!(rubato.issue = %issue, outcome = ?std::mem::discriminant(&result), "launch result");
                        settle_queue(app, companion_id, &issue, &result).await;
                        if let rubato_protocol::LaunchResult::Started {
                            command_id,
                            draft_restored,
                            restore_skipped,
                            ..
                        } = &result
                        {
                            // ADR 0053, plan 122 §D5: what became of the draft, and an approval
                            // that waited for this launch.
                            if *draft_restored {
                                tracing::info!(rubato.issue = %issue, launch_id = %command_id, "draft restored into the new worktree");
                            } else if let Some(why) = restore_skipped {
                                tracing::warn!(rubato.issue = %issue, launch_id = %command_id, reason = why.name(), "draft not restored");
                            }
                            approve::on_launch_started(app, *command_id).await;
                        }
                    }
                    // Unknown, or sent to another companion (#162): nothing was written.
                    Ok(None) => {
                        refuse_identity(&app.metrics);
                        let (rubato_protocol::LaunchResult::Started { command_id, .. }
                        | rubato_protocol::LaunchResult::Refused { command_id, .. }
                        | rubato_protocol::LaunchResult::Failed { command_id, .. }) = &result;
                        tracing::warn!(rubato.companion_id = %companion_id, rubato.launch_id = %command_id, "launch result for a command not sent to this companion");
                    }
                    Err(e) => tracing::error!(error = %e, "storing launch result"),
                }
                app.notify("launch");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::CapHit(hit) => {
                if !runs_issue(app, companion_id, &hit.issue).await {
                    refuse_identity(&app.metrics);
                    tracing::warn!(rubato.companion_id = %companion_id, rubato.issue = %hit.issue, "cap hit for an issue this companion does not run dropped");
                    return Some(ServerMessage::Ack { seq: env.seq });
                }
                tracing::warn!(rubato.issue = %hit.issue, rubato.session_id = ?hit.session_id, kind = ?hit.kind, value = hit.value, limit = hit.limit, "{}", rubato_protocol::otel::event::CAP_HIT);
                if let Err(e) = launch::record_cap_hit(&app.db, &hit, now).await {
                    tracing::error!(error = %e, "storing cap hit");
                }
                app.notify("cap hit");
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Stale(mut s) => {
                // Recorded under the pairing, never the companion the event names (#162).
                s.companion_id = companion_id;
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
            CompanionMessage::Draft(bundle) => {
                // The launch decides the owner; a bundle for a launch of another companion, or
                // for another issue, is not this companion's to send.
                match launch::launch_target(&app.db, bundle.launch_id).await {
                    Ok(Some(t)) if t.companion_id == companion_id && t.issue == bundle.issue => {
                        plan_drafts::on_draft(app, &bundle, &t.user).await;
                    }
                    Ok(_) => {
                        app.metrics
                            .drafts_refused
                            .add(1, &[KeyValue::new(attr::DROP_REASON, "unknown_launch")]);
                        tracing::warn!(rubato.issue = %bundle.issue, "draft bundle for a launch this companion did not start");
                    }
                    Err(e) => tracing::error!(error = %format!("{e:#}"), "draft: reading the launch failed"),
                }
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::HandoffReport(r) => {
                // Recorded on the approval only for this companion's own launch of its issue.
                approve::on_handoff_report(app, companion_id, &r).await;
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::RoomReply(r) => {
                // Taken only from the companion the turn was sent to (ADR 0057).
                rooms::on_room_reply(app, companion_id, &r).await;
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::SessionMessageResult(r) => {
                session_messages::on_result(app, companion_id, &r).await;
                Some(ServerMessage::Ack { seq: env.seq })
            }
            // ADR 0071, plan 226 §1: a launch this pairing started and still runs, parts its
            // workspace map has, text bounded again, the server's clock.
            CompanionMessage::AgentReport(r) => {
                let span = tracing::Span::current();
                span.record("rubato.launch_id", r.launch_id.to_string());
                match accept_report(app, companion_id, &r).await {
                    Ok(Accepted::Yes { issue, doing, parts }) => {
                        span.record("rubato.issue", issue.to_string());
                        span.record("rubato.outcome", "stored");
                        observe(
                            "record_agent_report",
                            companion_id,
                            env.seq,
                            agent_reports::record(&app.db, companion_id, r.launch_id, &doing, &parts, now)
                                .await
                                .map(|_| ()),
                        );
                        app.notify("agent report");
                    }
                    Ok(Accepted::No(reason)) => {
                        span.record("rubato.outcome", "dropped");
                        tracing::warn!(rubato.companion_id = %companion_id, rubato.launch_id = %r.launch_id, reason, "agent report refused");
                        app.metrics
                            .events_dropped
                            .add(1, &[KeyValue::new(attr::DROP_REASON, "agent_report")]);
                    }
                    Err(e) => {
                        span.record("rubato.outcome", "error");
                        observe("accept_agent_report", companion_id, env.seq, Err(e));
                    }
                }
                Some(ServerMessage::Ack { seq: env.seq })
            }
            // ADR 0072: a lead's fleet tool call; the answer goes back on the same socket.
            CompanionMessage::FleetRequest(r) => {
                if let Some(res) = fleet::on_request(app, companion_id, &pairing.user, &r).await {
                    app.send_to_companion(companion_id, ServerMessage::FleetResult(Box::new(res)));
                }
                Some(ServerMessage::Ack { seq: env.seq })
            }
            CompanionMessage::Ack { .. } => None,
        }
    }
    .instrument(span)
    .await
}

enum Accepted {
    Yes {
        issue: rubato_protocol::IssueRef,
        doing: String,
        parts: Vec<String>,
    },
    No(&'static str),
}

/// One line, at most `AGENT_DOING_MAX_CHARS` characters, newlines as spaces: the companion
/// bounds it too, and is not trusted to.
fn bounded_line(doing: &str) -> String {
    use rubato_protocol::limits::AGENT_DOING_MAX_CHARS;
    let flat: String = doing
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let flat = flat.trim();
    if flat.chars().count() <= AGENT_DOING_MAX_CHARS {
        return flat.to_owned();
    }
    let mut cut: String = flat.chars().take(AGENT_DOING_MAX_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Whether `r` is a report this pairing may make (ADR 0071): its launch went to this companion
/// (the server's `launches` row, never a snapshot's claim) and is in this companion's newest
/// snapshot. Parts the launch's workspace map does not have are dropped and counted.
async fn accept_report(
    app: &App,
    companion_id: Uuid,
    r: &rubato_protocol::AgentReport,
) -> anyhow::Result<Accepted> {
    let Some(target) = launch::launch_target(&app.db, r.launch_id).await? else {
        return Ok(Accepted::No("not_this_companion"));
    };
    if target.companion_id != companion_id {
        return Ok(Accepted::No("not_this_companion"));
    }
    let live = app
        .latest_sessions()
        .iter()
        .any(|s| s.companion_id == companion_id && s.launch_id == Some(r.launch_id));
    if !live {
        return Ok(Accepted::No("not_live"));
    }
    let map = app.map_of(&target.workspace).await;
    let known = |id: &str| {
        map.as_ref().is_some_and(|m| {
            m.map.nodes.iter().any(|n| n.id == id) || m.map.seams.iter().any(|s| s.id == id)
        })
    };
    let mut parts = Vec::new();
    for p in r
        .parts
        .iter()
        .take(rubato_protocol::limits::AGENT_PARTS_MAX)
    {
        if known(p) {
            parts.push(p.clone());
        } else {
            app.metrics
                .events_dropped
                .add(1, &[KeyValue::new(attr::DROP_REASON, "agent_report_part")]);
        }
    }
    Ok(Accepted::Yes {
        issue: target.issue,
        doing: bounded_line(&r.doing),
        parts,
    })
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

/// Set every session's companion and user to the pairing's. Returns how many claimed otherwise.
fn own_sessions(sessions: &mut [SessionState], companion_id: Uuid, user: &str) -> usize {
    let mut changed = 0;
    for s in sessions {
        if s.companion_id != companion_id || s.user != user {
            changed += 1;
            s.companion_id = companion_id;
            s.user = user.to_owned();
        }
    }
    changed
}

/// A cap hit counts only from a companion that was sent a launch for the issue or reports a
/// session on it (#162). A failed read refuses: the hit would otherwise block on a guess.
async fn runs_issue(app: &App, companion_id: Uuid, issue: &rubato_protocol::IssueRef) -> bool {
    match launch::companion_launched(&app.db, companion_id, issue).await {
        Ok(true) => return true,
        Ok(false) => {}
        Err(e) => tracing::error!(error = %e, "reading launches for a cap hit"),
    }
    match store::companion_has_session_on(&app.db, companion_id, issue).await {
        Ok(yes) => yes,
        Err(e) => {
            tracing::error!(error = %e, "reading the snapshot for a cap hit");
            false
        }
    }
}

/// An envelope that names another companion's issue or command: acknowledged, not applied.
fn refuse_identity(metrics: &Metrics) {
    metrics
        .events_dropped
        .add(1, &[KeyValue::new(attr::DROP_REASON, "identity")]);
}

/// How a socket's end left the gauge (#69), as the disconnect log line says it.
enum Left {
    /// This socket was the companion's: its sessions that had not ended count as unknown.
    Gone,
    /// The pairing was revoked: the companion is gone, every count is 0.
    Revoked,
    /// A newer socket for the companion is live: the counts are its.
    ToNewer,
}

impl Left {
    fn as_str(&self) -> &'static str {
        match self {
            Left::Gone => "unknown",
            Left::Revoked => "zeros",
            Left::ToNewer => "left_to_newer",
        }
    }
}

/// A companion that went away or quiet (#69): each session of its last snapshot that had not
/// ended counts as unknown, never as working (AGENTS.md); ended ones stay ended.
fn record_sessions_by_state_unknown(
    metrics: &Metrics,
    companion_id: Uuid,
    sessions: &[SessionState],
) {
    let unknown: Vec<SessionState> = sessions
        .iter()
        .cloned()
        .map(|mut s| {
            if s.status != SessionStatus::Ended {
                s.status = SessionStatus::Unknown;
            }
            s
        })
        .collect();
    record_sessions_by_state(metrics, companion_id, &unknown);
}

/// The poll tick's pass (#69): a connected companion silent for `companion_silent_secs` (a
/// laptop asleep with its socket half-open; the threshold `CompanionSilent` pages on) counts
/// its sessions as unknown, once, until it sends again (`touch_companion`). How many it marked.
pub fn mark_quiet(app: &App, now: chrono::DateTime<Utc>) -> usize {
    let silent = chrono::Duration::seconds(app.config.limits.companion_silent_secs as i64);
    let marked: Vec<(Uuid, chrono::DateTime<Utc>)> = app
        .companions
        .lock()
        .unwrap()
        .iter_mut()
        .filter(|(_, c)| !c.quiet && now - c.last_seen > silent)
        .map(|(id, c)| {
            c.quiet = true;
            (*id, c.last_seen)
        })
        .collect();
    for (id, last_seen) in &marked {
        let cached = app.sessions_of(*id).unwrap_or_default();
        record_sessions_by_state_unknown(&app.metrics, *id, &cached);
        tracing::info!(rubato.companion_id = %id, %last_seen, "companion quiet, sessions counted unknown");
    }
    marked.len()
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

/// The envelope as it is kept in `companion_events`. A hook's raw Claude Code payload is never
/// stored, even from a companion that still sends it (ADR 0052), and the new kinds are kept as
/// `{}` (plan 122 §D2): deduplication needs only the sequence, and the row's `kind` says what
/// it was.
fn stored_payload(m: &CompanionMessage) -> serde_json::Value {
    match m {
        CompanionMessage::Hook(h) if h.payload.is_some() => {
            let mut h = h.clone();
            h.payload = None;
            serde_json::to_value(CompanionMessage::Hook(h)).unwrap_or_default()
        }
        CompanionMessage::SessionMessageResult(_)
        | CompanionMessage::Draft(_)
        | CompanionMessage::AgentReport(_)
        | CompanionMessage::FleetRequest(_) => {
            serde_json::json!({})
        }
        other => serde_json::to_value(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::{capture, metric_reader};
    use rubato_protocol::otel::metric;
    use rubato_protocol::{IssueRef, StateSource};

    /// An old companion still sends the raw payload; the server never stores it (ADR 0052).
    #[test]
    fn a_hooks_raw_payload_is_not_stored() {
        let ev = rubato_protocol::HookEvent::from_payload(
            serde_json::json!({"session_id": "s", "hook_event_name": "PostToolUse", "cwd": "/w",
                "tool_name": "Bash", "tool_response": {"stdout": "TOKEN=abc"}}),
            chrono::Utc::now(),
        )
        .unwrap();
        let m = CompanionMessage::Hook(ev);
        let stored = stored_payload(&m).to_string();
        assert!(!stored.contains("TOKEN=abc"), "{stored}");
        assert!(stored.contains(r#""tool_name":"Bash""#), "{stored}");
        let r = CompanionMessage::SessionMessageResult(rubato_protocol::SessionMessageResult {
            message_id: Uuid::nil(),
            outcome: rubato_protocol::SessionMessageOutcome::Delivered,
            at: chrono::Utc::now(),
        });
        assert_eq!(stored_payload(&r), serde_json::json!({}));
    }

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

    #[test]
    fn a_foreign_identity_envelope_is_counted_as_dropped_identity() {
        let (reader, metrics) = metric_reader::start();
        refuse_identity(&metrics);
        let points = reader.points(metric::EVENTS_DROPPED);
        let identity = points
            .iter()
            .find(|p| p.attrs[attr::DROP_REASON] == "identity")
            .map(|p| p.value);
        assert_eq!(identity, Some(1), "{points:?}");
    }

    #[test]
    fn own_sessions_counts_only_the_claims_it_changed() {
        let me = Uuid::new_v4();
        let mut all = vec![
            session(me, SessionStatus::Working),
            session(Uuid::new_v4(), SessionStatus::Working),
        ];
        all[0].user = "alice".into();
        assert_eq!(own_sessions(&mut all, me, "alice"), 1);
        assert!(
            all.iter()
                .all(|s| s.companion_id == me && s.user == "alice")
        );
    }

    #[test]
    fn a_revoked_envelope_is_counted_as_dropped_revoked() {
        let (reader, metrics) = metric_reader::start();
        refuse_revoked(&metrics, Uuid::new_v4(), 4);
        let points = reader.points(metric::EVENTS_DROPPED);
        let revoked = points
            .iter()
            .find(|p| p.attrs[attr::DROP_REASON] == "revoked")
            .map(|p| p.value);
        assert_eq!(revoked, Some(1), "{points:?}");
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
            launch_id: None,
            hooks_lost: false,
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

    /// An app on a fresh database with in-memory metrics, and a pairing to send as.
    async fn spend_app() -> (
        App,
        metric_reader::Reader,
        pairing::PairingRow,
        tempfile::TempDir,
    ) {
        use crate::kernel::config::Config;
        use crate::kernel::github::FakeGitHub;
        use crate::kernel::state::AppState;
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
        let pool = crate::kernel::db::connect(&url).await.unwrap();
        let (reader, metrics) = metric_reader::start();
        let mut app = AppState::new(
            pool,
            Config::for_tests(url, "m0x6A"),
            std::sync::Arc::new(FakeGitHub::new()),
        );
        std::sync::Arc::get_mut(&mut app).unwrap().metrics = metrics;
        let pairing = pairing::PairingRow {
            companion_id: Uuid::new_v4(),
            user: "m0x6A".into(),
            hostname: "h".into(),
            created_at: Utc::now(),
            revoked_at: None,
            last_seen: None,
            server_seq: 0,
            last_companion_seq: 0,
            client: rubato_protocol::ClientKind::Interactive,
        };
        (app, reader, pairing, tmp)
    }

    /// #69: the quiet pass counts a silent companion's sessions as unknown once, its next
    /// envelope (`touch_companion`) counts them again, and a companion heard from recently is
    /// left alone.
    #[tokio::test(flavor = "current_thread")]
    async fn a_quiet_companion_counts_unknown_until_heard_from() {
        let (app, reader, pairing, _tmp) = spend_app().await;
        let id = pairing.companion_id;
        let fresh = Uuid::new_v4();
        let now = Utc::now();
        let silent = chrono::Duration::seconds(app.config.limits.companion_silent_secs as i64);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for (c, seen) in [
            (id, now - silent - chrono::Duration::seconds(1)),
            (fresh, now),
        ] {
            app.companions
                .lock()
                .unwrap()
                .insert(c, CompanionConn::new(tx.clone(), seen));
            app.set_sessions(c, vec![session(c, SessionStatus::Working)]);
            record_sessions_by_state(&app.metrics, c, &[session(c, SessionStatus::Working)]);
        }
        let state = |c: Uuid, s: &str| {
            reader
                .points(metric::SESSIONS_BY_STATE)
                .into_iter()
                .find(|p| {
                    p.attrs[attr::COMPANION_ID] == c.to_string()
                        && p.attrs[attr::SESSION_STATE] == s
                })
                .map(|p| p.value)
        };

        assert_eq!(mark_quiet(&app, now), 1, "only the silent one");
        assert_eq!(
            (state(id, "working"), state(id, "unknown")),
            (Some(0), Some(1))
        );
        assert_eq!(
            state(fresh, "working"),
            Some(1),
            "heard from recently: left alone"
        );
        assert_eq!(mark_quiet(&app, now), 0, "marked once");

        // Heard from again: the caller records the cached snapshot.
        assert!(app.touch_companion(id, now), "it had gone quiet");
        assert!(!app.touch_companion(id, now), "and is not any more");
        println!("checked 2 companions over 2 quiet passes and 2 touches");
    }

    fn spend_snapshot(companion_id: Uuid, seq: u64, usd: f64) -> CompanionEnvelope {
        let mut s = session(companion_id, SessionStatus::Working);
        s.issue = Some(IssueRef::new("m0x6A/rubato", 103));
        s.spend_usd = Some(usd);
        CompanionEnvelope {
            seq,
            sent_at: Utc::now(),
            traceparent: None,
            message: CompanionMessage::Sessions { sessions: vec![s] },
        }
    }

    fn spend_count(reader: &metric_reader::Reader, outcome: &str) -> u64 {
        reader
            .points(metric::SPEND_POINTS)
            .iter()
            .filter(|p| p.attrs[attr::OUTCOME] == outcome)
            .map(|p| p.value)
            .sum()
    }

    async fn spend_rows(app: &App) -> Vec<(String, f64)> {
        use sqlx::Row;
        sqlx::query("SELECT bucket, usd FROM spend_points ORDER BY bucket")
            .fetch_all(&app.db)
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get("bucket"), r.get("usd")))
            .collect()
    }

    /// Plan 122 §D6: every snapshot replaces that companion's entry in the kernel's session
    /// cache, which slices that may not name this one read; the stored snapshots, read back by
    /// `session_snapshots` at start, give the same picture.
    #[tokio::test(flavor = "current_thread")]
    async fn each_snapshot_replaces_the_companions_cached_sessions() {
        let (app, _reader, pairing, _tmp) = spend_app().await;
        let c = pairing.companion_id;
        assert!(app.latest_sessions().is_empty());
        on_envelope(&app, &pairing, spend_snapshot(c, 1, 1.0)).await;
        assert_eq!(app.latest_sessions().len(), 1);
        let mut idle = spend_snapshot(c, 2, 2.0);
        if let CompanionMessage::Sessions { sessions } = &mut idle.message {
            sessions[0].status = SessionStatus::Idle;
        }
        on_envelope(&app, &pairing, idle).await;
        let cached = app.latest_sessions();
        assert_eq!(cached.len(), 1, "replaced, not added");
        assert_eq!(cached[0].status, SessionStatus::Idle);
        let stored = store::session_snapshots(&app.db).await.unwrap();
        assert_eq!(stored.get(&c), Some(&cached));
    }

    /// ADR 0071, plan 226 §5: the Welcome says this server takes agent reports and pushes plan
    /// statuses, and the kernel hears that the companion was welcomed.
    #[tokio::test(flavor = "current_thread")]
    async fn the_welcome_announces_agent_mcp_and_is_broadcast() {
        let (app, _reader, pairing, _tmp) = spend_app().await;
        let mut welcomed = app.subscribe_welcomed();
        let mut hello = fixture("companion_hello.json");
        if let CompanionMessage::Hello {
            companion_id, user, ..
        } = &mut hello.message
        {
            *companion_id = pairing.companion_id;
            *user = pairing.user.clone();
        }
        match on_envelope(&app, &pairing, hello).await {
            Some(ServerMessage::Welcome { capabilities, .. }) => assert!(
                capabilities
                    .iter()
                    .any(|c| c == rubato_protocol::capability::AGENT_MCP),
                "{capabilities:?}"
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(welcomed.try_recv().unwrap(), pairing.companion_id);
    }

    // ----- agent reports (ADR 0071, plan 226 §1) ---------------------------------------------

    const LAUNCH: Uuid = Uuid::from_u128(0x9b2e4d1a_3c5f_4e6a_9b7c_8d9e0f1a2b3c);

    /// `spend_app` plus the map of workspace `default` (Rubato's own) and a launch of issue 7 sent
    /// to `launched_to`.
    async fn report_app(
        launched_to: Option<Uuid>,
    ) -> (
        App,
        metric_reader::Reader,
        pairing::PairingRow,
        tempfile::TempDir,
    ) {
        let (app, reader, pairing, tmp) = spend_app().await;
        let yaml = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/design/rubato-map.yml"),
        )
        .unwrap();
        app.set_map(
            "default",
            crate::kernel::state::LoadedMap {
                map: serde_yaml_ng::from_str(&yaml).unwrap(),
                version: "v".into(),
                loaded_at: Utc::now(),
                yaml,
            },
        )
        .await;
        let target = launched_to.unwrap_or(pairing.companion_id);
        sqlx::query("INSERT INTO launches (workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', ?, ?, 'm0x6A', 'm0x6A/rubato', 7, '{}', ?)")
            .bind(LAUNCH.to_string())
            .bind(target.to_string())
            .bind(crate::kernel::db::ts(Utc::now()))
            .execute(&app.db)
            .await
            .unwrap();
        (app, reader, pairing, tmp)
    }

    fn live_snapshot(companion_id: Uuid, seq: u64, launch: Option<Uuid>) -> CompanionEnvelope {
        let mut s = session(companion_id, SessionStatus::Working);
        s.issue = Some(IssueRef::new("m0x6A/rubato", 7));
        s.launch_id = launch;
        CompanionEnvelope {
            seq,
            sent_at: Utc::now(),
            traceparent: None,
            message: CompanionMessage::Sessions { sessions: vec![s] },
        }
    }

    fn report(seq: u64, doing: &str, parts: &[&str]) -> CompanionEnvelope {
        CompanionEnvelope {
            seq,
            sent_at: Utc::now(),
            traceparent: None,
            message: CompanionMessage::AgentReport(rubato_protocol::AgentReport {
                launch_id: LAUNCH,
                doing: doing.into(),
                parts: parts.iter().map(|p| (*p).to_owned()).collect(),
                at: Utc::now() + chrono::Duration::days(1),
            }),
        }
    }

    fn dropped(reader: &metric_reader::Reader, reason: &str) -> u64 {
        reader
            .points(metric::EVENTS_DROPPED)
            .iter()
            .filter(|p| p.attrs[attr::DROP_REASON] == reason)
            .map(|p| p.value)
            .sum()
    }

    async fn lines(app: &App) -> Vec<rubato_protocol::AgentLine> {
        crate::agent_reports::all(app).await.unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_report_for_this_companions_live_launch_is_stored_bounded_and_on_the_servers_clock() {
        let (app, reader, pairing, _tmp) = report_app(None).await;
        let c = pairing.companion_id;
        on_envelope(&app, &pairing, live_snapshot(c, 1, Some(LAUNCH))).await;
        let before = app.latest_sessions();
        let long = format!("designing\nthe seam {}", "x".repeat(300));
        let reply = on_envelope(
            &app,
            &pairing,
            report(2, &long, &["rubato-server", "not-on-the-map"]),
        )
        .await;
        assert!(matches!(reply, Some(ServerMessage::Ack { seq: 2 })));
        let got = lines(&app).await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].companion_id, c);
        assert_eq!(got[0].doing.chars().count(), 200, "{}", got[0].doing);
        assert!(
            got[0].doing.starts_with("designing the seam"),
            "newline became a space"
        );
        assert_eq!(got[0].parts, vec!["rubato-server".to_owned()]);
        assert!(
            got[0].reported_at < Utc::now() + chrono::Duration::minutes(1),
            "the server's clock, not the report's"
        );
        assert_eq!(dropped(&reader, "agent_report_part"), 1);
        assert_eq!(dropped(&reader, "agent_report"), 0);
        // Never *working* from a report: the sessions and what is stored of them are unchanged.
        assert_eq!(app.latest_sessions(), before);
        let stored = store::session_snapshots(&app.db).await.unwrap();
        assert_eq!(stored.get(&c), Some(&before));
        // ADR 0052: the text is kept only in agent_reports, never in the event log.
        use sqlx::Row;
        let payload: String = sqlx::query("SELECT payload FROM companion_events WHERE seq = 2")
            .fetch_one(&app.db)
            .await
            .unwrap()
            .get("payload");
        assert_eq!(payload, "{}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_report_for_a_launch_sent_elsewhere_or_not_live_is_dropped() {
        // The launch went to another companion; this one claims it in its own snapshot.
        let other = Uuid::new_v4();
        let (app, reader, pairing, _tmp) = report_app(Some(other)).await;
        let c = pairing.companion_id;
        on_envelope(&app, &pairing, live_snapshot(c, 1, Some(LAUNCH))).await;
        on_envelope(&app, &pairing, report(2, "forged", &[])).await;
        assert!(lines(&app).await.is_empty());
        assert_eq!(dropped(&reader, "agent_report"), 1);

        // Its own launch, but no longer in its snapshot.
        let (app, reader, pairing, _tmp) = report_app(None).await;
        let c = pairing.companion_id;
        on_envelope(&app, &pairing, live_snapshot(c, 1, None)).await;
        on_envelope(&app, &pairing, report(2, "late", &[])).await;
        assert!(lines(&app).await.is_empty());
        assert_eq!(dropped(&reader, "agent_report"), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_snapshot_without_the_launch_deletes_its_line() {
        let (app, _reader, pairing, _tmp) = report_app(None).await;
        let c = pairing.companion_id;
        on_envelope(&app, &pairing, live_snapshot(c, 1, Some(LAUNCH))).await;
        on_envelope(&app, &pairing, report(2, "working on it", &[])).await;
        assert_eq!(lines(&app).await.len(), 1);
        on_envelope(&app, &pairing, live_snapshot(c, 3, None)).await;
        use sqlx::Row;
        let n: i64 = sqlx::query("SELECT COUNT(*) AS n FROM agent_reports")
            .fetch_one(&app.db)
            .await
            .unwrap()
            .get("n");
        assert_eq!(n, 0);
    }

    /// The inbound span closes when its last holder lets go, and sqlx's SQLite worker thread
    /// holds the caller's span until after it has sent the result back. So a span may close a
    /// moment after `on_envelope` returns: wait for it, never read once (#272).
    async fn eventually<T>(what: &str, mut read: impl FnMut() -> Option<T>) -> T {
        for _ in 0..400 {
            if let Some(v) = read() {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("never saw {what}");
    }

    /// A thread's own subscriber whose spans go to an in-memory exporter, for the tests that
    /// read a span's kind and parent. The capture's global subscriber is installed first, so
    /// this one coming and going leaves every callsite with a listener.
    fn exported_spans() -> (
        opentelemetry_sdk::trace::InMemorySpanExporter,
        opentelemetry_sdk::trace::SdkTracerProvider,
        tracing::subscriber::DefaultGuard,
    ) {
        use opentelemetry::trace::TracerProvider as _;
        use tracing_subscriber::layer::SubscriberExt as _;
        drop(capture::start());
        let exporter = opentelemetry_sdk::trace::InMemorySpanExporter::default();
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let guard = tracing::subscriber::set_default(subscriber);
        (exporter, provider, guard)
    }

    fn traceparent_count(reader: &metric_reader::Reader, outcome: &str) -> u64 {
        reader
            .points(metric::INBOUND_TRACEPARENT)
            .iter()
            .filter(|p| p.attrs[attr::OUTCOME] == outcome)
            .map(|p| p.value)
            .sum()
    }

    const SENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    /// ADR 0081: the inbound span is kind server, in the trace the companion sent and under its
    /// send span. Fails on purpose when `set_parent` is taken out: the span then starts its own
    /// trace.
    #[tokio::test(flavor = "current_thread")]
    async fn an_envelope_with_a_traceparent_is_handled_under_the_span_that_sent_it() {
        let (app, reader, pairing, _tmp) = spend_app().await;
        let (exporter, _provider, _guard) = exported_spans();
        let mut env = fixture("companion_heartbeat.json");
        env.traceparent = Some(SENT.into());
        on_envelope(&app, &pairing, env).await;
        let inbound = eventually("the inbound span", || {
            let spans = exporter.get_finished_spans().unwrap();
            spans.into_iter().find(|s| s.name == span::INBOUND_EVENT)
        })
        .await;
        assert_eq!(inbound.span_kind, opentelemetry::trace::SpanKind::Server);
        assert_eq!(
            format!("{:032x}", inbound.span_context.trace_id()),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(
            format!("{:016x}", inbound.parent_span_id),
            "00f067aa0ba902b7"
        );
        assert_eq!(traceparent_count(&reader, "linked"), 1);
        assert_eq!(traceparent_count(&reader, "absent"), 0);
        assert_eq!(traceparent_count(&reader, "invalid"), 0);
    }

    /// An older companion sends none, and a value that does not parse is handled as none
    /// (plan 287 A2): both start their own trace, are still applied, and are counted apart.
    #[tokio::test(flavor = "current_thread")]
    async fn an_envelope_without_a_usable_traceparent_starts_its_own_trace_and_is_counted() {
        let (app, reader, pairing, _tmp) = spend_app().await;
        let (exporter, _provider, _guard) = exported_spans();
        let absent = fixture("companion_heartbeat.json");
        let seq = absent.seq;
        let mut invalid = absent.clone();
        invalid.seq = seq + 1;
        invalid.traceparent = Some(SENT.to_uppercase());
        let none = opentelemetry::trace::SpanId::INVALID;
        for env in [absent, invalid] {
            let seq = env.seq;
            let reply = on_envelope(&app, &pairing, env).await;
            assert_eq!(reply, Some(ServerMessage::Ack { seq }), "still applied");
        }
        let spans = eventually("both inbound spans", || {
            let spans: Vec<_> = exporter
                .get_finished_spans()
                .unwrap()
                .into_iter()
                .filter(|s| s.name == span::INBOUND_EVENT)
                .collect();
            (spans.len() == 2).then_some(spans)
        })
        .await;
        for s in &spans {
            assert_eq!(s.span_kind, opentelemetry::trace::SpanKind::Server);
            assert_eq!(s.parent_span_id, none, "a root span");
            assert_ne!(
                format!("{:032x}", s.span_context.trace_id()),
                "4bf92f3577b34da6a3ce929d0e0e4736"
            );
        }
        assert_eq!(traceparent_count(&reader, "absent"), 1);
        assert_eq!(traceparent_count(&reader, "invalid"), 1);
        assert_eq!(traceparent_count(&reader, "linked"), 0);
        println!("checked 2 envelopes without a usable traceparent");
    }

    /// Plan 226 §1: the span carries the launch, the issue and the outcome. Declared fields only;
    /// an undeclared one would be dropped silently.
    #[tokio::test(flavor = "current_thread")]
    async fn the_inbound_span_carries_the_launch_the_issue_and_the_outcome() {
        let (app, _reader, pairing, _tmp) = report_app(None).await;
        let c = pairing.companion_id;
        let (events, _guard) = capture::start();
        on_envelope(&app, &pairing, live_snapshot(c, 1, Some(LAUNCH))).await;
        on_envelope(&app, &pairing, report(2, "x", &[])).await;
        let s = eventually("an inbound span for the report", || {
            let events = events.lock().unwrap();
            events
                .iter()
                .find(|e| {
                    e.message == format!("span {}", span::INBOUND_EVENT)
                        && e.fields.get("kind").map(String::as_str) == Some("agent_report")
                })
                .cloned()
        })
        .await;
        assert_eq!(s.fields.get("rubato.launch_id"), Some(&LAUNCH.to_string()));
        assert_eq!(
            s.fields.get("rubato.issue").map(String::as_str),
            Some("m0x6A/rubato#7")
        );
        assert_eq!(
            s.fields.get("rubato.outcome").map(String::as_str),
            Some("stored")
        );
    }

    /// ADR 0012: a replayed envelope is only acked. Spend history never sees it.
    #[tokio::test(flavor = "current_thread")]
    async fn a_replayed_sessions_envelope_adds_no_spend_row_and_no_count() {
        let (app, reader, pairing, _tmp) = spend_app().await;
        let c = pairing.companion_id;
        let first = on_envelope(&app, &pairing, spend_snapshot(c, 1, 1.0)).await;
        assert!(matches!(first, Some(ServerMessage::Ack { seq: 1 })));
        assert_eq!(spend_rows(&app).await.len(), 1);
        assert_eq!(spend_count(&reader, "stored"), 1);

        app.clock.set(Utc::now() + chrono::Duration::hours(1));
        let replay = on_envelope(&app, &pairing, spend_snapshot(c, 1, 9.0)).await;
        assert!(matches!(replay, Some(ServerMessage::Ack { seq: 1 })));
        let rows = spend_rows(&app).await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].1, 1.0);
        assert_eq!(spend_count(&reader, "stored"), 1);
        assert_eq!(spend_count(&reader, "unchanged"), 0);
    }

    /// A failed spend write is a logged bookkeeping write: the snapshot is still stored and the
    /// envelope still acked (design 103 §2, Failure).
    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_spend_write_is_logged_and_the_snapshot_still_acked() {
        use sqlx::Row;
        let (app, reader, pairing, _tmp) = spend_app().await;
        let c = pairing.companion_id;
        sqlx::query("DROP TABLE spend_points")
            .execute(&app.db)
            .await
            .unwrap();
        let (events, _guard) = capture::start();
        let reply = on_envelope(&app, &pairing, spend_snapshot(c, 3, 2.0)).await;
        assert!(
            matches!(reply, Some(ServerMessage::Ack { seq: 3 })),
            "{reply:?}"
        );
        let stored: i64 = sqlx::query("SELECT COUNT(*) AS n FROM session_snapshots")
            .fetch_one(&app.db)
            .await
            .unwrap()
            .get("n");
        assert_eq!(stored, 1, "the snapshot is stored");
        let events = events.lock().unwrap();
        let failed: Vec<_> = events
            .iter()
            .filter(|e| e.message == "companion bookkeeping write failed")
            .collect();
        assert_eq!(failed.len(), 1, "{events:?}");
        assert_eq!(failed[0].level, tracing::Level::ERROR);
        assert_eq!(
            failed[0].fields.get("write").map(String::as_str),
            Some("record_spend")
        );
        assert_eq!(failed[0].fields.get("seq").map(String::as_str), Some("3"));
        assert_eq!(spend_count(&reader, "failed"), 1);
    }
}
