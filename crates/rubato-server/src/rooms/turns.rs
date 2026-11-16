//! Addressed turns (ADR 0057, plan 122 §D8). A message naming `@<owner>-agent` of an agent that
//! is in the room makes a turn: the room since that agent's last turn (or since it joined),
//! framed as quoted discussion with each author, queued as a `room_turn` session message for the
//! agent's launch. Nothing from before the join, or after leaving, reaches it.
//!
//! - While no plan is on the branch the turn is typed into the working session (`session`), and
//!   its reply is that prompt's `Stop`, found through the session log.
//! - With the plan on the branch it runs in a read-only fork (`fork`), and its reply is a
//!   `RoomReply` with the fork's session and cost.
//!
//! An agent's reply that names another agent makes a turn too. After
//! `Limits.room_agent_turn_limit` such turns with no person posting, Rubato dispatches no more and
//! says so once.

use super::agents::{self, AgentRow};
use super::notices::SYSTEM_AUTHOR;
use super::store;
use crate::kernel::state::App;
use crate::session_messages::{self, RoomTurnRef};
use crate::{plans, session_log, spend_history};
use anyhow::Result;
use opentelemetry::KeyValue;
use rubato_protocol::otel::attr;
use rubato_protocol::{
    HookEvent, HookEventKind, IssueRef, RoomAuthorKind, RoomKey, RoomMessage, RoomReply,
    RoomTurnMode, SessionMessageKind, WebResource, addressed_handles, agent_handle, room_turn_text,
};
use std::collections::HashSet;
use uuid::Uuid;

/// The words of the system message that stops agent-made turns.
pub const TURN_LIMIT_TEXT: &str = "Agent turns stopped:";

/// Why a reply was not taken, on `rubato.events.dropped`.
pub mod refused {
    pub const UNKNOWN_TURN: &str = "room_reply_unknown_turn";
    pub const FOREIGN_COMPANION: &str = "room_reply_foreign_companion";
}

pub(super) fn room_attr(room: &RoomKey) -> String {
    format!("{}#{}@{}", room.repo, room.number, room.branch)
}

fn turns(app: &App, outcome: &'static str, n: u64) {
    if n > 0 {
        app.metrics
            .rooms_agent_turns
            .add(n, &[KeyValue::new(attr::OUTCOME, outcome)]);
    }
}

pub(super) fn notify(app: &App, room: &RoomKey) {
    app.notify_resource(WebResource::Room {
        repo: room.repo.clone(),
        number: room.number,
        branch: room.branch.clone(),
    });
}

/// The agents a text addresses among those in the room, in the order named.
pub(super) fn addressed_agents<'a>(text: &str, present: &'a [AgentRow]) -> Vec<&'a AgentRow> {
    addressed_handles(text)
        .iter()
        .filter_map(|h| {
            present
                .iter()
                .find(|a| a.present() && a.agent.handle.eq_ignore_ascii_case(h))
        })
        .collect()
}

/// The plan is on the branch: building, so turns run in a read-only fork.
async fn building(app: &App, room: &RoomKey) -> Result<bool> {
    let issue = IssueRef::new(room.repo.clone(), room.number);
    Ok(plans::all(&app.db)
        .await?
        .iter()
        .any(|p| p.issue == issue && p.branch == room.branch && p.plan.is_some()))
}

/// The line a turn asked aloud carries, after the quoted room (ADR 0073, plan 146 design §3).
pub const ASKED_ALOUD: &str = "Asked aloud: start your reply with an answer of one or two sentences that can be spoken; details after.";

