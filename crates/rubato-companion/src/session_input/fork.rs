//! Fork turns (ADR 0057, plan 122 §D8): a room turn while building runs in a read-only fork of
//! the launch's session (`kernel::claude`), never typed into it. One fork at a time per launch:
//! each launch has its own worker, so a slow fork holds back only that launch's turns, and chat
//! keeps flowing. The worker answers `delivered` when the fork starts, then sends the fork's
//! answer as `RoomReply` through the outbox.
//!
//! The spend cap counts the session's spend plus this companion's fork costs for the launch. A
//! turn that would start at or past the cap does not run: it is answered `failed`, its reply
//! says why, and a `CapHit` is sent. A fork whose cost passes the cap sends the `CapHit` after
//! it answers, and no further fork runs. The working session is not stopped for it.

use crate::kernel::claude::{Claude, KillReason, RunningTurn, Turn};
use crate::sessions::Registry;
use chrono::Utc;
use rubato_protocol::otel::{attr, event, span};
use rubato_protocol::room::RoomReply;
use rubato_protocol::{
    CapHit, CapKind, CompanionMessage, IssueRef, Limits, SessionMessage, SessionMessageKind,
    SessionMessageOutcome, SessionMessageResult,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::Instrument;
use uuid::Uuid;

/// One fork turn to run.
#[derive(Debug, Clone)]
pub struct ForkTurn {
    pub msg: SessionMessage,
    /// The launch's live session, forked on the first turn.
    pub session_id: String,
    pub worktree: PathBuf,
}

/// What every worker shares.
pub struct ForkCtx {
    pub claude: Claude,
    pub registry: Arc<Mutex<Registry>>,
    pub out: mpsc::UnboundedSender<CompanionMessage>,
    pub limits: tokio::sync::watch::Receiver<Limits>,
    /// Fork costs per launch so far, in USD.
    pub spent: std::sync::Mutex<HashMap<Uuid, f64>>,
    /// The fork turn running per launch, so a `stop` can kill its process group.
    pub running: std::sync::Mutex<HashMap<Uuid, RunningTurn>>,
}

/// The workers, one per launch that has had a fork turn.
pub struct Forks {
    ctx: Arc<ForkCtx>,
    workers: HashMap<Uuid, mpsc::UnboundedSender<ForkTurn>>,
}

impl Forks {
    pub fn new(ctx: ForkCtx) -> Self {
        Self {
            ctx: Arc::new(ctx),
            workers: HashMap::new(),
        }
    }

    /// Queue a turn on its launch's worker; it starts once the launch's earlier fork ended.
    pub fn submit(&mut self, turn: ForkTurn) {
        let launch = turn.msg.launch_id;
        let tx = self
            .workers
            .entry(launch)
            .or_insert_with(|| spawn(self.ctx.clone()));
        if let Err(mpsc::error::SendError(turn)) = tx.send(turn) {
            // The worker ended (it panicked): start a new one.
            let tx = spawn(self.ctx.clone());
            let _ = tx.send(turn);
            self.workers.insert(launch, tx);
        }
    }

    /// Kill the fork turn running for `launch_id`, if one runs (`stop`, ADR 0062). Turns queued
    /// behind it find the session ended and do not run. Returns whether one was killed.
    pub fn stop(&self, launch_id: Uuid) -> bool {
        match self.ctx.running.lock().unwrap().get_mut(&launch_id) {
            Some(t) => {
                t.kill(KillReason::Stop);
                true
            }
            None => false,
        }
    }
}

fn spawn(ctx: Arc<ForkCtx>) -> mpsc::UnboundedSender<ForkTurn> {
    let (tx, mut rx) = mpsc::unbounded_channel::<ForkTurn>();
    tokio::spawn(async move {
        while let Some(turn) = rx.recv().await {
            ctx.run(turn).await;
        }
    });
    tx
}

/// `text` cut to `max` characters.
fn cut(text: &str, max: u64) -> String {
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_owned(),
        None => text.to_owned(),
    }
}

impl ForkCtx {
    pub async fn run(&self, turn: ForkTurn) {
        let SessionMessageKind::RoomTurn {
            room,
            turn_id,
            text,
            fork_session_id,
            ..
        } = &turn.msg.kind
        else {
            return;
        };
        let launch = turn.msg.launch_id;
        let label = format!("{}#{}@{}", room.repo, room.number, room.branch);
        let span = tracing::info_span!(
            span::ROOM_FORK_TURN,
            rubato.room = %label,
            rubato.launch_id = %launch,
            rubato.session_message.id = %turn.msg.id,
            rubato.outcome = tracing::field::Empty,
            rubato.exit_code = tracing::field::Empty,
            rubato.cost_usd = tracing::field::Empty,
        );
        async {
            let (session_spend, cap, issue, live) = {
                let reg = self.registry.lock().await;
                match reg.by_launch(launch) {
                    Some(e) => (
                        e.spend_usd.unwrap_or(0.0),
                        e.spend_cap_usd,
                        e.issue.clone(),
                        e.is_live(),
                    ),
                    None => (0.0, None, None, false),
                }
            };
            // Queued behind a fork a `stop` ended: the session is gone, so nothing runs.
            if !live {
                tracing::Span::current().record(attr::OUTCOME, "stopped");
                self.result(
                    &turn.msg,
                    SessionMessageOutcome::Refused {
                        reason: rubato_protocol::MessageRefusal::UnknownSession,
                    },
                );
                return;
            }
            let issue = issue.unwrap_or_else(|| turn.msg.issue.clone());
            let spent = || {
                session_spend + self.spent.lock().unwrap().get(&launch).copied().unwrap_or(0.0)
            };
            if let Some(limit) = cap
                && spent() >= limit
            {
                tracing::Span::current().record(attr::OUTCOME, "stopped");
                self.cap_hit(&issue, &turn.session_id, spent(), limit);
                let error = format!("spend cap reached: {:.2} of {limit:.2} USD", spent());
                self.result(&turn.msg, SessionMessageOutcome::Failed { error: error.clone() });
                self.reply(&turn.msg, *turn_id, String::new(), fork_session_id.clone(), None, Some(error));
                return;
            }
            self.result(&turn.msg, SessionMessageOutcome::Delivered);
            let (resume, fork) = match fork_session_id {
                Some(f) => (f.clone(), false),
                None => (turn.session_id.clone(), true),
            };
            // On the shared spawn (`kernel::claude`): its own process group, bounded, killed on
            // every exit path, and held here so a `stop` can kill it.
            let t = match self.claude.start(&turn.worktree, &resume, fork, text) {
                Ok((running, end)) => {
                    self.running.lock().unwrap().insert(launch, running);
                    let end = end.await;
                    self.running.lock().unwrap().remove(&launch);
                    match end {
                        Ok(end) => Turn::from_end(end),
                        Err(_) => failed_turn("the fork turn's watcher ended".into()),
                    }
                }
                Err(e) => failed_turn(format!("{e:#}")),
            };
            let s = tracing::Span::current();
            if let Some(c) = t.exit_code {
                s.record(attr::EXIT_CODE, c);
            }
            if let Some(c) = t.cost_usd {
                s.record(attr::COST_USD, c);
                *self.spent.lock().unwrap().entry(launch).or_insert(0.0) += c;
            }
            s.record(attr::OUTCOME, if t.error.is_none() { "answered" } else { "failed" });
            match &t.error {
                None => tracing::info!(rubato.room = %label, cost_usd = ?t.cost_usd, fork_session_id = ?t.session_id, fork, "room fork turn answered"),
                Some(e) => tracing::warn!(rubato.room = %label, error = %e, exit_code = ?t.exit_code, "room fork turn failed"),
            }
            let max = self.limits.borrow().session_log_max_chars;
            let fork_session = t.session_id.clone().or_else(|| fork_session_id.clone());
            self.reply(&turn.msg, *turn_id, cut(&t.text, max), fork_session, t.cost_usd, t.error.clone());
            if let Some(limit) = cap
                && spent() > limit
            {
                self.cap_hit(&issue, &turn.session_id, spent(), limit);
            }
        }
        .instrument(span)
        .await
    }

    fn result(&self, msg: &SessionMessage, outcome: SessionMessageOutcome) {
        let _ = self.out.send(CompanionMessage::SessionMessageResult(
            SessionMessageResult {
                message_id: msg.id,
                outcome,
                at: Utc::now(),
            },
        ));
    }

    fn reply(
        &self,
        msg: &SessionMessage,
        turn_id: Uuid,
        text: String,
        fork_session_id: Option<String>,
        cost_usd: Option<f64>,
        error: Option<String>,
    ) {
        let _ = self.out.send(CompanionMessage::RoomReply(RoomReply {
            message_id: msg.id,
            turn_id,
            text,
            fork_session_id,
            cost_usd,
            error,
            at: Utc::now(),
        }));
    }