/// Queue one turn for `agent`, made by `trigger` (already stored), as `addressed_by`.
pub(super) async fn dispatch_turn(
    app: &App,
    room: &RoomKey,
    agent: &AgentRow,
    trigger: &RoomMessage,
    turn_id: Uuid,
    addressed_by: &str,
) -> Result<()> {
    let since = agent.since();
    let context: Vec<(String, String)> = store::messages(&app.db, room)
        .await?
        .into_iter()
        .filter(|m| m.at > since || m.id == trigger.id)
        .map(|m| (m.author, m.text))
        .collect();
    let mut text = room_turn_text(&context, app.config.limits.room_context_max_chars as usize);
    // Rubato's own framing for a question asked aloud (ADR 0073): the answer is spoken.
    if trigger.spoken {
        text.push_str("\n\n");
        text.push_str(ASKED_ALOUD);
    }
    let mode = if building(app, room).await? {
        RoomTurnMode::Fork
    } else {
        RoomTurnMode::Session
    };
    let kind = SessionMessageKind::RoomTurn {
        room: room.clone(),
        turn_id,
        addressed_by: addressed_by.to_owned(),
        text,
        mode,
        fork_session_id: match mode {
            RoomTurnMode::Fork => agent.fork_session_id.clone(),
            RoomTurnMode::Session => None,
        },
    };
    // The owner lent the agent by calling it in or accepting the call: the turn is queued for
    // the owner's launch, so the companion's own-user check holds, and authored by whoever
    // addressed it.
    session_messages::enqueue(
        app,
        agent.agent.launch_id,
        kind,
        addressed_by,
        session_messages::CHAT_PRIORITY,
    )
    .await?;
    agents::addressed(&app.db, agent.id, trigger.at.max(since)).await?;
    tracing::info!(
        rubato.room = %room_attr(room),
        rubato.launch_id = %agent.agent.launch_id,
        turn_id = %turn_id,
        mode = ?mode,
        rubato.user = %addressed_by,
        messages = context.len(),
        "room turn queued"
    );
    Ok(())
}

/// Turns made by agents' messages since the last person posted, and whether the stop was said.
async fn agent_streak(app: &App, room: &RoomKey) -> Result<(usize, bool)> {
    let messages = store::messages(&app.db, room).await?;
    let start = messages
        .iter()
        .rposition(|m| m.author_kind == RoomAuthorKind::Person)
        .map_or(0, |i| i + 1);
    let tail = &messages[start..];
    let made = tail
        .iter()
        .filter(|m| m.author_kind == RoomAuthorKind::Agent)
        .map(|m| m.addressed.len())
        .sum();
    let said = tail
        .iter()
        .any(|m| m.author_kind == RoomAuthorKind::System && m.text.starts_with(TURN_LIMIT_TEXT));
    Ok((made, said))
}

fn system(text: String, turn_id: Option<Uuid>, at: chrono::DateTime<chrono::Utc>) -> RoomMessage {
    RoomMessage {
        id: Uuid::new_v4(),
        author_kind: RoomAuthorKind::System,
        author: SYSTEM_AUTHOR.into(),
        text,
        at,
        notice: None,
        addressed: vec![],
        turn_id,
        spoken: false,
    }
}

/// Post an agent's answer to turn `t`, and make the turns it addresses, up to the limit.
async fn post_reply(app: &App, t: &RoomTurnRef, text: &str) -> Result<bool> {
    let room = &t.room;
    let handle = agent_handle(&t.owner);
    super::callin::sweep_room(app, room).await?;
    let present: Vec<AgentRow> = agents::agents_in(&app.db, room)
        .await?
        .into_iter()
        .filter(|a| a.present() && !a.agent.handle.eq_ignore_ascii_case(&handle))
        .collect();
    let wanted = addressed_agents(text, &present);
    let (made, said) = agent_streak(app, room).await?;
    let limit = app.config.limits.room_agent_turn_limit as usize;
    let room_left = limit.saturating_sub(made);
    let (go, stop) = wanted.split_at(wanted.len().min(room_left));
    let now = app.clock.now();
    let m = RoomMessage {
        id: Uuid::new_v4(),
        author_kind: RoomAuthorKind::Agent,
        author: handle.clone(),
        text: text.to_owned(),
        at: now,
        notice: None,
        // Only the agents this message made a turn for.
        addressed: go
            .iter()
            .map(|a| a.agent.handle.to_ascii_lowercase())
            .collect(),
        turn_id: Some(t.turn_id),
        spoken: false,
    };
    if !store::insert(&app.db, room, &m).await? {
        tracing::debug!(turn_id = %t.turn_id, "room reply already posted");
        return Ok(false);
    }
    turns(app, "answered", 1);
    tracing::info!(rubato.room = %room_attr(room), turn_id = %t.turn_id, author = %handle, "room reply posted");
    if !go.is_empty() {
        let next = Uuid::new_v4();
        for a in go {
            dispatch_turn(app, room, a, &m, next, &handle).await?;
        }
    }
    if !stop.is_empty() {
        turns(app, "stopped", stop.len() as u64);
        if !said {
            app.metrics.rooms_turn_limit_hits.add(1, &[]);
            let notice = system(
                format!(
                    "{TURN_LIMIT_TEXT} {limit} turns in a row came from agents' messages. A person's message starts them again."
                ),
                None,
                now,
            );
            store::insert(&app.db, room, &notice).await?;
            tracing::warn!(rubato.room = %room_attr(room), limit, "room turn limit reached");
        }
    }
    notify(app, room);
    Ok(true)
}

/// A `Stop` from a launched session: when the prompt it ends carried a session-mode room turn,
/// its last reply is the agent's answer in the room.
pub async fn on_hook(app: &App, companion_id: Uuid, hook: &HookEvent) {
    if hook.hook_event_name != HookEventKind::Stop {
        return;
    }
    let Some(launch_id) = hook.launch_id else {
        return;
    };
    let payload = hook.payload.as_ref();
    let text = |k: &str| payload.and_then(|p| p.get(k)).and_then(|v| v.as_str());
    let (Some(prompt_id), Some(reply)) = (text("prompt_id"), text("last_assistant_message")) else {
        return;
    };
    let result = async {
        let Some(message_id) =
            session_log::prompt_message(&app.db, launch_id, &hook.session_id, prompt_id).await?
        else {
            return Ok(());
        };
        let Some(t) = session_messages::room_turn(&app.db, message_id).await? else {
            return Ok(());
        };
        if t.companion_id != companion_id
            || t.launch_id != launch_id
            || t.mode != RoomTurnMode::Session
        {
            return Ok(());
        }
        if reply.trim().is_empty() {
            return Ok(());
        }
        post_reply(app, &t, reply.trim()).await.map(|_| ())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %format!("{e:#}"), "room: posting a session reply failed");
    }
}

/// A fork turn's answer from the companion: posted as the agent's, its fork kept for the next
/// turn, its cost counted on the issue. Only from the companion the turn was sent to.
pub async fn on_room_reply(app: &App, companion_id: Uuid, r: &RoomReply) -> bool {
    let dropped = |why: &'static str| {
        app.metrics
            .events_dropped
            .add(1, &[KeyValue::new(attr::DROP_REASON, why)]);
    };
    let t = match session_messages::room_turn(&app.db, r.message_id).await {
        Ok(Some(t)) if t.turn_id == r.turn_id => t,
        Ok(_) => {
            dropped(refused::UNKNOWN_TURN);
            tracing::warn!(message_id = %r.message_id, turn_id = %r.turn_id, "room reply for a turn Rubato did not send");
            return false;
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "room reply: reading the turn failed");
            return false;
        }
    };
    if t.companion_id != companion_id {
        dropped(refused::FOREIGN_COMPANION);
        tracing::warn!(turn_id = %r.turn_id, %companion_id, "room reply from a companion the turn was not sent to");
        return false;
    }
    let result = async {
        let now = app.clock.now();
        if let Some(fork) = &r.fork_session_id {
            agents::set_fork_session(&app.db, &t.room, t.launch_id, fork).await?;
        }
        if let Some(usd) = r.cost_usd {
            let key = match &r.fork_session_id {
                Some(f) => format!("fork:{f}"),
                None => format!("fork-turn:{}", r.turn_id),
            };
            let issue = IssueRef::new(t.room.repo.clone(), t.room.number);
            spend_history::add_cost(&app.db, &app.metrics, companion_id, &key, &issue, usd, now)
                .await?;
        }
        let failed = r.error.clone().or_else(|| {
            r.text
                .trim()
                .is_empty()
                .then(|| "the turn gave no answer".to_owned())
        });
        if let Some(why) = failed {
            turns(app, "failed", 1);
            let m = system(
                format!("{} could not answer: {why}", agent_handle(&t.owner)),
                Some(t.turn_id),
                now,
            );
            store::insert(&app.db, &t.room, &m).await?;
            tracing::warn!(rubato.room = %room_attr(&t.room), turn_id = %t.turn_id, error = %why, "room fork turn failed");
            notify(app, &t.room);
            return Ok(true);
        }
        post_reply(app, &t, r.text.trim()).await
    }
    .await;
    match result {
        Ok(posted) => posted,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "room reply: recording failed");
            false
        }
    }
}

/// The handles a person's message makes turns for: every agent it names that is in.
pub(super) fn person_turns<'a>(text: &str, present: &'a [AgentRow]) -> Vec<&'a AgentRow> {
    let mut seen = HashSet::new();
    addressed_agents(text, present)
        .into_iter()
        .filter(|a| seen.insert(a.id))
        .collect()
}