    /// As the spend slice does: an event, not a log line.
    fn cap_hit(&self, issue: &IssueRef, session_id: &str, value: f64, limit: f64) {
        tracing::warn!(issue = %issue, spend = value, limit, source = "room_fork", "{}", event::CAP_HIT);
        let _ = self.out.send(CompanionMessage::CapHit(CapHit {
            issue: issue.clone(),
            session_id: Some(session_id.to_owned()),
            kind: CapKind::Spend,
            value,
            limit,
            at: Utc::now(),
        }));
    }
}

fn failed_turn(error: String) -> Turn {
    Turn {
        text: String::new(),
        session_id: None,
        cost_usd: None,
        exit_code: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::telemetry::capture;
    use crate::sessions::LaunchRecord;
    use rubato_protocol::RoomTurnMode;
    use rubato_protocol::room::RoomKey;
    use std::path::Path;

    /// A stand-in `claude` that counts its runs and answers with `cost`.
    fn standin(dir: &Path, cost: f64) -> Claude {
        let script = dir.join("claude");
        let body = format!(
            "#!/bin/sh\necho \"$*\" >> '{d}/runs'\ncat > /dev/null\necho '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fork-1\"}}'\necho '{{\"type\":\"result\",\"is_error\":false,\"result\":\"b keeps the seam\",\"session_id\":\"fork-1\",\"total_cost_usd\":{cost}}}'\n",
            d = dir.display()
        );
        let ok = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg(&body)
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        Claude {
            command: script.to_string_lossy().into_owned(),
            timeout: std::time::Duration::from_secs(10),
        }
    }

    fn ctx(
        dir: &Path,
        cost: f64,
        cap: f64,
    ) -> (ForkCtx, mpsc::UnboundedReceiver<CompanionMessage>) {
        let mut reg = Registry::new(Uuid::nil(), "m0x6A");
        reg.restore_launches(
            vec![LaunchRecord {
                launch_id: Uuid::from_u128(7),
                session_id: "s7".into(),
                worktree: dir.to_path_buf(),
                pane_id: None,
                tmux_session: None,
                issue: IssueRef::new("acme/pantry", 7),
                branch: Some("issue/7-restock".into()),
                spend_cap_usd: Some(cap),
                plan_path: None,
                assignment: None,
            }],
            None,
            false,
            Utc::now(),
        );
        let (out, rx) = mpsc::unbounded_channel();
        (
            ForkCtx {
                claude: standin(dir, cost),
                registry: Arc::new(Mutex::new(reg)),
                out,
                limits: tokio::sync::watch::channel(Limits::default()).1,
                spent: Default::default(),
                running: Default::default(),
            },
            rx,
        )
    }

    fn turn(dir: &Path, id: u128, fork_session_id: Option<&str>) -> ForkTurn {
        ForkTurn {
            msg: SessionMessage {
                id: Uuid::from_u128(id),
                user: "m0x6A".into(),
                launch_id: Uuid::from_u128(7),
                session_id: Some("s7".into()),
                issue: IssueRef::new("acme/pantry", 7),
                kind: SessionMessageKind::RoomTurn {
                    room: RoomKey {
                        repo: "acme/pantry".into(),
                        number: 7,
                        branch: "issue/7-restock".into(),
                    },
                    turn_id: Uuid::from_u128(100 + id),
                    addressed_by: "alice".into(),
                    text: "alice: why b?".into(),
                    mode: RoomTurnMode::Fork,
                    fork_session_id: fork_session_id.map(str::to_owned),
                },
                created_at: Utc::now(),
            },
            session_id: "s7".into(),
            worktree: dir.to_path_buf(),
        }
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<CompanionMessage>) -> Vec<CompanionMessage> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_fork_turn_answers_delivered_then_replies_with_its_cost_in_a_span() {
        let (spans, _g) = capture::spans();
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = ctx(dir.path(), 0.1, 20.0);
        ctx.run(turn(dir.path(), 1, None)).await;
        let got = drain(&mut rx);
        assert!(
            matches!(&got[0], CompanionMessage::SessionMessageResult(r) if r.outcome == SessionMessageOutcome::Delivered),
            "{got:?}"
        );
        match &got[1] {
            CompanionMessage::RoomReply(r) => {
                assert_eq!(r.message_id, Uuid::from_u128(1));
                assert_eq!(r.turn_id, Uuid::from_u128(101));
                assert_eq!(r.text, "b keeps the seam");
                assert_eq!(r.fork_session_id.as_deref(), Some("fork-1"));
                assert_eq!((r.cost_usd, r.error.as_deref()), (Some(0.1), None));
            }
            other => panic!("expected a room reply, got {other:?}"),
        }
        assert_eq!(got.len(), 2, "no cap hit under the cap: {got:?}");
        let runs = std::fs::read_to_string(dir.path().join("runs")).unwrap();
        assert!(runs.starts_with("-p --resume s7 --fork-session"), "{runs}");
        let spans = spans.lock().unwrap();
        let s = spans
            .iter()
            .find(|s| s.message == span::ROOM_FORK_TURN)
            .expect("a fork turn span");
        assert_eq!(
            s.fields.get(attr::ROOM).map(String::as_str),
            Some("acme/pantry#7@issue/7-restock")
        );
        assert_eq!(
            s.fields.get(attr::COST_USD).map(String::as_str),
            Some("0.1")
        );
        assert_eq!(s.fields.get(attr::EXIT_CODE).map(String::as_str), Some("0"));
        assert_eq!(
            s.fields.get(attr::OUTCOME).map(String::as_str),
            Some("answered")
        );
    }

    /// A fork that passes the cap sends a `CapHit`; the next turn does not run at all.
    #[tokio::test(flavor = "current_thread")]
    async fn the_cap_stops_fork_turns() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, mut rx) = ctx(dir.path(), 0.2, 0.15);
        ctx.run(turn(dir.path(), 1, None)).await;
        let got = drain(&mut rx);
        assert!(matches!(&got[1], CompanionMessage::RoomReply(r) if r.cost_usd == Some(0.2)));
        match &got[2] {
            CompanionMessage::CapHit(h) => {
                assert_eq!((h.kind, h.value, h.limit), (CapKind::Spend, 0.2, 0.15));
                assert_eq!(h.session_id.as_deref(), Some("s7"));
            }
            other => panic!("expected a cap hit, got {other:?}"),
        }
        ctx.run(turn(dir.path(), 2, Some("fork-1"))).await;
        let got = drain(&mut rx);
        assert!(matches!(&got[0], CompanionMessage::CapHit(_)), "{got:?}");
        assert!(
            matches!(&got[1], CompanionMessage::SessionMessageResult(r) if matches!(&r.outcome, SessionMessageOutcome::Failed { error } if error.contains("spend cap"))),
            "{got:?}"
        );
        assert!(
            matches!(&got[2], CompanionMessage::RoomReply(r) if r.error.as_deref().is_some_and(|e| e.contains("spend cap"))),
            "{got:?}"
        );
        let runs = std::fs::read_to_string(dir.path().join("runs")).unwrap();
        assert_eq!(
            runs.lines().count(),
            1,
            "the second turn did not run: {runs}"
        );
    }

    /// `stop` (ADR 0062) kills a running fork turn's group too: the turn ends `killed: stop` and
    /// says so in its reply.
    #[tokio::test(flavor = "current_thread")]
    async fn stop_kills_a_running_fork_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (mut ctx, mut rx) = ctx(dir.path(), 0.1, 20.0);
        let script = dir.path().join("slow-claude");
        let ok = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"")
            .arg("sh")
            .arg("#!/bin/sh\ncat > /dev/null\necho '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fork-1\"}'\nsleep 30\n")
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        ctx.claude.command = script.to_string_lossy().into_owned();
        let mut forks = Forks::new(ctx);
        assert!(!forks.stop(Uuid::from_u128(7)), "nothing runs yet");
        forks.submit(turn(dir.path(), 1, None));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !forks
            .ctx
            .running
            .lock()
            .unwrap()
            .contains_key(&Uuid::from_u128(7))
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the fork never started"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(forks.stop(Uuid::from_u128(7)));
        let reply = loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(CompanionMessage::RoomReply(r))) => break r,
                Ok(Some(_)) => continue,
                other => panic!("no room reply after the stop: {other:?}"),
            }
        };
        assert_eq!(reply.error.as_deref(), Some("killed: stop"));
        assert!(reply.text.is_empty());
    }
}
